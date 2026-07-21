//! Rubrica DI Liara: contatti (nome + numero) importati dal telefono su CONSENSO dell'utente e
//! memorizzati QUI, nel `liara.db` cifrato (AES-256-GCM at rest) — MAI sul server. Serve a "chiama
//! Marco"/"scrivi a Marco": i tool telefono risolvono nome→numero cercando in questo store, con
//! gestione OMONIMIA (più contatti con lo stesso nome → si mostrano tutti e si chiede quale).
//!
//! L'import è GRANULARE e su richiesta: il frontend legge la rubrica di sistema (JNI, permesso
//! READ_CONTACTS) → l'utente sceglie QUALI importare (selezione multipla) → `import`. Il pulsante
//! "Sincronizza rubrica" ripete l'operazione quando l'utente aggiunge contatti nuovi.
use crate::core::crypto::Crypto;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

pub struct Contacts {
    conn: Mutex<Connection>,
    crypto: Arc<Crypto>,
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct Contact {
    pub id: i64,
    pub name: String,
    pub number: String,
    /// L'utente ha modificato QUI (nome o numero) questo contatto → la sincronizzazione NON lo
    /// sovrascrive più e la UI lo marca "personalizzato".
    pub customized: bool,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Chiave di deduplica: le ULTIME 10 cifre del numero (le altre = formattazione o prefisso paese).
/// Così "333 123 4567", "+39 333 123 4567" e "0039 3331234567" mappano tutte alla stessa chiave e lo
/// stesso contatto non crea doppioni. Numeri < 10 cifre (fissi corti) usano tutte le cifre.
/// `pub(crate)`: commands/contacts.rs la usa per marcare i contatti di sistema GIÀ importati.
pub(crate) fn number_key(number: &str) -> String {
    let digits: String = number.chars().filter(|c| c.is_ascii_digit()).collect();
    let n = digits.chars().count();
    if n > 10 {
        digits.chars().skip(n - 10).collect()
    } else {
        digits
    }
}

/// Normalizza un nome per il match: minuscolo, accenti piatti, spazi collassati. "Marco Rossi" e
/// "marco  rossi" combaciano; la ricerca è per SOTTOSTRINGA (così "marco" trova "Marco Rossi").
/// `pub(crate)`: riusata da core/sms per la ricerca nel testo dei messaggi (stessa semantica).
pub(crate) fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.trim().to_lowercase().chars() {
        let c = match c {
            'à' | 'á' | 'â' => 'a', 'è' | 'é' | 'ê' => 'e', 'ì' | 'í' | 'î' => 'i',
            'ò' | 'ó' | 'ô' => 'o', 'ù' | 'ú' | 'û' => 'u', _ => c,
        };
        if c.is_whitespace() {
            if !prev_space { out.push(' '); prev_space = true; }
        } else {
            out.push(c); prev_space = false;
        }
    }
    out.trim().to_string()
}

impl Contacts {
    pub fn open(path: &str, crypto: Arc<Crypto>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        // name e number CIFRATI; number_key (solo cifre) in chiaro per la deduplica (non identifica da
        // solo). origin_key = number_key al MOMENTO dell'import di sistema: è l'IDENTITÀ del contatto
        // per la sincronizzazione — resta fissa anche se l'utente cambia il numero, così un ri-sync sa
        // che quel contatto di sistema è già stato importato (e non lo re-importa). customized = 1 se
        // l'utente l'ha modificato QUI → il sync non lo tocca più.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS contacts (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                number TEXT NOT NULL,
                number_key TEXT NOT NULL,
                origin_key TEXT NOT NULL DEFAULT '',
                customized INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                UNIQUE(number_key)
             );",
        )?;
        // Migrazione dei DB creati con lo schema vecchio (senza origin_key/customized): aggiungi le
        // colonne (idempotente: ignora "duplicate column") e inizializza origin_key = number_key per
        // i contatti già presenti (erano tutti import di sistema, origine = numero attuale).
        for stmt in [
            "ALTER TABLE contacts ADD COLUMN origin_key TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE contacts ADD COLUMN customized INTEGER NOT NULL DEFAULT 0",
        ] {
            let _ = conn.execute(stmt, []); // Err se la colonna esiste già → ok
        }
        conn.execute("UPDATE contacts SET origin_key = number_key WHERE origin_key = ''", [])?;
        Ok(Self { conn: Mutex::new(conn), crypto })
    }

    /// Importa dalla rubrica di SISTEMA una lista (nome, numero). Deduplica per ORIGINE:
    /// - contatto con quella `origin_key` GIÀ presente e PERSONALIZZATO → lasciato intatto (mai
    ///   riscritto: è il caso "l'ho modificato io, non ripristinarmi il numero di sistema");
    /// - già presente e NON personalizzato → aggiorna il nome se cambiato (il sistema è la fonte),
    ///   ma NON conta come nuovo;
    /// - non presente → inserito (origin_key = number_key, customized = 0) e contato come NUOVO.
    /// Ritorna quanti NUOVI contatti sono stati aggiunti (0 = niente da sincronizzare).
    pub fn import(&self, items: &[(String, String)]) -> Result<usize> {
        let c = self.conn.lock().unwrap();
        let mut new = 0usize;
        for (name, number) in items {
            let key = number_key(number);
            if key.is_empty() {
                continue; // niente cifre = non è un numero
            }
            // esiste già un contatto con questa ORIGINE?
            let existing: Option<(i64, bool)> = c
                .query_row(
                    "SELECT id, customized FROM contacts WHERE origin_key = ?1",
                    params![key],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? != 0)),
                )
                .ok();
            match existing {
                Some((_, true)) => continue, // personalizzato → NON toccare
                Some((id, false)) => {
                    // rinfresca il nome dal sistema (numero invariato: non personalizzato = origin==attuale)
                    c.execute(
                        "UPDATE contacts SET name = ?1 WHERE id = ?2",
                        params![self.crypto.encrypt(name)?, id],
                    )?;
                }
                None => {
                    // nuovo: number_key = origin_key = key. INSERT OR IGNORE per non morire su un
                    // eventuale conflitto UNIQUE(number_key) con un personalizzato che punta a questo numero.
                    let n = c.execute(
                        "INSERT OR IGNORE INTO contacts (name, number, number_key, origin_key, customized, created_at)
                         VALUES (?1, ?2, ?3, ?3, 0, ?4)",
                        params![self.crypto.encrypt(name)?, self.crypto.encrypt(number)?, key, now()],
                    )?;
                    new += n; // 1 se inserito, 0 se ignorato per conflitto
                }
            }
        }
        Ok(new)
    }

    /// Modifica un contatto NELLA rubrica dell'app (nome e/o numero) → lo marca `customized`, così la
    /// prossima sincronizzazione non lo sovrascrive. `origin_key` resta invariato (l'identità di sistema
    /// non cambia). Errore se il nuovo numero coincide con quello di un ALTRO contatto.
    pub fn update(&self, id: i64, name: &str, number: &str) -> Result<()> {
        let key = number_key(number);
        if key.is_empty() {
            anyhow::bail!("il numero non contiene cifre");
        }
        let c = self.conn.lock().unwrap();
        let clash: Option<i64> = c
            .query_row(
                "SELECT id FROM contacts WHERE number_key = ?1 AND id <> ?2",
                params![key, id],
                |r| r.get(0),
            )
            .ok();
        if clash.is_some() {
            anyhow::bail!("un altro contatto ha già questo numero");
        }
        let n = c.execute(
            "UPDATE contacts SET name = ?1, number = ?2, number_key = ?3, customized = 1 WHERE id = ?4",
            params![self.crypto.encrypt(name)?, self.crypto.encrypt(number)?, key, id],
        )?;
        if n == 0 {
            anyhow::bail!("contatto non trovato");
        }
        Ok(())
    }

    fn all_decrypted(&self) -> Result<Vec<Contact>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare("SELECT id, name, number, customized FROM contacts ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Contact {
                id: r.get(0)?,
                name: self.crypto.decrypt(&r.get::<_, String>(1)?),
                number: self.crypto.decrypt(&r.get::<_, String>(2)?),
                customized: r.get::<_, i64>(3)? != 0,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn all(&self, limit: usize) -> Result<Vec<Contact>> {
        let mut v = self.all_decrypted()?;
        v.truncate(limit);
        Ok(v)
    }

    /// Cerca per nome (sottostringa, accent-insensitive). Ritorna TUTTI i match → chi chiama gestisce
    /// l'omonimia (0 = non trovato, 1 = usa quello, >1 = chiedi quale). Preferisce i match che
    /// INIZIANO col termine (così "marco" mette "Marco Rossi" prima di "Gianmarco").
    pub fn search(&self, name: &str) -> Result<Vec<Contact>> {
        let q = norm(name);
        if q.is_empty() {
            return Ok(vec![]);
        }
        let mut hits: Vec<Contact> = self
            .all_decrypted()?
            .into_iter()
            .filter(|c| norm(&c.name).contains(&q))
            .collect();
        hits.sort_by_key(|c| !norm(&c.name).starts_with(&q)); // prefix-match prima
        Ok(hits)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.lock().unwrap().execute("DELETE FROM contacts WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn count(&self) -> Result<usize> {
        let c = self.conn.lock().unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// Stato di sincronizzazione per ORIGINE: `origin_key` → è personalizzato?. La UI lo usa per
    /// marcare ogni contatto di sistema: assente = NUOVO (preselezionabile), presente con `false` =
    /// GIÀ IMPORTATO, presente con `true` = PERSONALIZZATO (import bloccato, non ripristina il numero).
    pub fn sync_state(&self) -> Result<std::collections::HashMap<String, bool>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare("SELECT origin_key, customized FROM contacts")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))?;
        Ok(rows.filter_map(|r| r.ok()).filter(|(k, _)| !k.is_empty()).collect())
    }

    /// Mappa chiave-numero → nome, per la risoluzione INVERSA numero→nome (es. mostrare "da Marco"
    /// invece del numero grezzo negli SMS). Chiave = `number_key` (ultime 10 cifre).
    pub fn names_by_key(&self) -> Result<std::collections::HashMap<String, String>> {
        Ok(self
            .all_decrypted()?
            .into_iter()
            .map(|c| (number_key(&c.number), c.name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Contacts {
        let crypto = Arc::new(Crypto::from_key(&[3u8; 32]));
        Contacts::open(":memory:", crypto).unwrap()
    }

    #[test]
    fn import_dedup_per_numero() {
        let c = store();
        assert_eq!(c.import(&[("Marco Rossi".into(), "333 123 4567".into())]).unwrap(), 1);
        // stesso numero con formattazione diversa → aggiorna il nome, NON duplica
        assert_eq!(c.import(&[("Marco R.".into(), "+39 3331234567".into())]).unwrap(), 0);
        assert_eq!(c.count().unwrap(), 1);
        assert_eq!(c.all(10).unwrap()[0].name, "Marco R."); // nome aggiornato
    }

    #[test]
    fn search_omonimi_e_prefisso() {
        let c = store();
        c.import(&[
            ("Marco Rossi".into(), "3330000001".into()),
            ("Marco Bianchi".into(), "3330000002".into()),
            ("Gianmarco Verdi".into(), "3330000003".into()),
            ("Luca Neri".into(), "3330000004".into()),
        ]).unwrap();
        // due Marco → entrambi (omonimia), i prefix-match ("Marco …") prima di "Gianmarco"
        let marco = c.search("marco").unwrap();
        assert_eq!(marco.len(), 3);
        assert!(marco[0].name.starts_with("Marco") && marco[1].name.starts_with("Marco"));
        // accent-insensitive + un solo match
        assert_eq!(c.search("luca").unwrap().len(), 1);
        // niente match
        assert!(c.search("giuseppe").unwrap().is_empty());
    }

    #[test]
    fn numero_senza_cifre_scartato() {
        let c = store();
        assert_eq!(c.import(&[("Vuoto".into(), "n/d".into())]).unwrap(), 0);
        assert_eq!(c.count().unwrap(), 0);
    }

    #[test]
    fn re_sync_identico_non_ha_nulla_da_fare() {
        // il caso dell'owner: ri-sincronizzare gli STESSI contatti → 0 nuovi (niente da sincronizzare)
        let c = store();
        let batch = [
            ("Marco Rossi".to_string(), "3330000001".to_string()),
            ("Luca Neri".to_string(), "3330000004".to_string()),
        ];
        assert_eq!(c.import(&batch).unwrap(), 2);
        assert_eq!(c.import(&batch).unwrap(), 0, "ri-sync identico = 0 nuovi");
        assert_eq!(c.import(&batch).unwrap(), 0, "e ancora 0 (mai doppioni)");
        assert_eq!(c.count().unwrap(), 2);
    }

    #[test]
    fn contatto_personalizzato_non_viene_ripristinato_dal_sync() {
        let c = store();
        c.import(&[("Marco Rossi".into(), "3330000001".into())]).unwrap();
        let id = c.all(10).unwrap()[0].id;
        // l'utente cambia il numero nell'app
        c.update(id, "Marco Rossi", "3339999999").unwrap();
        let m = &c.all(10).unwrap()[0];
        assert_eq!(m.number, "3339999999");
        assert!(m.customized, "modificato = personalizzato");
        // ri-sincronizzo la rubrica di sistema (che ha ancora il numero ORIGINALE): NON deve
        // ripristinare il numero né creare un doppione
        assert_eq!(c.import(&[("Marco Rossi".into(), "3330000001".into())]).unwrap(), 0);
        assert_eq!(c.count().unwrap(), 1);
        assert_eq!(c.all(10).unwrap()[0].number, "3339999999", "il numero personalizzato resta");
        // e lo stato di sync segnala l'origine come personalizzata
        assert_eq!(c.sync_state().unwrap().get("3330000001"), Some(&true));
    }

    #[test]
    fn sync_state_marca_importati_e_personalizzati() {
        let c = store();
        c.import(&[
            ("Marco Rossi".into(), "3330000001".into()),
            ("Luca Neri".into(), "3330000004".into()),
        ]).unwrap();
        let luca = c.all(10).unwrap().into_iter().find(|x| x.name == "Luca Neri").unwrap();
        c.update(luca.id, "Lucactk", "3330000004").unwrap(); // solo nome → comunque personalizzato
        let st = c.sync_state().unwrap();
        assert_eq!(st.get("3330000001"), Some(&false), "importato, non personalizzato");
        assert_eq!(st.get("3330000004"), Some(&true), "personalizzato");
        assert_eq!(st.get("3339999999"), None, "mai visto = nuovo");
    }

    #[test]
    fn update_rifiuta_numero_di_un_altro_contatto() {
        let c = store();
        c.import(&[
            ("A".into(), "3330000001".into()),
            ("B".into(), "3330000002".into()),
        ]).unwrap();
        let a = c.all(10).unwrap().into_iter().find(|x| x.name == "A").unwrap();
        // portare A sul numero di B deve fallire (niente collisioni)
        assert!(c.update(a.id, "A", "3330000002").is_err());
    }
}
