//! SMS DI Liara: copia LOCALE dei messaggi del telefono, sincronizzata su CONSENSO dell'utente
//! (bottone "Sincronizza SMS", permesso READ_SMS) e memorizzata QUI nel `liara.db` cifrato
//! (AES-256-GCM at rest) — MAI sul server. Serve ai tool `sms_recent`/`sms_search`: "leggi gli
//! ultimi sms", "cosa mi ha scritto Marco".
//!
//! Numero e testo sono CIFRATI; in chiaro restano solo timestamp, verso (in/out) e la chiave di
//! deduplica (cifre del numero + timestamp: non ricostruisce il contenuto).
use crate::core::contacts::{norm, number_key};
use crate::core::crypto::Crypto;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

pub struct SmsStore {
    conn: Mutex<Connection>,
    crypto: Arc<Crypto>,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct Sms {
    pub id: i64,
    /// Numero dell'interlocutore (mittente se `kind=="in"`, destinatario se `kind=="out"`).
    pub number: String,
    pub body: String,
    /// Timestamp UNIX in SECONDI.
    pub ts: i64,
    /// "in" = ricevuto, "out" = inviato.
    pub kind: String,
}

impl SmsStore {
    pub fn open(path: &str, crypto: Arc<Crypto>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sms (
                id INTEGER PRIMARY KEY,
                number TEXT NOT NULL,
                body TEXT NOT NULL,
                ts INTEGER NOT NULL,
                kind TEXT NOT NULL,
                dedup TEXT NOT NULL,
                UNIQUE(dedup)
             );",
        )?;
        Ok(Self { conn: Mutex::new(conn), crypto })
    }

    /// Importa una lista di messaggi (numero, testo, ts-secondi, kind). Idempotente: la chiave di
    /// deduplica (cifre-numero:ts:kind) fa sì che ri-sincronizzare NON crei doppioni. Ritorna
    /// quanti NUOVI messaggi sono stati aggiunti.
    pub fn import(&self, items: &[(String, String, i64, String)]) -> Result<usize> {
        let c = self.conn.lock().unwrap();
        let before: i64 = c.query_row("SELECT COUNT(*) FROM sms", [], |r| r.get(0))?;
        for (number, body, ts, kind) in items {
            let key = number_key(number);
            if key.is_empty() || body.trim().is_empty() {
                continue; // senza numero o senza testo non è un messaggio utile
            }
            c.execute(
                "INSERT OR IGNORE INTO sms (number, body, ts, kind, dedup) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    self.crypto.encrypt(number)?,
                    self.crypto.encrypt(body)?,
                    ts,
                    kind,
                    format!("{key}:{ts}:{kind}")
                ],
            )?;
        }
        let after: i64 = c.query_row("SELECT COUNT(*) FROM sms", [], |r| r.get(0))?;
        Ok((after - before).max(0) as usize)
    }

    /// Gli ultimi `limit` messaggi, dal più recente.
    pub fn recent(&self, limit: usize) -> Result<Vec<Sms>> {
        let c = self.conn.lock().unwrap();
        let mut stmt =
            c.prepare("SELECT id, number, body, ts, kind FROM sms ORDER BY ts DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(Sms {
                id: r.get(0)?,
                number: self.crypto.decrypt(&r.get::<_, String>(1)?),
                body: self.crypto.decrypt(&r.get::<_, String>(2)?),
                ts: r.get(3)?,
                kind: r.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Cerca nel TESTO e nel NUMERO (sottostringa, accent-insensitive — stessa `norm` della
    /// rubrica). Dal più recente, al massimo `limit` risultati.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Sms>> {
        let q = norm(query);
        if q.is_empty() {
            return Ok(vec![]);
        }
        // decifra-e-filtra in RAM: il testo è cifrato, un LIKE SQL non può vederlo. I volumi sono
        // da rubrica personale (centinaia/migliaia di SMS), non da datawarehouse: va benissimo.
        let mut hits: Vec<Sms> = self
            .recent(usize::MAX / 2)?
            .into_iter()
            .filter(|s| norm(&s.body).contains(&q) || norm(&s.number).contains(&q))
            .collect();
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn count(&self) -> Result<usize> {
        let c = self.conn.lock().unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM sms", [], |r| r.get(0))?;
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SmsStore {
        let crypto = Arc::new(Crypto::from_key(&[4u8; 32]));
        SmsStore::open(":memory:", crypto).unwrap()
    }

    #[test]
    fn import_dedup_e_recent_in_ordine() {
        let s = store();
        let batch = vec![
            ("3331234567".to_string(), "Ci vediamo alle 15".to_string(), 100, "in".to_string()),
            ("3331234567".to_string(), "Ok perfetto".to_string(), 200, "out".to_string()),
        ];
        assert_eq!(s.import(&batch).unwrap(), 2);
        // ri-sincronizzare lo STESSO batch non duplica (chiave numero:ts:kind)
        assert_eq!(s.import(&batch).unwrap(), 0);
        assert_eq!(s.count().unwrap(), 2);
        // recent: prima il più recente
        let r = s.recent(10).unwrap();
        assert_eq!(r[0].body, "Ok perfetto");
        assert_eq!(r[1].kind, "in");
    }

    #[test]
    fn search_su_testo_e_numero_accent_insensitive() {
        let s = store();
        s.import(&vec![
            ("3330000001".into(), "Perché non vieni più tardi?".into(), 10, "in".into()),
            ("3330000002".into(), "La spesa è fatta".into(), 20, "in".into()),
        ])
        .unwrap();
        assert_eq!(s.search("perche", 10).unwrap().len(), 1); // accento piatto
        assert_eq!(s.search("0000002", 10).unwrap().len(), 1); // per numero
        assert!(s.search("pizza", 10).unwrap().is_empty());
    }

    #[test]
    fn messaggi_vuoti_scartati() {
        let s = store();
        assert_eq!(s.import(&vec![("n/d".into(), "ciao".into(), 1, "in".into())]).unwrap(), 0);
        assert_eq!(s.import(&vec![("333".into(), "  ".into(), 1, "in".into())]).unwrap(), 0);
    }
}
