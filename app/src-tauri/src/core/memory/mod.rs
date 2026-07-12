//! Persistent, local, private memory (pillar #1).
//! v1: structured profile + durable user facts + episodic log in SQLite.
//! Sensitive content is encrypted at rest (AES-256-GCM, key in OS keystore).
//! v2: in-RAM vector index (decrypted once at open) → recall is a fast in-memory cosine,
//! NOT a per-turn full DB scan + decrypt. Reconciles privacy (encrypted at rest) with speed.
mod facts;
mod notes;
mod settings;
mod store;
mod vector;
#[cfg(test)]
mod tests;

use crate::core::crypto::Crypto;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

pub struct Memory {
    conn: Mutex<Connection>,
    crypto: Arc<Crypto>,
    /// Decrypted vector index kept in RAM so recall never scans+decrypts the DB per turn.
    index: Mutex<Vec<VecEntry>>,
}

/// One row of the in-RAM vector index (mirrors a `memories` row, decrypted).
pub(super) struct VecEntry {
    pub id: i64,
    pub kind: String,
    pub emb: Vec<f32>,
    pub text: String,
    pub importance: f32,
    pub created_at: i64,
    pub valid: bool,
}

const ENC: &str = "enc:v1:";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
