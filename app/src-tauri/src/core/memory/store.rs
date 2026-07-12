//! v1 store: DB open/migration + structured profile, facts, episodes and conversations.
use super::{now, Memory, ENC};
use crate::core::crypto::Crypto;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

impl Memory {
    pub fn open(path: &str, crypto: Arc<Crypto>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS profile (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS facts (
                id INTEGER PRIMARY KEY,
                text TEXT UNIQUE NOT NULL,
                created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS episodes (
                id INTEGER PRIMARY KEY,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                data TEXT NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );
             -- Memory v2: semantic, temporal store. text encrypted; embedding = normalized f32 BLOB.
             -- valid_until NULL = currently true; set when a newer memory supersedes it (temporal KG).
             CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY,
                kind TEXT NOT NULL,
                text TEXT NOT NULL,
                embedding BLOB NOT NULL,
                importance REAL NOT NULL DEFAULT 0.5,
                created_at INTEGER NOT NULL,
                valid_until INTEGER
             );
             -- User notes (appunti): topic + body, encrypted at rest. Liara recalls and
             -- reorganizes them into tables/charts/HTML on request.
             CREATE TABLE IF NOT EXISTS notes (
                id INTEGER PRIMARY KEY,
                topic TEXT NOT NULL,
                text TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );",
        )?;
        let m = Self { conn: Mutex::new(conn), crypto, index: Mutex::new(Vec::new()) };
        m.migrate_encrypt()?;
        m.load_index(); // decrypt the vector store into RAM once
        Ok(m)
    }

    /// One-time: encrypt any legacy plaintext rows so everything-at-rest is ciphertext.
    fn migrate_encrypt(&self) -> Result<()> {
        let c = self.conn.lock().unwrap();
        // rows keyed by an INTEGER id (facts, episodes)
        let by_int = |sql: &str| -> Vec<(i64, String)> {
            c.prepare(sql)
                .and_then(|mut s| {
                    s.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                        .map(|rows| rows.filter_map(|x| x.ok()).collect())
                })
                .unwrap_or_default()
        };
        // rows keyed by a TEXT key/id (profile, conversations)
        let by_text = |sql: &str| -> Vec<(String, String)> {
            c.prepare(sql)
                .and_then(|mut s| {
                    s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                        .map(|rows| rows.filter_map(|x| x.ok()).collect())
                })
                .unwrap_or_default()
        };

        for (k, v) in by_text("SELECT key, value FROM profile") {
            if !v.starts_with(ENC) {
                c.execute("UPDATE profile SET value=?1 WHERE key=?2", params![self.crypto.encrypt(&v)?, k])?;
            }
        }
        for (id, t) in by_int("SELECT id, text FROM facts") {
            if !t.starts_with(ENC) {
                c.execute("UPDATE facts SET text=?1 WHERE id=?2", params![self.crypto.encrypt(&t)?, id])?;
            }
        }
        for (id, ct) in by_int("SELECT id, content FROM episodes") {
            if !ct.starts_with(ENC) {
                c.execute("UPDATE episodes SET content=?1 WHERE id=?2", params![self.crypto.encrypt(&ct)?, id])?;
            }
        }
        for (id, title) in by_text("SELECT id, title FROM conversations") {
            if !title.starts_with(ENC) {
                c.execute("UPDATE conversations SET title=?1 WHERE id=?2", params![self.crypto.encrypt(&title)?, id])?;
            }
        }
        for (id, data) in by_text("SELECT id, data FROM conversations") {
            if !data.starts_with(ENC) {
                c.execute("UPDATE conversations SET data=?1 WHERE id=?2", params![self.crypto.encrypt(&data)?, id])?;
            }
        }
        Ok(())
    }

    // --- structured profile (user-provided, all optional) ---

    /// Upsert a profile field by its human label. Empty value removes it.
    pub fn set_profile(&self, key: &str, value: &str) -> Result<()> {
        let c = self.conn.lock().unwrap();
        let v = value.trim();
        if v.is_empty() {
            c.execute("DELETE FROM profile WHERE key = ?1", params![key])?;
        } else {
            c.execute(
                "INSERT INTO profile (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
                params![key, self.crypto.encrypt(v)?, now()],
            )?;
        }
        Ok(())
    }

    pub fn profile_entries(&self) -> Result<Vec<(String, String)>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare("SELECT key, value FROM profile ORDER BY updated_at")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows
            .filter_map(|r| r.ok())
            .map(|(k, v)| (k, self.crypto.decrypt(&v)))
            .collect())
    }

    // --- episodic + auto-extracted facts ---

    pub fn add_episode(&self, role: &str, content: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO episodes (role, content, created_at) VALUES (?1, ?2, ?3)",
            params![role, self.crypto.encrypt(content)?, now()],
        )?;
        Ok(())
    }

    // (facts: add/list/delete/forget live in memory/facts.rs)

    // --- conversations (the chat trees, persisted) ---

    pub fn save_conversation(&self, id: &str, title: &str, data: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO conversations (id, title, data, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET title = ?2, data = ?3, updated_at = ?4",
            params![id, self.crypto.encrypt(title)?, self.crypto.encrypt(data)?, now()],
        )?;
        Ok(())
    }

    pub fn list_conversations(&self) -> Result<Vec<(String, String, i64)>> {
        let c = self.conn.lock().unwrap();
        let mut stmt =
            c.prepare("SELECT id, title, updated_at FROM conversations ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })?;
        Ok(rows
            .filter_map(|r| r.ok())
            .map(|(id, title, ts)| (id, self.crypto.decrypt(&title), ts))
            .collect())
    }

    pub fn load_conversation(&self, id: &str) -> Result<Option<String>> {
        let c = self.conn.lock().unwrap();
        match c.query_row(
            "SELECT data FROM conversations WHERE id = ?1",
            params![id],
            |r| r.get::<_, String>(0),
        ) {
            Ok(d) => Ok(Some(self.crypto.decrypt(&d))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// The "what I know about you" block injected into the system prompt.
    pub fn profile_block(&self) -> String {
        let prof = self.profile_entries().unwrap_or_default();
        let facts = self.facts().unwrap_or_default();
        if prof.is_empty() && facts.is_empty() {
            return String::new();
        }
        let mut s = String::from(
            "\n\nMemoria persistente sull'utente (conoscila e usala con naturalezza, non elencarla):\n",
        );
        if !prof.is_empty() {
            s.push_str("Profilo:\n");
            for (k, v) in prof {
                s.push_str("- ");
                s.push_str(&k);
                s.push_str(": ");
                s.push_str(&v);
                s.push('\n');
            }
        }
        if !facts.is_empty() {
            s.push_str("Altri dettagli appresi:\n");
            for f in facts {
                s.push_str("- ");
                s.push_str(&f);
                s.push('\n');
            }
        }
        s
    }
}
