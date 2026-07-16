//! Memory / profile / conversation persistence commands, plus the GPS setter.
use crate::AppState;
use tauri::State;

#[tauri::command]
pub fn memory_facts(state: State<AppState>) -> Vec<String> {
    state.memory.facts().unwrap_or_default()
}

#[tauri::command]
pub fn add_fact(text: String, state: State<AppState>) -> Result<bool, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(false);
    }
    state.memory.add_fact(t).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn forget_all(state: State<AppState>) -> Result<(), String> {
    state.memory.forget_all().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_fact(text: String, state: State<AppState>) -> Result<(), String> {
    state.memory.delete_fact(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_profile(state: State<AppState>) -> Vec<(String, String)> {
    state.memory.profile_entries().unwrap_or_default()
}

#[tauri::command]
pub fn set_profile(key: String, value: String, state: State<AppState>) -> Result<(), String> {
    state.memory.set_profile(&key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_conversations(state: State<AppState>) -> Vec<(String, String, i64)> {
    state.memory.list_conversations().unwrap_or_default()
}

#[tauri::command]
pub fn save_conversation(id: String, title: String, data: String, state: State<AppState>) -> Result<(), String> {
    state.memory.save_conversation(&id, &title, &data).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_conversation(id: String, state: State<AppState>) -> Result<Option<String>, String> {
    state.memory.load_conversation(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_conversation(id: String, state: State<AppState>) -> Result<(), String> {
    state.memory.delete_conversation(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_gps(latitude: f64, longitude: f64, state: State<AppState>) -> Result<(), String> {
    // a fresh device GPS fix; overrides a manual correction (per the user's wish)
    state.memory.set_location(latitude, longitude, "posizione GPS", "gps").map_err(|e| e.to_string())
}

/// Posizione corrente per la UI Impostazioni: (label, source) con source ∈ {"gps","manual"}. None se assente.
#[tauri::command]
pub fn get_location(state: State<AppState>) -> Option<(String, String)> {
    state.memory.location_display()
}

/// ID di rete di questo Liara (per la chat AI↔AI). È la CHIAVE PUBBLICA X25519 (base64url), stabile
/// per-dispositivo e base dell'E2E — la stessa che `peer_identity` restituisce. Manteniamo il nome
/// storico per il NetDrawer; il Milestone 1 (ID casuale) è superato dall'identità crittografica reale.
#[tauri::command]
pub fn my_network_id(state: State<AppState>) -> Result<String, String> {
    Ok(state.peer.public_id().to_string())
}

/// Posizione MANUALE: l'utente scrive una città → geocoding (Open-Meteo) → salva con source "manual".
/// Il bottone "Sincronizza" nel frontend rimette invece quella GPS (set_gps → source "gps").
#[tauri::command]
pub async fn set_manual_location(city: String, state: State<'_, AppState>) -> Result<String, String> {
    let city = city.trim().to_string();
    if city.is_empty() {
        return Err("Città vuota".into());
    }
    let memory = state.memory.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let (lat, lon, label) = crate::core::tools::builtin::weather::geocode(&city)
            .map_err(|e| format!("Luogo non trovato: {e}"))?;
        memory.set_location(lat, lon, &label, "manual").map_err(|e| e.to_string())?;
        Ok(label)
    })
    .await
    .map_err(|e| e.to_string())?
}
