//! Notes (appunti) tools: save notes Liara remembers, then recall them — she reorganizes
//! them into tables / charts / HTML on request (e.g. lecture notes → a summary table).
use crate::core::memory::Memory;
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

fn fmt_notes(notes: &[(i64, String, String)]) -> String {
    if notes.is_empty() {
        return "Nessun appunto trovato.".into();
    }
    notes
        .iter()
        .map(|(id, topic, text)| format!("#{id} [{topic}] {text}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct NoteAdd {
    pub mem: Arc<Memory>,
}
impl Tool for NoteAdd {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "note_add".into(),
            description: "Salva un appunto che ricorderai. Indica argomento (topic) e testo. \
Per appunti di studio, idee, liste — poi potrai riorganizzarli in tabelle, grafici o HTML."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string", "description": "Argomento/categoria dell'appunto" },
                    "text": { "type": "string", "description": "Il contenuto dell'appunto" }
                },
                "required": ["topic", "text"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let topic = args.get("topic").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'topic'"))?;
        let text = args.get("text").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'text'"))?;
        let id = self.mem.add_note(topic, text)?;
        Ok(format!("Appunto salvato (#{id}) in «{topic}»."))
    }
}

pub struct NoteList {
    pub mem: Arc<Memory>,
}
impl Tool for NoteList {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "note_list".into(),
            description: "Elenca gli appunti salvati, opzionalmente filtrati per argomento. \
Usalo prima di riorganizzarli in tabella, grafico o HTML."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "topic": { "type": "string", "description": "Filtra per argomento (opzionale)" } },
                "required": []
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let topic = args.get("topic").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
        Ok(fmt_notes(&self.mem.list_notes(topic)?))
    }
}

pub struct NoteSearch {
    pub mem: Arc<Memory>,
}
impl Tool for NoteSearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "note_search".into(),
            description: "Cerca tra gli appunti per parola chiave (argomento o testo).".into(),
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Parola/e da cercare" } },
                "required": ["query"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'query'"))?;
        Ok(fmt_notes(&self.mem.search_notes(query, 20)?))
    }
}
