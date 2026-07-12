//! Tool layer: trait + registry + built-ins.
//! Uses Qwen's native tool-call format (the model is trained on it) and is ready
//! for GBNF hardening. External MCP servers plug into the same registry later.
mod builtin;
mod registry;
pub use registry::ToolRegistry;

use anyhow::Result;
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// A draft (to, subject, body) a tool wants the UI to open in the compose form.
pub type PendingCompose = Arc<Mutex<Option<(String, String, String)>>>;

/// Description of a tool exposed to the model.
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments object.
    pub parameters: Value,
}

/// A callable tool. Implementations are pure functions of their args + the host.
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn execute(&self, args: &Value) -> Result<String>;

    /// Sensitive tools (network, filesystem, external actions) require user consent.
    fn sensitive(&self) -> bool {
        false
    }

    /// A short, human, ARGUMENT-AWARE description of what the tool is about to do,
    /// shown in the consent prompt (e.g. "leggere il file ~/x.txt"). Default: generic.
    fn consent_action(&self, _args: &Value) -> String {
        self.spec().description
    }
}
