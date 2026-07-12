//! Calendar commands: list, create and remove events.
use crate::core::calendar::Event;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub fn calendar_events(state: State<AppState>) -> Vec<Event> {
    state.calendar.all(100).unwrap_or_default()
}

#[tauri::command]
pub fn calendar_create(title: String, when: String, notes: String, state: State<AppState>) -> Result<i64, String> {
    state.calendar.add(&title, &when, &notes).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn calendar_remove(id: i64, state: State<AppState>) -> Result<(), String> {
    state.calendar.delete(id).map_err(|e| e.to_string())
}
