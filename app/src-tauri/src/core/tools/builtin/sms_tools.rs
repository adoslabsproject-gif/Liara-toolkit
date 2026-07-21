//! Tool SMS (lettura): `sms_recent` e `sms_search` leggono la copia LOCALE cifrata dei messaggi
//! (core/sms), popolata dall'utente col bottone "Sincronizza SMS" (consenso informato, permesso
//! READ_SMS). I numeri vengono mostrati col NOME di rubrica quando il contatto è importato.
use crate::core::contacts::Contacts;
use crate::core::sms::{Sms, SmsStore};
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

/// "19/07 10:30" — compatto ma inequivocabile per il modello (l'anno solo se diverso dal corrente).
fn fmt_ts(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(dt) => {
            let now = chrono::Local::now();
            use chrono::Datelike;
            if dt.year() == now.year() {
                dt.format("%d/%m %H:%M").to_string()
            } else {
                dt.format("%d/%m/%Y %H:%M").to_string()
            }
        }
        None => ts.to_string(),
    }
}

/// Righe leggibili per il modello: "[19/07 10:30] da Marco Rossi (3331234567): testo".
fn render(messages: &[Sms], contacts: &Contacts) -> String {
    let names = contacts.names_by_key().unwrap_or_default();
    messages
        .iter()
        .map(|m| {
            let key = crate::core::contacts::number_key(&m.number);
            let who = match names.get(&key) {
                Some(name) => format!("{} ({})", name, m.number),
                None => m.number.clone(),
            };
            let verso = if m.kind == "out" { "a" } else { "da" };
            format!("[{}] {} {}: {}", fmt_ts(m.ts), verso, who, m.body)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const NESSUN_SYNC: &str = "Nessun SMS disponibile: l'utente non ha ancora sincronizzato i messaggi. \
Può farlo dal menù (Rubrica → Sincronizza SMS).";

/// Gli ultimi SMS (ricevuti e inviati), dal più recente.
pub struct SmsRecent {
    pub store: Arc<SmsStore>,
    pub contacts: Arc<Contacts>,
}
impl Tool for SmsRecent {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "sms_recent".into(),
            description: "Legge gli ultimi SMS del telefono (ricevuti e inviati), dal più recente. \
Usalo quando l'utente chiede dei suoi messaggi: \"leggi gli ultimi sms\", \"chi mi ha scritto un sms?\"."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Quanti messaggi (default 10, max 50)" }
                },
                "required": []
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
            .unwrap_or(10)
            .clamp(1, 50) as usize;
        let msgs = self.store.recent(limit)?;
        if msgs.is_empty() {
            return Ok(NESSUN_SYNC.into());
        }
        Ok(render(&msgs, &self.contacts))
    }
}

/// Cerca negli SMS per testo, numero o nome di rubrica.
pub struct SmsSearch {
    pub store: Arc<SmsStore>,
    pub contacts: Arc<Contacts>,
}
impl Tool for SmsSearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "sms_search".into(),
            description: "Cerca negli SMS per testo, numero o nome del contatto. Usalo per domande tipo \
\"cosa mi ha scritto Marco?\" o \"trova l'sms con il codice\"."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Testo, numero o nome da cercare" }
                },
                "required": ["query"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("manca 'query'"))?;
        if self.store.count()? == 0 {
            return Ok(NESSUN_SYNC.into());
        }
        // la query può essere un NOME di rubrica ("marco") → si risolve nei numeri di quei contatti
        // e si confronta per CHIAVE-numero (ultime 10 cifre), perché rubrica e SMS possono avere lo
        // stesso numero con formattazioni diverse ("+39 333 000 0001" vs "3330000001").
        let mut msgs = self.store.search(query, 20)?;
        if msgs.is_empty() {
            let keys: std::collections::HashSet<String> = self
                .contacts
                .search(query)?
                .iter()
                .map(|c| crate::core::contacts::number_key(&c.number))
                .collect();
            if !keys.is_empty() {
                msgs = self
                    .store
                    .recent(usize::MAX / 2)?
                    .into_iter()
                    .filter(|m| keys.contains(&crate::core::contacts::number_key(&m.number)))
                    .take(20)
                    .collect();
            }
        }
        if msgs.is_empty() {
            return Ok(format!("Nessun SMS trovato per «{query}»."));
        }
        Ok(render(&msgs, &self.contacts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::crypto::Crypto;

    fn fixtures() -> (Arc<SmsStore>, Arc<Contacts>) {
        let crypto = Arc::new(Crypto::from_key(&[11u8; 32]));
        let store = Arc::new(SmsStore::open(":memory:", crypto.clone()).unwrap());
        let contacts = Arc::new(Contacts::open(":memory:", crypto).unwrap());
        contacts.import(&[("Marco Rossi".into(), "+39 333 000 0001".into())]).unwrap();
        store
            .import(&vec![
                ("3330000001".into(), "Ci vediamo alle 15".into(), 100, "in".into()),
                ("3339999999".into(), "Promo: solo oggi sconti".into(), 200, "in".into()),
            ])
            .unwrap();
        (store, contacts)
    }

    #[test]
    fn sms_recent_mostra_il_nome_di_rubrica() {
        let (store, contacts) = fixtures();
        let t = SmsRecent { store, contacts };
        let out = t.execute(&json!({})).unwrap();
        // il numero di Marco è in rubrica (dedup ultime-10-cifre: +39 uguale a nudo) → appare il nome
        assert!(out.contains("Marco Rossi"), "manca il nome risolto: {out}");
        assert!(out.contains("3339999999"), "il numero fuori rubrica resta grezzo");
        // ordine: più recente prima
        assert!(out.find("Promo").unwrap() < out.find("Ci vediamo").unwrap());
    }

    #[test]
    fn sms_search_per_testo_e_per_nome() {
        let (store, contacts) = fixtures();
        let t = SmsSearch { store, contacts };
        // per testo
        let x = t.execute(&json!({ "query": "sconti" })).unwrap();
        assert!(x.contains("Promo"));
        // per NOME di rubrica: "marco" non è nel testo → passa dalla risoluzione contatto→numero
        let m = t.execute(&json!({ "query": "marco" })).unwrap();
        assert!(m.contains("Ci vediamo alle 15"), "la ricerca per nome deve trovare gli sms di Marco: {m}");
        // niente
        assert!(t.execute(&json!({ "query": "bonifico" })).unwrap().contains("Nessun SMS"));
    }

    #[test]
    fn store_vuoto_spiega_la_sincronizzazione() {
        let crypto = Arc::new(Crypto::from_key(&[12u8; 32]));
        let t = SmsRecent {
            store: Arc::new(SmsStore::open(":memory:", crypto.clone()).unwrap()),
            contacts: Arc::new(Contacts::open(":memory:", crypto).unwrap()),
        };
        assert!(t.execute(&json!({})).unwrap().contains("Sincronizza SMS"));
    }
}
