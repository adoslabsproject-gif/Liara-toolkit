//! Voice Activity Detection (hands-free): silero VAD auto-endpoints the mic — tap once,
//! it records and stops itself on silence, then whisper transcribes the utterance.
use super::{audio_dir, s, start_recording, stop_recording, RecState, Stt};
use anyhow::{anyhow, Result};
use sherpa_rs::silero_vad::{SileroVad, SileroVadConfig};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VAD_RATE: u32 = 16000;

fn load_vad() -> Result<SileroVad> {
    let model = audio_dir().join("silero_vad.onnx");
    if !model.exists() {
        return Err(anyhow!("modello VAD non trovato in {}", model.display()));
    }
    let cfg = SileroVadConfig {
        model: s(model),
        min_silence_duration: 0.6,
        min_speech_duration: 0.25,
        max_speech_duration: 20.0,
        threshold: 0.5,
        sample_rate: VAD_RATE,
        window_size: 512,
        ..Default::default()
    };
    SileroVad::new(cfg, 30.0).map_err(|e| anyhow!("VAD load: {e}"))
}

/// Downsample mono f32 from `from` Hz to 16 kHz (linear interpolation). Whisper + silero want 16k.
pub(super) fn resample_16k(input: &[f32], from: u32) -> Vec<f32> {
    if from == VAD_RATE || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f32 / VAD_RATE as f32;
    let out_len = (input.len() as f32 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f32 * ratio;
        let i0 = src as usize;
        let i1 = (i0 + 1).min(input.len().saturating_sub(1));
        let frac = src - i0 as f32;
        out.push(input[i0] * (1.0 - frac) + input[i1] * frac);
    }
    out
}

/// Hands-free listen: capture until silence auto-endpoints a speech segment, then transcribe.
/// Returns the transcript, or None if nothing was said before the timeout.
pub fn listen_vad(rec: Arc<RecState>, stt: Arc<Mutex<Option<Stt>>>, max_wait_s: u64) -> Result<Option<String>> {
    let mut vad = load_vad()?;
    start_recording(rec.clone())?;
    let started = Instant::now();
    let mut spoke = false;
    let segment = loop {
        std::thread::sleep(Duration::from_millis(100));
        let device_rate = rec.rate.load(Ordering::SeqCst);
        let chunk = std::mem::take(&mut *rec.buffer.lock().unwrap());
        if !chunk.is_empty() {
            vad.accept_waveform(resample_16k(&chunk, device_rate));
        }
        if vad.is_speech() {
            spoke = true;
        }
        if !vad.is_empty() {
            let seg = vad.front();
            vad.pop();
            break Some(seg.samples);
        }
        let waited = started.elapsed().as_secs();
        if waited >= max_wait_s || (!spoke && waited >= 4) {
            break None; // overall timeout, or no speech started within 4s
        }
    };
    stop_recording(&rec);
    let Some(samples) = segment else { return Ok(None) };
    let mut g = stt.lock().unwrap();
    if g.is_none() {
        *g = Some(Stt::load()?);
    }
    Ok(Some(g.as_mut().unwrap().transcribe(&samples, VAD_RATE)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_passthrough_at_16k() {
        let x = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_16k(&x, 16000), x);
    }

    #[test]
    fn resample_48k_to_16k_thirds_the_length() {
        let input: Vec<f32> = (0..480).map(|i| i as f32).collect();
        let out = resample_16k(&input, 48000);
        assert_eq!(out.len(), 160); // 48k -> 16k is 1/3
        assert!((out[0] - 0.0).abs() < 1e-3);
    }

    #[test]
    fn resample_empty_is_empty() {
        assert!(resample_16k(&[], 48000).is_empty());
    }
}
