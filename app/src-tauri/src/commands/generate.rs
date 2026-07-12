//! The main streaming `generate` command (ReAct loop + memory formation) and stop control.
use crate::core::agent::{extraction_prompt, parse_facts, run_agent, Message, SYSTEM_PROMPT};
use crate::core::engine::{GenOptions, LlamaEngine};
use crate::AppState;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, State, WebviewWindow};

const SUPERSEDE_SIM: f32 = 0.82; // cosine above which a new fact may supersede a contradicting old one
/// #9 FIX: "GPU occupata". Il turno di conversazione la mette a true mentre gira; il task detached di
/// memory-formation (extraction/judge/reflection) SALTA se un turno è in corso → niente 2 decode
/// concorrenti sulla GPU mobile (Adreno/Mali) che saturano la coda e causano freeze/ANR.
const REFLECT_EVERY: i64 = 8; // consolidate recent episodes into durable reflections every N turns

/// RAII sul flag gpu_busy (vive in `AppState.gpu_busy`, review #6: niente più
/// statiche globali): il flag DEVE tornare false anche se il turno fallisce
/// (`run_agent(...)?`). Prima lo store(false) era una riga DOPO il `?`: al primo
/// errore il flag restava true PER SEMPRE → ogni memory-formation successiva
/// veniva saltata in silenzio fino al riavvio (memoria "morta" senza sintomi).
struct GpuBusyGuard(Arc<std::sync::atomic::AtomicBool>);
impl GpuBusyGuard {
    fn engage(flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        flag.store(true, Ordering::SeqCst);
        GpuBusyGuard(flag)
    }
}
impl Drop for GpuBusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod gpu_busy_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn guard_rilascia_su_drop_e_su_panic() {
        let flag = Arc::new(AtomicBool::new(false));
        // percorso normale
        {
            let _g = GpuBusyGuard::engage(flag.clone());
            assert!(flag.load(Ordering::SeqCst));
        }
        assert!(!flag.load(Ordering::SeqCst), "drop deve rilasciare");
        // percorso d'errore (unwind): è il caso che prima lasciava il flag a true
        let f2 = flag.clone();
        let _ = std::panic::catch_unwind(move || {
            let _g = GpuBusyGuard::engage(f2);
            panic!("turno fallito");
        });
        assert!(!flag.load(Ordering::SeqCst), "anche su errore il flag deve tornare false");
    }
}

/// Carica il motore principale. Su Android è UN SOLO modello VL (Qwen2.5-VL-3B) che fa testo E
/// visione → niente swap di modelli (lo swap crasha il backend OpenCL dell'Adreno). Contesto 2048
/// (anti-OOM). Su desktop è il modello testo (4B) a contesto pieno; la visione usa un motore a parte.
/// GPU che SATURANO con l'inferenza OpenCL bloccando l'UI (freeze → ANR "MainThread worked timeout"):
/// i MediaTek (Mali) e gli Snapdragon di fascia media (Adreno 6xx deboli, es. 765G "lito" dell'Oppo).
/// I TOP (S24 ecc.) reggono compute+UI insieme → tutto su GPU. Lista da estendere coi device che soffrono.
#[cfg(target_os = "android")]
fn needs_partial_gpu() -> bool {
    let p = std::process::Command::new("getprop")
        .arg("ro.board.platform")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_lowercase())
        .unwrap_or_default();
    p.starts_with("mt")                 // MediaTek / Mali
        || p.contains("mediatek")
        || p == "lito"                  // Snapdragon 765G (Adreno 620) — Oppo
        || p == "trinket" || p == "bengal" || p == "holi" // SD 6xx entry (Adreno deboli)
}

/// RAM realmente DISPONIBILE ORA (KB), da /proc/meminfo. Diversa da MemTotal: è ciò che il kernel può
/// dare senza uccidere processi. Serve a decidere quanti layer offloadare su GPU senza andare in OOM.
#[cfg(target_os = "android")]
fn mem_available_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    s.lines()
        .find(|l| l.starts_with("MemAvailable:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Capacità del dispositivo per il frontend: RAM totale (GB) e GPU debole. Il frontend BLOCCA il 4B
/// sui telefoni che non lo reggono (poca RAM o GPU media) → propone solo il Leggero.
#[tauri::command]
pub fn device_caps() -> serde_json::Value {
    #[cfg(target_os = "android")]
    {
        let ram_gb = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|kb| kb.parse::<u64>().ok())
            })
            .map(|kb| ((kb as f64 / 1024.0 / 1024.0) + 0.5) as u64)
            .unwrap_or(0);
        // `android` = fonte di VERITÀ della piattaforma (il backend la sa via cfg). Il frontend NON può
        // fidarsi di navigator.userAgent: sulla WebView Tauri Android spesso non contiene "Android" →
        // isAndroid falso → l'app mostrava il 12B desktop-only e scaricava il VL Qwen anche su telefono.
        serde_json::json!({ "ram_gb": ram_gb, "weak_gpu": needs_partial_gpu(), "android": true })
    }
    #[cfg(not(target_os = "android"))]
    {
        serde_json::json!({ "ram_gb": 16u64, "weak_gpu": false, "android": false })
    }
}

/// CRASH-DETECTOR GPU (Android). llama.cpp può lanciare un'eccezione C++ nel decode su OpenCL (es. il
/// 1.7B col thinking) che Rust NON può catturare → `abort`. Non potendo intercettarla, la RILEVIAMO:
/// prima di un decode GPU scriviamo `.gpu_pending=<model>`; a decode concluso un guard RAII lo cancella
/// (Drop gira su Ok e su Err, ma NON su abort). Se al load successivo il marker è ANCORA lì, quel modello
/// ha fatto abortire l'app → lo mettiamo in `.gpu_fallback` e lo carichiamo su CPU (n_gpu_layers=0):
/// più lento ma STABILE. Il 1.7B è piccolo, su CPU gira bene. Gli altri restano su GPU.
#[cfg(target_os = "android")]
pub(crate) mod gpu_guard {
    use std::path::PathBuf;
    fn base() -> PathBuf { crate::core::paths::models_base() }
    fn pending() -> PathBuf { base().join(".gpu_pending") }
    fn fallback() -> PathBuf { base().join(".gpu_fallback") }
    /// Un `.gpu_pending` sopravvissuto = decode GPU abortito prima → quel modello passa in fallback.
    pub fn absorb_crash() {
        if let Ok(m) = std::fs::read_to_string(pending()) {
            let m = m.trim();
            if !m.is_empty() && !in_fallback(m) {
                let mut cur = std::fs::read_to_string(fallback()).unwrap_or_default();
                cur.push_str(m);
                cur.push('\n');
                let _ = std::fs::write(fallback(), cur);
                eprintln!("LIARA_BOOT gpu-crash rilevato per {m} → CPU d'ora in poi");
            }
            let _ = std::fs::remove_file(pending());
        }
    }
    pub fn in_fallback(model: &str) -> bool {
        std::fs::read_to_string(fallback())
            .map(|s| s.lines().any(|l| l.trim() == model.trim()))
            .unwrap_or(false)
    }
    /// Guard RAII: marca il decode GPU in corso; il Drop lo cancella (NON gira su abort → marker resta).
    pub struct Pending;
    impl Pending {
        pub fn mark(model: &str) -> Self { let _ = std::fs::write(pending(), model); Pending }
    }
    impl Drop for Pending {
        fn drop(&mut self) { let _ = std::fs::remove_file(pending()); }
    }
}

pub(crate) fn load_engine(model_path: &str) -> anyhow::Result<LlamaEngine> {
    #[cfg(target_os = "android")]
    {
        // Android = SOLO testo, contesto 2048 (anti-OOM). Modello TUTTO su GPU (OpenCL) → max velocità.
        //
        // GPU offload. Su Adreno (Snapdragon) TUTTO su GPU = max velocità. Su Mali (MediaTek, es.
        // Oppo/Redmi) il compute OpenCL SATURA la GPU condivisa → la composizione UI si blocca → ANR
        // ("Input dispatching timed out, 5000ms") → Android killa l'app. Difesa: su Mali offload
        // PARZIALE (alcuni layer restano su CPU) → la GPU non satura → l'UI respira → niente ANR.
        // Resta comunque accelerato su GPU, solo "a fuoco più basso". Override con LIARA_GPU_LAYERS.
        // Crash-detector: se un decode GPU è abortito prima, il modello colpevole passa a CPU adesso.
        gpu_guard::absorb_crash();
        // Il 1.7B è INSTABILE sulla GPU Adreno: llama.cpp lancia un'eccezione C++ OpenCL (nel decode e
        // pure subito dopo il load) che Rust non può catturare → abort. 4B e Gemma reggono, il 1.7B no.
        // Lo forziamo su CPU su Android: è piccolo (1.1 GB), gira comunque veloce e SMETTE di crashare.
        let unstable_on_gpu = model_path.to_lowercase().contains("1.7b");
        let cpu_only = gpu_guard::in_fallback(model_path) || unstable_on_gpu;
        let weak = needs_partial_gpu();
        // OFFLOAD ADATTIVO ALLA RAM LIBERA (fix crash/hang di Gemma su APK, 2026-07-12, device S24 12GB
        // ma con ~3,6 GB liberi). I pesi su GPU (OpenCL) sono RESIDENTI e NON recuperabili dal kernel:
        // offloadare un modello più grande della RAM libera → lmkd ci uccide a metà load ("a volte
        // crasha") o il sistema thrasha paginando ("carica all'infinito"). I pesi mmap su CPU sono
        // invece file-backed → il kernel li recupera sotto pressione. Quindi per un modello troppo
        // grande offloadiamo SOLO i layer che ci stanno e lasciamo il resto mmap → niente OOM. Il 4B
        // (2,5 GB) continua a stare TUTTO su GPU come prima; solo Gemma (5,3 GB) va in parziale.
        let gpu_layers: u32 = if let Some(v) = std::env::var("LIARA_GPU_LAYERS").ok().and_then(|v| v.parse().ok()) {
            v // override manuale esplicito
        } else if cpu_only {
            0 // fallback CPU: questo modello ha già fatto abortire l'app su GPU (es. 1.7B col thinking)
        } else {
            let model_bytes = std::fs::metadata(model_path).map(|m| m.len()).unwrap_or(0);
            let avail = mem_available_kb().map(|k| k.saturating_mul(1024)).unwrap_or(u64::MAX);
            let full = if weak { 12 } else { 999 };
            // "Ci sta comodo": modello + ~1 GB di overhead non-recuperabile (KV cache, compute buffer
            // OpenCL, WebView, sistema) entra nella RAM libera. Empirico: il 4B (2,5 GB) gira a offload
            // PIENO con ~3,6 GB liberi → l'overhead reale è < ~1 GB. Così il 4B resta invariato.
            let fits = model_bytes == 0 || model_bytes.saturating_add(1_000_000_000) <= avail;
            if fits {
                full // ci sta tutto residente → offload pieno come prima (nessuna regressione sul 4B)
            } else {
                // NON ci sta (es. Gemma 5,3 GB con 3,6 GB liberi): offloadiamo solo una FRAZIONE
                // conservativa (~40% della RAM libera) come pesi GPU residenti; tutto il resto resta
                // mmap da disco (recuperabile dal kernel) → ampio slack per KV/compute/UI → niente OOM.
                // Più lento (paginazione), ma PARTE invece di crashare/appendersi. ~32 layer tipici.
                let gpu_target = (avail as f64 * 0.40) as u64;
                let est = ((gpu_target as f64 / model_bytes as f64) * 32.0) as u32;
                est.min(full).max(1)
            }
        };
        // Contesto: coi 24 tool + system il prompt fisso è ~3016 token → serve almeno ~4096.
        // 🔴 FIX CRASH (2026-07-02, device S24): 8192 sui "robusti" faceva OOM sulla GPU
        // Adreno DURANTE il decode (KV ~940MB + modello ~1GB + buffer OpenCL sulla RAM
        // condivisa) → llama.cpp lanciava un'eccezione C++ dopo ~80s di prefill → "Rust
        // cannot catch foreign exceptions" → abort. 4096 (~470MB KV) regge: prompt ~3000
        // + ~1000 di dialogo. Anche i "robusti" stanno a 4096 col 1.7B — l'8192 desktop
        // era un'assunzione non verificata sul mobile. Override con LIARA_N_CTX.
        let n_ctx: u32 = std::env::var("LIARA_N_CTX").ok().and_then(|v| v.parse().ok())
            .unwrap_or(4096);
        eprintln!("LIARA_BOOT gpu weak={weak} layers={gpu_layers} cpu_only={cpu_only} n_ctx={n_ctx}");
        // Gemma (multimodale): UN solo motore fa testo+vision col suo mmproj → 📎 su Android senza
        // secondo modello (niente OOM). Qwen (testo-only): load semplice, niente visione.
        match crate::core::paths::native_mmproj() {
            Some(mm) => LlamaEngine::load_vl(model_path, &mm, n_ctx, gpu_layers),
            None => LlamaEngine::load(model_path, n_ctx, gpu_layers),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        // Desktop (Mac/Windows): RAM abbondante → contesto LARGO (32K) per conversazioni lunghissime
        // e tutti i tool senza mai troncare. Qwen3 regge 32K nativi. Override con LIARA_N_CTX.
        let n_ctx: u32 = std::env::var("LIARA_N_CTX").ok().and_then(|v| v.parse().ok()).unwrap_or(32768);
        // Gemma (multimodale) → load_vl: il motore principale fa anche il vision (niente Qwen-VL
        // companion, rimosso). Qwen (testo-only) → load: nessuna visione.
        let eng = match crate::core::paths::native_mmproj() {
            Some(mm) => LlamaEngine::load_vl(model_path, &mm, n_ctx, 999),
            None => LlamaEngine::load(model_path, n_ctx, 999),
        };
        // Il device Metal è ora creato → ggml ha registrato il suo distruttore statico. Registriamo
        // DOPO un atexit che fa _exit(0): nell'ordine LIFO gira PRIMA del distruttore ggml, saltando
        // ggml_metal_device_free che va in abort. È la rete DEFINITIVA per il crash alla chiusura su
        // macOS: Cmd+Q / menu Quit chiamano exit() dentro AppKit (NSApplication terminate:) SENZA
        // passare da RunEvent::ExitRequested, quindi il fix in lib.rs non li copriva.
        if eng.is_ok() {
            register_hard_exit_atexit();
        }
        eng
    }
}

/// Registra (una sola volta) un handler `atexit` che termina il processo con `_exit(0)`, saltando i
/// distruttori C++ statici. Va chiamato DOPO che il device Metal di ggml è stato creato, così nell'ordine
/// LIFO di atexit/cxa_finalize il nostro handler precede `ggml_metal_device_free` (che altrimenti va in
/// abort al teardown). Solo desktop: su Android la GPU è OpenCL, il problema non si pone.
#[cfg(not(target_os = "android"))]
fn register_hard_exit_atexit() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        extern "C" fn hard_exit() {
            extern "C" {
                fn _exit(code: i32) -> !;
            }
            unsafe { _exit(0) }
        }
        extern "C" {
            fn atexit(cb: extern "C" fn()) -> i32;
        }
        unsafe {
            atexit(hard_exit);
        }
    });
}

/// Pre-carica il modello all'AVVIO (il frontend lo chiama dopo che la WebView è pronta). Cruciale su
/// Android: se il modello inizializza la GPU Adreno MENTRE la WebView sta ancora inizializzando la
/// stessa GPU → race → crash ("destroyed mutex"). Caricando qui, dietro un overlay che blocca
/// l'input, la GPU del modello parte da sola, pulita. Emette "loading-model" → "ready" (o "error:…").
#[tauri::command]
pub async fn warmup(state: State<'_, AppState>, window: WebviewWindow) -> Result<(), String> {
    let model_path = crate::core::paths::text_model(); // rilegge il modello ATTIVO (active_model), non il default fissato all'avvio
    let engine_slot = state.engine.clone();
    let w = window.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let mut guard = engine_slot.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            let _ = w.emit("status", "ready");
            return;
        }
        let _ = w.emit("status", "loading-model");
        match load_engine(&model_path) {
            Ok(e) => {
                *guard = Some(Arc::new(e));
                let _ = w.emit("status", "ready");
            }
            Err(err) => {
                let _ = w.emit("status", format!("error:{err}"));
            }
        }
    })
    .await;
    Ok(())
}

/// Stateless generation: the frontend owns the conversation tree and sends the
/// active branch. Memory (persistent, local) is injected before and updated after.
#[tauri::command]
pub async fn generate(
    messages: Vec<Message>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<String, String> {
    let model_path = crate::core::paths::text_model(); // rilegge il modello ATTIVO (active_model), non il default fissato all'avvio
    let engine_slot = state.engine.clone();
    let memory = state.memory.clone();
    let tools = state.tools.clone();
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::Relaxed); // reset before a new turn
    let pending = state.pending_compose.clone();
    pending.lock().unwrap().take(); // clear any stale draft
    let consent = state.consent.clone();
    let gpu_busy = state.gpu_busy.clone();
    let thinking = state.thinking.load(Ordering::Relaxed);
    // Il 1.7B col reasoning CRASHA (genera un decode che fa abortire llama.cpp — succede anche su CPU, quindi
    // NON è la GPU: è il modello che, essendo piccolo, col ragionamento produce output degenere). 4B e Gemma
    // reggono perché ragionano puliti. Finché il 1.7B non è riaddestrato, forziamo reasoning OFF SOLO su di
    // lui: risponde diretto e resta stabile. Gli altri modelli mantengono il thinking (default ON).
    let thinking = thinking && !model_path.to_lowercase().contains("1.7b");
    let w = window.clone();

    let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let engine = {
            let mut guard = engine_slot.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_none() {
                let _ = w.emit("status", "loading-model");
                let e = load_engine(&model_path)?;
                *guard = Some(Arc::new(e));
                let _ = w.emit("status", "ready");
            }
            guard.as_ref().unwrap().clone()
        };

        // 1) inject persistent memory: structured profile + v2 SEMANTIC RECALL of relevant memories
        let user_msg = messages.last().map(|m| m.content.clone()).unwrap_or_default();
        // skip the embedding (and recall) for trivial turns like "ok"/"ciao" — pure waste
        let query_emb = if user_msg.trim().chars().count() < 5 { None } else { engine.embed(&user_msg).ok() };
        // NB: NIENTE hint grafici nel prompt locale — provato (2026-07-12), faceva CRASHARE (foreign
        // exception C++ di llama.cpp nel decode quando il modello tentava l'output ```chart). I grafici
        // in locale vanno insegnati NEI PESI (gold seed grafici), non forzati a prompt. Cloud li fa (32B).
        let mut system = format!("{}{}", SYSTEM_PROMPT, memory.profile_block());
        if let Some(qe) = &query_emb {
            let mems = memory.recall(qe, 4);
            if !mems.is_empty() {
                system.push_str("\n\nRicordi che potrebbero essere pertinenti (usali con naturalezza solo se davvero utili, non elencarli):\n");
                for (t, _) in &mems {
                    system.push_str("- ");
                    system.push_str(t);
                    system.push('\n');
                }
            }
            // RAG docs live in their own namespace so they don't crowd out personal memory
            let docs = memory.recall_docs(qe, 3);
            if !docs.is_empty() {
                system.push_str("\n\nDai documenti che l'utente ha caricato (cita solo se pertinenti):\n");
                for (t, _) in &docs {
                    system.push_str("- ");
                    system.push_str(t);
                    system.push('\n');
                }
            }
        }

        // 2) ReAct loop: stream answer, run tools when the model asks. Eventi UI + consenso via
        // WindowSink (impl unica di AgentSink, condivisa con vision.rs) — niente più 4 closure sciolte.
        let mut sink = crate::commands::sink::WindowSink::new(w.clone(), memory.clone(), consent.clone());
        let gpu_guard = GpuBusyGuard::engage(gpu_busy.clone()); // #9: GPU occupata dal turno
        // Crash-detector: marca il decode GPU in corso. Se abortisce (eccezione C++ del 1.7B col thinking)
        // il Drop NON gira → il marker sopravvive → al riavvio quel modello passa su CPU. Sui modelli già
        // in fallback (CPU) non marchiamo (non crashano). Vive fino a fine turno → clear on drop su Ok/Err.
        #[cfg(target_os = "android")]
        let _gpu_pending = {
            let m = crate::core::paths::text_model();
            (!self::gpu_guard::in_fallback(&m)).then(|| self::gpu_guard::Pending::mark(&m))
        };
        let answer = run_agent(
            engine.as_ref(),
            &tools,
            &system,
            &messages,
            thinking,
            &cancel,
            &mut sink,
        )?;
        // #9: turno finito → la memory-formation può girare a GPU libera. Il drop
        // ESPLICITO qui (non a fine scope) va PRIMA dello spawn del task detached,
        // che legge GPU_BUSY al suo avvio. Su errore del `?` sopra, rilascia il Drop.
        drop(gpu_guard);
        let _ = w.emit("done", &answer);

        // if a tool prepared a draft, open the compose form pre-filled
        if let Some((to, subject, body)) = pending.lock().unwrap().take() {
            let _ = w.emit(
                "compose",
                serde_json::json!({ "to": to, "subject": subject, "body": body }),
            );
        }

        // 3) memory formation runs DETACHED → this command returns right after "done", so the
        //    next turn is never blocked by extraction / supersession / reflection.
        let mem = memory.clone();
        let eng = engine.clone();
        let canc = cancel.clone();
        let wm = w.clone();
        let um = user_msg.clone();
        let ans = answer.clone();
        let qe = query_emb.clone();
        tauri::async_runtime::spawn_blocking(move || {
            mem.add_episode("user", &um).ok();
            mem.add_episode("assistant", &ans).ok();
            if let Some(qe) = &qe {
                let _ = mem.remember("episode", &um, qe, 0.4);
            }
            // #9 FIX: se un NUOVO turno è già partito (GPU occupata), salta la memory-formation → niente
            // due decode concorrenti sulla GPU mobile (freeze/ANR). Gli episodi sopra sono già salvati.
            // review round-4 #6: il salto vale SOLO su Android (dove il 2° decode satura Adreno/Mali →
            // ANR). Su DESKTOP non c'è rischio ANR → estraiamo SEMPRE: l'utente non perde più i fatti
            // anche chattando veloce. (`cfg!` referenzia gpu_busy su ogni piattaforma → niente warning.)
            if cfg!(target_os = "android") && gpu_busy.load(Ordering::SeqCst) { return; }
            let exopts = GenOptions { max_tokens: 220, temperature: 0.0, stop: vec!["<|im_end|>".into()], cache_slot: 1, ..Default::default() };
            if let Ok(raw) = eng.generate(&extraction_prompt(&um, &ans), &exopts, &canc, &mut |_| {}) {
                let mut added = 0;
                // #19 FIX: traccia i fatti aggiunti in QUESTO turno, così un fatto del batch non ne ritira
                // un altro dello stesso batch (fatti contemporanei e veri) — supersede solo vs pre-esistenti.
                let mut this_batch: Vec<String> = Vec::new();
                for f in parse_facts(&raw) {
                    if mem.add_fact(&f).unwrap_or(false) {
                        if let Ok(fe) = eng.embed(&f) {
                            // SUPERSESSION: a contradicting fact on the same topic retires the old one
                            if let Some((old_id, old_text, sim)) = mem.most_similar_fact(&fe) {
                                // #20 FIX: dedup SEMANTICO — se è un rephrasing di un fatto esistente (sim > 0.9),
                                // annulla l'aggiunta: add_fact usa solo l'uguaglianza byte-esatta, quindi
                                // "Si chiama Marco" e "L'utente si chiama Marco" passavano entrambi e gonfiavano
                                // il profilo che profile_block inietta ogni turno.
                                if sim > 0.9 && old_text != f {
                                    let _ = mem.delete_fact(&f);
                                    continue;
                                }
                                if sim > SUPERSEDE_SIM && old_text != f && !this_batch.contains(&old_text) {
                                    let jp = format!(
                                        "<|im_start|>user\nFatto VECCHIO: \"{old_text}\"\nFatto NUOVO: \"{f}\"\n\
Il nuovo AGGIORNA o CONTRADDICE il vecchio (stesso argomento, informazione cambiata)? Rispondi solo SI o NO.\
<|im_end|>\n<|im_start|>assistant\n"
                                    );
                                    let jopts = GenOptions { max_tokens: 4, temperature: 0.0, stop: vec!["<|im_end|>".into()], cache_slot: 1, ..Default::default() };
                                    if let Ok(a) = eng.generate(&jp, &jopts, &canc, &mut |_| {}) {
                                        if a.trim().to_uppercase().starts_with("SI") {
                                            let _ = mem.supersede(old_id);
                                            // #6 FIX: il fatto superato va tolto ANCHE dalla tabella facts (che
                                            // profile_block inietta nel system OGNI turno), non solo dal vector
                                            // store — altrimenti resta per sempre nel profilo → contraddizioni.
                                            let _ = mem.delete_fact(&old_text);
                                        }
                                    }
                                }
                            }
                            let _ = mem.remember("fact", &f, &fe, 0.7);
                            this_batch.push(f.clone());
                        }
                        added += 1;
                    }
                }
                if added > 0 {
                    let _ = wm.emit("memory-updated", added);
                }
            }
            // REFLECTION: every N turns, consolidate recent episodes, then bound growth
            if mem.bump_turn() % REFLECT_EVERY == 0 {
                let eps = mem.recent_episode_texts(20);
                if eps.len() >= 6 {
                    let joined = eps.into_iter().rev().collect::<Vec<_>>().join("\n- ");
                    let rp = format!(
                        "<|im_start|>system\nDai messaggi dell'utente qui sotto estrai 1-3 fatti DUREVOLI e importanti su di lui \
(preferenze, relazioni, abitudini, dati personali stabili). Una riga per fatto, conciso, senza preamboli.\
<|im_end|>\n<|im_start|>user\n- {joined}<|im_end|>\n<|im_start|>assistant\n"
                    );
                    let ropts = GenOptions { max_tokens: 160, temperature: 0.2, stop: vec!["<|im_end|>".into()], cache_slot: 1, ..Default::default() };
                    if let Ok(insights) = eng.generate(&rp, &ropts, &canc, &mut |_| {}) {
                        for line in insights.lines() {
                            let ins = line.trim().trim_start_matches(['-', '•', '*']).trim();
                            if ins.len() > 8 {
                                if let Ok(ie) = eng.embed(ins) {
                                    // #27 FIX: non re-inserire una reflection quasi identica a una esistente
                                    // (viene re-derivata ogni 8 turni → affolla la top-k della recall). Skip
                                    // se c'è già qualcosa di molto simile in memoria (sim > 0.9).
                                    let dup = mem.recall(&ie, 1).first().map_or(false, |(_, s)| *s > 0.9);
                                    if !dup {
                                        let _ = mem.remember("reflection", ins, &ie, 0.85);
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = mem.prune_episodes(60);
                let _ = mem.prune_reflections(120); // #7: bound reflection growth too
            }
        });

        Ok(answer)
    })
    .await
    .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_generation(state: State<AppState>) {
    state.cancel.store(true, Ordering::Relaxed);
}

/// Android: RIMUOVE il task dell'app (`ActivityManager.AppTask.finishAndRemoveTask()`) prima di uscire.
/// SENZA questo, `_exit(0)` uccide il processo ma il task resta in foreground con `launchMode=singleTask`
/// → Android RILANCIA l'app da solo (il bug "allo switch chiedi di chiudere ma resta aperta"). Rimosso
/// il task, Android NON rilancia: l'app si chiude davvero e l'utente la riapre col modello nuovo.
/// Usa il Context dell'app via ndk_context (come [android_keystore]); getAppTasks() vale dal Context app.
#[cfg(target_os = "android")]
fn finish_and_remove_task() {
    use jni::objects::{JObject, JValue};
    let run = || -> anyhow::Result<()> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        // ActivityManager am = context.getSystemService("activity");
        let name = env.new_string("activity")?;
        let am = env
            .call_method(&context, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?
            .l()?;
        // List<AppTask> tasks = am.getAppTasks();
        let tasks = env.call_method(&am, "getAppTasks", "()Ljava/util/List;", &[])?.l()?;
        let n = env.call_method(&tasks, "size", "()I", &[])?.i()?;
        for i in 0..n {
            let task = env.call_method(&tasks, "get", "(I)Ljava/lang/Object;", &[JValue::Int(i)])?.l()?;
            let _ = env.call_method(&task, "finishAndRemoveTask", "()V", &[]); // best-effort per task
        }
        Ok(())
    };
    if let Err(e) = run() {
        eprintln!("LIARA exit: finishAndRemoveTask fallito ({e}) — esco comunque");
    }
}

/// Chiude completamente l'app (dialog "vuoi uscire?" sul tasto indietro Android + switch modello).
#[tauri::command]
pub fn exit_app(app: tauri::AppHandle) {
    // Switch modello / chiusura volontaria: terminiamo DURO con _exit, così il SO libera TUTTA la RAM e
    // la GPU del modello precedente. Con app.exit(0) su Android il processo poteva restare vivo → il
    // modello nuovo caricava SOPRA il vecchio → OOM. (Diverso da on_window_event: qui è ESPLICITO.)
    let _ = app;
    // Android: prima rimuovi il task, altrimenti Android rilancia l'app dopo _exit (bug switch "resta aperta").
    #[cfg(target_os = "android")]
    finish_and_remove_task();
    // process::exit esegue i destructor C++ (cxa_finalize) → ggml_metal_device_free va in ggml_abort
    // (lo STESSO crash della chiusura, ma al cambio modello). _exit termina senza destructor: il SO
    // libera RAM e GPU del modello precedente da sé. Uscita davvero "dura", come voleva il commento.
    extern "C" {
        fn _exit(code: i32) -> !;
    }
    unsafe { _exit(0) }
}

/// Attiva/disattiva il RAGIONAMENTO (thinking di Qwen3). ON di default: il LoRA v6 usa il ragionamento
/// per chiamare i tool correttamente (senza, i tool non partono o vengono usati male). Il frontend
/// mostra il ragionamento in un bubble a parte. Ha effetto dal messaggio successivo (o dopo un riavvio).
#[tauri::command]
pub fn set_thinking(on: bool, state: State<AppState>) {
    state.thinking.store(on, Ordering::Relaxed);
}

/// Riassume FEDELMENTE la conversazione in un testo compatto: è il cuore dell'anti-"rotten context".
/// Quando il contesto sta per riempirsi, il frontend chiama questo → apre una nuova chat che PARTE dal
/// summary (fatti/nomi/decisioni preservati) invece di degradare o troncare a metà. Usa lo slot 1
/// (ausiliario), quindi non tocca la cache della conversazione attiva.
#[tauri::command]
pub async fn summarize_conversation(
    messages: Vec<Message>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let engine_slot = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let engine = {
            let guard = engine_slot.lock().unwrap_or_else(|e| e.into_inner());
            guard.as_ref().ok_or_else(|| "modello non caricato".to_string())?.clone()
        };
        let convo: String = messages
            .iter()
            .filter(|m| m.role == "user" || m.role == "assistant")
            .map(|m| format!("{}: {}", if m.role == "user" { "Utente" } else { "Liara" }, m.content))
            .collect::<Vec<_>>()
            .join("\n\n");
        // Prefill <think></think> vuoto: il summary è un compito diretto, niente reasoning che sprechi token.
        let prompt = format!(
            "<|im_start|>system\nRiassumi FEDELMENTE la conversazione seguente: conserva fatti, nomi, numeri, \
date, decisioni e il filo del discorso. Conciso, in terza persona, in italiano. Non inventare nulla, non \
aggiungere commenti.<|im_end|>\n<|im_start|>user\n{convo}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
        );
        // #18 FIX: 400 tok erano un collo di bottiglia per riassumere una conversazione lunga. Alziamo a
        // 700; lo slot 1 ora ha n_ctx 2048 (fix #10) che basta a leggere la conversazione da riassumere.
        let opts = GenOptions { max_tokens: 700, temperature: 0.3, stop: vec!["<|im_end|>".into()], cache_slot: 1, ..Default::default() };
        let never = std::sync::atomic::AtomicBool::new(false);
        let summary = engine
            .generate(&prompt, &opts, &never, &mut |_| {})
            .map_err(|e| format!("summary: {e}"))?;
        Ok(summary.trim().to_string())
    })
    .await
    .map_err(|e| format!("task: {e}"))?
}
