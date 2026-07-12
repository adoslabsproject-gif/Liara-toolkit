//! Local speech commands: Piper TTS + whisper STT (sherpa-onnx, offline).
use crate::AppState;
use std::sync::{Mutex, OnceLock};
use tauri::State;

// Cache del motore TTS Piper per la sintesi via WebView (Android). L'inferenza sherpa NON usa
// cpal/rodio (che su Android crashano) → solo onnxruntime, che gira. Caricato una volta.
static TTS_ENGINE: OnceLock<Mutex<Option<crate::core::audio::Tts>>> = OnceLock::new();

/// WebView TTS (Android): sintetizza il testo con la voce Piper paola → ritorna un WAV che la
/// WebView riproduce (niente rodio nativo). La voce è la STESSA del desktop.
#[tauri::command]
pub async fn tts_synth(text: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let slot = TTS_ENGINE.get_or_init(|| Mutex::new(None));
        let mut g = slot.lock().unwrap();
        if g.is_none() {
            *g = Some(crate::core::audio::Tts::load()?);
        }
        let (samples, rate) = g.as_mut().unwrap().synth(&text)?;
        Ok(crate::core::audio::pcm_to_wav(&samples, rate))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// WebView STT (Android): la WebView registra (getUserMedia) e manda i campioni PCM mono f32 al
/// loro sample-rate; qui trascriviamo con whisper nativo (niente cpal). Riusa lo Stt in cache.
#[tauri::command]
pub async fn stt_transcribe(pcm: Vec<f32>, rate: u32, state: State<'_, AppState>) -> Result<String, String> {
    let stt_slot = state.stt.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        if pcm.len() < (rate as usize) / 4 {
            return Ok(String::new()); // < ~0.25s → ignora
        }
        let mut samples = pcm;
        samples.extend(std::iter::repeat(0.0f32).take((rate as usize) * 3 / 5)); // silenzio finale
        let mut g = stt_slot.lock().unwrap();
        if g.is_none() {
            *g = Some(crate::core::audio::Stt::load()?);
        }
        let raw = g.as_mut().unwrap().transcribe(&samples, rate);
        Ok(crate::core::audio::punctuate_question(&raw))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn tts_speak(text: String, state: State<AppState>) {
    state.tts.speak(&text); // queued, plays in order (streaming-friendly)
}

#[tauri::command]
pub fn tts_stop(state: State<AppState>) {
    state.tts.stop();
}

#[tauri::command]
pub fn stt_start(state: State<AppState>) -> Result<(), String> {
    crate::core::audio::start_recording(state.rec.clone()).map_err(|e| e.to_string())
}

/// Hands-free: listen and auto-stop on silence (silero VAD), then transcribe. One tap, no hold.
#[tauri::command]
pub async fn listen_hands_free(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let rec = state.rec.clone();
    let stt = state.stt.clone();
    tauri::async_runtime::spawn_blocking(move || crate::core::audio::listen_vad(rec, stt, 15))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stt_stop(state: State<'_, AppState>) -> Result<String, String> {
    let rec = state.rec.clone();
    let stt_slot = state.stt.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let (mut samples, rate) = crate::core::audio::stop_recording(&rec);
        // ignore taps shorter than ~0.25s
        if samples.len() < (rate as usize) / 4 {
            return Ok(String::new());
        }
        // trailing silence (~0.6s) so whisper finalizes the sentence → punctuation, "?" on questions
        samples.extend(std::iter::repeat(0.0f32).take((rate as usize) * 3 / 5));
        let mut g = stt_slot.lock().unwrap();
        if g.is_none() {
            *g = Some(crate::core::audio::Stt::load()?);
        }
        let raw = g.as_mut().unwrap().transcribe(&samples, rate);
        Ok(crate::core::audio::punctuate_question(&raw))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
