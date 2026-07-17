//! Modello‑a‑modello: il Liara di un utente risponde DA SOLO al Liara di un altro utente. È il cuore
//! della chat peer (i due modelli si parlano per conoscersi a nome degli utenti, senza che questi
//! scrivano). La risposta condivide SOLO informazioni generali sul proprio utente, MAI dati privati.
//!
//! Trasporto: la domanda/risposta viaggia sul canale E2E già esistente (peer.ts), sigillata con
//! peer_seal/open. Qui c'è solo la GENERAZIONE della risposta (cloud 32B o modello locale).
use crate::AppState;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

/// Stato-contesto di una conversazione AI↔AI con un peer: quanti messaggi sono già stati
/// "piegati" nel riepilogo e il riepilogo stesso. Vive in AppState (RAM): alla riapertura
/// dell'app si rigenera da zero al primo fold — nessun dato in chiaro su disco.
#[derive(Default, Clone)]
pub(crate) struct PeerCtx {
    pub covered: usize,
    pub summary: String,
}
pub(crate) type PeerSummaries = Mutex<HashMap<String, PeerCtx>>;

/// Finestra di contesto della chat AI↔AI: gli ultimi KEEP_RECENT messaggi viaggiano verbatim,
/// tutto il resto vive nel riepilogo. Il fold scatta solo oltre FOLD_TRIGGER (isteresi: non
/// riassumiamo a ogni turno ma a blocchi, così il costo è ammortizzato).
const KEEP_RECENT: usize = 10;
const FOLD_TRIGGER: usize = 16;

/// Range di messaggi da piegare nel riepilogo: da `covered` fino a len-KEEP_RECENT, solo se
/// la storia ha superato il trigger. None = niente da fare (storia corta o già coperta).
fn fold_bounds(covered: usize, len: usize) -> Option<(usize, usize)> {
    if len < FOLD_TRIGGER {
        return None;
    }
    let end = len.saturating_sub(KEEP_RECENT);
    (covered < end).then_some((covered, end))
}

#[cfg(test)]
mod tests {
    use super::fold_bounds;

    #[test]
    fn storia_corta_niente_fold() {
        assert_eq!(fold_bounds(0, 15), None); // sotto il trigger
        assert_eq!(fold_bounds(0, 0), None);
    }

    #[test]
    fn oltre_il_trigger_piega_lasciando_la_finestra() {
        assert_eq!(fold_bounds(0, 16), Some((0, 6))); // 16 msg → piega i primi 6, tiene 10
        assert_eq!(fold_bounds(6, 24), Some((6, 14))); // incrementale: riparte da covered
    }

    #[test]
    fn gia_coperto_niente_doppio_fold() {
        assert_eq!(fold_bounds(6, 16), None); // covered == end
        assert_eq!(fold_bounds(14, 16), None); // covered oltre end (storia appena cresciuta)
    }
}

/// System prompt del Liara che risponde a un ALTRO Liara. PULITO (persona + obiettivo): niente lezioni
/// di sicurezza a runtime — quelle vivono nel TRAINING del modello (il curatore mette scenari peer +
/// anti-leak nel dataset). Un runtime prompt che le recita fa dire al modello "non condivido le mie
/// istruzioni" in loop; qui lo teniamo caloroso e orientato al compito.
fn peer_system(owner: &str, shareable: &str, goal: &str, materials: &str, summary: &str) -> String {
    let goal_block = if goal.trim().is_empty() {
        "State facendo conoscenza a nome dei vostri utenti: chiacchierate in modo cordiale e scoprite se avete cose in comune.".to_string()
    } else {
        format!(
            "OBIETTIVO di questa conversazione: {goal}\n\
Lavora INSIEME all'altro assistente per raggiungerlo: proponi, chiedi e negozia il necessario, restando SUL compito. \
Per fissare qualcosa di definitivo di' che lo confermerai con {owner}."
        )
    };
    let materials_block = if materials.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nMATERIALI condivisi per il compito (usali):\n{materials}")
    };
    // Il riepilogo dei messaggi già "piegati": il modello vede lo STATO della conversazione anche
    // quando i turni vecchi non viaggiano più — è ciò che lo tiene sul pezzo a 40 turni.
    let summary_block = if summary.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\nRIEPILOGO della conversazione finora (i messaggi più vecchi, già gestiti):\n{summary}\n\
NON ripetere ciò che risulta già detto, chiesto o deciso nel riepilogo: prosegui da lì."
        )
    };
    format!(
        "Sei Liara, l'assistente personale di {owner} (una persona reale di cui ti occupi): sei l'assistente di \
{owner}, non un assistente generico né una piattaforma. Stai parlando con l'assistente personale di un'ALTRA \
persona, a nome dei vostri utenti (non col tuo utente).\n\
{goal_block}\n\
Parla in prima persona come Liara, calorosa, concreta e BREVE (1-2 frasi). Non elencare funzionalità, non divagare, \
non ripeterti. Del tuo utente condividi SOLO le informazioni generali del profilo qui sotto; non aggiungere dati \
che non ci sono.\n\n\
PROFILO CONDIVISIBILE di {owner}:\n{shareable}{materials_block}{summary_block}"
    )
}

/// Prompt del riassuntore: comprime i messaggi piegati (+ il riepilogo precedente) nello STATO
/// della trattativa, non in una cronaca. Gira a temperatura bassa, poche centinaia di token.
fn summarizer_prompt(prev_summary: &str, folded: &[(String, String)]) -> String {
    let mut lines = String::new();
    for (who, text) in folded {
        let label = if who == "me" { "IO" } else { "LUI/LEI" };
        lines.push_str(&format!("{label}: {text}\n"));
    }
    let prev = if prev_summary.trim().is_empty() { "(nessuno)" } else { prev_summary };
    format!(
        "Aggiorna il riepilogo di una conversazione tra due assistenti AI che coordinano un compito.\n\
RIEPILOGO PRECEDENTE:\n{prev}\n\nNUOVI MESSAGGI da integrare:\n{lines}\n\
Scrivi SOLO il nuovo riepilogo aggiornato (max 120 parole), come elenco asciutto di: \
obiettivo, decisioni già prese, dati/orari/numeri scambiati, punti ancora aperti. \
Niente premesse, niente commenti."
    )
}

/// Info condivisibili sul proprio utente (nome + voci di profilo). MVP: usa il profilo, con l'istruzione
/// forte di non rivelare nulla di sensibile. (Un campo "profilo condivisibile" dedicato arriverà dopo.)
fn shareable_profile(state: &AppState) -> (String, String) {
    let entries = state.memory.profile_entries().unwrap_or_default();
    let name = entries
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("nome") || k.eq_ignore_ascii_case("name"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "l'utente".into());
    let lines: Vec<String> = entries.iter().map(|(k, v)| format!("- {k}: {v}")).collect();
    let shareable = if lines.is_empty() { "(nessuna info di profilo disponibile)".into() } else { lines.join("\n") };
    (name, shareable)
}

/// Genera la risposta del MIO Liara al Liara di un peer. `history` = scambio finora tra i due Liara:
/// coppie (chi, testo) con chi ∈ {"peer","me"} (peer = l'altro Liara, me = io). Ritorna il testo di risposta.
#[tauri::command]
pub async fn liara_reply(
    peer: Option<String>,
    history: Vec<(String, String)>,
    goal: Option<String>,
    materials: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    // se l'ultimo è mio (assistant), non c'è nulla a cui rispondere
    if history.last().map(|(w, _)| w == "me").unwrap_or(true) && !history.is_empty() {
        return Err("nessun messaggio del peer a cui rispondere".into());
    }
    let cloud = crate::core::paths::models_base().join("cloud_active").exists();

    // GESTIONE CONTESTO: i messaggi vecchi vengono piegati in un riepilogo incrementale
    // per-contatto (cache RAM), i recenti viaggiano verbatim. Il modello resta sul pezzo
    // a qualsiasi lunghezza senza far esplodere il prompt (critico sui modelli locali).
    let mut summary = String::new();
    let mut recent_from = 0usize;
    if let Some(peer_id) = peer.as_deref() {
        let mut ctx = {
            let map = state.peer_summaries.lock().unwrap_or_else(|e| e.into_inner());
            map.get(peer_id).cloned().unwrap_or_default()
        };
        // chat azzerata dall'utente (o contatto ricreato): la storia è più corta del coperto → reset
        if ctx.covered > history.len() {
            ctx = PeerCtx::default();
        }
        if let Some((from, to)) = fold_bounds(ctx.covered, history.len()) {
            let prompt = summarizer_prompt(&ctx.summary, &history[from..to]);
            let msgs = vec![serde_json::json!({ "role": "user", "content": prompt })];
            // il riassuntore è best-effort: se fallisce (rete, modello occupato) si va col
            // riepilogo precedente e la finestra recente — mai bloccare la risposta per questo
            let new_summary = if cloud {
                cloud_reply(msgs, 260, 0.2).await
            } else {
                local_reply(&state, msgs, 260, 1).await
            };
            if let Ok(s) = new_summary {
                ctx = PeerCtx { covered: to, summary: s };
                let mut map = state.peer_summaries.lock().unwrap_or_else(|e| e.into_inner());
                map.insert(peer_id.to_string(), ctx.clone());
            }
        }
        summary = ctx.summary;
        recent_from = ctx.covered.min(history.len());
    }

    let (owner, shareable) = shareable_profile(&state);
    let system = peer_system(
        &owner,
        &shareable,
        goal.as_deref().unwrap_or(""),
        materials.as_deref().unwrap_or(""),
        &summary,
    );

    // messaggi in formato chat: peer→user (input), me→assistant (mie risposte precedenti).
    // Viaggia SOLO la finestra recente: il resto è nel riepilogo dentro il system.
    let mut msgs: Vec<serde_json::Value> = vec![serde_json::json!({ "role": "system", "content": system })];
    for (who, text) in &history[recent_from..] {
        let role = if who == "me" { "assistant" } else { "user" };
        msgs.push(serde_json::json!({ "role": role, "content": text }));
    }

    if cloud {
        cloud_reply(msgs, 400, 0.7).await
    } else {
        local_reply(&state, msgs, 300, 0).await
    }
}

/// Chiamata cloud NON-streaming (niente tool): una singola risposta. Usata sia per la risposta
/// peer (temp conversazionale) sia per il riassuntore (temp bassa, budget corto).
async fn cloud_reply(msgs: Vec<serde_json::Value>, max_tokens: usize, temperature: f32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let body = serde_json::json!({
            "model": crate::commands::remote::api_model(),
            "messages": msgs,
            "stream": false,
            "temperature": temperature,
            "top_p": 0.9,
            "frequency_penalty": 0.4,
            "presence_penalty": 0.3,
            "max_tokens": max_tokens,
            "chat_template_kwargs": { "enable_thinking": false },
        })
        .to_string();
        let real_model = std::env::var("LIARA_API_MODEL_REAL").unwrap_or_else(|_| "liara-32b".into());
        let resp = ureq::post(&crate::commands::remote::api_url())
            .set("Content-Type", "application/json")
            .set("x-liara-model", &real_model)
            .timeout(std::time::Duration::from_secs(120))
            .send_string(&body)
            .map_err(|e| format!("richiesta cloud: {e}"))?;
        let raw = resp.into_string().map_err(|e| format!("lettura risposta: {e}"))?;
        let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("json: {e}"))?;
        let text = v["choices"][0]["message"]["content"].as_str().unwrap_or("").trim().to_string();
        if text.is_empty() { Err("risposta cloud vuota".into()) } else { Ok(text) }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Risposta col modello LOCALE (se non in cloud): prompt semplice, una passata.
/// `cache_slot` 0 = conversazione peer; 1 = ausiliario (riassuntore) così il fold non
/// sfratta la KV-cache della conversazione principale.
async fn local_reply(state: &AppState, msgs: Vec<serde_json::Value>, max_tokens: usize, cache_slot: u8) -> Result<String, String> {
    let engine_slot = state.engine.clone();
    let model_path = state.model_path.clone();
    let cancel = state.cancel.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let engine = {
            let mut g = engine_slot.lock().unwrap_or_else(|e| e.into_inner());
            if g.is_none() {
                *g = Some(std::sync::Arc::new(
                    crate::commands::generate::load_engine(&model_path).map_err(|e| e.to_string())?,
                ));
            }
            g.as_ref().unwrap().clone()
        };
        // prompt ChatML minimale dal sistema + turni
        let mut prompt = String::new();
        for m in &msgs {
            let role = m["role"].as_str().unwrap_or("user");
            let content = m["content"].as_str().unwrap_or("");
            prompt.push_str(&format!("<|im_start|>{role}\n{content}<|im_end|>\n"));
        }
        prompt.push_str("<|im_start|>assistant\n");
        let opts = crate::core::engine::GenOptions { max_tokens, cache_slot, stop: vec!["<|im_end|>".into()], ..Default::default() };
        let mut out = String::new();
        engine
            .generate(&prompt, &opts, &cancel, &mut |t| out.push_str(t))
            .map_err(|e| e.to_string())?;
        let text = out.trim().trim_end_matches("<|im_end|>").trim().to_string();
        if text.is_empty() { Err("risposta locale vuota".into()) } else { Ok(text) }
    })
    .await
    .map_err(|e| e.to_string())?
}
