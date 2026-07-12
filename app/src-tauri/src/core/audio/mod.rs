//! Local speech: Piper TTS + whisper STT via sherpa-onnx (offline, no cloud, cross-platform).
//! Both engines are heavy to construct, so they are lazy-loaded and reused.
mod text;
mod vad;
pub use text::punctuate_question;
pub use vad::listen_vad;

use anyhow::{anyhow, Result};
use sherpa_rs::tts::{VitsTts, VitsTtsConfig};
use sherpa_rs::whisper::{WhisperConfig, WhisperRecognizer};
use std::path::PathBuf;

// Path unico dei modelli audio: delega a paths::audio_dir() (LIARA_AUDIO_DIR, poi models_base()/audio)
// — una sola fonte di verità, niente hardcode duplicato che divergeva da quella centrale.
fn audio_dir() -> PathBuf {
    crate::core::paths::audio_dir()
}

fn s(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Push-to-talk recording session: hold to record, release to transcribe.
/// The cpal stream (not Send on all platforms) lives entirely on its own thread.
pub struct RecState {
    active: AtomicBool,
    rate: AtomicU32,
    buffer: Mutex<Vec<f32>>,
}
impl Default for RecState {
    fn default() -> Self {
        Self { active: AtomicBool::new(false), rate: AtomicU32::new(16000), buffer: Mutex::new(Vec::new()) }
    }
}

/// Start capturing from the default mic until `stop_recording` is called.
pub fn start_recording(state: Arc<RecState>) -> Result<()> {
    if state.active.swap(true, Ordering::SeqCst) {
        return Ok(()); // already recording
    }
    state.buffer.lock().unwrap().clear();
    // Android: il mic nativo (cpal/oboe) abortisce. La registrazione su Android avviene nella WebView
    // (getUserMedia) e i campioni arrivano a whisper via comando — qui non tocchiamo cpal.
    #[cfg(target_os = "android")]
    { state.active.store(false, Ordering::SeqCst); return Ok(()); }
    #[cfg(not(target_os = "android"))]
    std::thread::spawn(move || {
        let cleanup = state.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let stop = |s: &RecState| s.active.store(false, Ordering::SeqCst);
        let host = cpal::default_host();
        let Some(device) = host.default_input_device() else { return stop(&state); };
        let Ok(config) = device.default_input_config() else { return stop(&state); };
        state.rate.store(config.sample_rate().0, Ordering::SeqCst);
        let channels = (config.channels() as usize).max(1);
        let st = state.clone();
        let err_fn = |e| eprintln!("errore microfono: {e}");
        let built = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &_| {
                    if !st.active.load(Ordering::Relaxed) { return; }
                    let mut g = st.buffer.lock().unwrap();
                    for frame in data.chunks(channels) {
                        g.push(frame.iter().sum::<f32>() / channels as f32);
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data: &[i16], _: &_| {
                    if !st.active.load(Ordering::Relaxed) { return; }
                    let mut g = st.buffer.lock().unwrap();
                    for frame in data.chunks(channels) {
                        g.push(frame.iter().map(|&x| x as f32 / 32768.0).sum::<f32>() / channels as f32);
                    }
                },
                err_fn,
                None,
            ),
            _ => return stop(&state),
        };
        let Ok(stream) = built else { return stop(&state); };
        if stream.play().is_err() {
            return stop(&state);
        }
        // hold the stream alive on this thread until released
        while state.active.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        drop(stream);
        }));
        cleanup.active.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// Stop the session and return what was captured.
pub fn stop_recording(state: &RecState) -> (Vec<f32>, u32) {
    state.active.store(false, Ordering::SeqCst);
    std::thread::sleep(std::time::Duration::from_millis(90)); // let the thread flush + drop the stream
    let samples = std::mem::take(&mut *state.buffer.lock().unwrap());
    (samples, state.rate.load(Ordering::SeqCst))
}

/// Piper (VITS) Italian text-to-speech.
pub struct Tts {
    inner: VitsTts,
}
impl Tts {
    pub fn load() -> Result<Self> {
        let d = audio_dir().join("vits-piper-it_IT-paola-medium");
        if !d.exists() {
            return Err(anyhow!("modello TTS non trovato in {}", d.display()));
        }
        let cfg = VitsTtsConfig {
            model: s(d.join("it_IT-paola-medium.onnx")),
            tokens: s(d.join("tokens.txt")),
            data_dir: s(d.join("espeak-ng-data")),
            length_scale: 1.0,
            ..Default::default()
        };
        Ok(Self { inner: VitsTts::new(cfg) })
    }

    /// Synthesize speech → (mono f32 samples, sample rate).
    pub fn synth(&mut self, text: &str) -> Result<(Vec<f32>, u32)> {
        let audio = self.inner.create(text, 0, 1.0).map_err(|e| anyhow!("TTS: {e}"))?;
        Ok((audio.samples, audio.sample_rate))
    }
}

/// Campioni f32 [-1,1] → file WAV PCM 16-bit mono. Su Android la voce paola si sintetizza qui e si
/// riproduce nella WebView (l'audio nativo crasha): questo dà alla WebView un blob pronto da suonare.
pub fn pcm_to_wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

/// Streaming TTS queue: a single worker thread owns the Piper engine + audio output
/// (rodio is !Send) and plays appended sentences IN ORDER. The agent can push sentences
/// as they're generated → Liara starts speaking before the answer is finished.
pub struct TtsQueue {
    tx: std::sync::mpsc::Sender<TtsMsg>,
}
enum TtsMsg {
    Speak(String),
    Stop,
}
impl TtsQueue {
    /// `on_idle` fires once whenever the queue drains (used to hide the "stop voice" button).
    pub fn start(on_idle: impl Fn() + Send + 'static) -> Self {
        use std::sync::mpsc::RecvTimeoutError;
        let (tx, rx) = std::sync::mpsc::channel::<TtsMsg>();
        // Android: l'I/O audio nativo (cpal/rodio/oboe) abortisce ("destroyed mutex"). La voce su
        // Android passerà dalla WebView (Stage 2): qui la coda drena senza toccare rodio → niente crash.
        #[cfg(target_os = "android")]
        std::thread::spawn(move || {
            drop(on_idle);
            let _ = RecvTimeoutError::Timeout; // (silenzia l'import inutilizzato su android)
            while rx.recv().is_ok() {}
        });
        #[cfg(not(target_os = "android"))]
        std::thread::spawn(move || {
            let Ok((_stream, handle)) = rodio::OutputStream::try_default() else { return };
            let Ok(sink) = rodio::Sink::try_new(&handle) else { return };
            let mut tts: Option<Tts> = None;
            let mut had_audio = false;
            loop {
                match rx.recv_timeout(std::time::Duration::from_millis(120)) {
                    Ok(TtsMsg::Stop) => {
                        sink.clear();
                        sink.play();
                        if had_audio { had_audio = false; on_idle(); }
                    }
                    Ok(TtsMsg::Speak(text)) => {
                        if tts.is_none() { tts = Tts::load().ok(); }
                        if let Some(t) = tts.as_mut() {
                            if let Ok((samples, rate)) = t.synth(&text) {
                                sink.append(rodio::buffer::SamplesBuffer::new(1, rate, samples));
                                had_audio = true;
                            }
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if had_audio && sink.empty() { had_audio = false; on_idle(); }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self { tx }
    }
    pub fn speak(&self, text: &str) {
        let _ = self.tx.send(TtsMsg::Speak(text.to_string()));
    }
    pub fn stop(&self) {
        let _ = self.tx.send(TtsMsg::Stop);
    }
}

/// Whisper speech-to-text (multilingual; we bias to Italian).
pub struct Stt {
    inner: WhisperRecognizer,
}
impl Stt {
    pub fn load() -> Result<Self> {
        let d = audio_dir().join("sherpa-onnx-whisper-base");
        if !d.exists() {
            return Err(anyhow!("modello STT non trovato in {}", d.display()));
        }
        let cfg = WhisperConfig {
            encoder: s(d.join("base-encoder.int8.onnx")),
            decoder: s(d.join("base-decoder.int8.onnx")),
            tokens: s(d.join("base-tokens.txt")),
            language: "it".into(),
            ..Default::default()
        };
        Ok(Self { inner: WhisperRecognizer::new(cfg).map_err(|e| anyhow!("STT load: {e}"))? })
    }

    /// Transcribe mono f32 samples at the given rate → text.
    pub fn transcribe(&mut self, samples: &[f32], sample_rate: u32) -> String {
        self.inner.transcribe(sample_rate, samples).text
    }
}
