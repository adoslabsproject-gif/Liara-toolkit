//! Email row CRUD: store, list, get, delete, recent and search (decrypt + filter in memory).
use super::{now, EmailFull, EmailStore, EmailSummary};
use anyhow::Result;
use rusqlite::params;

impl EmailStore {
    pub fn store_email(&self, uid: u32, folder: &str, sender: &str, subject: &str, body: &str, date: &str) -> Result<bool> {
        let n = self.conn.lock().unwrap().execute(
            "INSERT OR IGNORE INTO emails (uid, folder, sender, subject, body, date, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![uid, folder, self.crypto.encrypt(sender)?, self.crypto.encrypt(subject)?, self.crypto.encrypt(body)?, date, now()],
        )?;
        Ok(n > 0)
    }

    pub fn list(&self) -> Result<Vec<EmailSummary>> {
        self.list_in("INBOX")
    }

    pub fn list_in(&self, folder: &str) -> Result<Vec<EmailSummary>> {
        self.summaries("WHERE folder = ?1 AND deleted = 0", params![folder])
    }

    /// Soft-deleted emails (the Trash folder), recoverable until purged.
    pub fn list_trash(&self) -> Result<Vec<EmailSummary>> {
        self.summaries("WHERE deleted = 1", params![])
    }

    fn summaries(&self, where_clause: &str, p: impl rusqlite::Params) -> Result<Vec<EmailSummary>> {
        let c = self.conn.lock().unwrap();
        let sql = format!("SELECT id, sender, subject, date, seen FROM emails {where_clause} ORDER BY id DESC LIMIT 300");
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(p, |r| {
            Ok(EmailSummary {
                id: r.get(0)?,
                sender: self.crypto.decrypt(&r.get::<_, String>(1)?),
                subject: self.crypto.decrypt(&r.get::<_, String>(2)?),
                date: r.get(3)?,
                seen: r.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get(&self, id: i64) -> Result<Option<EmailFull>> {
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE emails SET seen = 1 WHERE id = ?1", params![id]).ok();
        match c.query_row(
            "SELECT id, sender, subject, date, body FROM emails WHERE id = ?1",
            params![id],
            |r| {
                Ok(EmailFull {
                    id: r.get(0)?,
                    sender: self.crypto.decrypt(&r.get::<_, String>(1)?),
                    subject: self.crypto.decrypt(&r.get::<_, String>(2)?),
                    date: r.get(3)?,
                    body: self.crypto.decrypt(&r.get::<_, String>(4)?),
                })
            },
        ) {
            Ok(e) => Ok(Some(e)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Soft-delete: move to Trash (recoverable). Use `purge` to delete permanently.
    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE emails SET deleted = 1 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn restore(&self, id: i64) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE emails SET deleted = 0 WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Permanently delete one email, or (id = 0) empty the whole Trash.
    pub fn purge(&self, id: i64) -> Result<()> {
        let c = self.conn.lock().unwrap();
        if id == 0 {
            c.execute("DELETE FROM emails WHERE deleted = 1", [])?;
        } else {
            c.execute("DELETE FROM emails WHERE id = ?1", params![id])?;
        }
        Ok(())
    }

    /// Most recent emails in a folder (with body) — for the agent tools.
    pub fn recent_in(&self, folder: &str, limit: usize) -> Result<Vec<EmailFull>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare(
            "SELECT id, sender, subject, date, body FROM emails WHERE folder = ?1 AND deleted = 0 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![folder, limit as i64], |r| {
            Ok(EmailFull {
                id: r.get(0)?,
                sender: self.crypto.decrypt(&r.get::<_, String>(1)?),
                subject: self.crypto.decrypt(&r.get::<_, String>(2)?),
                date: r.get(3)?,
                body: self.crypto.decrypt(&r.get::<_, String>(4)?),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<EmailFull>> {
        self.recent_in("INBOX", limit)
    }

    /// Search emails by sender/subject/body. Content is encrypted, so we decrypt and
    /// filter in memory (SQL LIKE can't match ciphertext).
    pub fn search(&self, q: &str, limit: usize) -> Result<Vec<EmailFull>> {
        let needle = q.to_lowercase();
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare(
            "SELECT id, sender, subject, date, body FROM emails WHERE deleted = 0 ORDER BY id DESC LIMIT 600",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(EmailFull {
                id: r.get(0)?,
                sender: self.crypto.decrypt(&r.get::<_, String>(1)?),
                subject: self.crypto.decrypt(&r.get::<_, String>(2)?),
                date: r.get(3)?,
                body: self.crypto.decrypt(&r.get::<_, String>(4)?),
            })
        })?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter(|e| {
                e.sender.to_lowercase().contains(&needle)
                    || e.subject.to_lowercase().contains(&needle)
                    || e.body.to_lowercase().contains(&needle)
            })
            .take(limit)
            .collect())
    }
}
