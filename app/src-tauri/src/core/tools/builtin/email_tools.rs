//! Email tools: read recent/sent, search, reply and draft (compose into the UI form).
use crate::core::email::{fetch_recent, EmailFull, EmailStore};
use crate::core::tools::{PendingCompose, Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

/// Extract the bare address from a "Name <addr@host>" sender string.
fn extract_addr(s: &str) -> String {
    if let Some(start) = s.find('<') {
        if let Some(end) = s[start..].find('>') {
            return s[start + 1..start + end].trim().to_string();
        }
    }
    s.trim().to_string()
}

fn fmt_mail(m: &EmailFull, body_chars: usize) -> String {
    let body: String = m.body.chars().take(body_chars).collect();
    format!(
        "Da: {}\nOggetto: {}\nData: {}\n{}\n",
        m.sender,
        m.subject,
        m.date,
        body.trim()
    )
}

/// Read the most recent received emails (already downloaded, local).
pub struct EmailRecent {
    pub store: Arc<EmailStore>,
}
impl Tool for EmailRecent {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "email_recent".into(),
            description: "Legge le email pi\u{00f9} recenti gi\u{00e0} scaricate dell'utente (mittente, oggetto, data, testo)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "count": { "type": "integer", "description": "Quante email leggere (default 3)" } },
                "required": []
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let n = args.get("count").and_then(|v| v.as_u64()).unwrap_or(3).clamp(1, 10) as usize;
        let mails = self.store.recent(n)?;
        if mails.is_empty() {
            return Ok("Nessuna email scaricata: l'utente deve configurare l'email e premere Scarica.".into());
        }
        Ok(mails.iter().map(|m| fmt_mail(m, 900)).collect::<Vec<_>>().join("---\n"))
    }
}

/// Read the most recent SENT emails (the user's outgoing mail).
pub struct EmailSent {
    pub store: Arc<EmailStore>,
}
impl Tool for EmailSent {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "email_sent".into(),
            description: "Legge le email INVIATE pi\u{00f9} recenti dall'utente (destinatario, oggetto, testo)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "count": { "type": "integer", "description": "Quante leggere (default 3)" } },
                "required": []
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let n = args.get("count").and_then(|v| v.as_u64()).unwrap_or(3).clamp(1, 10) as usize;
        // fetch the Sent folder live so it's never stale / empty
        let _ = fetch_recent(&self.store, "SENT", 15);
        let mails = self.store.recent_in("SENT", n)?;
        if mails.is_empty() {
            return Ok("Nessuna email inviata trovata (verifica la configurazione email).".into());
        }
        Ok(mails.iter().map(|m| fmt_mail(m, 900)).collect::<Vec<_>>().join("---\n"))
    }
}

/// Reply to the LAST received email — the model passes only the body text.
/// Recipient + subject are taken automatically from the most recent email.
pub struct EmailReply {
    pub store: Arc<EmailStore>,
    pub pending: PendingCompose,
}
impl Tool for EmailReply {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "email_reply".into(),
            description: "Risponde all'ULTIMA email ricevuta. Passa SOLO il testo della risposta: \
destinatario e oggetto vengono presi automaticamente dall'ultima email. \
Usa questo quando l'utente dice \u{00ab}rispondi\u{00bb}."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "body": { "type": "string", "description": "Testo della risposta" } },
                "required": ["body"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let mails = self.store.recent(1)?;
        let m = mails.first().ok_or_else(|| anyhow!("Nessuna email da cui rispondere."))?;
        let to = extract_addr(&m.sender);
        let subject = if m.subject.to_lowercase().starts_with("re:") {
            m.subject.clone()
        } else {
            format!("Re: {}", m.subject)
        };
        *self.pending.lock().unwrap() = Some((to.clone(), subject, body.to_string()));
        Ok(format!("Risposta pronta nel modulo email (a: {to}). Di' all'utente di rivederla e premere Invia."))
    }
}

/// Compose a brand-new email — the model provides recipient, subject and body.
pub struct EmailDraft {
    pub pending: PendingCompose,
}
impl Tool for EmailDraft {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "email_draft".into(),
            description: "Prepara una NUOVA email (destinatario, oggetto, testo) nel modulo di scrittura, \
pronta da rivedere e inviare. Usa questo quando l'utente chiede di scrivere un'email a qualcuno."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Indirizzo email del destinatario" },
                    "subject": { "type": "string", "description": "Oggetto del messaggio" },
                    "body": { "type": "string", "description": "Testo del messaggio" }
                },
                "required": ["to", "body"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let to_disp = to.clone();
        *self.pending.lock().unwrap() = Some((to, subject, body));
        Ok(format!("Bozza pronta (a: {to_disp}). Riepilogala all'utente e chiedi se INVIARLA: se conferma ('invia'), chiama email_send."))
    }
}

/// Invia DAVVERO l'email (SMTP). Usa la bozza preparata con email_draft, o gli argomenti dati.
/// Sensibile → consenso. Ritorna l'esito REALE: se l'invio fallisce, il modello DEVE dirlo (anti-fabbricazione).
pub struct EmailSend {
    pub store: Arc<EmailStore>,
    pub pending: PendingCompose,
}
impl Tool for EmailSend {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "email_send".into(),
            description: "INVIA l'email. Usalo SOLO quando l'utente conferma l'invio (es. 'invia', 'mandala', \
'spediscila') di una bozza già preparata con email_draft; se i campi non sono negli argomenti, usa quelli della bozza. \
Ritorna l'esito REALE: NON dire mai 'inviata' senza aver chiamato questo strumento e ottenuto conferma."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Destinatario (opzionale se c'è una bozza pronta)" },
                    "subject": { "type": "string", "description": "Oggetto (opzionale)" },
                    "body": { "type": "string", "description": "Testo (opzionale se c'è una bozza pronta)" }
                },
                "required": []
            }),
        }
    }
    fn sensitive(&self) -> bool { true }
    fn consent_action(&self, args: &Value) -> String {
        let draft = self.pending.lock().unwrap().clone();
        let to = args.get("to").and_then(|v| v.as_str()).map(|s| s.to_string())
            .or_else(|| draft.as_ref().map(|(t, _, _)| t.clone()))
            .unwrap_or_default();
        format!("inviare l'email a {}", if to.is_empty() { "?" } else { &to })
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let draft = self.pending.lock().unwrap().clone();
        let arg = |k: &str| args.get(k).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let to = arg("to").or_else(|| draft.as_ref().map(|(t, _, _)| t.clone())).unwrap_or_default();
        let subject = arg("subject").or_else(|| draft.as_ref().map(|(_, s, _)| s.clone())).unwrap_or_default();
        let body = arg("body").or_else(|| draft.as_ref().map(|(_, _, b)| b.clone())).unwrap_or_default();
        if to.is_empty() || body.is_empty() {
            return Err(anyhow!("Nessuna email da inviare: prepara prima la bozza con email_draft."));
        }
        crate::core::email::send_email(&self.store, &to, &subject, &body)?; // invio REALE, errore REALE
        *self.pending.lock().unwrap() = None; // bozza consumata
        Ok(format!("Email inviata a {to} (oggetto: \"{subject}\")."))
    }
}

/// Search the downloaded emails by sender/subject/body.
pub struct EmailSearch {
    pub store: Arc<EmailStore>,
}
impl Tool for EmailSearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "email_search".into(),
            description: "Cerca tra le email scaricate per mittente, oggetto o contenuto.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Parola, nome o oggetto da cercare" } },
                "required": ["query"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let q = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("manca il parametro 'query'"))?;
        let mails = self.store.search(q, 5)?;
        if mails.is_empty() {
            return Ok(format!("Nessuna email trovata per \"{q}\"."));
        }
        Ok(mails.iter().map(|m| fmt_mail(m, 500)).collect::<Vec<_>>().join("---\n"))
    }
}
