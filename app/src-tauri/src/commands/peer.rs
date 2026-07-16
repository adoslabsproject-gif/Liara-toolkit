//! Comandi del canale peer (chat Liara↔Liara E2E). Il TRASPORTO (WebSocket verso il relay) vive nel
//! frontend; qui stanno l'IDENTITÀ e la CRITTOGRAFIA: il frontend chiede a Rust di sigillare/aprire i
//! payload e non vede mai la chiave privata. La rubrica dei QR accettati (`PeerIndex`) è cifrata a riposo.
use crate::core::peer::{Peer, PeerIndex};
use crate::AppState;
use tauri::State;

/// L'ID pubblico di questo Liara (chiave pubblica X25519, base64url) — da mostrare nel QR e condividere.
#[tauri::command]
pub fn peer_identity(state: State<AppState>) -> String {
    state.peer.public_id().to_string()
}

/// Cifra `text` PER `peer_id` (E2E). Ritorna il payload base64url da spedire tal quale via relay.
#[tauri::command]
pub fn peer_seal(peer_id: String, text: String, state: State<AppState>) -> Result<String, String> {
    state.peer.seal(&peer_id, &text).map_err(|e| e.to_string())
}

/// Apre un payload ricevuto DA `peer_id`. Fallisce (non consegna) se manomesso o da mittente sbagliato.
#[tauri::command]
pub fn peer_open(peer_id: String, payload: String, state: State<AppState>) -> Result<String, String> {
    state.peer.open(&peer_id, &payload).map_err(|e| e.to_string())
}

/// I QR accettati (rubrica Liara). Su Android si affianca ai contatti nativi del telefono.
#[tauri::command]
pub fn peer_list(state: State<AppState>) -> Vec<Peer> {
    PeerIndex::list(&state.crypto)
}

/// Accetta/aggiorna un contatto Liara (dopo la scansione del QR). `added` = epoch ms dal frontend.
#[tauri::command]
pub fn peer_add(id: String, name: String, added: i64, state: State<AppState>) -> Result<Vec<Peer>, String> {
    if id.trim() == state.peer.public_id() {
        return Err("Non puoi aggiungere te stesso: quello è il TUO ID.".into());
    }
    PeerIndex::add(&state.crypto, &id, &name, added).map_err(|e| e.to_string())
}

/// Rimuove un contatto Liara dalla rubrica.
#[tauri::command]
pub fn peer_remove(id: String, state: State<AppState>) -> Result<Vec<Peer>, String> {
    PeerIndex::remove(&state.crypto, &id).map_err(|e| e.to_string())
}
