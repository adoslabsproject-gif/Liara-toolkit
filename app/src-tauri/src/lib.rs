pub mod core;
mod commands;

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};

use crate::commands::consent::ConsentGate;
use crate::core::calendar::Calendar;
use crate::core::contacts::Contacts;
use crate::core::crypto::Crypto;
use crate::core::email::EmailStore;
use crate::core::engine::Engine;
use crate::core::memory::Memory;
use crate::core::tools::{PendingCompose, ToolRegistry};

// Command handlers live in `commands/`; bring them all into scope for `generate_handler!`.
use crate::commands::*;

pub(crate) struct AppState {
    pub(crate) model_path: String,
    pub(crate) engine: Arc<Mutex<Option<Arc<dyn Engine>>>>,
    pub(crate) memory: Arc<Memory>,
    pub(crate) email: Arc<EmailStore>,
    pub(crate) tools: Arc<ToolRegistry>,
    pub(crate) cancel: Arc<AtomicBool>,
    /// Numero di GENERAZIONE (epoch): avanza a ogni Stop e a ogni nuovo turno cloud. Un run che
    /// scopre di non essere più il corrente è uno ZOMBIE (Stop premuto / turno nuovo partito) e si
    /// spegne senza emettere eventi. Serve al cloud: il thread remoto resta bloccato nella POST
    /// (~120s, non abortibile) e `cancel` da solo non basta — il turno nuovo lo resettava a false
    /// RIANIMANDO il vecchio run, i cui done/errori sporcavano il turno nuovo (bug "Stop in cloud
    /// → 'Liara non disponibile' quando riscrivo").
    pub(crate) gen_seq: Arc<AtomicU64>,
    /// review #6: flag di stato condivisi QUI, non più statiche globali di modulo
    pub(crate) gpu_busy: Arc<AtomicBool>,
    pub(crate) thinking: Arc<AtomicBool>,
    pub(crate) pending_compose: PendingCompose,
    pub(crate) calendar: Arc<Calendar>,
    /// Rubrica cifrata di Liara (import granulare dalla rubrica di sistema). Vedi core/contacts.
    pub(crate) contacts: Arc<Contacts>,
    /// Copia locale cifrata degli SMS (sync su consenso, permesso READ_SMS). Vedi core/sms.
    pub(crate) sms: Arc<crate::core::sms::SmsStore>,
    pub(crate) tts: Arc<crate::core::audio::TtsQueue>,
    pub(crate) stt: Arc<Mutex<Option<crate::core::audio::Stt>>>,
    pub(crate) rec: Arc<crate::core::audio::RecState>,
    pub(crate) consent: Arc<ConsentGate>,
    /// Identità crittografica per la chat peer (X25519, caricata all'avvio) + master key at-rest
    /// (per cifrare la rubrica dei QR accettati). Vedi core/peer.
    pub(crate) peer: Arc<crate::core::peer::Identity>,
    pub(crate) crypto: Arc<Crypto>,
    /// Riepiloghi incrementali della chat AI↔AI, per contatto (gestione contesto di liara_reply).
    pub(crate) peer_summaries: Arc<crate::commands::peer_ai::PeerSummaries>,
}

fn pick_model() -> String {
    crate::core::paths::text_model()
}

/// Build the REAL tool registry (throwaway in-memory stores) → catalog JSON. The `dump_tools`
/// bin prints this so the LoRA dataset is generated against the same tools the app exposes
/// (anti-drift: registry is the single source of truth).
pub fn tool_catalog() -> String {
    use crate::core::calendar::Calendar;
    use crate::core::crypto::Crypto;
    use crate::core::email::EmailStore;
    use crate::core::memory::Memory;
    let crypto = Arc::new(Crypto::ephemeral());
    let mem = Arc::new(Memory::open(":memory:", crypto.clone()).expect("mem"));
    let email = Arc::new(EmailStore::open(":memory:", crypto.clone()).expect("email"));
    let cal = Arc::new(Calendar::open(":memory:", crypto.clone()).expect("cal"));
    let contacts = Arc::new(Contacts::open(":memory:", crypto.clone()).expect("contacts"));
    let sms = Arc::new(crate::core::sms::SmsStore::open(":memory:", crypto).expect("sms"));
    let pending: PendingCompose = Arc::new(Mutex::new(None));
    ToolRegistry::build(email, pending, cal, mem, contacts, sms).catalog_json()
}

/// Build the REAL tool registry → routing JSON (tools-in-order + category keywords). The `dump_routing`
/// bin prints this so the dataset generator selects tools per-intent IDENTICALLY to the runtime
/// (anti-drift, twin of `tool_catalog`). Gate: `gate_routing_equiv.py`.
pub fn tool_routing() -> String {
    use crate::core::calendar::Calendar;
    use crate::core::crypto::Crypto;
    use crate::core::email::EmailStore;
    use crate::core::memory::Memory;
    let crypto = Arc::new(Crypto::ephemeral());
    let mem = Arc::new(Memory::open(":memory:", crypto.clone()).expect("mem"));
    let email = Arc::new(EmailStore::open(":memory:", crypto.clone()).expect("email"));
    let cal = Arc::new(Calendar::open(":memory:", crypto.clone()).expect("cal"));
    let contacts = Arc::new(Contacts::open(":memory:", crypto.clone()).expect("contacts"));
    let sms = Arc::new(crate::core::sms::SmsStore::open(":memory:", crypto).expect("sms"));
    let pending: PendingCompose = Arc::new(Mutex::new(None));
    ToolRegistry::build(email, pending, cal, mem, contacts, sms).routing_json()
}

/// ORACOLO di selezione per-intento: le categorie che il runtime attiverebbe per `request`, separate
/// da virgola (ordine stabile). Usato dal bin `select_cats` → gate `gate_routing_equiv.py` per provare
/// che il port Python (`app_routing.selected_categories`) è byte-identico a `selected_categories` Rust.
pub fn select_categories(request: &str) -> String {
    crate::core::tools::selected_categories(request).join(",")
}

/// Blocco `[AVAILABLE_TOOLS]` Mistral REALE (dal registry, sottoinsieme routed) per una richiesta.
/// Usato dal bin `dump_chat` + gate `verify_equiv_mistral.py` per la prova anti-drift Rust==mistral-common.
pub fn mistral_tools_block(request: &str) -> String {
    use crate::core::calendar::Calendar;
    use crate::core::crypto::Crypto;
    use crate::core::email::EmailStore;
    use crate::core::memory::Memory;
    let crypto = Arc::new(Crypto::ephemeral());
    let mem = Arc::new(Memory::open(":memory:", crypto.clone()).expect("mem"));
    let email = Arc::new(EmailStore::open(":memory:", crypto.clone()).expect("email"));
    let cal = Arc::new(Calendar::open(":memory:", crypto.clone()).expect("cal"));
    let contacts = Arc::new(Contacts::open(":memory:", crypto.clone()).expect("contacts"));
    let sms = Arc::new(crate::core::sms::SmsStore::open(":memory:", crypto).expect("sms"));
    let pending: PendingCompose = Arc::new(Mutex::new(None));
    ToolRegistry::build(email, pending, cal, mem, contacts, sms)
        .mistral_tools_block_for(request)
        .unwrap_or_default()
}

/// Log di avvio su file: scrive ogni tappa del boot in `boot.log` (cartella dati app). Se l'app
/// crasha durante l'inizializzazione (es. init GPU Vulkan su device incompatibili), l'ULTIMA riga
/// indica la tappa fallita → diagnosi SENZA adb (l'utente recupera il file). Anche su logcat (eprintln).
fn boot_log(dir: &std::path::Path, msg: &str) {
    eprintln!("LIARA_BOOT {msg}");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("boot.log"))
    {
        let _ = writeln!(f, "{msg}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 🔴 CRASH "l'app muore da sola" (tombstone SIGABRT, 2026-07-21): quando Android DISTRUGGE
    // l'activity (app in background, o Samsung che la ricicla — molto più frequente a telefono
    // STACCATO/non in carica), `tao::EventLoop::run` chiama `std::process::exit`, che esegue i
    // distruttori C++ globali (`__cxa_finalize`); questi distruggono un mutex ancora DETENUTO da un
    // thread di background (audio/warmup/ureq/webview) → bionic aborta con "destroying mutex with
    // owner or contenders" → crash. Il processo sta comunque terminando (activity distrutta): quindi
    // convertiamo quell'abort di teardown in un'uscita PULITA con `_exit(0)` (che NON esegue i
    // distruttori C++). Niente più tombstone: l'app si chiude netta e riparte pulita alla riapertura.
    // Solo Android; sul desktop l'uscita dura è già gestita in on_window_event/ExitRequested.
    #[cfg(target_os = "android")]
    unsafe {
        extern "C" {
            fn signal(signum: i32, handler: usize) -> usize;
            fn _exit(code: i32) -> !;
        }
        extern "C" fn on_abort(_sig: i32) {
            unsafe { _exit(0) }
        }
        const SIGABRT: i32 = 6;
        signal(SIGABRT, on_abort as usize);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Android: registra il contesto JNI così l'audio (cpal/oboe) non panica all'avvio.
            #[cfg(target_os = "android")]
            crate::core::android_ctx::init();
            let dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            std::fs::create_dir_all(&dir).ok();
            let _ = std::fs::remove_file(dir.join("boot.log")); // log pulito ad ogni avvio
            boot_log(&dir, "1-start");
            // PANIC HOOK diagnostico: qualunque panic Rust (su QUALSIASI thread — warmup, ureq cloud,
            // tool, audio) viene scritto in boot.log con messaggio e posizione, PRIMA che il processo
            // muoia. Così un crash "da solo, staccato dal PC" (che non si riesce a vedere in logcat live)
            // lascia una traccia leggibile: `run-as com.liara.app cat boot.log` mostra l'ultima riga
            // = la causa. Non cattura gli abort NATIVI (SIGSEGV/SIGABRT del C++), ma distingue subito
            // "panic Rust" da "kill di sistema/OOM" (in quel caso l'ultima riga resta uno stadio di boot).
            {
                let logdir = dir.clone();
                let prev = std::panic::take_hook();
                std::panic::set_hook(Box::new(move |info| {
                    boot_log(&logdir, &format!("PANIC {info}"));
                    prev(info);
                }));
            }
            // macOS (Apple Silicon, macOS 26.1): i "residency sets" di Metal — feature ggml ≥ macOS 15, con
            // un thread heartbeat in background — lanciano un'eccezione ObjC/Metal durante l'inferenza sul
            // M4 Pro → risale oltre il confine FFI in Rust ("foreign exception") → abort dell'app (crash
            // chiedendo appuntamenti/memoria). Li DISABILITIAMO col flag integrato di ggml: il modello gira
            // comunque su GPU Metal (percorso classico, pre-residency-sets), zero perdita pratica con 48GB.
            // Va settato PRIMA che il backend Metal si inizializzi (al primo load del modello).
            #[cfg(target_os = "macos")]
            std::env::set_var("GGML_METAL_NO_RESIDENCY", "1");
            // Android: i modelli vivono nella cartella dati INTERNA dell'app (la possiede al 100%,
            // niente problemi di scoped-storage). In dev li spingiamo via `adb run-as`; in produzione
            // li copieremo qui dagli asset dell'APK al primo avvio.
            #[cfg(target_os = "android")]
            {
                let models = dir.join("models");
                std::fs::create_dir_all(models.join("vl")).ok();
                std::fs::create_dir_all(models.join("audio")).ok();
                std::env::set_var("LIARA_MODELS_DIR", &models);
                // web_search via il NOSTRO SearXNG (server NHA, niente Cloudflare): DuckDuckGo diretto
                // blocca l'IP del telefono → risultati vuoti → il modello allucinava. Il server cerca
                // senza blocchi, query anonime e non loggate (vedi nginx /searxng/, access_log off).
                std::env::set_var("LIARA_SEARXNG", "https://nothumanallowed.com/searxng");
                // GPU Adreno: il driver rifiuta gli shader Vulkan "avanzati" (coopmat/bf16/int-dot) →
                // li disabilitiamo così GGML usa gli shader base che l'Adreno compila. Va settato
                // PRIMA che il modello inizializzi Vulkan.
                std::env::set_var("GGML_VK_DISABLE_COOPMAT", "1");
                std::env::set_var("GGML_VK_DISABLE_COOPMAT2", "1");
                std::env::set_var("GGML_VK_DISABLE_BFLOAT16", "1");
                std::env::set_var("GGML_VK_DISABLE_INTEGER_DOT_PRODUCT", "1");
                std::env::set_var("GGML_VK_DISABLE_F16", "1");
                // Adreno: la race del "destroyed mutex" allo startup viene dai thread async del driver.
                // Disabilitiamo l'async + permettiamo il fallback alla memoria di sistema (la GPU
                // Adreno condivide la RAM) → meno thread che corrono = niente race.
                std::env::set_var("GGML_VK_DISABLE_ASYNC", "1");
                std::env::set_var("GGML_VK_ALLOW_SYSMEM_FALLBACK", "1");
            }
            // Desktop (release): i modelli vivono nella data-dir dell'app (scrivibile su QUALSIASI Mac,
            // dove il DMG scarica al primo avvio). Senza questo, models_base() cadeva sul path assoluto
            // dello sviluppatore (/Users/zelistore/...) → su ogni altra macchina il download falliva con
            // EACCES. In DEBUG NON la settiamo (i modelli restano nella cartella del progetto = comodità
            // dev); un LIARA_MODELS_DIR già presente nell'ambiente vince sempre.
            #[cfg(all(not(target_os = "android"), not(debug_assertions)))]
            {
                if std::env::var_os("LIARA_MODELS_DIR").is_none() {
                    let models = dir.join("models");
                    std::fs::create_dir_all(models.join("vl")).ok();
                    std::fs::create_dir_all(models.join("audio")).ok();
                    std::env::set_var("LIARA_MODELS_DIR", &models);
                }
            }
            let db = dir.join("liara.db");
            let dbp = db.to_string_lossy().to_string();
            // at-rest encryption: AES-256-GCM, master key in the OS keystore
            boot_log(&dir, "2-crypto-init");
            let crypto = Arc::new(Crypto::init().expect("init crypto (OS keystore)"));
            boot_log(&dir, "3-crypto-ok");
            let memory = Arc::new(Memory::open(&dbp, crypto.clone()).expect("open memory db"));
            let email = Arc::new(EmailStore::open(&dbp, crypto.clone()).expect("open email store"));
            let calendar = Arc::new(Calendar::open(&dbp, crypto.clone()).expect("open calendar"));
            let contacts = Arc::new(Contacts::open(&dbp, crypto.clone()).expect("open contacts"));
            let sms = Arc::new(crate::core::sms::SmsStore::open(&dbp, crypto.clone()).expect("open sms"));
            boot_log(&dir, "4-db-ok");
            let pending_compose: PendingCompose = Arc::new(Mutex::new(None));
            let mut tools = ToolRegistry::build(
                email.clone(),
                pending_compose.clone(),
                calendar.clone(),
                memory.clone(),
                contacts.clone(),
                sms.clone(),
            );
            boot_log(&dir, "4a-before-mcp"); // marker diagnostici granulari: localizzano il crash "appena
            tools.add_dynamic(crate::core::mcp::connect_configured()); // MCP host (LIARA_MCP)
            let tools = Arc::new(tools);
            boot_log(&dir, "4b-tools-ok"); // apri" (signal 9 dopo 4-db-ok) — l'ULTIMO stage scritto = dove muore
            let model_path = pick_model();
            let engine: Arc<Mutex<Option<Arc<dyn Engine>>>> = Arc::new(Mutex::new(None));
            // Identità peer (X25519) caricata/creata UNA volta qui: seal/open poi lavorano in RAM.
            // Se fallisce (disco pieno ecc.) usiamo un'identità volatile → la chat peer degrada senza
            // impedire l'avvio dell'app.
            let peer = Arc::new(
                crate::core::peer::Identity::load_or_create(&crypto)
                    .unwrap_or_else(|e| {
                        eprintln!("LIARA-PEER: identità non persistibile ({e}) → volatile");
                        crate::core::peer::Identity::ephemeral()
                    }),
            );
            boot_log(&dir, "4c-peer-ok");
            app.manage(AppState {
                model_path: model_path.clone(),
                engine: engine.clone(),
                memory,
                email,
                tools,
                cancel: Arc::new(AtomicBool::new(false)),
                gen_seq: Arc::new(AtomicU64::new(0)),
                gpu_busy: Arc::new(AtomicBool::new(false)),
                // thinking ON di default: il LoRA v6 (attuale) USA il ragionamento per chiamare i tool
                // correttamente — senza, i tool non vengono invocati o male. (Era OFF per il v4, addestrato
                // col blocco <think> vuoto; il v6 lo ha superato.) L'utente può comunque toggolarlo.
                thinking: Arc::new(AtomicBool::new(true)),
                pending_compose,
                calendar,
                contacts,
                sms,
                tts: Arc::new(crate::core::audio::TtsQueue::start({
                    let h = app.handle().clone();
                    move || { let _ = h.emit("tts-idle", ()); }
                })),
                stt: Arc::new(Mutex::new(None)),
                rec: Arc::new(crate::core::audio::RecState::default()),
                consent: Arc::new(ConsentGate::default()),
                peer,
                crypto: crypto.clone(),
                peer_summaries: Arc::new(Default::default()),
            });
            boot_log(&dir, "5-state-ok"); // crash DOPO questo = warmup/GPU; crash a 4b = audio(TtsQueue)/state; a 4a = MCP
            // Warmup del modello in un thread NATIVO (deterministico; la WebView Android throttla i timer
            // JS, quindi non lo deleghiamo al frontend). Emette gli "status" che il frontend già ascolta.
            // #23 FIX (commento allineato alla realtà): lo chiamiamo SOLO all'avvio. Su Android il resume
            // NON rifà warmup DELIBERATAMENTE: reinizializzare il contesto GPU sul lifecycle causava il
            // "destroyed mutex"/crash Adreno (vedi memoria crash-avvio-android-gpu-race). Il modello resta
            // in background; se il SO lo killa per RAM, l'app riparte pulita all'apertura successiva.
            fn spawn_warmup(
                engine: Arc<Mutex<Option<Arc<dyn Engine>>>>,
                model_path: String,
                handle: tauri::AppHandle,
                delay_ms: u64,
            ) {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    // CLOUD: se l'utente usa Liara via API, NON caricare il modello locale al boot — spreco di
                    // RAM e CALORE (l'inferenza va al 32B, il locale resterebbe caricato e inutile). Prima il
                    // boot lo caricava sempre → il telefono scaldava anche in cloud. Flag scritto dal frontend
                    // (set_cloud_active) e riletto qui. Presente = cloud attivo → skip.
                    if crate::core::paths::models_base().join("cloud_active").exists() {
                        let _ = handle.emit("status", "cloud");
                        return;
                    }
                    let mut guard = engine.lock().unwrap();
                    if guard.is_some() {
                        let _ = handle.emit("status", "ready");
                        return;
                    }
                    // Modello non ancora scaricato? → il frontend mostra la schermata di download.
                    // A download finito il frontend richiama warmup(), che rientra qui e carica l'engine.
                    if !std::path::Path::new(&model_path).exists() {
                        let _ = handle.emit("status", "need-download");
                        return;
                    }
                    // GESTIONE RAM (anti-OOM-kill all'avvio): su Android con poca memoria libera, caricare
                    // ora il modello farebbe sforare la RAM e il sistema UCCIDE il processo (signal 9) — è
                    // il crash "appena apri l'app, dopo 2-3 avvii va" (a ogni kill si libera memoria). Invece
                    // di crashare, aspettiamo che ci sia RAM sufficiente (≈ dimensione del modello + 500 MB),
                    // avvisando il frontend; se dopo l'attesa non basta, NON carichiamo (meglio un avviso che
                    // il kill). Al retry — warmup() richiamato dal frontend — riproviamo.
                    #[cfg(target_os = "android")]
                    {
                        let read_avail = || -> Option<u64> {
                            let s = std::fs::read_to_string("/proc/meminfo").ok()?;
                            s.lines().find(|l| l.starts_with("MemAvailable:"))?
                                .split_whitespace().nth(1)?.parse().ok()
                        };
                        // RAM che serve DAVVERO libera per partire: NON tutta la dimensione del modello
                        // (i modelli grandi ora si caricano in parte via mmap, recuperabile → load_engine
                        // fa offload adattivo), ma solo il footprint residente minimo. Cappata a ~2 GB così
                        // Gemma (5,3 GB) non aspetta all'infinito una RAM libera che non arriverà mai.
                        let model_kb = std::fs::metadata(&model_path).map(|m| m.len() / 1024).unwrap_or(0);
                        let need_kb = model_kb.min(2_000_000) + 500_000;
                        let mut ok = false;
                        for _ in 0..20 {
                            if read_avail().map_or(true, |a| a >= need_kb) { ok = true; break; }
                            let _ = handle.emit("status", "low-memory");
                            std::thread::sleep(std::time::Duration::from_secs(3));
                        }
                        if !ok {
                            let _ = handle.emit("status", "low-memory-persist");
                            return;
                        }
                    }
                    let _ = handle.emit("status", "loading-model");
                    let logdir = crate::core::paths::models_base()
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_default();
                    boot_log(&logdir, "6-gpu-loading"); // se è l'ULTIMA riga del log → crasha nell'init GPU
                    match crate::commands::generate::load_engine(&model_path) {
                        Ok(e) => {
                            *guard = Some(Arc::new(e));
                            boot_log(&logdir, "7-model-ok");
                            let _ = handle.emit("status", "ready");
                        }
                        Err(err) => {
                            boot_log(&logdir, &format!("7-model-ERR {err}"));
                            let _ = handle.emit("status", format!("error:{err}"));
                        }
                    }
                });
            }
            spawn_warmup(engine.clone(), model_path.clone(), app.handle().clone(), 2500);
            Ok(())
        })
        .on_window_event(|_window, event| {
            // SOLO desktop: ggml/Metal va in abort nel teardown del device GPU alla chiusura
            // (ggml_metal_device_free → ggml_abort → SIGABRT, il "crash report" del Mac). Usciamo DURI
            // così il SO libera tutto senza eseguire il destructor buggy.
            // Su ANDROID NON lo facciamo: WindowEvent::Destroyed scatta anche per re-render/background
            // momentanei (es. quando si seleziona un modello da scaricare) → con process::exit l'app
            // "usciva" da sola. Su Android il ciclo di vita lo gestisce il sistema.
            #[cfg(not(target_os = "android"))]
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                // std::process::exit NON basta: esegue i destructor C++ (cxa_finalize) → il destructor
                // globale del device Metal di ggml (ggml_metal_device_free) va in ggml_abort → il "crash
                // report" del Mac alla chiusura. _exit (POSIX) termina SENZA eseguire alcun destructor:
                // il SO libera tutto (memoria, GPU) da sé. È l'uscita DAVVERO "dura" che il commento voleva.
                extern "C" {
                    fn _exit(code: i32) -> !;
                }
                unsafe { _exit(0) }
            }
            #[cfg(target_os = "android")]
            let _ = event;
        })
        .invoke_handler(tauri::generate_handler![
            generate,
            remote_generate,
            cloud_hello,
            warmup,
            memory_facts,
            add_fact,
            forget_all,
            delete_fact,
            get_profile,
            set_profile,
            list_conversations,
            save_conversation,
            load_conversation,
            delete_conversation,
            email_get_config,
            email_set_config,
            email_fetch,
            email_list,
            email_list_folder,
            email_restore,
            email_purge,
            email_get,
            email_delete,
            email_send,
            stop_generation,
            exit_app,
            calendar_events,
            calendar_create,
            calendar_remove,
            tts_speak,
            tts_stop,
            tts_synth,
            get_tts_voice,
            set_tts_voice,
            stt_start,
            stt_stop,
            stt_transcribe,
            listen_hands_free,
            set_gps,
            get_location,
            set_manual_location,
            my_network_id,
            ingest_document,
            extract_doc_text,
            consent_respond,
            permissions,
            set_permission,
            describe_image,
            download_model,
            model_present,
            set_active_model,
            set_cloud_active,
            active_model,
            cancel_download,
            delete_model,
            audio_present,
            extract_audio,
            model_outdated,
            fetch_models,
            device_caps,
            set_thinking,
            summarize_conversation,
            peer_identity,
            peer_seal,
            peer_open,
            peer_list,
            peer_add,
            peer_remove,
            pick_contact,
            scan_qr,
            contacts_sync,
            contacts_import,
            contacts_list,
            contacts_update,
            contacts_delete,
            sms_sync,
            sms_count,
            sms_list,
            liara_reply
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            // FIX crash Metal all'uscita (SOLO desktop): Tauri/macOS chiamerebbe exit() → cxa_finalize →
            // ggml_metal_device_free → ggml_abort (il "crash report"). Su ExitRequested usciamo DURI con
            // _exit: niente destructor C++, il SO libera RAM+GPU da sé.
            // ⚠️ SU ANDROID NO: `ExitRequested` scatta anche quando l'activity va in PAUSA (es. appare un
            // dialog di permessi di sistema — camera del lettore QR, selettore contatti) → l'app faceva
            // `_exit(0)` e "crashava" da sola (log: "exited cleanly (0)" con GrantPermissionsActivity sopra).
            // Su Android il ciclo di vita lo gestisce il sistema: NON usciamo mai a mano.
            #[cfg(not(target_os = "android"))]
            if let tauri::RunEvent::ExitRequested { .. } = event {
                extern "C" {
                    fn _exit(code: i32) -> !;
                }
                unsafe { _exit(0) }
            }
            #[cfg(target_os = "android")]
            let _ = event;
        });
}
