//! `EmailStore` lifecycle: open/migrate the DB and manage the (encrypted) config + password.
use super::{EmailStore, ENC, PASS_ACCOUNT, PASS_KEY};
use crate::core::crypto::{self, Crypto};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

impl EmailStore {
    pub fn open(path: &str, crypto: Arc<Crypto>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS email_config (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let has_folder: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('emails') WHERE name='folder'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has_folder == 0 {
            conn.execute_batch("DROP TABLE IF EXISTS emails;")?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS emails (
                id INTEGER PRIMARY KEY,
                uid INTEGER NOT NULL,
                folder TEXT NOT NULL DEFAULT 'INBOX',
                sender TEXT NOT NULL,
                subject TEXT NOT NULL,
                body TEXT NOT NULL,
                date TEXT NOT NULL,
                seen INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                UNIQUE(uid, folder)
             );",
        )?;
        // soft-delete flag for the Trash folder (recoverable); ignore error if it already exists
        let _ = conn.execute("ALTER TABLE emails ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0", []);
        let store = Self { conn: Mutex::new(conn), crypto };
        store.migrate()?;
        Ok(store)
    }

    /// One-time: move a legacy plaintext password into the keystore, and encrypt
    /// any legacy plaintext email content so nothing sensitive stays in the clear.
    fn migrate(&self) -> Result<()> {
        let c = self.conn.lock().unwrap();
        let store_pw = |c: &Connection, p: &str| -> Result<()> {
            c.execute(
                "INSERT INTO email_config (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![PASS_KEY, self.crypto.encrypt(p)?],
            )?;
            Ok(())
        };
        // legacy PLAINTEXT password in DB -> encrypted row
        if let Ok(p) = c.query_row(
            "SELECT value FROM email_config WHERE key='password'",
            [],
            |r| r.get::<_, String>(0),
        ) {
            if !p.is_empty() {
                store_pw(&c, &p)?;
            }
            c.execute("DELETE FROM email_config WHERE key='password'", [])?;
        }
        // legacy KEYSTORE password (from the earlier keystore design) -> encrypted row
        if let Some(p) = crypto::secret_get(PASS_ACCOUNT) {
            if !p.is_empty() {
                store_pw(&c, &p)?;
            }
            let _ = crypto::secret_delete(PASS_ACCOUNT);
        }
        // content: encrypt plaintext rows
        let rows: Vec<(i64, String, String, String)> = c
            .prepare("SELECT id, sender, subject, body FROM emails")
            .and_then(|mut s| {
                s.query_map([], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
                })
                .map(|rows| rows.filter_map(|x| x.ok()).collect())
            })
            .unwrap_or_default();
        for (id, sender, subject, body) in rows {
            if !sender.starts_with(ENC) {
                c.execute(
                    "UPDATE emails SET sender=?1, subject=?2, body=?3 WHERE id=?4",
                    params![self.crypto.encrypt(&sender)?, self.crypto.encrypt(&subject)?, self.crypto.encrypt(&body)?, id],
                )?;
            }
        }
        Ok(())
    }

    /// Save config. The password is stored AES-256-GCM-encrypted, never in plaintext.
    pub fn set_config(&self, cfg: HashMap<String, String>) -> Result<()> {
        let c = self.conn.lock().unwrap();
        for (k, v) in cfg {
            if k == "__has_password" {
                continue;
            }
            if k == "password" {
                if !v.is_empty() {
                    c.execute(
                        "INSERT INTO email_config (key, value) VALUES (?1, ?2)
                         ON CONFLICT(key) DO UPDATE SET value = ?2",
                        params![PASS_KEY, self.crypto.encrypt(&v)?],
                    )?;
                }
                continue;
            }
            c.execute(
                "INSERT INTO email_config (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![k, v],
            )?;
        }
        Ok(())
    }

    /// Config for the UI — never exposes the (encrypted) password row.
    pub fn get_config(&self) -> Result<HashMap<String, String>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare("SELECT key, value FROM email_config WHERE key != ?1")?;
        let rows = stmt.query_map(params![PASS_KEY], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// The decrypted account password (whitespace stripped: app-passwords show spaces).
    pub fn password(&self) -> Result<String> {
        let enc = self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT value FROM email_config WHERE key = ?1", params![PASS_KEY], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| anyhow!("Password mancante"))?;
        let p: String = self.crypto.decrypt(&enc).chars().filter(|c| !c.is_whitespace()).collect();
        if p.is_empty() {
            return Err(anyhow!("Password mancante"));
        }
        Ok(p)
    }

    pub fn has_password(&self) -> bool {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT value FROM email_config WHERE key = ?1", params![PASS_KEY], |r| {
                r.get::<_, String>(0)
            })
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }
}
