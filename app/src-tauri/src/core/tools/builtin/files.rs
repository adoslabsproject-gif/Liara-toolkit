//! File manager tools (read-only, confined to the user's home directory).
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

pub(super) fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
}

/// Resolve a user path, confined to the home directory (path-traversal guard, audit #5).
pub(super) fn safe_path(input: &str) -> Result<PathBuf> {
    safe_path_in(&home_dir(), input)
}

/// Nucleo testabile di safe_path, con `home` iniettata.
///
/// FIX (review 2026-07-02 #3): la vecchia versione faceva `canonicalize()` sul
/// path INTERO, che fallisce se il file non esiste → `fs_write` non poteva MAI
/// creare un file nuovo ("Percorso non trovato") e `fs_move` non poteva rinominare
/// verso un nome nuovo. Ora: normalizzazione LESSICALE di ./.. (rifiuta la fuga
/// sopra root), poi canonicalize del più profondo antenato ESISTENTE (risolve i
/// symlink), infine si riattacca la coda inesistente (ormai senza `..`) e si
/// verifica il confinamento in home.
fn safe_path_in(home: &std::path::Path, input: &str) -> Result<PathBuf> {
    let raw = if let Some(rest) = input.strip_prefix('~') {
        home.join(rest.trim_start_matches('/'))
    } else if input.starts_with('/') {
        PathBuf::from(input)
    } else {
        home.join(input)
    };
    let norm = lexical_normalize(&raw).ok_or_else(|| anyhow!("Percorso non valido: {input}"))?;

    // canonicalize dell'antenato esistente più profondo + coda inesistente
    let mut existing = norm.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let canon_base = loop {
        match existing.canonicalize() {
            Ok(c) => break c,
            Err(_) => {
                let Some(name) = existing.file_name() else {
                    return Err(anyhow!("Percorso non trovato: {input}"));
                };
                tail.push(name.to_os_string());
                if !existing.pop() {
                    return Err(anyhow!("Percorso non trovato: {input}"));
                }
            }
        }
    };
    let mut canon = canon_base;
    for c in tail.iter().rev() {
        canon.push(c);
    }

    // Confronto canonico-contro-canonico: se $HOME contiene a sua volta un
    // symlink (es. /var → /private/var su macOS), il confronto col path grezzo
    // bloccherebbe TUTTO (fail-closed ma app inusabile).
    let home_canon = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    if !canon.starts_with(&home_canon) {
        return Err(anyhow!("Accesso fuori dalla cartella utente non consentito."));
    }
    Ok(canon)
}

/// Risolve `.` e `..` per via lessicale, senza toccare il filesystem.
/// `None` se `..` tenta di salire sopra la radice del path.
fn lexical_normalize(p: &std::path::Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::RootDir | Component::Prefix(_) => out.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(s) => out.push(s),
        }
    }
    Some(out)
}

pub struct FsList;
impl Tool for FsList {
    fn sensitive(&self) -> bool {
        true
    }
    fn consent_action(&self, args: &Value) -> String {
        format!("elencare i file nella cartella {}", args.get("path").and_then(|v| v.as_str()).unwrap_or("?"))
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs_list".into(),
            description: "Elenca file e cartelle dentro una directory dell'utente (es. Downloads, Documenti).".into(),
            parameters: json!({ "type": "object", "properties": { "path": { "type": "string", "description": "Cartella" } }, "required": ["path"] }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let p = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'path'"))?;
        let dir = safe_path(p)?;
        if !dir.is_dir() {
            return Err(anyhow!("Non \u{00e8} una cartella: {p}"));
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let meta = e.metadata().ok();
            if meta.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
                out.push(format!("{name}/ (cartella)"));
            } else {
                let kb = meta.map(|m| m.len() / 1024).unwrap_or(0);
                out.push(format!("{name} ({kb} KB)"));
            }
        }
        out.sort();
        out.truncate(100);
        if out.is_empty() {
            return Ok("Cartella vuota.".into());
        }
        Ok(format!("Contenuto di {}:\n{}", dir.display(), out.join("\n")))
    }
}

pub struct FsRead;
impl Tool for FsRead {
    fn sensitive(&self) -> bool {
        true
    }
    fn consent_action(&self, args: &Value) -> String {
        format!("leggere il file {}", args.get("path").and_then(|v| v.as_str()).unwrap_or("?"))
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs_read".into(),
            description: "Legge il contenuto testuale di un file dell'utente.".into(),
            parameters: json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let p = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'path'"))?;
        let file = safe_path(p)?;
        if !file.is_file() {
            return Err(anyhow!("Non \u{00e8} un file: {p}"));
        }
        let bytes = std::fs::read(&file)?;
        if bytes.len() > 8_000_000 {
            return Err(anyhow!("File troppo grande (>8MB)."));
        }
        // PDFs: extract text instead of failing as binary
        if crate::core::extract::is_pdf(p) {
            return match crate::core::extract::pdf_to_text(&bytes) {
                Some(text) => {
                    let s: String = text.chars().take(8000).collect();
                    Ok(format!("Contenuto del PDF {}:\n\n{s}", file.display()))
                }
                None => Err(anyhow!("PDF senza testo estraibile (forse scansione/immagine).")),
            };
        }
        match String::from_utf8(bytes) {
            Ok(s) => {
                let s: String = s.chars().take(8000).collect();
                Ok(format!("Contenuto di {}:\n\n{s}", file.display()))
            }
            Err(_) => Err(anyhow!("File binario non testuale.")),
        }
    }
}

pub struct FsSearch;
impl Tool for FsSearch {
    fn sensitive(&self) -> bool {
        true
    }
    fn consent_action(&self, args: &Value) -> String {
        format!("cercare file (\"{}\") nei tuoi documenti", args.get("query").and_then(|v| v.as_str()).unwrap_or("?"))
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs_search".into(),
            description: "Cerca file per nome dentro una cartella dell'utente.".into(),
            parameters: json!({ "type": "object", "properties": { "query": { "type": "string" }, "dir": { "type": "string" } }, "required": ["query"] }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let q = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'query'"))?.to_lowercase();
        let dir_in = args.get("dir").and_then(|v| v.as_str()).unwrap_or("~");
        let dir = safe_path(dir_in)?;
        // noisy/huge trees that would fill results before reaching what the user means
        const SKIP: &[&str] =
            &["node_modules", "Library", "target", ".git", ".cache", ".cargo", ".Trash", "venv", "__pycache__", "build", "dist"];
        // breadth-first: shallow matches (e.g. ~/Cartella) surface first, not buried under deep trees
        let mut found: Vec<String> = Vec::new();
        let mut queue: std::collections::VecDeque<(PathBuf, u32)> = std::collections::VecDeque::new();
        queue.push_back((dir, 0));
        while let Some((d, depth)) = queue.pop_front() {
            if found.len() >= 60 {
                break;
            }
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            let mut subdirs = Vec::new();
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || SKIP.contains(&name.as_str()) {
                    continue;
                }
                let is_dir = e.path().is_dir();
                if name.to_lowercase().contains(&q) {
                    found.push(format!("{}{}", e.path().display(), if is_dir { "/" } else { "" }));
                }
                if is_dir && depth < 4 {
                    subdirs.push(e.path());
                }
            }
            for sd in subdirs {
                queue.push_back((sd, depth + 1));
            }
        }
        if found.is_empty() {
            return Ok(format!("Nessun file o cartella trovati per \"{q}\"."));
        }
        Ok(found.join("\n"))
    }
}

#[cfg(test)]
mod safe_path_tests {
    use super::safe_path_in;
    use std::path::PathBuf;

    /// home di prova REALE su disco (canonicalizzata: /var → /private/var su macOS).
    fn test_home(tag: &str) -> PathBuf {
        let h = std::env::temp_dir().join(format!("liara_home_{}_{tag}", std::process::id()));
        std::fs::create_dir_all(h.join("Documenti")).unwrap();
        h.canonicalize().unwrap()
    }

    #[test]
    fn file_nuovo_in_dir_esistente_ok() {
        // ANTI-REGRESSIONE bug #3: la vecchia safe_path (canonicalize sull'intero
        // path) qui FALLIVA con "Percorso non trovato" → fs_write non creava mai.
        let home = test_home("nuovo");
        let p = safe_path_in(&home, home.join("Documenti/nota.txt").to_str().unwrap()).unwrap();
        assert_eq!(p, home.join("Documenti/nota.txt"));
    }

    #[test]
    fn dir_annidate_inesistenti_ok() {
        let home = test_home("annidate");
        // fs_write fa create_dir_all(parent): la risoluzione deve accettare
        // anche una catena di cartelle non ancora esistenti.
        let p = safe_path_in(&home, home.join("note/2026/luglio/todo.txt").to_str().unwrap()).unwrap();
        assert!(p.starts_with(&home));
    }

    #[test]
    fn traversal_su_coda_inesistente_bloccato() {
        // 🚨 il punto delicato del fix: senza normalizzazione lessicale,
        // "home/nuova/../../.." riattaccato dopo il canonicalize del parent
        // passerebbe il starts_with LESSICALE pur uscendo dalla home.
        let home = test_home("traversal");
        let evil = home.join("nuova_dir/../../../etc/passwd");
        assert!(safe_path_in(&home, evil.to_str().unwrap()).is_err());
    }

    #[test]
    fn assoluto_fuori_home_bloccato() {
        let home = test_home("assoluto");
        assert!(safe_path_in(&home, "/etc/passwd").is_err());
        assert!(safe_path_in(&home, "/etc/nonesiste/nuovo.txt").is_err());
    }

    #[test]
    fn file_esistente_e_tilde_ok() {
        let home = test_home("esistente");
        std::fs::write(home.join("Documenti/x.txt"), "ciao").unwrap();
        let p = safe_path_in(&home, home.join("Documenti/x.txt").to_str().unwrap()).unwrap();
        assert!(p.ends_with("Documenti/x.txt"));
        // "~/..." risolve dentro la home iniettata
        let t = safe_path_in(&home, "~/Documenti/x.txt").unwrap();
        assert_eq!(t, p);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_che_evade_bloccato() {
        let home = test_home("symlink");
        let outside = std::env::temp_dir().join(format!("liara_outside_{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        let link = home.join("evasione");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        // sia il symlink stesso sia un file (anche inesistente) attraverso di esso
        assert!(safe_path_in(&home, link.to_str().unwrap()).is_err());
        assert!(safe_path_in(&home, link.join("nuovo.txt").to_str().unwrap()).is_err());
    }

    #[test]
    fn parent_dir_oltre_root_bloccato() {
        let home = test_home("root");
        assert!(safe_path_in(&home, "/../../..").is_err());
    }
}
