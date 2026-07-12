//! Platform-aware model locations. The hardcoded macOS paths break on Android, so every model
//! path resolves from LIARA_MODELS_DIR — which the app sets to its PRIVATE data dir on mobile
//! (where the bundled models are copied at first launch) and leaves at the project folder on desktop.
use std::path::PathBuf;

/// Base folder that holds the model files. Override with LIARA_MODELS_DIR (set on Android to the
/// app data dir). Desktop default = the project models/ folder.
pub fn models_base() -> PathBuf {
    std::env::var("LIARA_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/Users/zelistore/zeli-local/models"))
}

/// The text LLM (GGUF). LIARA_MODEL still wins (used for the LoRA-fused model / dev overrides).
pub fn text_model() -> String {
    if let Ok(m) = std::env::var("LIARA_MODEL") {
        return m;
    }
    // Preferenza del selettore: il file `active_model` contiene il nome del GGUF scelto dall'utente.
    // Lo applichiamo solo se quel file è davvero presente.
    let base = models_base();
    if let Ok(name) = std::fs::read_to_string(base.join("active_model")) {
        let name = name.trim();
        if !name.is_empty() && base.join(name).exists() {
            return base.join(name).to_string_lossy().into_owned();
        }
    }
    // Fallback ROBUSTO: il PRIMO modello di testo realmente presente (escludendo mmproj e file .part
    // incompleti). Prima puntava a un nome hardcoded "qwen3-4b-instruct-q4.gguf" che non esiste più →
    // "modello non trovato" ogni volta che active_model mancava (es. mmproj ancora in download).
    if let Ok(entries) = std::fs::read_dir(&base) {
        let mut ggufs: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".gguf") && !n.contains("mmproj"))
            .collect();
        ggufs.sort();
        if let Some(first) = ggufs.first() {
            return base.join(first).to_string_lossy().into_owned();
        }
    }
    // Nessun modello presente: path indicativo (il chiamante mostrerà "scarica un modello").
    base.join("liara-1.7b-it.gguf").to_string_lossy().into_owned()
}

/// Folder with the audio models (Piper TTS / whisper STT / silero VAD).
pub fn audio_dir() -> PathBuf {
    std::env::var("LIARA_AUDIO_DIR").map(PathBuf::from).unwrap_or_else(|_| models_base().join("audio"))
}

/// mmproj NATIVO del modello di testo attivo. Gemma 4 è multimodale: vede le immagini col PROPRIO
/// proiettore, quindi UN solo motore fa testo+vision (niente motore VL separato → niente OOM su
/// APK 8GB). Ritorna Some(path) se il modello attivo è Gemma e il suo mmproj è presente accanto;
/// None per i Qwen (liara-1.7b/4b sono `qwen3` TESTO-ONLY → niente visione: si usa Gemma). Un futuro
/// Qwen-VL nativo si aggancerebbe QUI (come Gemma), non con un companion. Override: LIARA_NATIVE_MMPROJ.
pub fn native_mmproj() -> Option<String> {
    if let Ok(p) = std::env::var("LIARA_NATIVE_MMPROJ") {
        return Some(p);
    }
    let model = text_model();
    let name = std::path::Path::new(&model).file_name()?.to_string_lossy().to_lowercase();
    if !name.contains("gemma") {
        return None;
    }
    // Il mmproj Gemma è scaricato dal frontend accanto al modello. Corrisponde alla taglia: E4B o 12B.
    let mm_name = if name.contains("12b") {
        "mmproj-gemma-4-12b-f16.gguf"
    } else {
        "mmproj-gemma-4-e4b-f16.gguf"
    };
    let mm = models_base().join(mm_name);
    mm.exists().then(|| mm.to_string_lossy().into_owned())
}
