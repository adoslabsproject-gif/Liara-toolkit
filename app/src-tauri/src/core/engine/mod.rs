//! Inference engine abstraction.
//! Implementations plug in behind this trait so agent/memory/RAG never change.
use anyhow::Result;
use std::sync::atomic::AtomicBool;

pub(crate) mod utf8;
pub mod llama;
pub use llama::LlamaEngine;

use anyhow::Context;
use llama_cpp_2::llama_backend::LlamaBackend;
use std::sync::{Mutex, OnceLock};

/// UN SOLO LlamaBackend globale, condiviso. llama.cpp NON permette init multipli: una seconda
/// `LlamaBackend::init()` FALLISCE. La visione ora è NATIVA nel modello di testo (Gemma 4 vede col
/// proprio mmproj via `load_vl` → stesso LlamaEngine fa testo+immagini): niente più motore VL
/// separato. Il backend condiviso resta perché più contesti (warmup + describe) lo toccano in
/// concorrenza. Leaked a 'static, coerente col design (modelli già leaked per il prefix-caching).
pub fn shared_backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<&'static LlamaBackend> = OnceLock::new();
    if let Some(b) = BACKEND.get() {
        return Ok(*b);
    }
    // double-checked locking: init una sola volta anche con thread concorrenti (warmup + describe)
    static INIT_LOCK: Mutex<()> = Mutex::new(());
    let _g = INIT_LOCK.lock().unwrap();
    if let Some(b) = BACKEND.get() {
        return Ok(*b);
    }
    let b: &'static LlamaBackend =
        Box::leak(Box::new(LlamaBackend::init().context("llama backend init")?));
    let _ = BACKEND.set(b);
    Ok(b)
}

#[derive(Clone, Debug)]
pub struct GenOptions {
    pub max_tokens: usize,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub min_p: f32,
    /// Repetition penalty (>1 discourages loops on small models).
    pub repeat_penalty: f32,
    /// Optional GBNF grammar to force valid output (e.g. JSON tool calls).
    pub grammar: Option<String>,
    pub stop: Vec<String>,
    /// KV-cache slot: 0 = main conversation (prefix-cached across turns), 1 = auxiliary
    /// (fact extraction etc.) so it doesn't evict the conversation's cache.
    pub cache_slot: u8,
}
impl Default for GenOptions {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.05,
            repeat_penalty: 1.1,
            grammar: None,
            stop: vec![],
            cache_slot: 0,
        }
    }
}

/// A streamed, on-device inference engine.
pub trait Engine: Send + Sync {
    fn id(&self) -> &str;
    /// Generate; `on_token` is called for every new token (streaming).
    /// The loop stops early if `cancel` becomes true (Stop button).
    fn generate(
        &self,
        prompt: &str,
        opts: &GenOptions,
        cancel: &AtomicBool,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String>;
    /// Embed text (shared by memory + RAG). Implemented in a later module.
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    /// Describe an image (multimodal). Default: text-only engines don't support it. The unified
    /// VL engine (Android: stesso modello per testo E visione → niente swap, niente crash) lo fa.
    fn describe(
        &self,
        _image: &[u8],
        _prompt: &str,
        _max_tokens: usize,
        _cancel: &AtomicBool,
        _on_token: &mut dyn FnMut(&str),
    ) -> Result<String> {
        anyhow::bail!("questo motore non supporta la visione")
    }
    /// Whether this engine can describe images (the VL engine loaded its mmproj).
    fn has_vision(&self) -> bool {
        false
    }
    /// Whether this is a Gemma model. The agent loop uses this to speak Gemma's prompt/tool
    /// dialect (<start_of_turn>/<end_of_turn>) instead of Qwen ChatML — otherwise Gemma imitates
    /// the Qwen markers it sees in the prompt and leaks <|im_end|> / <|tool_response|> into replies.
    fn is_gemma(&self) -> bool {
        false
    }
    /// Dialetto di prompt del modello: guida template, EOS atomico, formato tool-call e risultato
    /// (`agent_loop`). Ogni famiglia parla il suo nativo → niente marker testuali fragili. Default Qwen.
    fn dialect(&self) -> crate::core::agent::Dialect {
        crate::core::agent::Dialect::Qwen
    }
}
