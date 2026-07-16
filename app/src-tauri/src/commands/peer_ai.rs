//! Modello‑a‑modello: il Liara di un utente risponde DA SOLO al Liara di un altro utente. È il cuore
//! della chat peer (i due modelli si parlano per conoscersi a nome degli utenti, senza che questi
//! scrivano). La risposta condivide SOLO informazioni generali sul proprio utente, MAI dati privati.
//!
//! Trasporto: la domanda/risposta viaggia sul canale E2E già esistente (peer.ts), sigillata con
//! peer_seal/open. Qui c'è solo la GENERAZIONE della risposta (cloud 32B o modello locale).
use crate::AppState;
use tauri::State;

/// System prompt del Liara che risponde a un ALTRO Liara. PULITO (persona + obiettivo): niente lezioni
/// di sicurezza a runtime — quelle vivono nel TRAINING del modello (il curatore mette scenari peer +
/// anti-leak nel dataset). Un runtime prompt che le recita fa dire al modello "non condivido le mie
/// istruzioni" in loop; qui lo teniamo caloroso e orientato al compito.
fn peer_system(owner: &str, shareable: &str, goal: &str, materials: &str) -> String {
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
    format!(
        "Sei Liara, l'assistente personale di {owner} (una persona reale di cui ti occupi): sei l'assistente di \
{owner}, non un assistente generico né una piattaforma. Stai parlando con l'assistente personale di un'ALTRA \
persona, a nome dei vostri utenti (non col tuo utente).\n\
{goal_block}\n\
Parla in prima persona come Liara, calorosa, concreta e BREVE (1-2 frasi). Non elencare funzionalità, non divagare, \
non ripeterti. Del tuo utente condividi SOLO le informazioni generali del profilo qui sotto; non aggiungere dati \
che non ci sono.\n\n\
PROFILO CONDIVISIBILE di {owner}:\n{shareable}{materials_block}"
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
    history: Vec<(String, String)>,
    goal: Option<String>,
    materials: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let (owner, shareable) = shareable_profile(&state);
    let system = peer_system(
        &owner,
        &shareable,
        goal.as_deref().unwrap_or(""),
        materials.as_deref().unwrap_or(""),
    );

    // messaggi in formato chat: peer→user (input), me→assistant (mie risposte precedenti)
    let mut msgs: Vec<serde_json::Value> = vec![serde_json::json!({ "role": "system", "content": system })];
    for (who, text) in &history {
        let role = if who == "me" { "assistant" } else { "user" };
        msgs.push(serde_json::json!({ "role": role, "content": text }));
    }
    // se l'ultimo è mio (assistant), non c'è nulla a cui rispondere
    if history.last().map(|(w, _)| w == "me").unwrap_or(true) && !history.is_empty() {
        return Err("nessun messaggio del peer a cui rispondere".into());
    }

    let cloud = crate::core::paths::models_base().join("cloud_active").exists();
    if cloud {
        cloud_reply(msgs).await
    } else {
        local_reply(&state, msgs).await
    }
}

/// Chiamata cloud NON-streaming al 32B (niente tool): una singola risposta breve.
async fn cloud_reply(msgs: Vec<serde_json::Value>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let body = serde_json::json!({
            "model": crate::commands::remote::api_model(),
            "messages": msgs,
            "stream": false,
            "temperature": 0.7,
            "top_p": 0.9,
            "frequency_penalty": 0.4,
            "presence_penalty": 0.3,
            "max_tokens": 400,
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
async fn local_reply(state: &AppState, msgs: Vec<serde_json::Value>) -> Result<String, String> {
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
        let opts = crate::core::engine::GenOptions { max_tokens: 300, stop: vec!["<|im_end|>".into()], ..Default::default() };
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
