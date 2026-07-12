//! SMTP send: build the message and relay it via rustls TLS (no OpenSSL).
use super::EmailStore;
use anyhow::{anyhow, Result};

/// Send an email via SMTP using the stored config (rustls TLS, no OpenSSL).
pub fn send_email(store: &EmailStore, to: &str, subject: &str, body: &str) -> Result<()> {
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{Message, SmtpTransport, Transport};

    let cfg = store.get_config()?;
    let smtp_host = cfg
        .get("smtp_host")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Host SMTP mancante"))?;
    let smtp_port: u16 = cfg.get("smtp_port").and_then(|p| p.trim().parse().ok()).unwrap_or(465);
    let user = cfg
        .get("email")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Email mancante"))?;
    let pass = store.password()?;

    let email = Message::builder()
        .from(user.parse().map_err(|e| anyhow!("Mittente non valido: {e}"))?)
        .to(to.trim().parse().map_err(|e| anyhow!("Destinatario non valido: {e}"))?)
        .subject(subject)
        .body(body.to_string())
        .map_err(|e| anyhow!("Messaggio non valido: {e}"))?;

    let creds = Credentials::new(user.clone(), pass);
    let builder = if smtp_port == 587 {
        SmtpTransport::starttls_relay(&smtp_host)
    } else {
        SmtpTransport::relay(&smtp_host)
    }
    .map_err(|e| anyhow!("SMTP: {e}"))?;
    let mailer = builder.port(smtp_port).credentials(creds).build();
    mailer.send(&email).map_err(|e| anyhow!("Invio fallito: {e}"))?;
    Ok(())
}
