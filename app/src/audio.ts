// Audio: TTS (Piper) + registrazione mic + feedback aptico. Tutto lo stato mutabile (coda TTS,
// contesto di registrazione, flag Android) è INCAPSULATO qui — prima era sparso come variabili di
// modulo dentro App.tsx. L'audio nativo (cpal/rodio) crasha nella WebView Android: là si sintetizza
// via backend e si riproduce/registra nella WebView.
import { invoke } from "@tauri-apps/api/core";
import { takeSentences } from "./text";

// Android: dedotto dallo userAgent (inaffidabile) e CORRETTO all'avvio da device_caps. Incapsulato,
// con setter/getter, così App.tsx non tocca più una variabile di modulo condivisa.
let android = /Android/i.test(navigator.userAgent);
export function setAndroid(v: boolean) { android = v; }
export function getAndroid(): boolean { return android; }

// feedback aptico (WebView Android); no-op su desktop
export function haptic(pattern: number | number[] = 30) {
  try { (navigator as { vibrate?: (p: number | number[]) => void }).vibrate?.(pattern); } catch { /* none */ }
}

// --- Android: voce Piper sintetizzata nativamente, riprodotta nella WebView (rodio nativo crasha) ---
let ttsQ: string[] = [];          // code di URL WAV in attesa
let ttsBusy = false;
let ttsEl: HTMLAudioElement | null = null;
// Callback "TTS ferma": su Android la voce suona nella WebView (non da rodio), quindi il backend NON emette
// mai "tts-idle" (scarta on_idle). Senza questo, `speaking` restava true e il pulsante "Ferma voce" non
// spariva mai a fine parlato. Lo chiamiamo quando la coda si svuota. App.tsx lo lega a setSpeaking(false).
let onTtsIdleCb: (() => void) | null = null;
export function setOnTtsIdle(cb: () => void) { onTtsIdleCb = cb; }
function ttsNext() {
  if (ttsBusy || ttsQ.length === 0) return;
  ttsBusy = true;
  const url = ttsQ.shift()!;
  ttsEl = new Audio(url);
  const done = () => {
    URL.revokeObjectURL(url); ttsBusy = false; ttsEl = null;
    ttsNext();
    if (!ttsBusy && ttsQ.length === 0) onTtsIdleCb?.(); // coda vuota → TTS finita → nascondi "Ferma voce"
  };
  ttsEl.onended = done;
  ttsEl.onerror = done;
  ttsEl.play().catch(done);
}
async function speakAndroid(text: string) {
  try {
    const bytes = await invoke<number[]>("tts_synth", { text: text.slice(0, 600) });
    const blob = new Blob([new Uint8Array(bytes)], { type: "audio/wav" });
    ttsQ.push(URL.createObjectURL(blob));
    ttsNext();
  } catch (e) { console.error("LIARA-TTS-ERR", e); }
}
function stopTtsAndroid() {
  ttsQ = [];
  if (ttsEl) { ttsEl.pause(); ttsEl = null; }
  ttsBusy = false;
}

export function speak(text: string) {
  const t = text.trim();
  if (!t) return;
  if (android) { speakAndroid(t); return; }
  invoke("tts_speak", { text: t.slice(0, 600) }).catch(() => {});
}
export function stopSpeak() {
  if (android) { stopTtsAndroid(); return; }
  invoke("tts_stop").catch(() => {});
}

// --- Android: registrazione mic nella WebView (cpal nativo crasha) → PCM 16kHz a whisper nativo ---
let recCtx: AudioContext | null = null;
let recStream: MediaStream | null = null;
let recProc: ScriptProcessorNode | null = null;
let recChunks: Float32Array[] = [];
let recCancelled = false; // #5b: se stopRec arriva PRIMA che getUserMedia risolva (tap veloce), rilasciamo il mic
export async function startRecAndroid() {
  recChunks = [];
  recCancelled = false;
  const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  // Race: se nel frattempo è stato chiesto lo stop (pointerup arrivato durante il getUserMedia), NON tenere
  // il mic aperto — fermalo subito. Senza questo, un tap veloce lasciava il microfono occupato per sempre.
  if (recCancelled) { stream.getTracks().forEach((tr) => tr.stop()); recStream = null; return; }
  recStream = stream;
  recCtx = new AudioContext({ sampleRate: 16000 });
  const src = recCtx.createMediaStreamSource(recStream);
  recProc = recCtx.createScriptProcessor(4096, 1, 1);
  recProc.onaudioprocess = (e) => { recChunks.push(new Float32Array(e.inputBuffer.getChannelData(0))); };
  src.connect(recProc);
  recProc.connect(recCtx.destination);
}
export function stopRecAndroid(): { pcm: number[]; rate: number } {
  recCancelled = true; // se startRecAndroid è ancora dentro getUserMedia, rilascerà lo stream appena arriva
  const rate = Math.round(recCtx?.sampleRate ?? 16000);
  try { recProc?.disconnect(); } catch { /* ok */ }
  recStream?.getTracks().forEach((tr) => tr.stop());
  recCtx?.close().catch(() => {});
  const len = recChunks.reduce((a, b) => a + b.length, 0);
  const pcm = new Float32Array(len);
  let off = 0;
  for (const c of recChunks) { pcm.set(c, off); off += c.length; }
  recChunks = []; recCtx = null; recStream = null; recProc = null;
  return { pcm: Array.from(pcm), rate };
}

// streaming TTS: pronuncia ogni frase COMPLETA appena arriva, tiene la coda parziale. La logica di
// taglio è pura in `takeSentences` (text.ts, testata); qui aggiungiamo solo la pronuncia.
export function flushSpeak(buf: { current: string }): boolean {
  const { sentences, rest } = takeSentences(buf.current);
  sentences.forEach(speak);
  buf.current = rest;
  return sentences.length > 0;
}

