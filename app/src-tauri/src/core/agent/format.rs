//! Prompt formatting (ChatML / Qwen): the Message type, system + extraction prompts.
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
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
Rispondi in italiano, chiara e concisa. Non firmarti.";

/// Render a full conversation into the ChatML prompt (Qwen family),
/// leaving the assistant turn open for generation.
///
/// `thinking` (Qwen3): ON di default (il LoRA v6 usa il ragionamento per chiamare i tool; senza,
/// i tool non partono o vengono usati male). Il frontend lo commuta con
/// `set_thinking` → vive in `AppState.thinking` e arriva qui come PARAMETRO
/// (review 2026-07-02 #6: era una statica globale che accoppiava moduli lontani).
pub fn format_chat(system: &str, messages: &[Message], thinking: bool, gemma: bool) -> String {
    if gemma {
        return format_chat_gemma(system, messages);
    }
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

/// Prompt in formato GEMMA 4 (<|turn>/<turn|>). Il system va nel PRIMO turno user
/// (Gemma non ha un system role separato nel suo template). Ruoli: assistant→model, tutto il resto
/// (user, tool)→user. NIENTE prefill <think> (è di Qwen3). Così Gemma non vede i marker ChatML e
/// smette di leakare <|im_end|> / <|tool_response|> nelle risposte.
fn format_chat_gemma(system: &str, messages: &[Message]) -> String {
    let mut p = String::new();
    let mut sys_injected = false;
    for m in messages {
        let role = if m.role == "assistant" { "model" } else { "user" };
        p.push_str("<|turn>");
        p.push_str(role);
        p.push('\n');
        if role == "user" && !sys_injected && !system.is_empty() {
            p.push_str(system);
            p.push_str("\n\n");
            sys_injected = true;
        }
        p.push_str(&m.content);
        p.push_str("<turn|>\n");
    }
    p.push_str("<|turn>model\n");
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
        let p = format_chat("sys", &msgs, false, false);
        assert!(p.contains("<|im_start|>system\nsys<|im_end|>"));
        assert!(p.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"));

        // thinking ON: nessun prefill, il modello ragiona da sé.
        let p_think = format_chat("sys", &msgs, true, false);
        assert!(p_think.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn format_chat_gemma_uses_start_of_turn_no_chatml() {
        let msgs = vec![Message { role: "user".into(), content: "ciao".into() }];
        let p = format_chat("sys", &msgs, false, true);
        // Anti-leak: il prompt Gemma NON deve contenere i marker ChatML (che Gemma imiterebbe
        // generando <|im_end|>), né il prefill <think> (è di Qwen3).
        assert!(!p.contains("<|im_start|>"));
        assert!(!p.contains("<|im_end|>"));
        assert!(!p.contains("<think>"));
        // system nel PRIMO turno user, chiusura col turno model aperto.
        assert!(p.contains("<|turn>user\nsys\n\nciao<turn|>"));
        assert!(p.ends_with("<|turn>model\n"));
    }
}
