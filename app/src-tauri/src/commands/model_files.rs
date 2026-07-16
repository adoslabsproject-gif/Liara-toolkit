//! Gestione dei FILE dei modelli scaricati: eliminazione per liberare spazio. Il download vive in
//! `download.rs`; qui l'operazione inversa. Sicuro contro il path-traversal (accetta SOLO nomi file
//! semplici, mai path) e resetta `active_model` se punta a un file rimosso, così `text_model()` ne
//! sceglie automaticamente un altro presente invece di puntare al vuoto.
use crate::core::paths::models_base;

/// Un nome-file è accettabile solo se sta dentro `models_base` (niente separatori di path né `..`).
fn is_safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && !name.contains("..")
}

/// Elimina i file-modello indicati per NOME semplice (es. "gemma-4-e4b-it.gguf"), tipicamente il
/// GGUF e il suo eventuale mmproj — il frontend passa tutti i file appartenenti al modello. Ritorna
/// quanti file sono stati davvero rimossi (un file già assente non è un errore). Se uno dei file era
/// il modello ATTIVO, azzera `active_model` così il prossimo avvio non cerca un GGUF inesistente.
#[tauri::command]
pub fn delete_model(files: Vec<String>) -> Result<u32, String> {
    let base = models_base();
    let active = std::fs::read_to_string(base.join("active_model")).ok().map(|s| s.trim().to_string());
    let mut deleted = 0u32;
    let mut reset_active = false;
    for name in &files {
        if !is_safe_name(name) {
            return Err(format!("nome file non valido: {name}"));
        }
        let path = base.join(name);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("impossibile eliminare {name}: {e}"))?;
            deleted += 1;
        }
        if active.as_deref() == Some(name.as_str()) {
            reset_active = true;
        }
    }
    if reset_active {
        let _ = std::fs::remove_file(base.join("active_model"));
    }
    Ok(deleted)
}

/// I modelli audio (TTS/STT) sono presenti E aggiornati? La guardia è il modello Kokoro: esiste SOLO
/// nel bundle nuovo (voce Kokoro + whisper-small). Così chi ha ancora il bundle vecchio (Piper +
/// whisper-base, con silero ma senza Kokoro) risulta "assente" e riscarica quello nuovo.
fn audio_ready() -> bool {
    let a = models_base().join("audio");
    let kokoro = a.join("kokoro-multi-lang-v1_0");
    // il lexicon è la guardia della v3: la v2 (senza lexicon/dict) faceva ABORTARE Kokoro all'ascolto →
    // chi ha la v2 rotta risulta "assente" e riscarica la v3 corretta.
    kokoro.join("model.onnx").exists()
        && kokoro.join("lexicon-us-en.txt").exists()
        && a.join("sherpa-onnx-whisper-small").join("small-encoder.int8.onnx").exists()
}

#[tauri::command]
pub fn audio_present() -> bool {
    audio_ready()
}

/// Estrae `liara-audio.zip` (scaricato on-demand da GitHub in models_base) dentro `models/audio`, poi
/// rimuove lo zip. Sostituisce il vecchio bundle da 511MB: APK/DMG restano leggeri, l'audio arriva
/// solo quando l'utente usa voce/microfono. Anti zip-slip: `enclosed_name` rifiuta i path che escono
/// dalla cartella. Idempotente: se l'audio c'è già, non fa nulla.
#[tauri::command]
pub fn extract_audio() -> Result<(), String> {
    let base = models_base();
    if audio_ready() {
        return Ok(()); // già estratto (bundle nuovo)
    }
    let zip_path = base.join("liara-audio-v3.zip");
    if !zip_path.exists() {
        return Err("liara-audio-v3.zip non trovato: scaricalo prima".into());
    }
    let file = std::fs::File::open(&zip_path).map_err(|e| format!("apro zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("leggo zip: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("entry {i}: {e}"))?;
        // anti zip-slip: scarta i nomi che uscirebbero da models_base (../, path assoluti)
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = base.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| format!("mkdir: {e}"))?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
            }
            let mut outf = std::fs::File::create(&out).map_err(|e| format!("creo {out:?}: {e}"))?;
            std::io::copy(&mut entry, &mut outf).map_err(|e| format!("estraggo: {e}"))?;
        }
    }
    let _ = std::fs::remove_file(&zip_path); // libera lo spazio dello zip dopo l'estrazione
    // Upgrade dal bundle vecchio: rimuovi i modelli Piper/whisper-base ormai inutilizzati (~500MB)
    // per non lasciarli a occupare spazio sul device. Sicuro: sono cartelle nostre, non più referenziate.
    let audio = base.join("audio");
    for old in ["vits-piper-it_IT-paola-medium", "sherpa-onnx-whisper-base"] {
        let _ = std::fs::remove_dir_all(audio.join(old));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rifiuta_path_traversal_e_nomi_vuoti() {
        // SICUREZZA: mai permettere di uscire da models_base (un `rm` arbitrario sarebbe critico).
        assert!(!is_safe_name("../secrets.txt"));
        assert!(!is_safe_name("sub/dir/x.gguf"));
        assert!(!is_safe_name("a\\b.gguf"));
        assert!(!is_safe_name(""));
        assert!(is_safe_name("gemma-4-e4b-it.gguf")); // nome semplice = ok
        // e il comando rifiuta PRIMA di toccare il filesystem
        assert!(delete_model(vec!["../boom".into()]).is_err());
    }

    #[test]
    fn file_inesistente_non_e_errore_zero_rimossi() {
        // nome valido ma assente → Ok(0), non un errore (idempotenza dell'eliminazione)
        let n = delete_model(vec!["__inesistente_test_zeliai_xyz__.gguf".into()]).unwrap();
        assert_eq!(n, 0);
    }
}
