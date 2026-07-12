//! Interactive consent gate + the permission-management commands.
use crate::AppState;
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use tauri::State;

/// Interactive consent gate: the agent blocks here until the user responds in the UI
/// (or a timeout denies). Server-enforced — the model cannot bypass it.
#[derive(Default)]
pub(crate) struct ConsentGate {
    answer: Mutex<Option<bool>>,
    cv: Condvar,
}
impl ConsentGate {
    /// Block until the user decides; deny on timeout (90s).
    pub(crate) fn request(&self) -> bool {
        let mut guard = self.answer.lock().unwrap();
        *guard = None;
        let (mut guard, _) = self
            .cv
            .wait_timeout_while(guard, std::time::Duration::from_secs(90), |a| a.is_none())
            .unwrap();
        guard.take().unwrap_or(false)
    }
    pub(crate) fn respond(&self, allow: bool) {
        *self.answer.lock().unwrap() = Some(allow);
        self.cv.notify_all();
    }
}

#[tauri::command]
pub fn consent_respond(allow: bool, remember: bool, tool: String, state: State<AppState>) {
    if remember {
        let _ = state.memory.set_permission(&tool, if allow { "allow" } else { "deny" });
    }
    state.consent.respond(allow);
}

/// (tool, description, permission-state) for every sensitive tool — for the permissions UI.
#[tauri::command]
pub fn permissions(state: State<AppState>) -> Vec<(String, String, String)> {
    let set: HashMap<String, String> = state.memory.list_permissions().into_iter().collect();
    state
        .tools
        .sensitive_tools()
        .into_iter()
        .map(|(name, desc)| {
            let st = set.get(&name).cloned().unwrap_or_else(|| "ask".into());
            (name, desc, st)
        })
        .collect()
}

#[tauri::command]
pub fn set_permission(tool: String, value: String, state: State<AppState>) -> Result<(), String> {
    state.memory.set_permission(&tool, &value).map_err(|e| e.to_string())
}
