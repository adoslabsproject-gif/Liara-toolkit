//! Canale peer E2E (chat Liara↔Liara).
//!
//! IDENTITÀ = keypair X25519. La CHIAVE PUBBLICA è l'ID che condividi via QR; nessun numero di
//! telefono richiesto (un Mac senza SIM ha comunque un'identità valida — modello Signal/Session).
//! I messaggi sono cifrati con `crypto_box` (X25519 + XSalsa20-Poly1305, NaCl): il relay instrada
//! SOLO ciphertext, non può leggere nulla. La SECRET è cifrata a riposo dalla master key
//! (hardware-wrapped su Android, vedi core/crypto) e caricata UNA volta all'avvio → seal/open non
//! toccano più il keystore.
//!
//! ⚠️ LA RUBRICA È QUELLA DEL TELEFONO. I contatti Liara vivono nei CONTATTI NATIVI del telefono;
//! qui teniamo solo l'INDICE che lega un peer_id (chiave pubblica) al contatto, perché la rubrica
//! Android non ha un campo nativo per un "Liara ID". `PeerIndex` = i QR accettati (id → nickname),
//! cifrato a riposo. Su desktop (niente rubrica di sistema) questo indice È la rubrica.
//!
//! Livello crypto: box statico AUTENTICATO E2E (Milestone 2). Il forward-secrecy con double-ratchet
//! (stile Signal/vodozemac) è pianificato come Milestone 4 e non cambia il contratto dei comandi.
use crate::core::crypto::Crypto;
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use crypto_box::{
    aead::{Aead, AeadCore, OsRng},
    PublicKey, SalsaBox, SecretKey,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const NONCE_LEN: usize = 24; // XSalsa20 nonce

fn id_encode(pk: &PublicKey) -> String {
    URL_SAFE_NO_PAD.encode(pk.as_bytes())
}

/// Un ID peer è la chiave pubblica X25519 in base64url (32 byte → 43 char). Valida forma e lunghezza.
fn id_decode(id: &str) -> Result<PublicKey> {
    let bytes = URL_SAFE_NO_PAD
        .decode(id.trim())
        .map_err(|_| anyhow!("ID peer non valido (base64url atteso)"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("ID peer non valido (attesi 32 byte)"))?;
    Ok(PublicKey::from_bytes(arr))
}

fn secret_path() -> PathBuf {
    // accanto ai modelli (come net_id/active_model/cloud_active): una sola radice di stato
    crate::core::paths::models_base().join("peer_secret")
}

/// L'identità crittografica di QUESTO Liara. Caricata una volta, condivisa via `Arc` in AppState.
pub struct Identity {
    secret: SecretKey,
    public_id: String,
}

impl Identity {
    /// Carica la secret cifrata da disco, o la genera al primo avvio. La secret è avvolta dalla
    /// master key at-rest (`Crypto`) → su disco resta solo ciphertext; su Android è hardware-wrapped.
    pub fn load_or_create(crypto: &Crypto) -> Result<Self> {
        let path = secret_path();
        if let Ok(blob) = std::fs::read(&path) {
            let raw = crypto.decrypt_blob(&blob);
            if let Ok(arr) = <[u8; 32]>::try_from(raw.as_slice()) {
                let secret = SecretKey::from_bytes(arr);
                let public_id = id_encode(&secret.public_key());
                return Ok(Self { secret, public_id });
            }
            // blob illeggibile (chiave cambiata / corruzione): non sovrascriviamo in silenzio,
            // ma rigeneriamo un'identità nuova (i vecchi contatti non ti raggiungeranno più — è
            // il prezzo, identico alla perdita chiave di WhatsApp; loggato forte).
            eprintln!("LIARA-PEER: peer_secret illeggibile → rigenero l'identità");
        }
        let secret = SecretKey::generate(&mut OsRng);
        let wrapped = crypto.encrypt_blob(&secret.to_bytes()).context("cifro peer_secret")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(&path, &wrapped).context("scrivo peer_secret")?;
        let public_id = id_encode(&secret.public_key());
        Ok(Self { secret, public_id })
    }

    /// Identità usa-e-getta (dump_tools/test): mai persistita.
    pub fn ephemeral() -> Self {
        let secret = SecretKey::generate(&mut OsRng);
        let public_id = id_encode(&secret.public_key());
        Self { secret, public_id }
    }

    /// L'ID pubblico (base64url della chiave pubblica) — questo è ciò che va nel QR.
    pub fn public_id(&self) -> &str {
        &self.public_id
    }

    /// Cifra `plaintext` PER il peer `peer_id`. Output = base64url(nonce[24] ‖ ciphertext).
    /// Autenticato: solo io e il peer possiamo produrre/leggere questo (mutua autenticazione X25519).
    pub fn seal(&self, peer_id: &str, plaintext: &str) -> Result<String> {
        let their = id_decode(peer_id)?;
        let boxx = SalsaBox::new(&their, &self.secret);
        let nonce = SalsaBox::generate_nonce(&mut OsRng);
        let ct = boxx
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| anyhow!("cifratura peer fallita"))?;
        let mut blob = nonce.as_slice().to_vec();
        blob.extend_from_slice(&ct);
        Ok(URL_SAFE_NO_PAD.encode(blob))
    }

    /// Apre un payload ricevuto DAL peer `peer_id`. Fallisce (non degrada) se autenticazione o
    /// formato non tornano — un messaggio manomesso o da mittente sbagliato NON viene consegnato.
    pub fn open(&self, peer_id: &str, payload_b64: &str) -> Result<String> {
        let their = id_decode(peer_id)?;
        let blob = URL_SAFE_NO_PAD
            .decode(payload_b64.trim())
            .map_err(|_| anyhow!("payload non valido (base64url atteso)"))?;
        if blob.len() < NONCE_LEN {
            return Err(anyhow!("payload troppo corto per contenere il nonce"));
        }
        let (nonce, ct) = blob.split_at(NONCE_LEN);
        let boxx = SalsaBox::new(&their, &self.secret);
        let pt = boxx
            .decrypt(crypto_box::Nonce::from_slice(nonce), ct)
            .map_err(|_| anyhow!("decifratura peer fallita (mittente errato o messaggio manomesso)"))?;
        String::from_utf8(pt).map_err(|_| anyhow!("il messaggio decifrato non è UTF-8"))
    }
}

/// Un contatto Liara accettato via QR. `name` è una copia comodità (dal contatto del telefono su
/// Android, o inserito a mano su desktop); la fonte di verità dell'identità è `id`.
#[derive(Serialize, Deserialize, Clone)]
pub struct Peer {
    pub id: String,
    pub name: String,
    /// epoch ms in cui è stato accettato (passato dal frontend: niente clock nel core).
    #[serde(default)]
    pub added: i64,
}

/// L'indice dei QR accettati (id → contatto). Persistito CIFRATO accanto ai modelli. NON è la rubrica
/// del telefono: è il ponte id↔contatto che Android non sa tenere nativamente.
pub struct PeerIndex;

impl PeerIndex {
    fn path() -> PathBuf {
        crate::core::paths::models_base().join("peers.enc")
    }

    pub fn list(crypto: &Crypto) -> Vec<Peer> {
        let Ok(blob) = std::fs::read(Self::path()) else {
            return Vec::new();
        };
        let raw = crypto.decrypt_blob(&blob);
        serde_json::from_slice(&raw).unwrap_or_default()
    }

    fn save(crypto: &Crypto, peers: &[Peer]) -> Result<()> {
        let json = serde_json::to_vec(peers).context("serializzo rubrica peer")?;
        let blob = crypto.encrypt_blob(&json).context("cifro rubrica peer")?;
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(&path, &blob).context("scrivo rubrica peer")?;
        Ok(())
    }

    /// Aggiunge/aggiorna un contatto (idempotente sull'`id`). Ritorna la lista aggiornata.
    pub fn add(crypto: &Crypto, id: &str, name: &str, added: i64) -> Result<Vec<Peer>> {
        id_decode(id)?; // rifiuta ID malformati: la rubrica resta pulita
        let mut peers = Self::list(crypto);
        let name = name.trim();
        if let Some(p) = peers.iter_mut().find(|p| p.id == id) {
            if !name.is_empty() {
                p.name = name.to_string();
            }
        } else {
            peers.push(Peer { id: id.to_string(), name: name.to_string(), added });
        }
        Self::save(crypto, &peers)?;
        Ok(peers)
    }

    pub fn remove(crypto: &Crypto, id: &str) -> Result<Vec<Peer>> {
        let mut peers = Self::list(crypto);
        peers.retain(|p| p.id != id);
        Self::save(crypto, &peers)?;
        Ok(peers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let a = Identity::ephemeral();
        let b = Identity::ephemeral();
        let sealed = a.seal(b.public_id(), "ciao E2E 🔐").unwrap();
        // b apre ciò che a ha sigillato per lui
        assert_eq!(b.open(a.public_id(), &sealed).unwrap(), "ciao E2E 🔐");
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let a = Identity::ephemeral();
        let b = Identity::ephemeral();
        let mut sealed = a.seal(b.public_id(), "segreto").unwrap();
        let mid = sealed.len() / 2;
        let ch = if &sealed[mid..mid + 1] == "A" { "B" } else { "A" };
        sealed.replace_range(mid..mid + 1, ch);
        assert!(b.open(a.public_id(), &sealed).is_err(), "un payload manomesso non deve aprirsi");
    }

    #[test]
    fn wrong_sender_cannot_open() {
        let a = Identity::ephemeral();
        let b = Identity::ephemeral();
        let c = Identity::ephemeral();
        let sealed = a.seal(b.public_id(), "solo per b").unwrap();
        // c non è il mittente atteso → l'autenticazione fallisce
        assert!(b.open(c.public_id(), &sealed).is_err());
    }
}
