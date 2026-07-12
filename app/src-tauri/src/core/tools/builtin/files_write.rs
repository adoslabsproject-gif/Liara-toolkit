//! Mutating file tools (write / move / delete) — confined to the home dir and consent-gated.
use super::files::safe_path;
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

fn arg<'a>(args: &'a Value, k: &str) -> Result<&'a str> {
    args.get(k).and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca '{k}'"))
}

pub struct FsWrite;
impl Tool for FsWrite {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs_write".into(),
            description: "Crea o sovrascrive un file di testo nella home dell'utente. Indica path e content.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Percorso del file, es. ~/note/lista.txt" },
                    "content": { "type": "string", "description": "Contenuto da scrivere" }
                },
                "required": ["path", "content"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let file = safe_path(arg(args, "path")?)?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(&file, content)?;
        Ok(format!("Scritto {} ({} byte).", file.display(), content.len()))
    }
    fn sensitive(&self) -> bool {
        true
    }
    fn consent_action(&self, args: &Value) -> String {
        format!("scrivere il file {}", args.get("path").and_then(|v| v.as_str()).unwrap_or("?"))
    }
}

pub struct FsMove;
impl Tool for FsMove {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs_move".into(),
            description: "Sposta o rinomina un file nella home dell'utente. Indica from e to.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Percorso attuale" },
                    "to": { "type": "string", "description": "Nuovo percorso o nome" }
                },
                "required": ["from", "to"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let from = safe_path(arg(args, "from")?)?;
        let to = safe_path(arg(args, "to")?)?;
        if !from.exists() {
            return Err(anyhow!("Origine inesistente: {}", from.display()));
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::rename(&from, &to)?;
        Ok(format!("Spostato {} → {}", from.display(), to.display()))
    }
    fn sensitive(&self) -> bool {
        true
    }
    fn consent_action(&self, args: &Value) -> String {
        format!(
            "spostare {} in {}",
            args.get("from").and_then(|v| v.as_str()).unwrap_or("?"),
            args.get("to").and_then(|v| v.as_str()).unwrap_or("?")
        )
    }
}

pub struct FsDelete;
impl Tool for FsDelete {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs_delete".into(),
            description: "Elimina un file nella home dell'utente. Indica path.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Percorso del file da eliminare" } },
                "required": ["path"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let file = safe_path(arg(args, "path")?)?;
        if !file.is_file() {
            return Err(anyhow!("Non è un file: {}", file.display()));
        }
        std::fs::remove_file(&file)?;
        Ok(format!("Eliminato {}", file.display()))
    }
    fn sensitive(&self) -> bool {
        true
    }
    fn consent_action(&self, args: &Value) -> String {
        format!("ELIMINARE il file {}", args.get("path").and_then(|v| v.as_str()).unwrap_or("?"))
    }
}
