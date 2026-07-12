//! At-rest encryption for sensitive content.
//!
//! Design (threat model: lost/stolen device, disk image, file-reading malware):
//! - AES-256-GCM (RustCrypto, pure Rust, AEAD → confidentiality + tamper detection).
//!   No OpenSSL → builds clean on Android, identical code on every platform.
//! - The 256-bit master key is RANDOM, generated once, and stored in the OS keystore
//!   (hardware-backed where available). It is NEVER hardcoded, never written to the DB.
//! - Ciphertext is `enc:v1:<base64(nonce[12] || ciphertext)>`; values without that
//!   prefix are treated as legacy plaintext (transparent migration on next write).
//! - Nonces are 96-bit RANDOM per message (OsRng). At a personal-assistant's volume this is
//!   far below the birthday bound, so reuse is not a practical risk; the `enc:v1:` prefix
//!   leaves room for a future `enc:v2:` (e.g. XChaCha20-Poly1305's 192-bit nonce) if ever needed.
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;

#[cfg(target_os = "android")]
mod android_keystore;

const SERVICE: &str = "com.liara.app";
const KEY_ACCOUNT: &str = "db-master-key-v1";
const PREFIX: &str = "enc:v1:";

pub struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    /// Load (or generate on first run) the master key from the OS keystore.
    pub fn init() -> Result<Self> {
        let key = load_or_create_key()?;
        Ok(Self::from_key(&key))
    }

    pub(crate) fn from_key(key: &[u8; 32]) -> Self {
        Self { cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key)) }
    }

    /// Throwaway instance with a random key — for building a registry just to read tool specs
    /// (the dump_tools bin), never persisted.
    pub fn ephemeral() -> Self {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self::from_key(&key)
    }

    /// Encrypt a string into the `enc:v1:` envelope. Empty stays empty (nothing to hide).
    ///
    /// FALLISCE invece di degradare (review 2026-07-02 #2): la vecchia versione, su
    /// errore di cifratura, ritornava il PLAINTEXT in silenzio ("never lose data") —
    /// cioè scriveva PII in chiaro su disco senza nemmeno un log, l'esatto degrado
    /// silenzioso che il resto del codice combatte. Un dato che non si può cifrare
    /// NON si scrive: il chiamante fallisce la write e l'utente lo vede.
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }
        let mut nonce = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
            .map_err(|_| anyhow!("cifratura fallita: il dato NON viene salvato in chiaro"))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ct);
        Ok(format!("{PREFIX}{}", STANDARD.encode(blob)))
    }

    /// Encrypt raw bytes (e.g. embedding BLOBs) → `nonce[12] || ciphertext`. Empty stays empty.
    /// Come `encrypt`: su errore FALLISCE, mai bytes in chiaro su disco.
    pub fn encrypt_blob(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let mut nonce = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), data)
            .map_err(|_| anyhow!("cifratura blob fallita: il dato NON viene salvato in chiaro"))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ct);
        Ok(blob)
    }

    /// Decrypt a `nonce[12] || ciphertext` blob. If it doesn't authenticate (e.g. legacy
    /// plaintext blob), returns the input unchanged so old rows still read.
    pub fn decrypt_blob(&self, blob: &[u8]) -> Vec<u8> {
        if blob.len() < 12 {
            return blob.to_vec();
        }
        let (nonce, ct) = blob.split_at(12);
        match self.cipher.decrypt(Nonce::from_slice(nonce), ct) {
            Ok(pt) => pt,
            Err(_) => blob.to_vec(),
        }
    }

    /// Decrypt; values without our prefix are returned unchanged (legacy plaintext).
    ///
    /// TAMPER-DETECTION (review round-4): un valore SENZA prefisso `enc:v1:` è legacy plaintext e va
    /// ritornato in silenzio (atteso). Ma un valore CON il prefisso che poi NON si decifra è
    /// corruzione / manomissione / chiave sbagliata — NON legacy plaintext: prima era un fallimento
    /// MUTO (l'autenticazione AEAD, il cui scopo è proprio rilevare la manomissione, veniva sprecata).
    /// Ora lo segnaliamo forte su log. Il valore ritornato resta invariato (zero regressione: evitiamo
    /// che un mismatch di chiave transitorio faccia "sparire" tutti i dati dell'utente).
    pub fn decrypt(&self, value: &str) -> String {
        let Some(b64) = value.strip_prefix(PREFIX) else {
            return value.to_string();
        };
        let Ok(blob) = STANDARD.decode(b64) else {
            eprintln!("LIARA-CRYPTO: valore enc:v1: con base64 non valido (corruzione)");
            return value.to_string();
        };
        if blob.len() < 12 {
            eprintln!("LIARA-CRYPTO: valore enc:v1: troppo corto per contenere il nonce (corruzione)");
            return value.to_string();
        }
        let (nonce, ct) = blob.split_at(12);
        match self.cipher.decrypt(Nonce::from_slice(nonce), ct) {
            Ok(pt) => String::from_utf8_lossy(&pt).into_owned(),
            Err(_) => {
                eprintln!("LIARA-CRYPTO: autenticazione AEAD fallita su un valore enc:v1: (manomissione o chiave errata)");
                value.to_string()
            }
        }
    }
}

/// Fetch the 256-bit key.
/// PRODUCTION (release): always the OS keystore — a random per-device key, created on
/// first run, never shared, never in code. The env override below is compiled out.
/// DEV (debug builds only): `LIARA_MASTER_KEY` env may supply the key to avoid the
/// macOS login-keychain prompt on unsigned binaries. Release ignores it entirely.
fn load_or_create_key() -> Result<[u8; 32]> {
    #[cfg(debug_assertions)]
    if let Ok(b64) = std::env::var("LIARA_MASTER_KEY") {
        let bytes = STANDARD
            .decode(b64.trim())
            .map_err(|e| anyhow!("LIARA_MASTER_KEY non è base64 valido: {e}"))?;
        return bytes.try_into().map_err(|_| anyhow!("LIARA_MASTER_KEY deve essere 32 byte"));
    }
    // Android (review 2026-07-02 #3): chiave master AVVOLTA dal keystore hardware
    // (AndroidKeyStore via JNI, vedi android_keystore.rs + KeystoreBridge.kt) —
    // su disco resta SOLO il blob avvolto: root/backup ADB non bastano più.
    // Il file in chiaro sopravvive SOLO come fallback se il keystore fallisce
    // (era il difetto del crate `keyring`: qui il fallimento è esplicito e loggato).
    #[cfg(target_os = "android")]
    {
        android_wrapped_key()
    }
    // Desktop: il keystore OS è affidabile e hardware-backed → usalo.
    #[cfg(not(target_os = "android"))]
    {
        keyring_key()
    }
}

/// Android: chiave master con wrapping hardware. Ordine:
///   1. blob avvolto (`.master_key.v2`) → unwrap dal keystore
///   2. file legacy in chiaro (`.master_key`) → MIGRAZIONE: wrap → scrivi v2 →
///      cancella il legacy (solo se il wrap è riuscito: mai perdere l'accesso al DB)
///   3. primo avvio → genera random, wrap; se il keystore non funziona,
///      fallback sul file in chiaro (degrado ESPLICITO, loggato forte)
#[cfg(target_os = "android")]
fn android_wrapped_key() -> Result<[u8; 32]> {
    use std::io::Write;
    let base = std::env::var("LIARA_MODELS_DIR")
        .ok()
        .and_then(|m| std::path::Path::new(&m).parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&base).ok();
    let wrapped_path = base.join(".master_key.v2");
    let legacy_path = base.join(".master_key");
    // Diagnostica PERSISTENTE (gli eprintln su Android non arrivano a logcat):
    // ogni evento chiave finisce in un file leggibile via `adb run-as`.
    let klog = |msg: &str| {
        eprintln!("LIARA-KEYSTORE: {msg}");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(base.join("keystore_jni.log")) {
            let _ = writeln!(f, "{msg}");
        }
    };

    // 1) blob avvolto esistente
    if let Ok(blob) = std::fs::read(&wrapped_path) {
        match android_keystore::unwrap(&blob) {
            Ok(k) if k.len() == 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&k);
                return Ok(key);
            }
            Ok(_) => klog("blob avvolto di lunghezza errata — rigenero"),
            Err(e) => klog(&format!("unwrap fallito ({e}) — il keystore è cambiato? rigenero")),
        }
        // il blob non si apre più: la chiave di wrapping è persa (factory reset del
        // keystore ecc.) → i dati cifrati sono comunque irrecuperabili. Si rigenera
        // (comportamento identico al vecchio keyring), MA con log esplicito.
    }

    // 2) migrazione dal file legacy in chiaro
    if let Ok(b64) = std::fs::read_to_string(&legacy_path) {
        if let Ok(bytes) = STANDARD.decode(b64.trim()) {
            if let Ok(key) = <[u8; 32]>::try_from(bytes) {
                match android_keystore::wrap(&key) {
                    Ok(blob) => {
                        if std::fs::write(&wrapped_path, &blob).is_ok() {
                            let _ = std::fs::remove_file(&legacy_path);
                            klog("chiave migrata al wrapping hardware, file legacy rimosso");
                        }
                    }
                    Err(e) => klog(&format!("wrap fallito in migrazione ({e}) — resto sul file legacy")),
                }
                return Ok(key);
            }
        }
    }

    // 3) primo avvio: genera e avvolgi
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    match android_keystore::wrap(&key) {
        Ok(blob) => {
            std::fs::write(&wrapped_path, &blob).map_err(|e| anyhow!("scrivo blob avvolto: {e}"))?;
        }
        Err(e) => {
            // degrado esplicito: il threat model torna "sandbox-only", ma l'app vive
            klog(&format!("keystore NON disponibile ({e}) — fallback su file in sandbox"));
            let mut f = std::fs::File::create(&legacy_path).map_err(|e| anyhow!("crea file chiave: {e}"))?;
            f.write_all(STANDARD.encode(key).as_bytes()).map_err(|e| anyhow!("scrivi file chiave: {e}"))?;
        }
    }
    Ok(key)
}

/// Chiave dal keystore OS (la crea al primo avvio se manca).
fn keyring_key() -> Result<[u8; 32]> {
    let entry = keyring::Entry::new(SERVICE, KEY_ACCOUNT)?;
    match entry.get_password() {
        Ok(b64) => {
            let bytes = STANDARD.decode(b64).map_err(|e| anyhow!("chiave keystore corrotta: {e}"))?;
            bytes.try_into().map_err(|_| anyhow!("lunghezza chiave non valida"))
        }
        Err(_) => {
            let mut key = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            entry
                .set_password(&STANDARD.encode(key))
                .map_err(|e| anyhow!("impossibile salvare la chiave nel keystore: {e}"))?;
            Ok(key)
        }
    }
}

/// Store/retrieve/delete a named secret (e.g. the email password) in the OS keystore.
pub fn secret_set(account: &str, value: &str) -> Result<()> {
    keyring::Entry::new(SERVICE, account)?
        .set_password(value)
        .map_err(|e| anyhow!("keystore set: {e}"))
}
pub fn secret_get(account: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, account).ok()?.get_password().ok()
}
pub fn secret_delete(account: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, account)?;
    match entry.delete_credential() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow!("keystore delete: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_crypto() -> Crypto {
        Crypto::from_key(&[7u8; 32])
    }

    #[test]
    fn roundtrip() {
        let c = test_crypto();
        let enc = c.encrypt("ciao Liara, segreto!").unwrap();
        assert!(enc.starts_with(PREFIX));
        assert_ne!(enc, "ciao Liara, segreto!");
        assert_eq!(c.decrypt(&enc), "ciao Liara, segreto!");
    }

    #[test]
    fn legacy_plaintext_passthrough() {
        let c = test_crypto();
        assert_eq!(c.decrypt("vecchio testo in chiaro"), "vecchio testo in chiaro");
    }

    #[test]
    fn tampered_ciphertext_non_si_decifra() {
        // review round-4: un enc:v1: manomesso NON deve decifrare al plaintext originale (AEAD lo
        // rifiuta) → torna il valore manomesso invariato, non il segreto.
        let c = test_crypto();
        let enc = c.encrypt("segreto").unwrap();
        let mut chars: Vec<char> = enc.chars().collect();
        let mid = chars.len() / 2; // dentro il base64, non nel prefisso
        chars[mid] = if chars[mid] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert_ne!(c.decrypt(&tampered), "segreto", "un ciphertext manomesso non deve rivelare il segreto");
    }

    #[test]
    fn empty_stays_empty() {
        let c = test_crypto();
        assert_eq!(c.encrypt("").unwrap(), "");
        assert_eq!(c.decrypt(""), "");
    }

    #[test]
    fn distinct_nonces() {
        let c = test_crypto();
        assert_ne!(c.encrypt("x").unwrap(), c.encrypt("x").unwrap()); // random nonce each time
    }

    #[test]
    fn encrypt_non_degrada_mai_in_plaintext() {
        // ANTI-REGRESSIONE (review #2): qualunque esito di encrypt DEVE essere o
        // l'envelope enc:v1: o un errore — MAI il plaintext restituito come "successo".
        let c = test_crypto();
        for s in ["pii sensibile", "à😀\u{0}binari", "x"] {
            match c.encrypt(s) {
                Ok(out) => assert!(out.starts_with(PREFIX), "plaintext leakato: {out}"),
                Err(_) => {} // fallire è accettabile, degradare no
            }
        }
        let blob = c.encrypt_blob(&[1, 2, 3]).unwrap();
        assert_ne!(blob, vec![1, 2, 3], "blob in chiaro");
        assert!(blob.len() > 3 + 12, "manca nonce+tag");
    }
}
