//! Prompt formatting (ChatML/Qwen · Gemma · Mistral): the Message type, system + extraction prompts.
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Dialetto di prompt del modello LOCALE. Ogni famiglia parla il SUO formato nativo, chiuso dal SUO
/// **EOS atomico** (un token vero, non un marker testuale da "compitare"): così il papiro/marker-leak
/// sparisce per costruzione — il modello non deve imitare `<|im_end|>` a mano, lo emette il tokenizer.
/// Rilevato dal modello (`Engine::dialect`). Prima era un `bool gemma`; enum perché i dialetti sono
/// quattro e aggiungerne uno resta una modifica localizzata ("come si fece per Gemma").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    /// ChatML (Qwen, LFM2, Hermes): `<|im_start|>role … <|im_end|>`, tool-call `<tool_call>{json}</tool_call>`.
    Qwen,
    /// Gemma: `<|turn>role … <turn|>`, system nel PRIMO user, tool-call nativo. EOS `<turn|>`.
    Gemma,
    /// Mistral (Nemo, Ministral, Velvet): `[INST]…[/INST]`, system nell'ULTIMO user, tool-call
    /// `[TOOL_CALLS][{json}]`, risultato `[TOOL_RESULTS]…[/TOOL_RESULTS]`. EOS `</s>`. (Nemo-2407.jinja)
    Mistral,
    /// Cohere (Aya Expanse, Command-R): `<|START_OF_TURN_TOKEN|><|ROLE_TOKEN|>…<|END_OF_TURN_TOKEN|>`.
    /// EOS `<|END_OF_TURN_TOKEN|>`. Tool-call nativo Cohere (JSON list) — vedi parse/format.
    Cohere,
}

/// SYSTEM_PROMPT MINIMO (2026-07-03): distillato da ~690 a ~110 token. Le regole
/// dettagliate di ogni tool (email_recent vs reply, formato date, chart, anti-invenzione
/// per-caso) NON stanno più qui: sono INTERIORIZZATE NEI PESI dal dataset (i gold
/// conversazionali + gli esempi tool le dimostrano nelle RISPOSTE, che il masking allena).
/// È la stessa tecnica del 32B nha-v2 (system 2100→85 tok, distillato).
///
/// 🔴 PERCHÉ: il prompt fisso (system 690 + catalogo 2326 = ~3000 tok) faceva un prefill
/// così lungo da far crashare la GPU Adreno mobile (eccezione OpenCL), e troncato dava
/// rigurgito. Con system ~110 + catalogo compatto ~800 = ~900 tok: prefill corto (niente
/// crash), prompt COMPLETO (niente troncamento/rigurgito), spazio per rispondere e chiudere.
///
/// ⚠️ ANTI-DRIFT: questa costante è la FONTE. `dump_prompt` la esporta in system_prompt.txt,
/// il dataset di training la usa VERBATIM (persona.py) → training == runtime. Non modificare
/// senza riesportare e RI-ADDESTRARE, o il modello sbanda (mismatch di distribuzione).
pub const SYSTEM_PROMPT: &str =
    "Sei Liara, assistente personale locale e privata, con memoria dell'utente. \
USA SEMPRE gli strumenti per agire (email, agenda, file, web, meteo, note, calcoli, data e ora): \
non rispondere a parole quando puoi usare uno strumento. \
Quando uno strumento ti restituisce dei dati, riportali SUBITO e per intero: nomi, numeri, date, fonti. \
NON inventare MAI nomi, numeri, indirizzi o fatti su aziende, persone o luoghi reali: \
se non li sai con certezza usa web_search, e se non trovi nulla dillo con onestà. \
Parla in italiano in modo naturale e discorsivo, come in una vera conversazione: spiega quanto serve, \
e quando è utile fai una domanda di chiarimento o proponi il passo successivo. Evita le risposte \
telegrafiche, ma senza dilungarti. NON ripeterti e non ripetere quanto hai già detto. Non firmarti.";

/// Render a full conversation into the ChatML prompt (Qwen family),
/// leaving the assistant turn open for generation.
///
/// `thinking` (Qwen3): ON di default (il LoRA v6 usa il ragionamento per chiamare i tool; senza,
/// i tool non partono o vengono usati male). Il frontend lo commuta con
/// `set_thinking` → vive in `AppState.thinking` e arriva qui come PARAMETRO
/// (review 2026-07-02 #6: era una statica globale che accoppiava moduli lontani).
pub fn format_chat(system: &str, messages: &[Message], thinking: bool, dialect: Dialect) -> String {
    match dialect {
        Dialect::Gemma => format_chat_gemma(system, messages),
        Dialect::Mistral => format_chat_mistral(system, messages),
        Dialect::Cohere => format_chat_cohere(system, messages),
        Dialect::Qwen => format_chat_qwen(system, messages, thinking),
    }
}

fn format_chat_qwen(system: &str, messages: &[Message], thinking: bool) -> String {
    let mut p = String::new();
    p.push_str("<|im_start|>system\n");
    p.push_str(system);
    p.push_str("<|im_end|>\n");
    for m in messages {
        p.push_str("<|im_start|>");
        p.push_str(&m.role);
        p.push('\n');
        p.push_str(&m.content);
        p.push_str("<|im_end|>\n");
    }
    p.push_str("<|im_start|>assistant\n");
    // Thinking OFF: pre-riempiamo il blocco di ragionamento VUOTO, così Qwen3 lo considera già concluso
    // e genera SOLO la risposta. Metodo affidabile (il /no_think testuale non basta sui modelli tunati).
    // Su Qwen2.5 (1.5B) è innocuo. Thinking ON: il modello genera <think>...</think> → il frontend lo
    // isola nel bubble dedicato.
    if !thinking {
        p.push_str("<think>\n\n</think>\n\n");
    }
    p
}

/// Prompt in formato MISTRAL (Nemo/Small — template Mistral-Nemo-Instruct-2407). BOS, ogni user in
/// `[INST]…[/INST]`, assistant chiuso da `</s>` (EOS atomico), il SYSTEM è folded nell'ULTIMO turno
/// user (`[INST]{system}\n\n{content}[/INST]`) — Mistral non ha un system role separato nel template.
/// I tool-call e i risultati arrivano GIÀ testualizzati nel content dei messaggi (`[TOOL_CALLS][{…}]`,
/// `[TOOL_RESULTS]…[/TOOL_RESULTS]`, vedi agent_loop) → qui non li riprocessiamo. ⚠️ train==runtime:
/// deve combaciare col renderer `to_mistral`+mistral_common del training (check da fare col curatore).
fn format_chat_mistral(system: &str, messages: &[Message]) -> String {
    let last_user = messages.iter().rposition(|m| m.role == "user");
    let mut p = String::from("<s>");
    for (i, m) in messages.iter().enumerate() {
        match m.role.as_str() {
            "user" | "tool" => {
                p.push_str("[INST]");
                if Some(i) == last_user && !system.is_empty() {
                    p.push_str(system);
                    p.push_str("\n\n");
                }
                p.push_str(&m.content);
                p.push_str("[/INST]");
            }
            _ => {
                // assistant (anche coi tool-call già testualizzati nel content) → chiuso dall'EOS atomico
                p.push_str(&m.content);
                p.push_str("</s>");
            }
        }
    }
    p
}

/// Prompt in formato COHERE (Aya Expanse / Command-R). Turni delimitati da
/// `<|START_OF_TURN_TOKEN|><|ROLE_TOKEN|> … <|END_OF_TURN_TOKEN|>` (EOS atomico), preceduti da BOS.
/// Ruoli: system→SYSTEM, user/tool→USER, assistant→CHATBOT. Chiude col turno CHATBOT aperto.
fn format_chat_cohere(system: &str, messages: &[Message]) -> String {
    let mut p = String::from("<BOS_TOKEN>");
    if !system.is_empty() {
        p.push_str("<|START_OF_TURN_TOKEN|><|SYSTEM_TOKEN|>");
        p.push_str(system);
        p.push_str("<|END_OF_TURN_TOKEN|>");
    }
    for m in messages {
        let tok = if m.role == "assistant" { "<|CHATBOT_TOKEN|>" } else { "<|USER_TOKEN|>" };
        p.push_str("<|START_OF_TURN_TOKEN|>");
        p.push_str(tok);
        p.push_str(&m.content);
        p.push_str("<|END_OF_TURN_TOKEN|>");
    }
    p.push_str("<|START_OF_TURN_TOKEN|><|CHATBOT_TOKEN|>");
    p
}

/// Prompt in formato GEMMA NATIVO (`<start_of_turn>`/`<end_of_turn>`, EOS atomico a 1 token). Il system
/// va nel PRIMO turno user (Gemma non ha un system role separato). Ruoli: assistant→model, il resto
/// (user, tool)→user. BOS `<bos>` in testa. NIENTE prefill <think> (è di Qwen3).
/// 🔴 FIX (2026-07-18): prima usava i marker CUSTOM `<|turn>`/`<turn|>` (3-4 token, fragili → il modello
/// poteva non chiuderli = papiro, come Minerva). I nativi sono UN token atomico: impossibile sbagliarli.
/// ⚠️ train==runtime: i Gemma vanno RI-ADDESTRATI su questo formato (renderer `to_gemma` allineato).
fn format_chat_gemma(system: &str, messages: &[Message]) -> String {
    let mut p = String::from("<bos>");
    let mut sys_injected = false;
    for m in messages {
        let role = if m.role == "assistant" { "model" } else { "user" };
        p.push_str("<start_of_turn>");
        p.push_str(role);
        p.push('\n');
        if role == "user" && !sys_injected && !system.is_empty() {
            p.push_str(system);
            p.push_str("\n\n");
            sys_injected = true;
        }
        p.push_str(&m.content);
        p.push_str("<end_of_turn>\n");
    }
    p.push_str("<start_of_turn>model\n");
    p
}

/// Build the memory-extraction prompt. Broad on purpose: any durable fact that
/// helps know the user better.
pub fn extraction_prompt(user: &str, assistant: &str) -> String {
    let system = "Sei un estrattore di memoria personale. Dall'ultimo scambio estrai OGNI \
fatto durevole e utile per conoscere meglio l'UTENTE: nome, et\u{00e0}, famiglia e relazioni, \
lavoro o studio, dove vive, gusti e preferenze, obiettivi, abitudini, opinioni, progetti, \
date importanti, salute, qualsiasi cosa stabile. NON estrarre fatti temporanei, domande, \
o cose sull'assistente. Rispondi SOLO con un array JSON di stringhe brevi in italiano, \
in terza persona (es. [\"Si chiama Marco\", \"Ama il jazz\"]). Se non c'\u{00e8} nulla di utile, \
rispondi [].";
    let mut p = String::new();
    p.push_str("<|im_start|>system\n");
    p.push_str(system);
    p.push_str("<|im_end|>\n<|im_start|>user\nUTENTE: ");
    p.push_str(user);
    p.push_str("\nASSISTENTE: ");
    p.push_str(assistant);
    p.push_str("<|im_end|>\n<|im_start|>assistant\n");
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_chat_uses_chatml() {
        let msgs = vec![Message { role: "user".into(), content: "ciao".into() }];
        // thinking OFF (default mobile): il turno assistant è pre-riempito col
        // blocco <think> vuoto → Qwen3 genera SOLO la risposta (niente reasoning).
        let p = format_chat("sys", &msgs, false, Dialect::Qwen);
        assert!(p.contains("<|im_start|>system\nsys<|im_end|>"));
        assert!(p.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"));

        // thinking ON: nessun prefill, il modello ragiona da sé.
        let p_think = format_chat("sys", &msgs, true, Dialect::Qwen);
        assert!(p_think.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn format_chat_gemma_nativo_start_end_of_turn() {
        let msgs = vec![Message { role: "user".into(), content: "ciao".into() }];
        let p = format_chat("sys", &msgs, false, Dialect::Gemma);
        // native tokens ATOMICI, niente marker custom/ChatML né prefill <think>
        assert!(!p.contains("<|im_start|>") && !p.contains("<think>"));
        assert!(!p.contains("<|turn>") && !p.contains("<turn|>"), "niente marker custom vecchi");
        assert!(p.starts_with("<bos><start_of_turn>user\nsys\n\nciao<end_of_turn>\n"));
        assert!(p.ends_with("<start_of_turn>model\n"));
    }

    #[test]
    fn format_chat_mistral_inst_system_ultimo_user() {
        let msgs = vec![
            Message { role: "user".into(), content: "prima".into() },
            Message { role: "assistant".into(), content: "ok".into() },
            Message { role: "user".into(), content: "ciao".into() },
        ];
        let p = format_chat("sys", &msgs, false, Dialect::Mistral);
        assert!(p.starts_with("<s>[INST]prima[/INST]ok</s>"), "1° user senza system, assistant chiuso da </s>");
        // il SYSTEM è folded nell'ULTIMO turno user
        assert!(p.ends_with("[INST]sys\n\nciao[/INST]"));
        assert!(!p.contains("<|im_start|>") && !p.contains("<start_of_turn>"));
    }

    #[test]
    fn format_chat_cohere_start_end_of_turn_token() {
        let msgs = vec![Message { role: "user".into(), content: "ciao".into() }];
        let p = format_chat("sys", &msgs, false, Dialect::Cohere);
        assert!(p.starts_with("<BOS_TOKEN><|START_OF_TURN_TOKEN|><|SYSTEM_TOKEN|>sys<|END_OF_TURN_TOKEN|>"));
        assert!(p.contains("<|USER_TOKEN|>ciao<|END_OF_TURN_TOKEN|>"));
        assert!(p.ends_with("<|START_OF_TURN_TOKEN|><|CHATBOT_TOKEN|>"));
    }
}
