//! `WindowSink`: l'unica implementazione di `AgentSink` (review round-3 #4). Traduce gli eventi
//! dell'agente in eventi Tauri verso la WebView e applica il gate di consenso (permesso salvato,
//! altrimenti chiede all'utente). Vive QUI e non nel core perché conosce Tauri/Memory/ConsentGate —
//! il core resta puro. `generate.rs` e `vision.rs` la usano ENTRAMBI: niente più wiring duplicato.
use std::sync::Arc;

use tauri::{Emitter, WebviewWindow};

use crate::commands::consent::ConsentGate;
use crate::core::agent::AgentSink;
use crate::core::memory::Memory;

pub(crate) struct WindowSink {
    window: WebviewWindow,
    memory: Arc<Memory>,
    consent: Arc<ConsentGate>,
}

impl WindowSink {
    pub(crate) fn new(window: WebviewWindow, memory: Arc<Memory>, consent: Arc<ConsentGate>) -> Self {
        Self { window, memory, consent }
    }
}

impl AgentSink for WindowSink {
    fn on_token(&mut self, piece: &str) {
        let _ = self.window.emit("token", piece);
    }
    fn on_tool(&mut self, name: &str, args: &str) {
        let _ = self.window.emit("tool", serde_json::json!({ "name": name, "args": args }));
    }
    fn on_tool_result(&mut self, name: &str, result: &str) {
        let _ = self.window.emit("tool-result", serde_json::json!({ "name": name, "result": result }));
    }
    fn on_consent(&mut self, tool: &str, action: &str) -> bool {
        // consenso lato server: decisione salvata, oppure chiedi interattivamente all'utente.
        match self.memory.get_permission(tool).as_deref() {
            Some("allow") => true,
            Some("deny") => false,
            _ => {
                let _ = self
                    .window
                    .emit("consent-request", serde_json::json!({ "tool": tool, "action": action }));
                self.consent.request()
            }
        }
    }
}
