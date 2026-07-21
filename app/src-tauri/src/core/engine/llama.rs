//! llama.cpp engine (in-process, cross-platform) behind the `Engine` trait.
//!
//! Performance: the generation context is PERSISTENT and reuses the KV cache across
//! turns and tool-steps (prefix caching) — only the new suffix is decoded, not the whole
//! prompt every time. The embedding context is likewise created once and reused.
use super::{Engine, GenOptions};
use anyhow::{anyhow, Context, Result};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use llama_cpp_2::context::params::{KvCacheType, LlamaContextParams};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::mtmd::{mtmd_default_marker, MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

/// #10 FIX: lo slot ausiliario (>=1: extraction/judge/summarize) NON serve il contesto pieno.
/// Creava una 2a KV cache enorme (~940MB a n_ctx 8192/32K) subito dopo la risposta → OOM (signal 9).
/// Lo cappiamo a 2048 (basta anche a un summarize di conversazione media). Slot 0 = contesto pieno.
const AUX_SLOT_CTX: u32 = 2048;

/// Contesto del contesto di embedding (n_ctx E n_batch): l'input di embed() va
/// TRONCATO a questo (guard anti-SIGABRT, stessa classe del crash 0.1.6).
const EMBED_CTX: u32 = 2048;

/// Dimensione del batch di PREFILL. n_batch è QUANTI token si decodificano in un
/// colpo durante il prefill del prompt: piccolo = la GPU mobile respira tra i
/// chunk (primo token rapido, niente freeze→crash). NON è n_ctx (la finestra di
/// contesto resta piena).
/// Android/Adreno: 256 (non 512) → picco di memoria GPU più basso durante il prefill del prompt lungo
/// (system + 24 tool + memoria ≈ 3000 token) → riduce i "cede" INTERMITTENTI del backend OpenCL sotto
/// pressione RAM/GPU (crash agenda che appariva solo a memoria tirata). Desktop resta 512 (Metal/CUDA reggono).
#[cfg(target_os = "android")]
const PREFILL_BATCH: u32 = 256;
#[cfg(not(target_os = "android"))]
const PREFILL_BATCH: u32 = 512;

/// Contesto EFFETTIVO di uno slot. ⚠️ OGNI guard anti-overflow in generate() deve
/// usare QUESTO, non self.n_ctx: usare il n_ctx pieno su uno slot cappato faceva
/// passare prompt > 2048 al contesto da 2048 → eccezione C++ nel decode → SIGABRT
/// (la classe di crash del fix 0.1.6, che copriva solo lo slot 0).
fn slot_ctx(n_ctx: u32, slot: usize) -> u32 {
    if slot >= 1 { n_ctx.min(AUX_SLOT_CTX) } else { n_ctx }
}

/// Budget di token del PROMPT per uno slot: contesto effettivo meno la generazione
/// attesa e un margine. Mai 0: sotto al minimo si trimma comunque (poi il guard a
/// runtime ferma la generazione con grazia) — un budget 0 saltava il trim e il
/// prompt intero sfondava il contesto (SIGABRT).
fn prompt_budget(slot_ctx: u32, max_tokens: usize) -> usize {
    (slot_ctx as usize).saturating_sub(max_tokens + 64).max(64)
}

#[cfg(test)]
mod guard_tests {
    use super::{prompt_budget, slot_ctx, AUX_SLOT_CTX};

    #[test]
    fn slot_aux_e_cappato_slot0_pieno() {
        // ANTI-REGRESSIONE SIGABRT: il guard di generate() usa slot_ctx; se
        // tornasse a self.n_ctx, sullo slot 1 (2048) passerebbe un prompt da
        // 32K → eccezione C++ nel decode → abort dell'app (summarize lungo).
        assert_eq!(slot_ctx(32768, 0), 32768);
        assert_eq!(slot_ctx(32768, 1), AUX_SLOT_CTX);
        assert_eq!(slot_ctx(1024, 1), 1024); // mai sopra il n_ctx reale
    }

    #[test]
    fn budget_mai_zero_anche_con_max_tokens_enormi() {
        // budget==0 saltava il trim ("&& budget > 0") → prompt intero al decode.
        assert_eq!(prompt_budget(2048, 700), 2048 - 700 - 64);
        assert!(prompt_budget(1024, 1024) >= 64, "il trim deve scattare comunque");
        assert!(prompt_budget(2048, 4096) >= 64);
    }
}

/// A persistent generation context plus the tokens currently held in its KV cache.
struct GenState {
    ctx: LlamaContext<'static>,
    tokens: Vec<LlamaToken>,
}
/// SAFETY: the raw llama_context pointer is `!Send` by default, but a llama.cpp context
/// has no thread affinity and we access it only under a `Mutex` (never concurrently), so
/// moving it between worker threads sequentially is sound.
unsafe impl Send for GenState {}

struct EmbState {
    ctx: LlamaContext<'static>,
}
unsafe impl Send for EmbState {}

pub struct LlamaEngine {
    backend: &'static LlamaBackend,
    model: &'static LlamaModel,
    id: String,
    n_ctx: u32,
    threads: i32,
    // slot 0 = conversation, slot 1 = auxiliary (extraction) — independent prefix caches
    gen: [Mutex<Option<GenState>>; 2],
    emb: Mutex<Option<EmbState>>,
    // Confini-turno per il trim overflow, PER-DIALETTO: ChatML <|im_start|>/<|im_end|> (Qwen) e Gemma
    // <start_of_turn>/<end_of_turn>. Senza i marker Gemma, su un modello Gemma lo snap falliva e il
    // fallback tagliava a 640 → decapitava system+tool (bug catalogo 30-tool, mobile n_ctx 4096).
    turn_starts: Vec<LlamaToken>,
    turn_ends: Vec<LlamaToken>,
    // Multimodale (VL): il path del mmproj è noto da load_vl, ma il proiettore (~1.2GB) si carica
    // SOLO alla prima immagine (lazy) — caricarlo all'avvio insieme al modello causa OOM. All'avvio
    // resta in RAM solo il modello (testo). `mtmd` è il proiettore una volta caricato.
    mmproj_path: Option<String>,
    mtmd: Mutex<Option<MtmdContext>>,
    // seed del sampler conversazionale (per-engine, review #6)
    seed: std::sync::atomic::AtomicU32,
}

impl LlamaEngine {
    /// Carica un modello SOLO testo.
    pub fn load(model_path: &str, n_ctx: u32, n_gpu_layers: u32) -> Result<Self> {
        Self::build(model_path, n_ctx, n_gpu_layers, None)
    }
    /// Carica un modello VISIONE+testo (VL + mmproj): UN solo motore fa entrambi. Su Android è la
    /// chiave per avere la visione senza crash (niente swap text↔VL → niente race del teardown OpenCL).
    pub fn load_vl(model_path: &str, mmproj_path: &str, n_ctx: u32, n_gpu_layers: u32) -> Result<Self> {
        Self::build(model_path, n_ctx, n_gpu_layers, Some(mmproj_path))
    }
    fn build(model_path: &str, n_ctx: u32, n_gpu_layers: u32, mmproj: Option<&str>) -> Result<Self> {
        // Silenzia i log interni di llama.cpp: su Android stampava "Grammar still awaiting trigger"
        // su logcat AD OGNI token → I/O che può rallentare drammaticamente la generazione.
        llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default().with_logs_enabled(false));
        // Controlla l'esistenza PRIMA: load_from_file panica (non ritorna Err) su file mancante, e quel
        // panic avvelena il Mutex dell'engine → tutti i turni dopo falliscono con PoisonError. Meglio
        // un errore pulito mostrato in chat.
        if !std::path::Path::new(model_path).exists() {
            anyhow::bail!("modello non trovato: {model_path}");
        }
        // DELIBERATE: backend+model are leaked to 'static so the PERSISTENT KV-cache contexts
        // (which borrow the model) can live in this struct across turns — that prefix-caching is
        // a core perf win. Trade-off (audit #9): swapping the model means restarting the app, and
        // a reload would leak ~2GB. Acceptable for a single-model assistant; a multi-model future
        // would need an engine-pool redesign (contexts owned alongside their model, no leak).
        let backend: &'static LlamaBackend = super::shared_backend()?;
        // mlock: blocca i pesi del modello in RAM. Così NON vengono mai paginati sul disco (niente
        // thrashing → niente "minuti per token") e costringe Android a sfrattare le app in background
        // per fare spazio → l'app si "prende" la RAM da sola, senza che l'utente debba liberarla.
        // mlock LOCCA il file modello in RAM (anti-thrashing sul desktop). MA su Android con la GPU
        // (OpenCL) i pesi sono GIÀ caricati nei buffer GPU (RAM condivisa): mlock terrebbe ANCHE una
        // seconda copia del file in RAM → ~2× memoria → l'app viene killata (OOM, signal 9). Su
        // Android NIENTE mlock: dopo l'upload alla GPU le pagine del file mmap si liberano.
        #[cfg(target_os = "android")]
        let model_params = LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers);
        #[cfg(not(target_os = "android"))]
        let model_params = LlamaModelParams::default()
            .with_n_gpu_layers(n_gpu_layers)
            .with_use_mlock(true);
        let model: &'static LlamaModel = Box::leak(Box::new(
            LlamaModel::load_from_file(backend, model_path, &model_params)
                .with_context(|| format!("loading model {model_path}"))?,
        ));
        let id = std::path::Path::new(model_path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".into());
        let threads = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4);
        // Marker single-token dei confini-turno, per OGNI dialetto che gira su questo engine (Qwen ChatML,
        // Gemma nativo), così il trim overflow snappa ai confini-messaggio a prescindere dal modello.
        // Solo i marker che tokenizzano a UN token speciale sono usabili per lo snap.
        let one_tok = |s: &str| model.str_to_token(s, AddBos::Never).ok().filter(|v| v.len() == 1).map(|v| v[0]);
        // per-dialetto: ChatML, Gemma nativo, Mistral ([INST]/</s>), Cohere. `one_tok` tiene SOLO i
        // marker che sono UN token per QUESTO modello → gli altri (non nel vocab) sono scartati, safe.
        let turn_starts: Vec<LlamaToken> = ["<|im_start|>", "<start_of_turn>", "[INST]", "<|START_OF_TURN_TOKEN|>"]
            .iter().filter_map(|s| one_tok(s)).collect();
        let turn_ends: Vec<LlamaToken> = ["<|im_end|>", "<end_of_turn>", "</s>", "<|END_OF_TURN_TOKEN|>"]
            .iter().filter_map(|s| one_tok(s)).collect();
        // mmproj: verifichiamo SOLO che esista (fail-fast), ma NON lo carichiamo qui — il proiettore
        // (~1.2GB) si carica alla prima immagine (lazy in describe) per non andare OOM all'avvio.
        if let Some(mm) = mmproj {
            if !std::path::Path::new(mm).exists() {
                anyhow::bail!("mmproj non trovato: {mm}");
            }
        }
        let mmproj_path = mmproj.map(|s| s.to_string());
        Ok(Self {
            backend,
            model,
            id,
            n_ctx,
            threads,
            gen: [Mutex::new(None), Mutex::new(None)],
            emb: Mutex::new(None),
            turn_starts,
            turn_ends,
            mmproj_path,
            mtmd: Mutex::new(None),
            seed: std::sync::atomic::AtomicU32::new(0x9E3779B9),
        })
    }

    fn ctx_params(&self, slot: usize) -> LlamaContextParams {
        // DIAGNOSTICA: KV f16 + niente flash-attention (FA su CPU può essere lentissima). Misuriamo
        // la velocità pura con i timer, poi decidiamo. (KV quant/FA reintrodotte solo se non rallentano.)
        let _ = KvCacheType::Q8_0; // (silenzia l'import durante la diagnostica)
        let n_ctx = slot_ctx(self.n_ctx, slot);
        LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            // 🔴 FIX CRASH INFERENZA (2026-07-02, verificato device S24): n_batch NON
            // deve essere = n_ctx. Col prompt fisso ~3000 token (system + catalogo 24
            // tool), un batch di prefill da 8192 satura la GPU Adreno per ~1 minuto
            // SENZA emettere token → l'app "sembra bloccata" → il sistema ricicla
            // l'Activity → tao chiama std::process::exit() mentre il thread di decode
            // tiene un mutex → "destroying mutex with owner or contenders" → SIGABRT.
            // Prefill a CHUNK di PREFILL_BATCH: la GPU respira tra i chunk, il primo
            // token arriva in pochi secondi, la finestra di crash si chiude.
            .with_n_batch(n_ctx.min(PREFILL_BATCH))
            .with_n_threads(self.threads)
            .with_n_threads_batch(self.threads)
    }

    fn build_sampler(&self, opts: &GenOptions) -> Result<LlamaSampler> {
        let mut s: Vec<LlamaSampler> = Vec::new();
        if let Some(g) = &opts.grammar {
            s.push(
                LlamaSampler::grammar_lazy(self.model, g, "root", ["<tool_call>"], &[])
                    .map_err(|e| anyhow::anyhow!("grammatica GBNF non valida: {e}"))?,
            );
        }
        if opts.temperature <= 0.0 {
            // #25 FIX: penalty leggera anche sul ramo greedy → l'extraction (temp 0, 220 tok) non entra
            // in loop di ripetizione deterministico. L'argmax resta, ma i token ripetuti sono penalizzati.
            s.push(LlamaSampler::penalties(256, 1.1, 0.0, 0.0));
            s.push(LlamaSampler::greedy());
        } else {
            // anti-loop chain for small models: repeat penalty + top_k + top_p + min_p + temp
            // Finestra AMPIA (256) + frequency/presence penalty: i modelli piccoli col reasoning entrano
            // in loop di ragionamento LUNGHI (il "papiro") che una finestra di 64 non copre più → poi
            // DEGENERANO in token spazzatura. 256 + freq/presence 0.4 spezza il loop prima del collasso.
            s.push(LlamaSampler::penalties(256, opts.repeat_penalty.max(1.15), 0.4, 0.4));
            s.push(LlamaSampler::top_k(opts.top_k));
            s.push(LlamaSampler::top_p(opts.top_p, 1));
            s.push(LlamaSampler::min_p(opts.min_p, 1));
            s.push(LlamaSampler::temp(opts.temperature));
            // #13 FIX: seed VARIABILE sul percorso conversazionale (era dist(1234) fisso → "Rigenera" dava
            // sempre la stessa risposta bit-a-bit, vanificando la temperatura). Contatore incrementale
            // PER-ENGINE (review #6: era una statica globale di funzione).
            // (extraction/judge sono greedy sopra → restano deterministici by design.)
            s.push(LlamaSampler::dist(
                self.seed.fetch_add(0x6D2B79F5, std::sync::atomic::Ordering::Relaxed) | 1,
            ));
        }
        Ok(LlamaSampler::chain_simple(s))
    }
}

impl Engine for LlamaEngine {
    fn id(&self) -> &str {
        &self.id
    }

    fn generate(
        &self,
        prompt: &str,
        opts: &GenOptions,
        cancel: &std::sync::atomic::AtomicBool,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String> {
        let slot = (opts.cache_slot as usize).min(self.gen.len() - 1);
        // Recupero dal mutex AVVELENATO: se un turno precedente è panicato durante il decode, un
        // `.unwrap()` qui farebbe panicare OGNI turno successivo (engine "brickato" fino al riavvio).
        // Recuperiamo lo stato E resettiamo la KV-cache (che potrebbe essere incoerente) → l'engine
        // riparte pulito invece di restare morto. (La cache si ricostruisce al prossimo prefill.)
        let mut guard = self.gen[slot].lock().unwrap_or_else(|poisoned| {
            let mut g = poisoned.into_inner();
            if let Some(st) = g.as_mut() {
                st.ctx.clear_kv_cache();
                st.tokens.clear();
            }
            g
        });
        if guard.is_none() {
            let ctx = self.model.new_context(self.backend, self.ctx_params(slot)).context("new context")?;
            *guard = Some(GenState { ctx, tokens: Vec::new() });
        }
        let st = guard.as_mut().unwrap();

        let mut tokens = self.model.str_to_token(prompt, AddBos::Always).context("tokenize")?;
        // CONTEXT-OVERFLOW GUARD (sliding window): a long-running chat would eventually exceed
        // n_ctx and crash on decode. Keep the system-prompt HEAD + the most-recent TAIL, drop
        // the middle, so positions always fit. The head preserves persona; the tail, continuity.
        // ⚠️ sul contesto EFFETTIVO dello slot (slot 1 = 2048), NON su self.n_ctx.
        let n_ctx_slot = slot_ctx(self.n_ctx, slot);
        let budget = prompt_budget(n_ctx_slot, opts.max_tokens);
        if tokens.len() > budget {
            // #4 FIX: la testa DEVE contenere il system+tool INTERO, non un taglio secco a 640 che lo
            // decapitava (ChatML malformato: system aperto e mai chiuso, e i tool/persona persi). La snappiamo
            // alla fine del PRIMO blocco — il system — includendo il primo <|im_end|>. Cap al budget se enorme.
            // Snap alla fine del PRIMO turno (system+tool) per QUALSIASI dialetto: primo turn-end
            // presente (ChatML <|im_end|> o Gemma <end_of_turn>). Il fallback 640 resta solo se il
            // modello non ha nessun marker noto (raro) → non decapita più i tool su Gemma.
            let head = tokens
                .iter()
                .position(|t| self.turn_ends.contains(t))
                .map(|p| (p + 1).min(budget))
                .unwrap_or_else(|| (budget / 3).min(640));
            let tail = budget.saturating_sub(head);
            let mut tail_start = tokens.len().saturating_sub(tail);
            // MESSAGE-AWARE: snap il taglio al prossimo inizio-turno (ChatML <|im_start|> o Gemma
            // <start_of_turn>) così si tengono turni INTERI e non si dà mai mezzo messaggio al modello.
            if let Some(off) = tokens[tail_start..]
                .iter()
                .position(|t| self.turn_starts.contains(t))
            {
                tail_start += off;
            }
            let mut trimmed = Vec::with_capacity(head + (tokens.len() - tail_start));
            trimmed.extend_from_slice(&tokens[..head]);
            trimmed.extend_from_slice(&tokens[tail_start..]);
            tokens = trimmed;
        }
        // longest common prefix with what's already in the KV cache
        let mut common = 0usize;
        while common < tokens.len() && common < st.tokens.len() && tokens[common] == st.tokens[common] {
            common += 1;
        }
        // always re-decode at least the last token (need fresh logits to sample from)
        if common >= tokens.len() {
            common = tokens.len().saturating_sub(1);
        }
        // evict the diverging suffix from the KV cache.
        // 🔴 IBRIDI/RICORRENTI (LFM2, Granite hybrid, Mamba…): llama.cpp NON supporta la rimozione
        // PARZIALE dello stato (una conv/SSM non si riavvolge a metà) → seq_rm ritorna FALSE. Il
        // vecchio `.ok()` buttava via il false, lo stato restava incoerente e il decode successivo
        // falliva ("decode prompt" al 2° turno del 1.2B). Fallback: cache azzerata e re-prefill
        // COMPLETO — sui piccoli è questione di secondi, la correttezza vince sul prefix-caching.
        if common < st.tokens.len() {
            let partial_ok = st
                .ctx
                .clear_kv_cache_seq(Some(0), Some(common as u32), None)
                .unwrap_or(false);
            if partial_ok {
                st.tokens.truncate(common);
            } else {
                st.ctx.clear_kv_cache_seq(Some(0), None, None).ok();
                st.tokens.clear();
                common = 0;
            }
        }

        // decode only the new suffix, IN CHUNK di PREFILL_BATCH token.
        // 🔴 FIX CRASH INFERENZA (2026-07-02, device S24): prima si decodificava
        // TUTTO il suffix in UN batch. Col prompt ~3000 token (system+24 tool) e
        // n_batch=512 → GGML_ASSERT(n_tokens_all <= n_batch) → SIGABRT immediato;
        // e anche con n_batch grande, un batch da 3000 su GPU Adreno bloccava per
        // ~1 min → l'app "sembrava ferma" → il sistema uccideva l'Activity →
        // process::exit durante il lock → "destroying mutex" → crash. A chunk la
        // GPU respira e il primo token arriva in pochi secondi.
        let suffix = &tokens[common..];
        let last = tokens.len() - 1;
        let chunk_size = (PREFILL_BATCH as usize).max(1);
        let mut batch = LlamaBatch::new(chunk_size, 1);
        let t_prefill = std::time::Instant::now();
        let mut sample_idx = 0i32;
        let mut off = 0usize;
        while off < suffix.len() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break; // prefill lungo interrompibile: niente decode zombie
            }
            let end = (off + chunk_size).min(suffix.len());
            batch.clear();
            for j in off..end {
                let pos = common + j;
                batch.add(suffix[j], pos as i32, &[0], pos == last)?;
            }
            st.ctx.decode(&mut batch).context("decode prompt")?;
            sample_idx = batch.n_tokens() - 1;
            off = end;
        }
        let prefill_ms = t_prefill.elapsed().as_millis();
        st.tokens = tokens;

        let mut sampler = self.build_sampler(opts)?;
        let mut out = String::new();
        let mut utf8 = super::utf8::Utf8Stream::new();
        let mut n_cur = st.tokens.len() as i32;
        let mut generated = 0usize;

        // #FIX CRASH (SIGABRT "Rust cannot catch foreign exceptions"): il decode NON deve MAI superare
        // il contesto DELLO SLOT. Se il KV cache si riempie (reasoning-papiro lungo), llama.cpp lancia
        // un'eccezione C++ che Rust NON può catturare → abort()/SIGABRT istantaneo dell'intera app. Ci
        // fermiamo con GRAZIA un passo prima del limite: la risposta finisce troncata, ma l'app resta viva.
        let n_ctx_limit = n_ctx_slot as i32 - 1;
        let t_decode = std::time::Instant::now();
        while generated < opts.max_tokens {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            if n_cur >= n_ctx_limit {
                on_token("…");
                break;
            }
            let token = sampler.sample(&st.ctx, sample_idx);
            sampler.accept(token);
            if self.model.is_eog_token(token) {
                break;
            }

            let bytes = self.model.token_to_piece_bytes(token, 32, true, None)?;
            let piece = utf8.push(&bytes);
            if !piece.is_empty() {
                on_token(&piece);
                out.push_str(&piece);
            }
            if !opts.stop.is_empty() && opts.stop.iter().any(|s| out.ends_with(s)) {
                break;
            }

            batch.clear();
            batch.add(token, n_cur, &[0], true)?;
            // 🔴 CONSISTENZA cache↔st.tokens: DECODIFICA PRIMA di aggiornare lo stato. Se il decode
            // fallisce (ritorna Err), st.tokens NON deve contenere un token che la KV-cache non ha:
            // altrimenti il turno DOPO trova un prefisso "valido" più lungo del reale → posizioni
            // sfalsate → decode in errore/degenere → la CONVERSAZIONE resta rotta finché non se ne
            // apre una NUOVA (che diverge dal system e resetta la cache). Era il bug "Liara non
            // disponibile, ma basta una nuova chat". Ora l'errore è pulito e non corrompe lo stato.
            st.ctx.decode(&mut batch).context("decode")?;
            st.tokens.push(token);
            n_cur += 1;
            generated += 1;
            sample_idx = 0;
        }
        let dt = t_decode.elapsed().as_secs_f64();
        eprintln!(
            "LIARA-TIMING prefill={}ms decode={}tok {:.2}s = {:.2} tok/s",
            prefill_ms, generated, dt, generated as f64 / dt.max(0.001)
        );
        Ok(out)
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        use llama_cpp_2::context::params::LlamaPoolingType;
        let mut guard = self.emb.lock().unwrap();
        if guard.is_none() {
            let params = LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(EMBED_CTX))
                .with_n_batch(EMBED_CTX)
                .with_embeddings(true)
                .with_pooling_type(LlamaPoolingType::Mean)
                .with_n_threads(self.threads)
                .with_n_threads_batch(self.threads);
            let c = self.model.new_context(self.backend, params).context("embed context")?;
            *guard = Some(EmbState { ctx: c });
        }
        let ctx = &mut guard.as_mut().unwrap().ctx;
        ctx.clear_kv_cache(); // each embedding is independent
        let mut tokens = self.model.str_to_token(text, AddBos::Always).context("tokenize embed")?;
        // GUARD anti-SIGABRT: il contesto embedding è 2048; un testo più lungo
        // (messaggio incollato, chunk documento) sfonderebbe n_batch → eccezione
        // C++ nel decode → abort dell'app. Tronchiamo: per la similarità semantica
        // i primi 2048 token bastano, e un embedding parziale >> un crash.
        tokens.truncate(EMBED_CTX as usize);
        let mut batch = LlamaBatch::new(tokens.len().max(8), 1);
        let last = tokens.len().saturating_sub(1);
        for (i, tok) in tokens.iter().enumerate() {
            batch.add(*tok, i as i32, &[0], i == last)?;
        }
        ctx.decode(&mut batch).context("decode embed")?;
        let emb = ctx.embeddings_seq_ith(0).map_err(|e| anyhow::anyhow!("embeddings: {e}"))?;
        let mut v = emb.to_vec();
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }

    fn has_vision(&self) -> bool {
        self.mmproj_path.is_some()
    }
    fn is_gemma(&self) -> bool {
        self.id.to_lowercase().contains("gemma")
    }
    /// Dialetto dal nome file del GGUF (il catalogo `dialect` è la fonte a monte; qui il segnale
    /// robusto per l'engine). gemma→Gemma · mistral/nemo/ministral/velvet→Mistral ·
    /// aya/command-r/cohere→Cohere · resto (qwen, lfm2, hermes)→Qwen/ChatML.
    fn dialect(&self) -> crate::core::agent::Dialect {
        use crate::core::agent::Dialect;
        let id = self.id.to_lowercase();
        if id.contains("gemma") {
            Dialect::Gemma
        } else if id.contains("mistral") || id.contains("nemo") || id.contains("ministral") || id.contains("velvet") {
            Dialect::Mistral
        } else if id.contains("aya") || id.contains("command-r") || id.contains("commandr") || id.contains("cohere") {
            Dialect::Cohere
        } else {
            Dialect::Qwen
        }
    }

    /// Descrive un'immagine usando lo STESSO modello (VL) — niente secondo motore, niente swap.
    fn describe(
        &self,
        image: &[u8],
        prompt: &str,
        max_tokens: usize,
        cancel: &AtomicBool,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String> {
        // RAM-saving (telefono): libera le cache KV del testo (gen) PRIMA della visione → fa spazio
        // al mmproj + all'encoder immagine, evitando l'OOM. Il testo riprefilla al prossimo messaggio.
        for slot in &self.gen {
            *slot.lock().unwrap() = None;
        }
        // Carica il mmproj alla PRIMA immagine (lazy) e tienilo per le successive.
        let mut guard = self.mtmd.lock().unwrap();
        if guard.is_none() {
            let mm = self
                .mmproj_path
                .as_ref()
                .ok_or_else(|| anyhow!("questo motore non ha la visione"))?;
            // L'encoder immagine (clip) sull'Adreno OpenCL crasha (bug noto del VLM su Adreno). Su
            // Android lo facciamo girare su CPU (use_gpu:false) — è leggero — mentre il LINGUAGGIO
            // resta sulla GPU. Su desktop tutto su GPU (Metal gestisce bene il VLM).
            #[cfg(target_os = "android")]
            let use_gpu = false;
            #[cfg(not(target_os = "android"))]
            let use_gpu = true;
            let params = MtmdContextParams { use_gpu, n_threads: self.threads, ..Default::default() };
            *guard = Some(
                MtmdContext::init_from_file(mm, self.model, &params)
                    .map_err(|e| anyhow!("init mtmd (mmproj): {e}"))?,
            );
        }
        let mtmd = guard.as_ref().unwrap();
        // Contesto visione ridotto (1024): un'immagine 512px ≈ poche centinaia di token + ~400 di
        // descrizione → 1024 basta, e la KV cache più piccola risparmia RAM (anti-OOM sul telefono).
        let vis_ctx: u32 = self.n_ctx.min(1024);
        let cparams = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(vis_ctx))
            .with_n_batch(vis_ctx)
            .with_n_threads(self.threads)
            .with_n_threads_batch(self.threads);
        let mut ctx = self.model.new_context(self.backend, cparams).context("vision context")?;

        let q = if prompt.trim().is_empty() { "Descrivi dettagliatamente questa immagine." } else { prompt };
        let marker = mtmd_default_marker();
        // Template del VL: Gemma NATIVO <start_of_turn>/<end_of_turn>; Qwen2.5-VL usa <|im_start|>.
        // Col template sbagliato il modello riceve un prompt malformato → descrizione degradata.
        let text = if self.id.to_lowercase().contains("gemma") {
            format!("<bos><start_of_turn>user\n{marker}{q}<end_of_turn>\n<start_of_turn>model\n")
        } else {
            format!(
                "<|im_start|>system\nSei Liara, un'assistente attenta. Rispondi in italiano.<|im_end|>\n\
<|im_start|>user\n{marker}{q}<|im_end|>\n<|im_start|>assistant\n"
            )
        };
        let bitmap = MtmdBitmap::from_buffer(mtmd, image, false)
            .map_err(|e| anyhow!("immagine non leggibile: {e}"))?;
        let input = MtmdInputText { text, add_special: true, parse_special: true };
        let chunks = mtmd.tokenize(input, &[&bitmap]).map_err(|e| anyhow!("tokenize immagine: {e}"))?;

        let n_batch = vis_ctx as i32;
        let mut n_past = chunks
            .eval_chunks(mtmd, &ctx, 0, 0, n_batch, true)
            .map_err(|e| anyhow!("valutazione immagine: {e}"))?;

        let mut sampler = LlamaSampler::chain_simple(vec![
            // #12 FIX: catena anti-loop anche nella descrizione immagine — i VL piccoli loopano
            // ("un tavolo, un tavolo…") e la descrizione degenerata inquina il ReAct. Come build_sampler.
            LlamaSampler::penalties(256, 1.3, 0.4, 0.4),
            LlamaSampler::top_k(40),
            LlamaSampler::min_p(0.05, 1),
            LlamaSampler::temp(0.3),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::dist(1234),
        ]);
        let mut out = String::new();
        let mut utf8 = super::utf8::Utf8Stream::new();
        let mut generated = 0usize;
        while generated < max_tokens {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            // GUARD anti-SIGABRT (stessa classe del fix 0.1.6, che copriva solo il
            // testo): decodificare alla posizione >= vis_ctx lancia un'eccezione C++
            // che Rust non cattura → abort. Ci fermiamo con grazia un passo prima.
            if n_past >= vis_ctx as i32 - 1 {
                on_token("…");
                break;
            }
            let token = sampler.sample(&ctx, -1);
            sampler.accept(token);
            if self.model.is_eog_token(token) {
                break;
            }
            let bytes = self.model.token_to_piece_bytes(token, 32, true, None)?;
            let piece = utf8.push(&bytes);
            if !piece.is_empty() {
                on_token(&piece);
                out.push_str(&piece);
            }
            let mut batch = LlamaBatch::new(1, 1);
            batch.add(token, n_past, &[0], true)?;
            ctx.decode(&mut batch).context("decode vision")?;
            n_past += 1;
            generated += 1;
        }
        Ok(out)
    }
}
