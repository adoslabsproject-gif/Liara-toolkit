//! Local email (IMAP receive). Manual config (host/port/user/app-password),
//! no OAuth, no Google API. Emails are stored in the local DB; Liara can read them.
//! Content (sender/subject/body) is encrypted at rest; the password lives in the OS keystore.
mod imap;
mod query;
mod smtp;
mod store;
#[cfg(test)]
mod tests;

pub use imap::fetch_recent;
pub use smtp::send_email;

use crate::core::crypto::Crypto;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

const PASS_ACCOUNT: &str = "email-password"; // legacy keystore entry (migrated into the DB)
const PASS_KEY: &str = "password_enc"; // encrypted password row in email_config
const ENC: &str = "enc:v1:";

pub struct EmailStore {
    conn: Mutex<Connection>,
    crypto: Arc<Crypto>,
}

#[derive(serde::Serialize)]
pub struct EmailSummary {
    pub id: i64,
    pub sender: String,
    pub subject: String,
    pub date: String,
    pub seen: bool,
}

#[derive(serde::Serialize)]
pub struct EmailFull {
    pub id: i64,
    pub sender: String,
    pub subject: String,
    pub date: String,
    pub body: String,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
