//! Tool RUBRICA: ricerca contatti per nome nello store cifrato di Liara (core/contacts) e
//! risoluzione nome→numero condivisa coi tool telefono (phone_call/sms_send accettano un NOME).
//! Gestione OMONIMIA esplicita: 0 match = non trovato (chiedi/il numero), 1 = si usa quello,
//! >1 = si elencano tutti e si chiede all'utente quale.
use crate::core::contacts::{Contact, Contacts};
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

/// Esito della risoluzione di un target telefonico ("333…" o "Marco") contro la rubrica.
pub(crate) enum Resolved {
    /// Numero pronto all'uso (dal contatto se `name` è Some, o com'era se già numerico).
    Number { number: String, name: Option<String> },
    NotFound(String),
    Ambiguous(Vec<Contact>),
}

/// Se `raw` contiene lettere è un NOME → si cerca in rubrica; altrimenti è già un numero.
pub(crate) fn resolve_target(contacts: &Contacts, raw: &str) -> Result<Resolved> {
    if !raw.chars().any(|c| c.is_alphabetic()) {
        return Ok(Resolved::Number { number: raw.to_string(), name: None });
    }
    let hits = contacts.search(raw)?;
    Ok(match hits.len() {
        0 => Resolved::NotFound(raw.to_string()),
        1 => Resolved::Number { number: hits[0].number.clone(), name: Some(hits[0].name.clone()) },
        _ => Resolved::Ambiguous(hits),
    })
}

/// Messaggio per l'omonimia: elenca i contatti e chiede all'utente quale intende.
pub(crate) fn ambiguous_message(name: &str, hits: &[Contact]) -> String {
    let elenco: Vec<String> =
        hits.iter().map(|c| format!("• {} — {}", c.name, c.number)).collect();
    format!(
        "In rubrica ci sono {} contatti che corrispondono a «{}»:\n{}\nChiedi all'utente quale intende.",
        hits.len(),
        name,
        elenco.join("\n")
    )
}

/// Messaggio per il non-trovato: niente azioni, si chiede all'utente come procedere.
pub(crate) fn not_found_message(name: &str) -> String {
    format!(
        "«{name}» non è nella rubrica di Liara. Chiedi all'utente il numero, oppure di sincronizzare \
la rubrica dal menù (Rubrica → Sincronizza rubrica)."
    )
}

/// Cerca un contatto per nome e ritorna nome+numero (con omonimia esplicita).
pub struct ContactSearch {
    pub contacts: Arc<Contacts>,
}
impl Tool for ContactSearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "contact_search".into(),
            description: "Cerca un contatto nella rubrica di Liara per nome e restituisce nome e numero. \
Usalo quando serve il numero di una persona (\"che numero ha Marco?\") o per verificare chi c'è in rubrica."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Nome (anche parziale) del contatto da cercare" }
                },
                "required": ["name"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("manca 'name'"))?;
        let hits = self.contacts.search(name)?;
        Ok(match hits.len() {
            0 => not_found_message(name),
            1 => format!("{} — {}", hits[0].name, hits[0].number),
            _ => ambiguous_message(name, &hits),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::crypto::Crypto;

    fn contacts() -> Arc<Contacts> {
        let c = Arc::new(Contacts::open(":memory:", Arc::new(Crypto::from_key(&[6u8; 32]))).unwrap());
        c.import(&[
            ("Marco Rossi".into(), "3330000001".into()),
            ("Marco Bianchi".into(), "3330000002".into()),
            ("Luca Neri".into(), "3330000004".into()),
        ])
        .unwrap();
        c
    }

    #[test]
    fn contact_search_0_1_e_omonimia() {
        let t = ContactSearch { contacts: contacts() };
        // 1 match → nome e numero secchi
        let uno = t.execute(&json!({ "name": "luca" })).unwrap();
        assert!(uno.contains("Luca Neri") && uno.contains("3330000004"));
        // >1 → elenca TUTTI e chiede quale
        let due = t.execute(&json!({ "name": "marco" })).unwrap();
        assert!(due.contains("Marco Rossi") && due.contains("Marco Bianchi") && due.contains("quale"));
        // 0 → non trovato, suggerisce la sincronizzazione
        let zero = t.execute(&json!({ "name": "giuseppe" })).unwrap();
        assert!(zero.contains("non è nella rubrica"));
    }

    #[test]
    fn resolve_target_numero_passa_invariato() {
        let c = contacts();
        match resolve_target(&c, "+39 333 111 2222").unwrap() {
            Resolved::Number { number, name } => {
                assert_eq!(number, "+39 333 111 2222");
                assert!(name.is_none());
            }
            _ => panic!("un numero deve restare un numero"),
        }
    }

    #[test]
    fn resolve_target_nome_unico_e_ambiguo() {
        let c = contacts();
        match resolve_target(&c, "luca").unwrap() {
            Resolved::Number { number, name } => {
                assert_eq!(number, "3330000004");
                assert_eq!(name.as_deref(), Some("Luca Neri"));
            }
            _ => panic!("luca è unico → numero risolto"),
        }
        assert!(matches!(resolve_target(&c, "marco").unwrap(), Resolved::Ambiguous(v) if v.len() == 2));
        assert!(matches!(resolve_target(&c, "giuseppe").unwrap(), Resolved::NotFound(_)));
    }
}
