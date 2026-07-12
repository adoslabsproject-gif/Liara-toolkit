//! Modalità "Liara via API" (32B cloud). Invece di far girare llama.cpp in locale, l'inferenza va al
//! `Qwen3-VL-32B` sul server NHA (`nothumanallowed.com/api/v1/liara/chat`, OpenAI chat/completions con
//! tool-calling hermes nativo). MA il ciclo agentico gira QUI sul dispositivo: il 32B decide QUALE tool
//! chiamare (restituisce `tool_calls`), noi lo eseguiamo in LOCALE col `ToolRegistry` (memoria/sensori/
//! file restano on-device) e rimandiamo il risultato. Streaming + eventi UI IDENTICI al locale (stesso
//! `WindowSink`), così il frontend non distingue le due modalità.
//!
//! ⚠️ PRIVACY: in questa modalità la conversazione E i risultati dei tool (contenuto dei file letti, la
//! memoria, la posizione) vengono inviati al server. È l'opposto della promessa on-device → va dietro un
//! consenso esplicito (gestito dal frontend prima di attivare la modalità cloud).
use crate::core::agent::{AgentSink, Message};
use crate::AppState;

/// System prompt della modalità CLOUD. NON è il SYSTEM_PROMPT locale (che dice "locale e privata" ed è
/// il contratto di training del modello on-device): qui l'identità è corretta alla realtà — Liara gira
/// sul server GPU di Zeli srl, NON sul dispositivo. Solo i modelli locali al 100% possono dirsi on-device.
/// Le regole di comportamento (usa gli strumenti, non inventare, italiano) restano le stesse.
// ⚠️ STRUTTURA CRITICA (testato 2026-07-12): il tool-directive DEVE venire SUBITO, come nel
// SYSTEM_PROMPT locale. Mettere davanti 3 frasi sull'identità cloud (come una prima versione) DISTRAE
// il modello → smette di chiamare i tool e chiede chiarimenti (bug "dimmi la località" invece di
// eseguire weather). Qui l'identità cloud è una BREVE clausola iniziale, poi subito "USA SEMPRE gli
// strumenti". Verificato: chiama i tool E risponde "server Zeli" a "dove giri?".
// ⚠️ BILANCIAMENTO (testato 2026-07-12, 3 scenari): AZIONI→strumenti, CHIACCHIERA→conversa, GRAFICO→blocco
// chart. "USA SEMPRE gli strumenti / non rispondere a parole" (v1) era troppo forte → il 32B RIFIUTAVA di
// conversare ("non rispondo a parole"). Troppo debole sul grafico → chiamava calculator. Questa versione
// regge tutti e tre. Il grafico va detto ESPLICITO ("non calcolare, scrivi tu ```chart") o riverte a calculator.
const CLOUD_SYSTEM_PROMPT: &str =
    "Sei Liara, assistente personale con memoria dell'utente, in esecuzione sul server di Zeli srl \
(modalità cloud, non sul dispositivo). \
Usa gli strumenti quando servono per AGIRE o per dati reali (email, agenda, file, web, meteo, note, calcoli, data/ora). \
Per SPOSTARE o RIPROGRAMMARE un appuntamento esistente: prima calendar_delete quello vecchio (per id), POI calendar_add \
il nuovo orario — NON fare solo calendar_add o crei un DOPPIONE. \
Sei anche una compagna con cui parlare: a domande, opinioni, chiacchiere, spiegazioni rispondi NORMALMENTE, \
con calore e personalità — non serve uno strumento per conversare. \
Se l'utente chiede un GRAFICO (torta/barre/linee): NON usare strumenti e NON fare calcoli, scrivi TU direttamente \
un blocco ```chart col JSON {\"type\":\"pie|bar|line|area\",\"data\":[{\"name\":\"...\",\"value\":numero}]} \
usando i valori che ti ha dato. \
NON inventare MAI nomi, numeri o fatti reali: se non li sai con certezza usa web_search, e se non trovi nulla dillo. \
Rispondi in italiano, chiara e concisa. Non firmarti.";
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, State, WebviewWindow};

const DEFAULT_URL: &str = "https://nothumanallowed.com/api/v1/liara/chat";
const DEFAULT_MODEL: &str = "nha-v1";
const MAX_ROUNDS: usize = 8; // giri ReAct massimi per turno (anti-loop)

fn api_url() -> String {
    std::env::var("LIARA_API_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}
fn api_model() -> String {
    std::env::var("LIARA_API_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string())
}

/// UNA chiamata al 32B (NON-streaming). ⚠️ `stream:false` è OBBLIGATORIO per l'AFFIDABILITÀ dei tool: in
/// streaming il 32B NON è coerente nel formato del tool-call (a volte `<tool_call>{…}`, a volte JSON NUDO
/// `{"name":…,"arguments":…}`) → non li catturavamo → il tool NON partiva (es. appuntamento non salvato).
/// Non-stream ritorna `tool_calls` STRUTTURATI e affidabili. Il testo (risposta) si emette via on_token.
/// Trade-off: la risposta appare tutta insieme (non parola-per-parola). Lo streaming affidabile tornerà
/// quando i modelli formatteranno i tool in modo coerente (gold seed) o con un hybrid dedicato.
fn call_once(
    url: &str,
    model: &str,
    msgs: &[Value],
    tools: &[Value],
    sink: &mut dyn AgentSink,
    _cancel: &AtomicBool,
) -> anyhow::Result<(String, Vec<Value>)> {
    let body = json!({
        "model": model,
        "messages": msgs,
        "tools": tools,
        "tool_choice": "auto",
        "stream": false,
        // reasoning OFF: nel loop agentico vogliamo tool-calling diretto (più veloce, meno token).
        "chat_template_kwargs": { "enable_thinking": false },
    });
    let resp = ureq::post(url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| anyhow::anyhow!("richiesta API Liara: {e}"))?;
    let raw = resp
        .into_string()
        .map_err(|e| anyhow::anyhow!("lettura risposta API: {e}"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| anyhow::anyhow!("JSON API: {e}"))?;
    let msg = &v["choices"][0]["message"];
    let content = msg["content"].as_str().unwrap_or("").to_string();
    // emetti il testo del modello (sui giri con solo tool-call è vuoto; sul giro finale è la risposta)
    if !content.is_empty() {
        sink.on_token(&content);
    }
    let tool_calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
    Ok((content, tool_calls))
}

/// Comando: genera una risposta usando il 32B cloud. Firma allineata a `generate` (stesso frontend).
#[tauri::command]
pub async fn remote_generate(
    messages: Vec<Message>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<String, String> {
    let memory = state.memory.clone();
    let tools = state.tools.clone();
    let consent = state.consent.clone();
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::Relaxed);
    let w = window.clone();

    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let mut sink = crate::commands::sink::WindowSink::new(w.clone(), memory.clone(), consent.clone());
        // Sistema + profilo utente. (Recall semantico dei ricordi: TODO — richiede embed; in cloud lo
        // teniamo locale in una fase successiva, per non spedire ogni query di memoria al server.)
        let system = format!("{}{}", CLOUD_SYSTEM_PROMPT, memory.profile_block());
        let mut msgs: Vec<Value> = vec![json!({ "role": "system", "content": system })];
        for m in &messages {
            msgs.push(json!({ "role": m.role, "content": m.content }));
        }
        let tool_defs = tools.openai_tools();
        let url = api_url();
        let model = api_model();

        let _ = w.emit("status", "cloud");
        let mut final_answer = String::new();
        for _round in 0..MAX_ROUNDS {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            // call_once (non-stream) emette il testo via on_token e ci ritorna (testo, tool_calls STRUTTURATI).
            let (content, tool_calls) = call_once(&url, &model, &msgs, &tool_defs, &mut sink, &cancel)?;
            if tool_calls.is_empty() {
                final_answer = content; // risposta finale (nessun altro tool richiesto)
                break;
            }
            // registra il turno assistant coi tool_calls, poi esegui ogni tool IN LOCALE
            msgs.push(json!({ "role": "assistant", "content": content, "tool_calls": tool_calls }));
            for tc in &tool_calls {
                let id = tc["id"].as_str().unwrap_or("");
                let name = tc["function"]["name"].as_str().unwrap_or("");
                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let args: Value = serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
                sink.on_tool(name, args_str);
                // consenso per i tool sensibili (stesso gate del locale)
                if tools.is_sensitive(name) {
                    let action = tools.consent_action(name, &args);
                    if !sink.on_consent(name, &action) {
                        let res = "L'utente ha negato il permesso per questo strumento.";
                        sink.on_tool_result(name, res);
                        msgs.push(json!({ "role": "tool", "tool_call_id": id, "name": name, "content": res }));
                        continue;
                    }
                }
                let result = tools.execute(name, &args).unwrap_or_else(|e| format!("Errore: {e}"));
                sink.on_tool_result(name, &result);
                msgs.push(json!({ "role": "tool", "tool_call_id": id, "name": name, "content": result }));
            }
        }
        let _ = w.emit("status", "ready");
        let _ = w.emit("done", &final_answer);
        Ok(final_answer)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
