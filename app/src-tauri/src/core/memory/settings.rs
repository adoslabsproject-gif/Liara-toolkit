//! Settings rows: current location (manual/GPS) and per-tool consent permissions.
use super::{now, Memory};
use anyhow::Result;
use rusqlite::params;

impl Memory {
    // --- current location (manual correction or GPS), encrypted at rest ---

    pub fn set_location(&self, lat: f64, lon: f64, label: &str, source: &str) -> Result<()> {
        let blob = serde_json::json!({ "lat": lat, "lon": lon, "label": label, "source": source, "ts": now() })
            .to_string();
        self.conn.lock().unwrap().execute(
            "INSERT INTO settings (key, value) VALUES ('location', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![self.crypto.encrypt(&blob)?],
        )?;
        Ok(())
    }

    /// Current stored location → (lat, lon, label).
    pub fn location(&self) -> Option<(f64, f64, String)> {
        let enc: String = self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT value FROM settings WHERE key='location'", [], |r| r.get(0))
            .ok()?;
        let j: serde_json::Value = serde_json::from_str(&self.crypto.decrypt(&enc)).ok()?;
        Some((
            j.get("lat")?.as_f64()?,
            j.get("lon")?.as_f64()?,
            j.get("label")?.as_str()?.to_string(),
        ))
    }

    /// Posizione per la UI Impostazioni: (label, source) — source ∈ {"gps","manual"}. None se non impostata.
    pub fn location_display(&self) -> Option<(String, String)> {
        let enc: String = self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT value FROM settings WHERE key='location'", [], |r| r.get(0))
            .ok()?;
        let j: serde_json::Value = serde_json::from_str(&self.crypto.decrypt(&enc)).ok()?;
        Some((
            j.get("label")?.as_str()?.to_string(),
            j.get("source").and_then(|s| s.as_str()).unwrap_or("manual").to_string(),
        ))
    }

    // --- per-tool consent (allow | ask | deny), persisted ---

    pub fn get_permission(&self, tool: &str) -> Option<String> {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT value FROM settings WHERE key = ?1", params![format!("perm:{tool}")], |r| {
                r.get::<_, String>(0)
            })
            .ok()
    }

    pub fn set_permission(&self, tool: &str, state: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![format!("perm:{tool}"), state],
        )?;
        Ok(())
    }

    /// All explicitly-set tool permissions (tool, state).
    pub fn list_permissions(&self) -> Vec<(String, String)> {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare("SELECT key, value FROM settings WHERE key LIKE 'perm:%'") else {
            return Vec::new();
        };
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
        match rows {
            Ok(it) => it
                .filter_map(|x| x.ok())
                .map(|(k, v)| (k.trim_start_matches("perm:").to_string(), v))
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}
