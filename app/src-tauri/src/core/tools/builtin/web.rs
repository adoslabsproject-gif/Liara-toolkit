//! Web tools: page fetch (SSRF-guarded) and web search (SearXNG or DuckDuckGo).
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

/// Builder unico degli agent HTTP (dedup review #5): certificati CA INCLUSI
/// (webpki-roots), non quelli di sistema — su Android lo store CA nativo non si
/// legge in modo affidabile → l'HTTPS falliva ("non raggiungo il sito").
fn agent_builder() -> ureq::AgentBuilder {
    let root_store = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    ureq::AgentBuilder::new().tls_config(Arc::new(config))
}

/// Agent per ENDPOINT FISSI/di config (DuckDuckGo, SearXNG — che può essere in
/// LAN by design): redirect standard, NESSUN filtro IP. `pub(super)` perché anche
/// weather.rs (meteo/geocoding/geo-IP, endpoint pubblici fissi) deve usare QUESTO
/// agent coi root CA inclusi — su Android lo store CA nativo è inaffidabile e
/// `ureq::get` nudo falliva l'HTTPS.
pub(super) fn http_agent() -> ureq::Agent {
    agent_builder().build()
}

/// Resolver anti DNS-REBINDING (review 2026-07-02 #4): il filtro IP vive DENTRO
/// la risoluzione usata per CONNETTERSI. Prima il check (is_blocked_host) e la
/// connessione (ureq) risolvevano il DNS due volte: un DNS malevolo poteva
/// rispondere pubblico al check e privato alla connessione (TOCTOU). Così anche
/// se il DNS cambia risposta, un IP privato non viene MAI contattato.
struct SafeResolver;
impl ureq::Resolver for SafeResolver {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<std::net::SocketAddr>> {
        use std::net::ToSocketAddrs;
        let addrs: Vec<std::net::SocketAddr> = netloc.to_socket_addrs()?.collect();
        let safe: Vec<std::net::SocketAddr> =
            addrs.into_iter().filter(|a| !is_blocked_ip(a.ip())).collect();
        if safe.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "host risolve solo a IP privati/riservati (guard SSRF)",
            ));
        }
        Ok(safe)
    }
}

/// Extract the host from a URL (no external crate). Gli IPv6 letterali stanno
/// tra parentesi quadre (`http://[::1]:8080/`): vanno estratti PRIMA dello split
/// su ':' — altrimenti l'host diventava "[" e il guard SSRF non scattava mai.
fn host_of(url: &str) -> Option<String> {
    let after = url.split("://").nth(1).unwrap_or(url);
    if let Some(rest) = after.strip_prefix('[') {
        let end = rest.find(']')?;
        let h = &rest[..end];
        return if h.is_empty() { None } else { Some(format!("[{h}]")) };
    }
    let host = after.split(['/', ':', '?', '#']).next()?;
    if host.is_empty() { None } else { Some(host.to_string()) }
}

/// Regole IP anti-SSRF (pura → testabile): loopback / non specificato / privato /
/// link-local (IMDS 169.254…) per v4; loopback / unspecified / ULA fc00::/7 /
/// link-local fe80::/10 / IPv4-mapped (ricorsione sulle regole v4) per v6.
fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_unspecified() || v4.is_private() || v4.is_link_local()
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return true;
            }
            let seg = v6.segments();
            // fc00::/7 (unique-local) — la "LAN privata" di IPv6, prima non coperta
            if (seg[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // fe80::/10 (link-local)
            if (seg[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // ::ffff:a.b.c.d (IPv4-mapped): applica le regole v4
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(std::net::IpAddr::V4(v4));
            }
            false
        }
    }
}

/// Anti-SSRF: block loopback / private / link-local (IMDS) hosts.
fn is_blocked_host(host: &str) -> bool {
    use std::net::ToSocketAddrs;
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    // IPv6 letterale: to_socket_addrs vuole la forma senza parentesi
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return is_blocked_ip(ip);
    }
    if let Ok(addrs) = (bare, 80u16).to_socket_addrs() {
        for a in addrs {
            if is_blocked_ip(a.ip()) {
                return true;
            }
        }
    }
    false
}

const MAX_REDIRECT_HOPS: usize = 4;
const MAX_BODY_BYTES: u64 = 1_048_576; // 1MB: il testo utile è troncato molto prima

/// GET con guard SSRF applicato a OGNI hop di redirect (review 2026-07-02 #5):
/// prima i redirect erano seguiti da ureq SENZA ricontrollo → un sito pubblico
/// poteva fare 302 verso 192.168.x.x / 169.254.169.254 e bypassare il guard.
fn get_checked(url: &str) -> Result<ureq::Response> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECT_HOPS {
        let host = host_of(&current).ok_or_else(|| anyhow!("URL non valido"))?;
        if is_blocked_host(&host) {
            return Err(anyhow!("URL non consentito (host locale o privato)"));
        }
        let resp = match no_redirect_agent().get(&current).timeout(std::time::Duration::from_secs(8)).call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(anyhow!("Richiesta fallita: {e}")),
        };
        if matches!(resp.status(), 301 | 302 | 303 | 307 | 308) {
            let loc = resp.header("location").ok_or_else(|| anyhow!("redirect senza Location"))?;
            current = resolve_location(&current, loc);
            continue;
        }
        return Ok(resp);
    }
    Err(anyhow!("Troppi redirect"))
}

/// Agent per URL ARBITRARI (get_checked): auto-redirect OFF (i redirect li segue
/// get_checked ricontrollando ogni hop) + SafeResolver (anti DNS-rebinding: il
/// filtro IP è nella risoluzione stessa della connessione).
fn no_redirect_agent() -> ureq::Agent {
    agent_builder().redirects(0).resolver(SafeResolver).build()
}

/// Risolve un header Location (assoluto, protocol-relative, absolute-path o
/// relativo) rispetto all'URL corrente. Pura → testabile.
fn resolve_location(base: &str, loc: &str) -> String {
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return loc.to_string();
    }
    let scheme = if base.starts_with("http://") { "http" } else { "https" };
    if let Some(rest) = loc.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    let host = host_of(base).unwrap_or_default();
    if loc.starts_with('/') {
        return format!("{scheme}://{host}{loc}");
    }
    // relativo alla directory del path corrente
    let path = base.splitn(4, '/').nth(3).unwrap_or("");
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => format!("{scheme}://{host}/{dir}/{loc}"),
        _ => format!("{scheme}://{host}/{loc}"),
    }
}

/// Corpo della risposta come testo, CAPPATO a MAX_BODY_BYTES (anti-OOM: una
/// risposta enorme/streaming non deve gonfiare la RAM del telefono).
fn body_text_capped(resp: ureq::Response) -> Result<String> {
    use std::io::Read;
    let mut buf: Vec<u8> = Vec::new();
    resp.into_reader()
        .take(MAX_BODY_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| anyhow!("Lettura fallita: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Fetch a web page and return its readable text. Online tool, SSRF-guarded.
pub struct WebFetch;
impl Tool for WebFetch {
    fn sensitive(&self) -> bool {
        // Niente gate di consenso sul web: il consenso interattivo su mobile bloccava la chiamata e
        // il modello rispondeva "non posso navigare". La navigazione è esplicitamente richiesta dall'utente.
        false
    }
    fn consent_action(&self, args: &Value) -> String {
        format!("visitare la pagina web {}", args.get("url").and_then(|v| v.as_str()).unwrap_or("?"))
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_fetch".into(),
            description: "Scarica una pagina web da un URL e restituisce il testo leggibile (per leggere articoli o pagine)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "url": { "type": "string", "description": "L'URL da visitare, es. https://esempio.com" } },
                "required": ["url"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let raw = args.get("url").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'url'"))?;
        let url = if raw.starts_with("http://") || raw.starts_with("https://") {
            raw.to_string()
        } else {
            format!("https://{raw}")
        };
        let host = host_of(&url).ok_or_else(|| anyhow!("URL non valido"))?;
        if is_blocked_host(&host) {
            return Err(anyhow!("URL non consentito (host locale o privato)"));
        }
        // AI-native: prefer the site's llms.txt (concise machine-readable summary) if present
        let scheme = if url.starts_with("http://") { "http" } else { "https" };
        let llms_url = format!("{scheme}://{host}/llms.txt");
        if let Ok(r) = get_checked(&llms_url) {
            if r.status() == 200 {
                if let Ok(s) = body_text_capped(r) {
                    let s = s.trim();
                    if s.starts_with('#') || s.len() > 80 {
                        let s: String = s.chars().take(3500).collect();
                        return Ok(format!("Da {llms_url} (descrizione per AI del sito):\n\n{s}"));
                    }
                }
            }
        }

        let resp = get_checked(&url)?;
        if resp.status() >= 400 {
            return Err(anyhow!("Richiesta fallita: HTTP {}", resp.status()));
        }
        let html = body_text_capped(resp)?;
        let text = html2text::from_read(html.as_bytes(), 100);
        let text: String = text.chars().take(4000).collect();
        Ok(format!("Contenuto di {url}:\n\n{}", text.trim()))
    }
}

pub(super) fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Scarica un URL e ne estrae il testo leggibile (html2text), troncato a ~2500 char. None se fallisce
/// o host bloccato. Usata da web_search per servire SUBITO il contenuto del 1° risultato: il modello
/// piccolo spesso non fa il secondo web_fetch e inventa → glielo mettiamo già davanti.
fn fetch_readable(url: &str) -> Option<String> {
    // get_checked ricontrolla l'host a ogni hop di redirect (i risultati di una
    // ricerca sono URL arbitrari: senza, un 302 verso la LAN bypassava il guard).
    let resp = get_checked(url).ok()?;
    if resp.status() >= 400 {
        return None;
    }
    let html = body_text_capped(resp).ok()?;
    let text = html2text::from_read(html.as_bytes(), 100);
    let text: String = text.chars().take(2500).collect();
    let text = text.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

pub struct WebSearch;
impl Tool for WebSearch {
    fn sensitive(&self) -> bool {
        // Niente gate di consenso sul web: il consenso interattivo su mobile bloccava la chiamata e
        // il modello rispondeva "non posso navigare". La navigazione è esplicitamente richiesta dall'utente.
        false
    }
    fn consent_action(&self, args: &Value) -> String {
        format!("cercare sul web: {}", args.get("query").and_then(|v| v.as_str()).unwrap_or("?"))
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description: "Cerca sul web e restituisce i primi risultati (per trovare informazioni aggiornate). Poi puoi usare web_fetch su un link.".into(),
            parameters: json!({ "type": "object", "properties": { "query": { "type": "string", "description": "Cosa cercare" } }, "required": ["query"] }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let q = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'query'"))?;
        let base = std::env::var("LIARA_SEARXNG").unwrap_or_default();
        if base.trim().is_empty() {
            // free path: DuckDuckGo done right (POST like the real browser form)
            let results = ddg_search(q)?;
            if results.is_empty() {
                return Ok(format!(
                    "Nessun risultato per \"{q}\" (la ricerca pubblica pu\u{00f2} essere temporaneamente limitata). \
Per ricerca sempre affidabile, configura LIARA_SEARXNG."
                ));
            }
            let out: String = results
                .iter()
                .take(6)
                .enumerate()
                .map(|(i, (title, link, snippet))| format!("{}. {title}\n   {snippet}\n   {link}", i + 1))
                .collect::<Vec<_>>()
                .join("\n\n");
            return Ok(format!("Risultati web per \"{q}\":\n\n{out}"));
        }
        let url = format!(
            "{}/search?q={}&format=json&language=it",
            base.trim().trim_end_matches('/'),
            urlencode(q)
        );
        let resp = http_agent().get(&url)
            .set("User-Agent", "Liara/1.0")
            .timeout(std::time::Duration::from_secs(8))
            .call()
            .map_err(|e| anyhow!("Ricerca fallita (SearXNG raggiungibile?): {e}"))?;
        let body = resp.into_string().map_err(|e| anyhow!("Lettura fallita: {e}"))?;
        let json: Value = serde_json::from_str(&body)
            .map_err(|_| anyhow!("Risposta non-JSON da SearXNG (abilita il formato json nell'istanza)."))?;
        let empty = Vec::new();
        let results = json.get("results").and_then(|v| v.as_array()).unwrap_or(&empty);
        if results.is_empty() {
            return Ok(format!("Nessun risultato web per \"{q}\"."));
        }
        let out: String = results
            .iter()
            .take(6)
            .enumerate()
            .map(|(i, r)| {
                let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let content = r.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let link = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
                format!("{}. {title}\n   {content}\n   {link}", i + 1)
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        // Apri automaticamente il 1° risultato e includi il testo: il modello (specie il piccolo) spesso
        // non fa il secondo web_fetch e INVENTA. Servendo già il contenuto, estrae numero/orari/info dal
        // testo reale in un solo passo. (Best-effort: se il fetch fallisce, restano comunque i risultati.)
        let page = results
            .iter()
            .find_map(|r| r.get("url").and_then(|v| v.as_str()))
            .and_then(fetch_readable)
            .map(|t| format!("\n\n📄 Contenuto del 1° risultato (estrai da qui numero/indirizzo/orari; se non c'è, dillo, non inventare):\n{t}"))
            .unwrap_or_default();
        Ok(format!("Risultati web per \"{q}\":\n\n{out}{page}"))
    }
}

// DuckDuckGo "done right": POST come il form reale, con header da browser (supera l'anti-bot del GET)
fn ddg_search(q: &str) -> Result<Vec<(String, String, String)>> {
    let resp = http_agent().post("https://html.duckduckgo.com/html/")
        .set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15")
        .set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .set("Accept-Language", "it-IT,it;q=0.9,en;q=0.8")
        .set("Referer", "https://html.duckduckgo.com/")
        .set("Origin", "https://html.duckduckgo.com")
        .timeout(std::time::Duration::from_secs(8))
        .send_form(&[("q", q), ("kl", "it-it"), ("df", "")])
        .map_err(|e| anyhow!("Ricerca fallita: {e}"))?;
    let html = resp.into_string().map_err(|e| anyhow!("Lettura fallita: {e}"))?;
    Ok(parse_ddg(&html))
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => { out.push(v); i += 3; }
                Err(_) => { out.push(b[i]); i += 1; }
            },
            b'+' => { out.push(b' '); i += 1; }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&#x27;", "'").replace("&#39;", "'")
        .replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
        .replace("&nbsp;", " ").trim().to_string()
}

#[cfg(test)]
mod ssrf_tests {
    use super::{host_of, is_blocked_host, is_blocked_ip, resolve_location};
    use std::net::IpAddr;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn ip_privati_e_speciali_bloccati() {
        for bad in ["127.0.0.1", "0.0.0.0", "10.1.2.3", "192.168.1.1", "172.20.0.1", "169.254.169.254"] {
            assert!(is_blocked_ip(ip(bad)), "{bad} deve essere bloccato");
        }
        for ok in ["1.1.1.1", "8.8.8.8", "104.16.0.1"] {
            assert!(!is_blocked_ip(ip(ok)), "{ok} NON va bloccato");
        }
    }

    #[test]
    fn ipv6_ula_linklocal_e_mapped_bloccati() {
        // 🚨 gap pre-fix: fc00::/7, fe80::/10 e ::ffff:v4 passavano il guard
        for bad in ["::1", "::", "fd00::1", "fc00::1", "fe80::1", "::ffff:127.0.0.1", "::ffff:192.168.1.1"] {
            assert!(is_blocked_ip(ip(bad)), "{bad} deve essere bloccato");
        }
        assert!(!is_blocked_ip(ip("2606:4700:4700::1111")), "IPv6 pubblico NON va bloccato");
    }

    #[test]
    fn host_of_gestisce_ipv6_letterali() {
        // 🚨 pre-fix: lo split su ':' spezzava "[::1]:8080" in "[" → guard mai attivo
        assert_eq!(host_of("http://[::1]:8080/x").as_deref(), Some("[::1]"));
        assert_eq!(host_of("https://[fd00::1]/").as_deref(), Some("[fd00::1]"));
        assert_eq!(host_of("https://example.com:443/p?q").as_deref(), Some("example.com"));
        assert_eq!(host_of("example.com/p").as_deref(), Some("example.com"));
    }

    #[test]
    fn blocked_host_su_letterali_senza_dns() {
        assert!(is_blocked_host("localhost"));
        assert!(is_blocked_host("127.0.0.1"));
        assert!(is_blocked_host("[::1]"));
        assert!(is_blocked_host("[fd00::1]"));
        assert!(is_blocked_host("169.254.169.254")); // IMDS
    }

    #[test]
    fn resolve_location_tutte_le_forme() {
        let base = "https://example.com/dir/page";
        assert_eq!(resolve_location(base, "https://other.org/x"), "https://other.org/x");
        assert_eq!(resolve_location(base, "//cdn.example.com/y"), "https://cdn.example.com/y");
        assert_eq!(resolve_location(base, "/abs"), "https://example.com/abs");
        assert_eq!(resolve_location(base, "rel.html"), "https://example.com/dir/rel.html");
        assert_eq!(resolve_location("https://example.com", "rel"), "https://example.com/rel");
        // il target del redirect verso la LAN esiste come URL… ma get_checked lo
        // ricontrolla con is_blocked_host a ogni hop (test sopra)
        assert_eq!(resolve_location(base, "http://192.168.1.1/admin"), "http://192.168.1.1/admin");
    }
}

fn parse_ddg(html: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    for block in html.split("result__a").skip(1) {
        let href = block.split("href=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("");
        let link = match href.split("uddg=").nth(1) {
            Some(s) => url_decode(s.split('&').next().unwrap_or(s)),
            None => href.trim_start_matches("//").to_string(),
        };
        let title = block.split('>').nth(1).and_then(|s| s.split("</a>").next()).map(strip_tags).unwrap_or_default();
        let snippet = block
            .split("result__snippet")
            .nth(1)
            .and_then(|s| s.splitn(2, '>').nth(1))
            .and_then(|s| s.split("</a>").next())
            .map(strip_tags)
            .unwrap_or_default();
        if !title.is_empty() {
            results.push((title, link, snippet));
        }
        if results.len() >= 8 {
            break;
        }
    }
    results
}
