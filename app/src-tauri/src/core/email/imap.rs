//! IMAP fetch: connect over rustls TLS, select the folder, and store recent messages.
use super::EmailStore;
use anyhow::{anyhow, Result};
use mailparse::MailHeaderMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

/// Timeout della connessione IMAP e delle letture/scritture. Senza, un server che accetta la TCP ma
/// poi resta muto (o una rete che cade a metà) congelava `email_fetch` PER SEMPRE — e `email_sent`
/// fa il fetch DENTRO il turno del modello, quindi avrebbe congelato la risposta in chat.
const IMAP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IMAP_IO_TIMEOUT: Duration = Duration::from_secs(30);

fn header(p: &mailparse::ParsedMail, name: &str) -> String {
    p.headers.get_first_value(name).unwrap_or_default()
}

/// Prefer text/plain; fall back to any text body.
fn extract_text(p: &mailparse::ParsedMail) -> String {
    if p.subparts.is_empty() {
        return p.get_body().unwrap_or_default();
    }
    for sp in &p.subparts {
        if sp.ctype.mimetype == "text/plain" {
            if let Ok(b) = sp.get_body() {
                if !b.trim().is_empty() {
                    return b;
                }
            }
        }
    }
    for sp in &p.subparts {
        let t = extract_text(sp);
        if !t.trim().is_empty() {
            return t;
        }
    }
    p.get_body().unwrap_or_default()
}

/// Connect via IMAP using the stored config and store the most recent emails.
/// Returns how many new emails were stored.
pub fn fetch_recent(store: &EmailStore, folder: &str, limit: u32) -> Result<usize> {
    let cfg = store.get_config()?;
    let host = cfg
        .get("imap_host")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Host IMAP mancante"))?;
    let host = host.split(':').next().unwrap_or(&host).trim().to_string();
    let port: u16 = cfg
        .get("imap_port")
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(993);
    let user = cfg
        .get("email")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Email mancante"))?;
    let pass = store.password()?;

    // rustls TLS (pure Rust, cross-compiles for Android — no OpenSSL)
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| anyhow!("TLS config: {e}"))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(host.clone())
        .map_err(|_| anyhow!("Host non valido"))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| anyhow!("TLS init: {e}"))?;
    // connect_timeout vuole un SocketAddr risolto → niente hang sulla connessione; poi read/write
    // timeout così una lettura non termina mai in un'attesa infinita.
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| anyhow!("DNS IMAP fallito per {host}: {e}"))?
        .next()
        .ok_or_else(|| anyhow!("host IMAP non risolto: {host}"))?;
    let tcp = TcpStream::connect_timeout(&addr, IMAP_CONNECT_TIMEOUT)
        .map_err(|e| anyhow!("Connessione TCP fallita: {e}"))?;
    tcp.set_read_timeout(Some(IMAP_IO_TIMEOUT)).ok();
    tcp.set_write_timeout(Some(IMAP_IO_TIMEOUT)).ok();
    let tls = rustls::StreamOwned::new(conn, tcp);

    let mut client = imap::Client::new(tls);
    client
        .read_greeting()
        .map_err(|e| anyhow!("Greeting IMAP: {e}"))?;
    let mut session = client
        .login(user.as_str(), pass.as_str())
        .map_err(|e| anyhow!("Login fallito: {}. Per Gmail/Yahoo serve una PASSWORD PER LE APP (non quella dell'account).", e.0))?;

    let imap_name = if folder == "SENT" {
        let by_attr: Option<String> = {
            let names = session.list(Some(""), Some("*"))?;
            names
                .iter()
                .find(|n| {
                    n.attributes().iter().any(|a| {
                        matches!(a, imap::types::NameAttribute::Custom(s) if s.eq_ignore_ascii_case("\\Sent"))
                    })
                })
                .map(|n| n.name().to_string())
        };
        match by_attr {
            Some(n) => n,
            None => {
                let candidates = [
                    "[Gmail]/Sent Mail",
                    "[Gmail]/Posta inviata",
                    "Sent",
                    "Sent Items",
                    "Posta inviata",
                    "INBOX.Sent",
                ];
                match candidates.iter().find(|c| session.select(c).is_ok()) {
                    Some(c) => (*c).to_string(),
                    None => {
                        let _ = session.logout();
                        return Ok(0);
                    }
                }
            }
        }
    } else {
        "INBOX".to_string()
    };

    let mailbox = session.select(&imap_name)?;
    let total = mailbox.exists;
    if total == 0 {
        let _ = session.logout();
        return Ok(0);
    }
    let start = total.saturating_sub(limit) + 1;
    let seq = format!("{start}:{total}");
    // BODY.PEEK[] invece di RFC822: scarica il messaggio SENZA settare il flag \Seen sul server, così
    // le email non risultano "già lette" su tutti gli altri client dell'utente. `.body()` restituisce
    // comunque la sezione BODY[] (il server toglie il .PEEK nella risposta).
    let fetches = session.fetch(seq, "(BODY.PEEK[] UID)")?;

    let mut stored = 0usize;
    for msg in fetches.iter() {
        let uid = msg.uid.unwrap_or(0);
        if let Some(body) = msg.body() {
            if let Ok(parsed) = mailparse::parse_mail(body) {
                let subject = header(&parsed, "Subject");
                let from = header(&parsed, "From");
                let date = header(&parsed, "Date");
                let text = extract_text(&parsed);
                if store.store_email(uid, folder, &from, &subject, &text, &date)? {
                    stored += 1;
                }
            }
        }
    }
    let _ = session.logout();
    Ok(stored)
}
