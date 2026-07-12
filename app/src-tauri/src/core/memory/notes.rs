//! User notes (appunti): durable, encrypted, recalled and reorganized by Liara into
//! tables / charts / HTML on request. Topic + body are encrypted at rest.
use super::{now, Memory};
use anyhow::Result;
use rusqlite::params;

/// One note: id, topic, body (both decrypted).
pub type Note = (i64, String, String);

impl Memory {
    pub fn add_note(&self, topic: &str, text: &str) -> Result<i64> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO notes (topic, text, created_at) VALUES (?1, ?2, ?3)",
            params![self.crypto.encrypt(topic)?, self.crypto.encrypt(text)?, now()],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// All notes (newest first), optionally filtered to a topic (case-insensitive contains).
    pub fn list_notes(&self, topic: Option<&str>) -> Result<Vec<Note>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare("SELECT id, topic, text FROM notes ORDER BY id DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?;
        let needle = topic.map(|t| t.to_lowercase());
        Ok(rows
            .filter_map(|r| r.ok())
            .map(|(id, tp, tx)| (id, self.crypto.decrypt(&tp), self.crypto.decrypt(&tx)))
            .filter(|(_, tp, _)| match &needle {
                Some(n) => tp.to_lowercase().contains(n),
                None => true,
            })
            .collect())
    }

    /// Notes whose topic or body contains `query` (case-insensitive).
    pub fn search_notes(&self, query: &str, limit: usize) -> Result<Vec<Note>> {
        let q = query.to_lowercase();
        Ok(self
            .list_notes(None)?
            .into_iter()
            .filter(|(_, tp, tx)| tp.to_lowercase().contains(&q) || tx.to_lowercase().contains(&q))
            .take(limit)
            .collect())
    }

    pub fn delete_note(&self, id: i64) -> Result<()> {
        self.conn.lock().unwrap().execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::crypto::Crypto;
    use std::sync::Arc;

    fn mem() -> Memory {
        Memory::open(":memory:", Arc::new(Crypto::from_key(&[5u8; 32]))).unwrap()
    }

    #[test]
    fn add_list_newest_first() {
        let m = mem();
        m.add_note("Storia", "Rivoluzione francese 1789").unwrap();
        m.add_note("Mate", "Pitagora a^2+b^2=c^2").unwrap();
        let all = m.list_notes(None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].1, "Mate"); // newest first
    }

    #[test]
    fn filter_by_topic() {
        let m = mem();
        m.add_note("Storia", "x").unwrap();
        m.add_note("Mate", "y").unwrap();
        let hist = m.list_notes(Some("storia")).unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].2, "x");
    }

    #[test]
    fn search_matches_topic_or_body() {
        let m = mem();
        m.add_note("Storia", "Rivoluzione francese").unwrap();
        m.add_note("Mate", "teorema di Pitagora").unwrap();
        assert_eq!(m.search_notes("pitagora", 10).unwrap().len(), 1);
        assert_eq!(m.search_notes("francese", 10).unwrap().len(), 1);
    }

    #[test]
    fn notes_encrypted_at_rest() {
        let m = mem();
        let id = m.add_note("Segreto", "contenuto riservato").unwrap();
        let (tp, tx): (String, String) = m
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT topic, text FROM notes WHERE id=?1", params![id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert!(tp.starts_with("enc:v1:") && tx.starts_with("enc:v1:"));
        assert_ne!(tp, "Segreto");
    }

    #[test]
    fn delete_removes() {
        let m = mem();
        let id = m.add_note("T", "x").unwrap();
        m.delete_note(id).unwrap();
        assert!(m.list_notes(None).unwrap().is_empty());
    }
}
