//! Durable facts about the user (auto-learned or added). Encrypted at rest; dedup/delete
//! operate on the DECRYPTED text since random-nonce ciphertext can't match at the SQL level.
use super::{now, Memory};
use anyhow::Result;
use rusqlite::params;

impl Memory {
    /// Insert a fact; returns true if it was new (dedup on decrypted text).
    pub fn add_fact(&self, text: &str) -> Result<bool> {
        if self.facts()?.iter().any(|f| f == text) {
            return Ok(false);
        }
        self.conn.lock().unwrap().execute(
            "INSERT INTO facts (text, created_at) VALUES (?1, ?2)",
            params![self.crypto.encrypt(text)?, now()],
        )?;
        Ok(true)
    }

    pub fn facts(&self) -> Result<Vec<String>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare("SELECT text FROM facts ORDER BY created_at")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).map(|t| self.crypto.decrypt(&t)).collect())
    }

    /// Delete a single fact by its (decrypted) text.
    pub fn delete_fact(&self, text: &str) -> Result<()> {
        {
            let c = self.conn.lock().unwrap();
            let mut stmt = c.prepare("SELECT id, text FROM facts")?;
            let rows: Vec<(i64, String)> =
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|x| x.ok()).collect();
            drop(stmt);
            for (id, enc) in rows {
                if self.crypto.decrypt(&enc) == text {
                    c.execute("DELETE FROM facts WHERE id = ?1", params![id])?;
                    break;
                }
            }
        }
        // ALSO remove the vector copy (kind='fact'), else recall keeps surfacing the "deleted" fact.
        let ids: Vec<i64> = self
            .index
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.kind == "fact" && e.text == text)
            .map(|e| e.id)
            .collect();
        {
            let c = self.conn.lock().unwrap();
            for id in &ids {
                c.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
            }
        }
        self.index.lock().unwrap().retain(|e| !ids.contains(&e.id));
        Ok(())
    }

    /// Clears ALL auto-learned memory: facts + episodes + reflections, in BOTH the legacy tables
    /// and the vector store (+ RAM index). Leaves the user-provided profile and ingested docs.
    pub fn forget_all(&self) -> Result<()> {
        self.conn.lock().unwrap().execute_batch(
            "DELETE FROM facts; DELETE FROM episodes;
             DELETE FROM memories WHERE kind IN ('fact','episode','reflection');",
        )?;
        self.index.lock().unwrap().retain(|e| e.kind == "doc");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::crypto::Crypto;
    use std::sync::Arc;

    fn mem() -> Memory {
        Memory::open(":memory:", Arc::new(Crypto::from_key(&[3u8; 32]))).unwrap()
    }

    #[test]
    fn delete_fact_removes_only_that_one() {
        let m = mem();
        m.add_fact("ama il caffè").unwrap();
        m.add_fact("vive a Roma").unwrap();
        m.delete_fact("ama il caffè").unwrap();
        let facts = m.facts().unwrap();
        assert_eq!(facts, vec!["vive a Roma".to_string()]);
    }

    #[test]
    fn delete_fact_also_clears_vector_copy() {
        // La copia vettoriale dei FACT si osserva via most_similar_fact (la recall
        // li esclude by design, audit #26: già iniettati da profile_block).
        let m = mem();
        m.remember("fact", "vive a Roma", &[1.0, 0.0], 0.7).unwrap(); // vector copy in `memories`
        m.add_fact("vive a Roma").unwrap();
        assert!(m.most_similar_fact(&[1.0, 0.0]).unwrap().1.contains("Roma"),
                "pre-condizione: il vettore del fatto deve esistere");
        m.delete_fact("vive a Roma").unwrap();
        assert!(m.most_similar_fact(&[1.0, 0.0]).map_or(true, |(_, t, _)| !t.contains("Roma")),
                "il vettore deve sparire anche dalla copia in `memories`");
    }

    #[test]
    fn forget_all_clears_vectors_but_keeps_docs() {
        let m = mem();
        m.remember("fact", "x", &[1.0, 0.0], 0.7).unwrap();
        m.remember("episode", "ep", &[1.0, 0.0], 0.3).unwrap();
        m.remember("doc", "[doc:manuale] capitolo", &[0.0, 1.0], 0.6).unwrap();
        assert!(!m.recall(&[1.0, 0.0], 5).is_empty(), "pre-condizione: l'episodio è richiamabile");
        m.forget_all().unwrap();
        assert!(m.recall(&[1.0, 0.0], 5).is_empty());
        assert!(m.most_similar_fact(&[1.0, 0.0]).is_none(), "anche i vettori dei fact vanno svuotati");
        assert_eq!(m.recall_docs(&[0.0, 1.0], 5).len(), 1); // docs preserved
    }

    #[test]
    fn delete_missing_fact_is_noop() {
        let m = mem();
        m.add_fact("x").unwrap();
        m.delete_fact("inesistente").unwrap();
        assert_eq!(m.facts().unwrap().len(), 1);
    }
}
