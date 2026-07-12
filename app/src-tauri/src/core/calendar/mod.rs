//! Internal agenda/calendar. Local SQLite. Liara can query/create/edit/delete events.
//! Title and notes are encrypted at rest; timestamps stay clear (needed for sorting).
use crate::core::crypto::Crypto;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

const ENC: &str = "enc:v1:";

pub struct Calendar {
    conn: Mutex<Connection>,
    crypto: Arc<Crypto>,
}

#[derive(serde::Serialize, Clone)]
pub struct Event {
    pub id: i64,
    pub title: String,
    pub when_str: String,
    pub notes: String,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn ts_of(dt: chrono::NaiveDateTime) -> i64 {
    use chrono::{Local, TimeZone};
    Local.from_local_datetime(&dt).single().map(|x| x.timestamp()).unwrap_or(0)
}

/// Extract a time (h, m) from text like "16:00", "16.30", "alle 16".
fn extract_time(s: &str) -> Option<(u32, u32)> {
    for tok in s.split([' ', ',', ';', '\t']) {
        if let Some((hh, mm)) = tok.trim().split_once([':', '.']) {
            if let (Ok(h), Ok(m)) = (hh.parse::<u32>(), mm.parse::<u32>()) {
                if h < 24 && m < 60 {
                    return Some((h, m));
                }
            }
        }
    }
    if let Some(i) = s.find("alle ") {
        let num: String = s[i + 5..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(h) = num.parse::<u32>() {
            if h < 24 {
                return Some((h, 0));
            }
        }
    }
    None
}

fn parse_weekday(s: &str) -> Option<u32> {
    for (name, n) in [
        ("lunedì", 0u32), ("lunedi", 0), ("martedì", 1), ("martedi", 1),
        ("mercoledì", 2), ("mercoledi", 2), ("giovedì", 3), ("giovedi", 3),
        ("venerdì", 4), ("venerdi", 4), ("sabato", 5), ("domenica", 6),
    ] {
        if s.contains(name) {
            return Some(n);
        }
    }
    None
}

/// Best-effort parse of an ISO/human/relative (Italian) date string into an epoch.
fn parse_ts(s: &str) -> i64 {
    use chrono::{Datelike, Days, Local, NaiveDate, NaiveDateTime};
    let raw = s.trim();
    for fmt in ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%d/%m/%Y %H:%M", "%d/%m/%Y %H.%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(raw, fmt) {
            return ts_of(dt);
        }
    }
    for fmt in ["%Y-%m-%d", "%d/%m/%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(raw, fmt) {
            if let Some(dt) = d.and_hms_opt(9, 0, 0) {
                return ts_of(dt);
            }
        }
    }
    // relative Italian dates
    let low = raw.to_lowercase();
    let today = Local::now().date_naive();
    let (h, m) = extract_time(&low).unwrap_or((9, 0));
    let date: Option<NaiveDate> = if low.contains("dopodomani") {
        today.checked_add_days(Days::new(2))
    } else if low.contains("domani") {
        today.checked_add_days(Days::new(1))
    } else if low.contains("oggi") || low.contains("stasera") || low.contains("stamattina") {
        Some(today)
    } else if let Some(i) = low.find("tra ") {
        let n: String = low[i + 4..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if low.contains("giorn") {
            n.parse::<u64>().ok().and_then(|d| today.checked_add_days(Days::new(d)))
        } else {
            None
        }
    } else if let Some(wd) = parse_weekday(&low) {
        let cur = today.weekday().num_days_from_monday();
        let mut diff = (wd as i64 - cur as i64).rem_euclid(7);
        if diff == 0 {
            diff = 7; // "venerdì prossimo" said on a Friday → next week
        }
        today.checked_add_days(Days::new(diff as u64))
    } else {
        None
    };
    if let Some(d) = date {
        if let Some(dt) = d.and_hms_opt(h, m, 0) {
            return ts_of(dt);
        }
    }
    0
}

impl Calendar {
    pub fn open(path: &str, crypto: Arc<Crypto>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                when_ts INTEGER NOT NULL,
                when_str TEXT NOT NULL,
                notes TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
             );",
        )?;
        let cal = Self { conn: Mutex::new(conn), crypto };
        cal.migrate()?;
        Ok(cal)
    }

    /// One-time: encrypt legacy plaintext title/notes.
    fn migrate(&self) -> Result<()> {
        let c = self.conn.lock().unwrap();
        let rows: Vec<(i64, String, String)> = c
            .prepare("SELECT id, title, notes FROM events")
            .and_then(|mut s| {
                s.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                    .map(|rows| rows.filter_map(|x| x.ok()).collect())
            })
            .unwrap_or_default();
        for (id, title, notes) in rows {
            if !title.starts_with(ENC) {
                c.execute(
                    "UPDATE events SET title=?1, notes=?2 WHERE id=?3",
                    params![self.crypto.encrypt(&title)?, self.crypto.encrypt(&notes)?, id],
                )?;
            }
        }
        Ok(())
    }

    pub fn add(&self, title: &str, when_str: &str, notes: &str) -> Result<i64> {
        let ts = parse_ts(when_str);
        // store the resolved date (so the list shows a real date, not "venerdì prossimo")
        let display = if ts != 0 {
            use chrono::{Local, TimeZone};
            Local
                .timestamp_opt(ts, 0)
                .single()
                .map(|dt| dt.format("%d/%m/%Y %H:%M").to_string())
                .unwrap_or_else(|| when_str.to_string())
        } else {
            when_str.to_string()
        };
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO events (title, when_ts, when_str, notes, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![self.crypto.encrypt(title)?, ts, display, self.crypto.encrypt(notes)?, now()],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// Modifica un evento (campi opzionali). NB: oggi NESSUN tool/comando la espone — è API pronta per
    /// un futuro "modifica evento" (UI o tool `calendar_update`). Dichiarata dead-code, non silenziosa.
    #[allow(dead_code)]
    pub fn update(&self, id: i64, title: Option<&str>, when_str: Option<&str>, notes: Option<&str>) -> Result<()> {
        let c = self.conn.lock().unwrap();
        if let Some(t) = title {
            c.execute("UPDATE events SET title = ?1 WHERE id = ?2", params![self.crypto.encrypt(t)?, id])?;
        }
        if let Some(w) = when_str {
            c.execute("UPDATE events SET when_str = ?1, when_ts = ?2 WHERE id = ?3", params![w, parse_ts(w), id])?;
        }
        if let Some(n) = notes {
            c.execute("UPDATE events SET notes = ?1 WHERE id = ?2", params![self.crypto.encrypt(n)?, id])?;
        }
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.lock().unwrap().execute("DELETE FROM events WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn rows(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Event>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare(sql)?;
        let rows = stmt.query_map(p, |r| {
            Ok(Event {
                id: r.get(0)?,
                title: self.crypto.decrypt(&r.get::<_, String>(1)?),
                when_str: r.get(2)?,
                notes: self.crypto.decrypt(&r.get::<_, String>(3)?),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Upcoming events (from now), plus undated ones.
    pub fn upcoming(&self, limit: i64) -> Result<Vec<Event>> {
        self.rows(
            "SELECT id, title, when_str, notes FROM events WHERE when_ts >= ?1 OR when_ts = 0 ORDER BY (when_ts = 0), when_ts LIMIT ?2",
            &[&now(), &limit],
        )
    }

    /// All events, soonest first (for the UI).
    pub fn all(&self, limit: i64) -> Result<Vec<Event>> {
        self.rows(
            "SELECT id, title, when_str, notes FROM events ORDER BY (when_ts = 0), when_ts LIMIT ?1",
            &[&limit],
        )
    }

    pub fn search(&self, q: &str, limit: i64) -> Result<Vec<Event>> {
        // content is encrypted → decrypt and filter in memory (SQL LIKE can't match ciphertext)
        let needle = q.to_lowercase();
        let all = self.all(500)?;
        Ok(all
            .into_iter()
            .filter(|e| e.title.to_lowercase().contains(&needle) || e.notes.to_lowercase().contains(&needle))
            .take(limit as usize)
            .collect())
    }
}
