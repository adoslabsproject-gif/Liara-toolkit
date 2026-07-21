//! Calendar / agenda tools: add, list, search and delete events.
use crate::core::calendar::{Calendar, Event};
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

fn fmt_event(e: &Event) -> String {
    if e.notes.trim().is_empty() {
        format!("#{} — {} ({})", e.id, e.title, e.when_str)
    } else {
        format!("#{} — {} ({}) [{}]", e.id, e.title, e.when_str, e.notes)
    }
}

pub struct CalendarAdd {
    pub cal: Arc<Calendar>,
}
impl Tool for CalendarAdd {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "calendar_add".into(),
            description: "Crea un evento o appuntamento nell'agenda. Indica titolo e data/ora \
(formato AAAA-MM-GG HH:MM) ed eventuali note."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Titolo dell'evento" },
                    "when": { "type": "string", "description": "Data e ora, es. 2026-06-27 15:00" },
                    "notes": { "type": "string", "description": "Note opzionali" }
                },
                "required": ["title", "when"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let title = args.get("title").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'title'"))?;
        let when = args.get("when").and_then(|v| v.as_str()).unwrap_or("");
        let notes = args.get("notes").and_then(|v| v.as_str()).unwrap_or("");
        let id = self.cal.add(title, when, notes)?;
        Ok(format!("Evento creato (#{id}): {title} — {when}"))
    }
}

pub struct CalendarList {
    pub cal: Arc<Calendar>,
}
impl Tool for CalendarList {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "calendar_list".into(),
            description: "Elenca i prossimi eventi/appuntamenti in agenda.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "count": { "type": "integer", "description": "Quanti elencare (default 10)" } },
                "required": []
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let n = args.get("count").and_then(|v| v.as_u64()).unwrap_or(10).clamp(1, 50) as i64;
        let events = self.cal.upcoming(n)?;
        if events.is_empty() {
            return Ok("L'agenda \u{00e8} vuota: nessun evento.".into());
        }
        Ok(events.iter().map(fmt_event).collect::<Vec<_>>().join("\n"))
    }
}

pub struct CalendarSearch {
    pub cal: Arc<Calendar>,
}
impl Tool for CalendarSearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "calendar_search".into(),
            description: "Cerca eventi in agenda per titolo o note.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Parola da cercare" } },
                "required": ["query"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let q = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'query'"))?;
        let events = self.cal.search(q, 20)?;
        if events.is_empty() {
            return Ok(format!("Nessun evento trovato per \"{q}\"."));
        }
        Ok(events.iter().map(fmt_event).collect::<Vec<_>>().join("\n"))
    }
}

pub struct CalendarDelete {
    pub cal: Arc<Calendar>,
}
impl Tool for CalendarDelete {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "calendar_delete".into(),
            description: "Elimina un evento dall'agenda dato il suo numero (id). Se non sai l'id, prima elenca/cerca."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "Numero dell'evento" } },
                "required": ["id"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let id = args.get("id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow!("manca 'id'"))?;
        self.cal.delete(id)?;
        Ok(format!("Evento #{id} eliminato."))
    }
}

/// MODIFICA un appuntamento esistente (in-place, stesso id): cambia data/ora (sposta), titolo
/// (rinomina) e/o note. Un tool solo per ogni edit → per "sposta" il modello NON deve più fare
/// add+delete (che sbagliava lasciando doppioni): passa id + i soli campi da cambiare. Usa
/// `Calendar::update` (il metodo core). Sostituisce l'idea del vecchio calendar_move (più stretto).
pub struct CalendarUpdate {
    pub cal: Arc<Calendar>,
}
impl Tool for CalendarUpdate {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "calendar_update".into(),
            description: "Modifica un appuntamento ESISTENTE dato il suo numero (id): cambia la data/ora \
(per SPOSTARLO, formato AAAA-MM-GG HH:MM), il titolo (per rinominarlo) e/o le note. Passa SOLO i campi \
da cambiare. Usa SEMPRE questo per spostare/modificare un evento (l'appuntamento NON si duplica). Se \
non sai l'id, prima elenca/cerca."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Numero dell'evento da modificare" },
                    "when": { "type": "string", "description": "Nuova data e ora, es. 2026-07-25 16:00 (per spostarlo)" },
                    "title": { "type": "string", "description": "Nuovo titolo (per rinominarlo)" },
                    "notes": { "type": "string", "description": "Nuove note" }
                },
                "required": ["id"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let id = args.get("id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow!("manca 'id'"))?;
        let when = args.get("when").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty());
        let title = args.get("title").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty());
        let notes = args.get("notes").and_then(|v| v.as_str()); // note vuote = cancella le note (valido)
        if when.is_none() && title.is_none() && notes.is_none() {
            return Err(anyhow!("niente da modificare: indica almeno when, title o notes"));
        }
        self.cal.update(id, title, when, notes)?;
        // messaggio parlante di cosa è cambiato
        let mut done = Vec::new();
        if let Some(w) = when { done.push(format!("spostato a {w}")); }
        if let Some(t) = title { done.push(format!("rinominato in \"{t}\"")); }
        if notes.is_some() { done.push("note aggiornate".into()); }
        Ok(format!("Evento #{id} {} (nessun doppione).", done.join(", ")))
    }
}
