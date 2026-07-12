use app_lib::core::engine::{Engine, GenOptions, LlamaEngine};
use std::io::Write;

fn cos(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn gen(eng: &LlamaEngine, user: &str) -> String {
    let prompt = format!(
        "<|im_start|>system\nSei Liara, assistente locale.<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n"
    );
    let opts = GenOptions { max_tokens: 80, temperature: 0.7, stop: vec!["<|im_end|>".into()], ..Default::default() };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    eng.generate(&prompt, &opts, &cancel, &mut |t| { print!("{t}"); std::io::stdout().flush().ok(); }).unwrap_or_default()
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("uso: smoke <model.gguf>");
    eprintln!("[carico] {path}");
    let eng = LlamaEngine::load(&path, 4096, 999)?;
    eprintln!("[ok] {}\n", eng.id());

    // due generazioni di fila sullo stesso engine (slot 0): la 2ª riusa il prefisso (system) via KV-cache
    eprintln!("--- gen 1 ---");
    let g1 = gen(&eng, "In una frase: chi sei?");
    eprintln!("\n--- gen 2 (prefix-cached) ---");
    let g2 = gen(&eng, "Quanto fa 12 per 8? Solo il numero.");
    eprintln!();
    let ok_gen = !g1.trim().is_empty() && !g2.trim().is_empty() && g2.contains("96");
    eprintln!("gen coerenti + cache non corrotta: {}", if ok_gen { "✅" } else { "⚠️ (controlla 2ª risposta)" });

    // embeddings (context riusato)
    let e1 = eng.embed("Il gatto dorme")?;
    let e2 = eng.embed("Un felino riposa")?;
    let e3 = eng.embed("La borsa è in rialzo")?;
    eprintln!("embeddings dim {}: sim(simili)={:.3} > sim(diversi)={:.3} → {}",
        e1.len(), cos(&e1, &e2), cos(&e1, &e3),
        if cos(&e1, &e2) > cos(&e1, &e3) { "✅" } else { "❌" });
    Ok(())
}
