//! Scarica il modello GGUF dal server al primo avvio (architettura "modello non bundlato"): l'APK è
//! leggero, il modello arriva da https://nothumanallowed.com/models/. Download con RESUME (riprende
//! da dove si era interrotto su rete mobile), verifica SHA256 in streaming (un download corrotto non
//! produce un modello rotto), progresso emesso al frontend (evento "download-progress").
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ring::digest::{Context, SHA256};
use tauri::{Emitter, WebviewWindow};

/// Flag di annullamento del download (settato da cancel_download, controllato nel loop di lettura).
/// Il file .part resta su disco → un download successivo riprende da dove era (resume).
static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);
/// #21 FIX: guard di re-entrancy. Due download_model concorrenti (doppio tap) appenderebbero sullo stesso
/// .part → byte interlacciati → checksum sempre fallito. Con questo la 2ª chiamata viene rifiutata.
static DOWNLOAD_BUSY: AtomicBool = AtomicBool::new(false);

/// Annulla il download in corso. Il .part NON viene cancellato (così si può riprendere).
#[tauri::command]
pub fn cancel_download() {
    DOWNLOAD_CANCEL.store(true, Ordering::Relaxed);
}

/// Agente HTTPS con root CA inclusi (lo store nativo Android non è affidabile) — come per il web tool.
fn agent() -> ureq::Agent {
    let root_store = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    ureq::AgentBuilder::new()
        .tls_config(Arc::new(config))
        .timeout_connect(std::time::Duration::from_secs(45))   // solo la connessione iniziale
        .timeout_read(std::time::Duration::from_secs(300))     // 5 min per-chunk (rete mobile lenta, niente falsi timeout)
        .build()                                               // NESSUN timeout totale → un 2,5 GB può durare quanto serve
}

/// Il modello è già nella cartella interna? (deciso il primo avvio: se no → schermata di download)
#[tauri::command]
pub async fn model_present(filename: String) -> bool {
    crate::core::paths::models_base().join(&filename).exists()
}

/// Scarica `url` in `models_base()/filename`, con resume e verifica sha256. Idempotente: se il file
/// finale esiste già, ritorna subito. Emette "download-progress" {downloaded, total, done}.
#[tauri::command]
pub async fn download_model(
    url: String,
    sha256: String,
    bytes: u64,
    filename: String,
    window: WebviewWindow,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        // #21 FIX: rifiuta un secondo download concorrente (doppio tap) invece di corrompere il .part.
        if DOWNLOAD_BUSY.swap(true, Ordering::SeqCst) {
            return Err("un download è già in corso".to_string());
        }
        // Guard RAII: rilascia il flag qualunque sia l'esito (successo, errore, early-return).
        struct BusyGuard;
        impl Drop for BusyGuard {
            fn drop(&mut self) { DOWNLOAD_BUSY.store(false, Ordering::SeqCst); }
        }
        let _busy = BusyGuard;
        let base = crate::core::paths::models_base();
        std::fs::create_dir_all(&base).map_err(|e| format!("mkdir: {e}"))?;
        let dest = base.join(&filename);
        // Già presente E aggiornato (lo SHA salvato combacia con quello atteso)? → niente da fare.
        // Se lo SHA differisce (modello nuovo sul server), cancelliamo e ri-scarichiamo: è così che
        // un utente riceve l'AGGIORNAMENTO del modello senza reinstallare l'app.
        if dest.exists() {
            let saved = std::fs::read_to_string(base.join(format!("{filename}.sha"))).unwrap_or_default();
            if sha256.is_empty() || saved.trim() == sha256 {
                let _ = window.emit("download-progress", serde_json::json!({"downloaded": bytes, "total": bytes, "done": true}));
                return Ok(());
            }
            let _ = std::fs::remove_file(&dest); // versione obsoleta → via, si ri-scarica sotto
        }
        let tmp = base.join(format!("{filename}.part"));
        // #22 FIX: un .part può appartenere a uno sha DIVERSO (modello aggiornato o download interrotto di
        // un'altra versione). Se non è "di questo sha", NON facciamo resume (sarebbe corrotto → sha finale
        // sempre errato → re-download da capo su rete mobile). Lo rimuoviamo e lo marchiamo con lo sha corrente.
        let tmp_owner = base.join(format!("{filename}.part.sha"));
        if !sha256.is_empty()
            && std::fs::read_to_string(&tmp_owner).unwrap_or_default().trim() != sha256
        {
            let _ = std::fs::remove_file(&tmp);
            let _ = std::fs::write(&tmp_owner, &sha256);
        }

        // RESUME: se c'è un .part, ne ricalcoliamo lo sha e ripartiamo dai byte mancanti.
        let mut ctx = Context::new(&SHA256);
        let mut downloaded: u64 = 0;
        if let Ok(mut f) = std::fs::File::open(&tmp) {
            let mut b = vec![0u8; 1 << 20];
            loop {
                let n = f.read(&mut b).map_err(|e| format!("leggo .part: {e}"))?;
                if n == 0 { break; }
                ctx.update(&b[..n]);
                downloaded += n as u64;
            }
        }

        let agent = agent();
        let mut builder = agent.get(&url); // NIENTE timeout totale: l'agent ha già connect/read; un 2,5 GB dura quanto serve
        if downloaded > 0 {
            builder = builder.set("Range", &format!("bytes={downloaded}-"));
        }
        // Errore di rete o HTTP (403 permessi, 404, 5xx, 416): ureq ritorna Err su status >= 400. Lo
        // propaghiamo come stringa → il frontend mostra "Riprova". MAI un crash per un server ostile.
        let resp = match builder.call() {
            Ok(r) => r,
            Err(e) => {
                // #7 FIX: se il RESUME fallisce (es. 416 Range-oltre-EOF perché il .part è più grande del
                // file sul server, o modello sostituito), RIMUOVIAMO il .part → il prossimo tentativo riparte
                // da zero. Prima il .part restava e "Riprova" rimandava lo stesso Range → loop permanente.
                if downloaded > 0 {
                    let _ = std::fs::remove_file(&tmp);
                }
                return Err(format!("download non riuscito: {e} — riprova"));
            }
        };

        // se il server IGNORA il Range (200 invece di 206), ricominciamo da zero per non corrompere
        if downloaded > 0 && resp.status() == 200 {
            ctx = Context::new(&SHA256);
            downloaded = 0;
            let _ = std::fs::remove_file(&tmp);
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&tmp)
            .map_err(|e| format!("apro .part: {e}"))?;
        let mut reader = resp.into_reader();
        let mut buf = vec![0u8; 1 << 20];
        let mut last_emit = downloaded;
        DOWNLOAD_CANCEL.store(false, Ordering::Relaxed); // reset prima di leggere
        loop {
            if DOWNLOAD_CANCEL.load(Ordering::Relaxed) {
                file.flush().ok();
                return Err("annullato".to_string()); // il .part resta su disco → si riprende dopo
            }
            let n = reader.read(&mut buf).map_err(|e| format!("download: {e}"))?;
            if n == 0 { break; }
            file.write_all(&buf[..n]).map_err(|e| format!("scrivo: {e}"))?;
            ctx.update(&buf[..n]);
            downloaded += n as u64;
            if downloaded - last_emit >= 4_000_000 {
                last_emit = downloaded;
                // flush su disco SUBITO: se lo standby/rete taglia, il .part è aggiornato → al resume
                // si riprende da QUI (Range), non da capo. È il fix del "arriva al 99% e ricomincia".
                file.flush().ok();
                let _ = window.emit("download-progress", serde_json::json!({"downloaded": downloaded, "total": bytes}));
            }
        }
        file.flush().ok();
        drop(file);

        // verifica integrità: un download corrotto NON deve diventare un modello rotto
        let hex: String = ctx.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect();
        if !sha256.is_empty() && hex != sha256 {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("checksum non valido: atteso {sha256}, ottenuto {hex}"));
        }
        std::fs::rename(&tmp, &dest).map_err(|e| format!("rename: {e}"))?;
        // salva lo SHA scaricato accanto al file → confronto veloce (stringa) per rilevare aggiornamenti
        if !sha256.is_empty() {
            let _ = std::fs::write(base.join(format!("{filename}.sha")), &sha256);
        }
        let _ = window.emit("download-progress", serde_json::json!({"downloaded": bytes, "total": bytes, "done": true}));
        Ok(())
    })
    .await
    .map_err(|e| format!("task: {e}"))?
}

/// Imposta il modello attivo: scrive `active_model` nella cartella modelli. Al PROSSIMO avvio l'app
/// caricherà questo GGUF — lo switch avviene col riavvio, così RAM/GPU del modello precedente sono
/// liberate pulite (un reload a caldo lascerebbe ~2GB di GPU allocati sull'Adreno).
#[tauri::command]
pub async fn set_active_model(filename: String) -> Result<(), String> {
    let base = crate::core::paths::models_base();
    std::fs::create_dir_all(&base).map_err(|e| format!("mkdir: {e}"))?;
    std::fs::write(base.join("active_model"), filename.trim()).map_err(|e| format!("scrivo preferenza: {e}"))?;
    Ok(())
}

/// Nome del GGUF attualmente selezionato (stringa vuota = default 4B).
#[tauri::command]
pub async fn active_model() -> String {
    let base = crate::core::paths::models_base();
    std::fs::read_to_string(base.join("active_model")).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// true se il modello è già scaricato MA con SHA diverso da quello atteso → c'è una versione nuova
/// sul server (es. un LoRA aggiornato). Il frontend lo usa per mostrare "Aggiorna modello".
#[tauri::command]
pub async fn model_outdated(filename: String, sha256: String) -> bool {
    let base = crate::core::paths::models_base();
    if sha256.is_empty() || !base.join(&filename).exists() {
        return false;
    }
    let saved = std::fs::read_to_string(base.join(format!("{filename}.sha"))).unwrap_or_default();
    saved.trim() != sha256
}

/// Scarica models.json dal server: la lista modelli DINAMICA. Aggiungere/aggiornare un modello =
/// modificare models.json su NHA, SENZA ribuildare e ridistribuire l'APK. Stesso agent TLS dei download.
///
/// NB (onestà, review round-3): al momento il frontend NON chiama ancora questo endpoint — la lista
/// modelli è hardcoded in App.tsx (`MODELS`). L'endpoint è pronto lato backend; il cablaggio del
/// frontend (fetch all'avvio con fallback alla lista interna se offline) è ancora da fare.
#[tauri::command]
pub async fn fetch_models() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<String, String> {
        let url = "https://nothumanallowed.com/models/models.json";
        let resp = agent()
            .get(url)
            .timeout(std::time::Duration::from_secs(20))
            .call()
            .map_err(|e| format!("models.json: {e}"))?;
        resp.into_string().map_err(|e| format!("lettura models.json: {e}"))
    })
    .await
    .map_err(|e| format!("task: {e}"))?
}
