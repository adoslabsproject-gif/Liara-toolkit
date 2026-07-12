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
