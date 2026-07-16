//! Vision command. Two-stage so vision and tools work IN THE SAME TURN: the VL model first
//! describes the attached image, then that description is fed as context into the normal ReAct
//! loop (text engine + tools) — so "Liara guarda la foto E usa i tool" works together.
use crate::core::agent::{run_agent, Message, SYSTEM_PROMPT};
use crate::AppState;
use base64::Engine as _;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, State, WebviewWindow};

#[tauri::command]
pub async fn describe_image(
    image_b64: String,
    prompt: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<String, String> {
    let engine_slot = state.engine.clone();
    let model_path = state.model_path.clone();
    let memory = state.memory.clone();
    let tools = state.tools.clone();
    let consent = state.consent.clone();
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::Relaxed);
    let thinking = state.thinking.load(Ordering::Relaxed);
    let w = window.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let payload = image_b64.rsplit(',').next().unwrap_or(&image_b64);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload.as_bytes())
            .map_err(|e| anyhow::anyhow!("immagine non valida: {e}"))?;

        // Motore principale. La visione è NATIVA nel modello: Gemma 4 vede col proprio mmproj (un
        // solo LlamaEngine fa testo+immagini). I modelli di testo (Qwen 1.7B/4B) NON vedono.
        let engine = {
            let mut g = engine_slot.lock().unwrap();
            if g.is_none() {
                let _ = w.emit("status", "loading-model");
                *g = Some(Arc::new(crate::commands::generate::load_engine(&model_path)?));
            }
            g.as_ref().unwrap().clone()
        };

        // 1) descrizione dell'immagine col motore stesso (niente più companion Qwen-VL separato: era la
        // vecchia architettura, rimossa). Se il modello attivo non è VL nativo → messaggio chiaro, non
        // un errore criptico su un file 3B inesistente. (Il frontend nasconde già il 📎 per questi modelli:
        // questo è la rete di sicurezza per il caso limite.)
        if !engine.has_vision() {
            let _ = w.emit("status", "ready");
            anyhow::bail!(
                "Questo modello non vede le immagini. Per foto e documenti scarica e seleziona un modello Gemma 4 (💎 E4B o 💠 12B), che ha la visione nativa."
            );
        }
        let _ = w.emit("status", "vision-look");
        let q_img = "Descrivi in dettaglio cosa mostra l'immagine: oggetti, persone, testo leggibile, contesto.";
        let description = engine.describe(&bytes, q_img, 400, &cancel, &mut |_| {})?;
        // DEBUG (opt-in): salva la descrizione grezza dell'encoder (leggibile via `adb run-as cat`).
        // Dietro LIARA_DEBUG_VISION esplicita: scrivere il contenuto di ogni foto IN CHIARO su disco a
        // ogni analisi è incoerente con la cifratura at-rest del resto — di default NON si fa.
        if std::env::var("LIARA_DEBUG_VISION").is_ok() {
            if let Ok(md) = std::env::var("LIARA_MODELS_DIR") {
                let _ = std::fs::write(
                    std::path::Path::new(&md).join("last_vision.txt"),
                    format!("img_bytes={}\nhas_vision={}\n---\n{description}", bytes.len(), engine.has_vision()),
                );
            }
        }
        let _ = w.emit("status", "ready");
        let system = format!("{}{}", SYSTEM_PROMPT, memory.profile_block());
        let q = if prompt.trim().is_empty() { "Cosa c'è nell'immagine?" } else { prompt.trim() };
        let user = format!("[Ho allegato un'immagine. Contenuto rilevato dalla vista: {description}]\n\n{q}");
        let messages = vec![Message { role: "user".into(), content: user }];

        // eventi UI + consenso via WindowSink (stessa impl di generate.rs, zero duplicazione)
        let mut sink = crate::commands::sink::WindowSink::new(w.clone(), memory.clone(), consent.clone());
        let answer = run_agent(
            engine.as_ref(),
            &tools,
            &system,
            &messages,
            thinking,
            1024, // descrizione immagine: budget standard
            &cancel,
            &mut sink,
        )?;
        let _ = w.emit("done", &answer);
        Ok(answer)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
