//! Characterization tests: pin the observable behavior of the EmailStore (config + at-rest
//! encryption + CRUD + dedup + search) so the modularization can't change it silently.
use super::*;
use crate::core::crypto::Crypto;
use std::collections::HashMap;
use std::sync::Arc;

fn store() -> EmailStore {
    EmailStore::open(":memory:", Arc::new(Crypto::from_key(&[9u8; 32]))).unwrap()
}

fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn config_roundtrip_never_exposes_password() {
    let s = store();
    s.set_config(cfg(&[("imap_host", "imap.gmail.com"), ("email", "x@gmail.com"), ("password", "secret-pw")]))
        .unwrap();
    let got = s.get_config().unwrap();
    assert_eq!(got.get("imap_host").map(String::as_str), Some("imap.gmail.com"));
    assert_eq!(got.get("email").map(String::as_str), Some("x@gmail.com"));
    // get_config must NEVER return the password (encrypted or not)
    assert!(!got.contains_key("password"));
    assert!(!got.contains_key("password_enc"));
    assert!(s.has_password());
}

#[test]
fn password_strips_whitespace() {
    // Gmail app-passwords are shown in 4-letter groups with spaces — they must be stripped.
    let s = store();
    s.set_config(cfg(&[("password", "abcd efgh ijkl mnop")])).unwrap();
    assert_eq!(s.password().unwrap(), "abcdefghijklmnop");
}

#[test]
fn empty_password_not_stored() {
    let s = store();
    s.set_config(cfg(&[("imap_host", "h"), ("password", "")])).unwrap();
    assert!(!s.has_password());
}

#[test]
fn store_email_dedup_by_uid_folder() {
    let s = store();
    assert!(s.store_email(1, "INBOX", "a@x.it", "Ciao", "corpo", "2026-06-27").unwrap());
    assert!(!s.store_email(1, "INBOX", "a@x.it", "Ciao", "corpo", "2026-06-27").unwrap(), "stesso uid+cartella = duplicato");
    // same uid, different folder is a distinct message
    assert!(s.store_email(1, "SENT", "a@x.it", "Ciao", "corpo", "2026-06-27").unwrap());
}

#[test]
fn list_get_delete() {
    let s = store();
    s.store_email(10, "INBOX", "mittente@x.it", "Oggetto", "Testo del corpo", "2026-06-27").unwrap();
    let list = s.list_in("INBOX").unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].sender, "mittente@x.it");
    assert_eq!(list[0].subject, "Oggetto");
    let full = s.get(list[0].id).unwrap().unwrap();
    assert_eq!(full.body, "Testo del corpo");
    let id = list[0].id;
    // delete is a SOFT-delete: moves to Trash (recoverable), out of the inbox
    s.delete(id).unwrap();
    assert!(s.list_in("INBOX").unwrap().is_empty());
    assert_eq!(s.list_trash().unwrap().len(), 1);
    // restore brings it back to the inbox
    s.restore(id).unwrap();
    assert_eq!(s.list_in("INBOX").unwrap().len(), 1);
    assert!(s.list_trash().unwrap().is_empty());
    // purge deletes permanently
    s.delete(id).unwrap();
    s.purge(id).unwrap();
    assert!(s.list_trash().unwrap().is_empty());
    assert!(s.get(id).unwrap().is_none());
}

#[test]
fn email_content_encrypted_at_rest() {
    let s = store();
    s.store_email(20, "INBOX", "segreto@x.it", "Oggetto Segreto", "corpo segreto", "2026-06-27").unwrap();
    let (sender, subject): (String, String) = s
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT sender, subject FROM emails WHERE uid=20", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert!(sender.starts_with(ENC), "il mittente deve essere cifrato a riposo");
    assert!(subject.starts_with(ENC), "l'oggetto deve essere cifrato a riposo");
}

#[test]
fn search_finds_by_decrypted_text() {
    let s = store();
    s.store_email(30, "INBOX", "banca@x.it", "Estratto conto", "saldo disponibile", "2026-06-27").unwrap();
    s.store_email(31, "INBOX", "amico@x.it", "Pranzo?", "ci vediamo", "2026-06-27").unwrap();
    let hits = s.search("estratto", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].subject, "Estratto conto");
}
