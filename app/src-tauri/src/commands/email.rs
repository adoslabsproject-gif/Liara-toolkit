//! Email commands: config, fetch (IMAP), list/get/delete and send (SMTP).
use crate::core::email::{fetch_recent, EmailFull, EmailSummary};
use crate::AppState;
use std::collections::HashMap;
use tauri::State;

#[tauri::command]
pub fn email_get_config(state: State<AppState>) -> HashMap<String, String> {
    let mut cfg = state.email.get_config().unwrap_or_default();
    cfg.remove("password"); // password lives in the OS keystore, never returned to the UI
    if state.email.has_password() {
        cfg.insert("__has_password".into(), "1".into());
    }
    cfg
}

#[tauri::command]
pub fn email_set_config(config: HashMap<String, String>, state: State<AppState>) -> Result<(), String> {
    state.email.set_config(config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn email_fetch(state: State<'_, AppState>) -> Result<usize, String> {
    let email = state.email.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<usize> {
        // INBOX errors (e.g. wrong password) must surface — not be hidden as "0 emails"
        let inbox = fetch_recent(&email, "INBOX", 30)?;
        let sent = fetch_recent(&email, "SENT", 30).unwrap_or(0);
        Ok(inbox + sent)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn email_list(state: State<AppState>) -> Vec<EmailSummary> {
    state.email.list().unwrap_or_default()
}

/// List a folder: "INBOX" | "SENT" | "DRAFTS"; "TRASH" returns soft-deleted emails.
#[tauri::command]
pub fn email_list_folder(folder: String, state: State<AppState>) -> Vec<EmailSummary> {
    if folder == "TRASH" {
        state.email.list_trash().unwrap_or_default()
    } else {
        state.email.list_in(&folder).unwrap_or_default()
    }
}

#[tauri::command]
pub fn email_restore(id: i64, state: State<AppState>) -> Result<(), String> {
    state.email.restore(id).map_err(|e| e.to_string())
}

/// Permanently delete one email, or empty the whole Trash with id = 0.
#[tauri::command]
pub fn email_purge(id: i64, state: State<AppState>) -> Result<(), String> {
    state.email.purge(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn email_get(id: i64, state: State<AppState>) -> Option<EmailFull> {
    state.email.get(id).ok().flatten()
}

#[tauri::command]
pub fn email_delete(id: i64, state: State<AppState>) -> Result<(), String> {
    state.email.delete(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn email_send(to: String, subject: String, body: String, state: State<'_, AppState>) -> Result<(), String> {
    let email = state.email.clone();
    tauri::async_runtime::spawn_blocking(move || crate::core::email::send_email(&email, &to, &subject, &body))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
