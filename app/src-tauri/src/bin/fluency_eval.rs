//! EVAL-FLUENZA per-modello — gira i prompt conversazionali sull'ENGINE REALE dell'app
//! (stesso `LlamaEngine`, stesso `SYSTEM_PROMPT` runtime, stesso `format_chat`, stesso sampling
//! anti-loop di `agent_loop`), così i numeri riflettono il comportamento in produzione, non un
//! setup finto. Serve a scegliere il base dei modelli piccoli (Qwen cinese-pesante vs Gemma) SUI DATI.
//!
//! Uso:  fluency_eval <model.gguf> <prompts.jsonl>   (stdout = JSONL {_cat,prompt,answer,think_len})
//! Env:  THINK=0 disattiva il reasoning (default = 1 = runtime reale: è quando i piccoli Qwen loopano).
//!
//! Il detector meccanico (loop/runaway/encoding/inglese) e la lettura a mano stanno in
//! `eval/fluency_eval_runner.py::score` — questo bin PRODUCE solo le risposte reali.
use app_lib::core::agent::{format_chat, render_full, Dialect, Message, SYSTEM_PROMPT};
use app_lib::core::engine::{Engine, GenOptions, LlamaEngine};
use std::io::Write;
use std::sync::atomic::AtomicBool;

/// Isola la prosa che l'utente legge: tutto ciò che segue `</think>` (il reasoning va nel bubble a
/// parte, come fa lo StreamRouter runtime). Senza tag chiuso → è già prosa diretta (thinking OFF).
fn strip_think(s: &str) -> &str {
    match s.rfind("</think>") {
        Some(i) => s[i + "</think>".len()..].trim_start(),
        None => s.trim_start(),
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let model = args.next().expect("uso: fluency_eval <model.gguf> <prompts.jsonl>");
    let prompts_path = args.next().expect("uso: fluency_eval <model.gguf> <prompts.jsonl>");
    let thinking = std::env::var("THINK").ok().as_deref() != Some("0"); // default ON = runtime
    let gemma = model.to_lowercase().contains("gemma");
    let bitnet = model.to_lowercase().contains("bitnet");
    eprintln!("[carico] {model}  (gemma={gemma}, bitnet={bitnet}, thinking={thinking})");
    let eng = LlamaEngine::load(&model, 4096, 999)?;
    eprintln!("[ok] engine su\n");

    let data = std::fs::read_to_string(&prompts_path)?;
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line)?;
        let prompt = v["prompt"].as_str().unwrap_or("");
        let cat = v["_cat"].as_str().unwrap_or("");
        let msgs = [Message { role: "user".into(), content: prompt.into() }];
        // prompt + sampling IDENTICI ad agent_loop.rs (risposta finale): max 1024, temp 0.7, stop per
        // famiglia; le penalty anti-loop (finestra 256 + freq/presence 0.4) le applica build_sampler
        // dentro `generate`, quindi non vanno ripetute qui.
        // BitNet-2B-4T ha il SUO template («Role: content<|eot_id|>» + «Assistant: »), NON chatml:
        // costruirlo a mano è l'unico modo per un test FEDELE (l'app non ha ancora un ramo BitNet in
        // format_chat). Qwen/Gemma restano su format_chat.
        // Dialetto REALE dal motore (stessa detection del runtime) → train==runtime anche nell'eval.
        let dialect = eng.dialect();
        let full = if bitnet {
            format!("System: {SYSTEM_PROMPT}<|eot_id|>User: {prompt}<|eot_id|>Assistant: ")
        } else {
            format_chat(SYSTEM_PROMPT, &msgs, thinking, dialect, None)
        };
        let opts = GenOptions {
            max_tokens: 1024,
            temperature: 0.7,
            stop: if bitnet {
                vec!["<|eot_id|>".into()]
            } else {
                match dialect {
                    Dialect::Qwen => vec!["</tool_call>".into(), "<|im_end|>".into()],
                    Dialect::Gemma => vec!["<tool_call|>".into(), "</tool_call>".into(), "<end_of_turn>".into()],
                    Dialect::Mistral => vec!["</s>".into()],
                    Dialect::Cohere => vec!["<|END_OF_TURN_TOKEN|>".into()],
                }
            },
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let raw = eng.generate(&full, &opts, &cancel, &mut |_| {}).unwrap_or_default();
        // FEDELTÀ RUNTIME: passa l'output per lo StreamRouter dell'app (nasconde thinking-channel Gemma
        // e blocco tool-call, come in produzione) → poi strip del <think> di Qwen (isolato nel bubble).
        // Così `answer` == la prosa che l'utente LEGGE, non i marker interni: i numeri non si gonfiano.
        let visible = render_full(&raw);
        let answer = strip_think(&visible);
        let think_len = raw.len().saturating_sub(answer.len());
        let out = serde_json::json!({
            "_cat": cat, "prompt": prompt, "answer": answer, "think_len": think_len,
        });
        // Flush per riga: il teardown Metal di llama.cpp aborta all'uscita del processo (bug upstream
        // noto, ggml-metal-device.m GGML_ASSERT rsets), quindi lo stdout bufferizzato su file andrebbe
        // perso. Scrivendo+flushando ogni riga, l'output è completo PRIMA di qualunque crash d'uscita.
        let mut so = std::io::stdout();
        writeln!(so, "{out}").ok();
        so.flush().ok();
        eprintln!("· [{cat}] {}… → {}ch (think {}ch)",
            &prompt.chars().take(28).collect::<String>(), answer.len(), think_len);
    }
    Ok(())
}
