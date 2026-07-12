//! Memory v2: semantic + temporal vector store. The vectors live DECRYPTED in a RAM index
//! (loaded once at open); recall is an in-memory cosine, so it never scans+decrypts the whole
//! DB every turn. Writes go to both the index (RAM) and the DB (encrypted at rest).
use super::{now, Memory, VecEntry};
use anyhow::Result;
use rusqlite::params;
use std::cmp::Ordering;

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}
fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// Recall scoring weights (named, not magic): score = sim · imp_weight · recency_weight.
const RECENCY_HALFLIFE_DAYS: f32 = 45.0; // recency decays to 1/2 after this many days
const IMP_FLOOR: f32 = 0.6; // importance influence floor (1.0 - floor is its swing)
const REC_FLOOR: f32 = 0.7; // recency influence floor

impl Memory {
    /// Load the encrypted vector store into the in-RAM index (called once, at open).
    pub(super) fn load_index(&self) {
        let entries: Vec<VecEntry> = {
            let c = self.conn.lock().unwrap();
            let Ok(mut stmt) = c.prepare(
                "SELECT id, kind, text, embedding, importance, created_at, valid_until FROM memories",
            ) else {
                return;
            };
            let mapped = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, f64>(4)? as f32,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                ))
            });
            let Ok(mapped) = mapped else { return };
            mapped
                .filter_map(|x| x.ok())
                .map(|(id, kind, enc, blob, imp, created, vu)| VecEntry {
                    id,
                    kind,
                    emb: blob_to_vec(&self.crypto.decrypt_blob(&blob)),
                    text: self.crypto.decrypt(&enc),
                    importance: imp,
                    created_at: created,
                    valid: vu.is_none(),
                })
                .collect()
        };
        *self.index.lock().unwrap() = entries;
    }

    /// Store a semantic memory (text AND embedding encrypted at rest; mirrored in the RAM index).
    pub fn remember(&self, kind: &str, text: &str, embedding: &[f32], importance: f32) -> Result<i64> {
        let ts = now();
        let id = {
            let c = self.conn.lock().unwrap();
            c.execute(
                "INSERT INTO memories (kind, text, embedding, importance, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![kind, self.crypto.encrypt(text)?, self.crypto.encrypt_blob(&vec_to_blob(embedding))?, importance as f64, ts],
            )?;
            c.last_insert_rowid()
        };
        self.index.lock().unwrap().push(VecEntry {
            id,
            kind: kind.to_string(),
            emb: embedding.to_vec(),
            text: text.to_string(),
            importance,
            created_at: ts,
            valid: true,
        });
        Ok(id)
    }

    /// Semantic recall of PERSONAL memory (facts/episodes/reflections) — excludes RAG docs,
    /// so an ingested document can't crowd the user's own memories out of the top-k.
    pub fn recall(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        // #26 FIX: escludiamo kind=="fact" — i fatti sono GIÀ iniettati nel system da profile_block ogni
        // turno. Includerli anche nella recall = doppia iniezione (spreco token + over-weight). Restano
        // reflection ed episode, che profile_block NON inietta.
        self.recall_where(query, k, |kind| kind != "doc" && kind != "fact")
    }

    /// Semantic recall restricted to ingested document chunks (RAG namespace).
    pub fn recall_docs(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        self.recall_where(query, k, |kind| kind == "doc")
    }

    fn recall_where(&self, query: &[f32], k: usize, pred: impl Fn(&str) -> bool) -> Vec<(String, f32)> {
        let now_ts = now();
        let idx = self.index.lock().unwrap();
        let mut scored: Vec<(String, f32)> = idx
            .iter()
            .filter(|e| e.valid && e.emb.len() == query.len() && pred(&e.kind))
            .map(|e| {
                let sim = cosine(query, &e.emb);
                let age_days = (now_ts - e.created_at).max(0) as f32 / 86_400.0;
                let recency = 1.0 / (1.0 + age_days / RECENCY_HALFLIFE_DAYS);
                let score = sim
                    * (IMP_FLOOR + (1.0 - IMP_FLOOR) * e.importance.clamp(0.0, 1.0))
                    * (REC_FLOOR + (1.0 - REC_FLOOR) * recency);
                (e.text.clone(), score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        scored.truncate(k);
        scored
    }

    /// Most semantically similar CURRENT fact to an embedding → (id, text, similarity).
    /// Used to detect contradictions (supersession): same topic, changed info.
    pub fn most_similar_fact(&self, query: &[f32]) -> Option<(i64, String, f32)> {
        self.index
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.valid && e.kind == "fact" && e.emb.len() == query.len())
            .map(|e| (e.id, e.text.clone(), cosine(query, &e.emb)))
            .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(Ordering::Equal))
    }

    /// Recent episodic memory texts (for the reflection pass), newest first.
    pub fn recent_episode_texts(&self, limit: i64) -> Vec<String> {
        let idx = self.index.lock().unwrap();
        let mut eps: Vec<(i64, String)> = idx
            .iter()
            .filter(|e| e.valid && e.kind == "episode")
            .map(|e| (e.id, e.text.clone()))
            .collect();
        eps.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        eps.into_iter().take(limit.max(0) as usize).map(|(_, t)| t).collect()
    }

    pub fn prune_episodes(&self, keep: i64) -> Result<()> {
        self.prune_kind("episode", keep)
    }

    pub fn prune_reflections(&self, keep: i64) -> Result<()> {
        self.prune_kind("reflection", keep)
    }

    /// Cap a memory kind to its `keep` most-recent rows (bounds unlimited growth + recall cost),
    /// in both the DB and the RAM index. Facts/docs (user data) are never auto-pruned.
    fn prune_kind(&self, kind: &str, keep: i64) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "DELETE FROM memories WHERE kind = ?1 AND id NOT IN
             (SELECT id FROM memories WHERE kind = ?1 ORDER BY id DESC LIMIT ?2)",
            params![kind, keep],
        )?;
        let mut idx = self.index.lock().unwrap();
        let mut ids: Vec<i64> = idx.iter().filter(|e| e.kind == kind).map(|e| e.id).collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        let keepset: std::collections::HashSet<i64> = ids.into_iter().take(keep.max(0) as usize).collect();
        idx.retain(|e| e.kind != kind || keepset.contains(&e.id));
        Ok(())
    }

    /// Increment and return the turn counter (drives periodic reflection).
    pub fn bump_turn(&self) -> i64 {
        let c = self.conn.lock().unwrap();
        let cur: i64 = c
            .query_row("SELECT value FROM settings WHERE key='turn_count'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let next = cur + 1;
        let _ = c.execute(
            "INSERT INTO settings (key, value) VALUES ('turn_count', ?1) ON CONFLICT(key) DO UPDATE SET value=?1",
            params![next.to_string()],
        );
        next
    }

    /// Temporal supersession: mark a memory as no longer current (kept for history).
    pub fn supersede(&self, id: i64) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE memories SET valid_until = ?1 WHERE id = ?2", params![now(), id])?;
        if let Some(e) = self.index.lock().unwrap().iter_mut().find(|e| e.id == id) {
            e.valid = false;
        }
        Ok(())
    }

    pub fn memory_count(&self) -> i64 {
        self.index.lock().unwrap().iter().filter(|e| e.valid).count() as i64
    }
}
