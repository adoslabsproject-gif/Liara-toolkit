// Catalogo dei modelli scaricabili (config, non logica): id, file, URL, SHA256, dimensioni, e — per
// Gemma — l'mmproj della visione nativa.
//
// Dal 2026-07-16 il catalogo è DINAMICO: all'avvio l'app scarica models.json dal server (comando
// `fetch_models`) — aggiungere/aggiornare un modello = modificare models.json su NHA, SENZA
// ribuildare l'app. La lista qui sotto è il FALLBACK (primo avvio offline / server giù) e l'ultimo
// catalogo buono viene cacheato in localStorage.
//
// Download da HuggingFace (CDN CloudFront, resume affidabile): il server tedesco (HTTP/2) piantava
// i download mobile al ~40%. Repo pubblico adoslabs/liara-personal-ai.
export const M = "https://huggingface.co/adoslabs/liara-personal-ai/resolve/main";

export type Model = {
  id: string;
  lang: string;
  flag: string;
  icon: string;
  size: string;
  tag: string; // etichetta corta mostrata in header (es. "Gemma E4B")
  gb: string;
  sub: string;
  file: string;
  url: string;
  sha: string;
  bytes: number;
  desktopOnly?: boolean;
  /// RAM minima (GB) per scaricarlo/usarlo: il gate pre-download usa QUESTO, non euristiche sul nome.
  ramMinGb?: number;
  /// Temperatura di default per questo modello (0.1–1.5). Assente → euristica per dimensione.
  tempDefault?: number;
  /// Dialetto prompt (informativo: il Rust lo rileva dal GGUF via is_gemma; qui serve al catalogo/curatore).
  dialect?: "chatml" | "gemma";
  /// "deprecated": non offerto ai nuovi utenti, visibile SOLO se già installato/attivo (migrazione dolce).
  /// "coming": annunciato ma senza file → mai selezionabile (filtrato da loadCatalog).
  status?: "live" | "deprecated" | "coming";
  /// Quant per-dispositivo: i campi top-level sono il DEFAULT (es. q4km); una variante, se presente,
  /// li SOVRASCRIVE per quel device (es. desktop → q6k). Risolto una volta al load (resolveVariants):
  /// il resto dell'app vede sempre un Model piatto.
  variants?: { mobile?: ModelVariant; desktop?: ModelVariant };
  mmprojNative?: { file: string; sha: string; bytes: number; url?: string };
};

export type ModelVariant = { quant?: string; file: string; url: string; sha: string; bytes: number; gb?: string };

/// Temperatura di partenza PER MODELLO (l'utente medio non tocca lo slider: il primo impatto è il
/// default). Dal catalogo (`tempDefault`, ritoccabile dal server senza rebuild) o euristica: i
/// piccoli (<2,6 GB di GGUF ≈ classe ≤3B) partono precisi a 0.35, i grandi conversano a 0.7.
export function defaultTemp(m: Model): number {
  if (typeof m.tempDefault === "number" && m.tempDefault >= 0.1 && m.tempDefault <= 1.5) return m.tempDefault;
  return m.bytes > 0 && m.bytes < 2_600_000_000 ? 0.35 : 0.7;
}

/// Temperatura EFFETTIVA per un modello: override utente (slider, salvato per-modello) o default.
export function localTemp(m: Model): number {
  try {
    const v = parseFloat(localStorage.getItem(`liara_temp:${m.id}`) || "");
    if (Number.isFinite(v)) return Math.min(1.5, Math.max(0.1, v));
  } catch { /* */ }
  return defaultTemp(m);
}

/// Appiattisce le varianti quant sul device corrente. Chiamare UNA volta su catalogo caricato.
export function resolveVariants(list: Model[], isAndroid: boolean): Model[] {
  return list.map((m) => {
    const v = isAndroid ? m.variants?.mobile : m.variants?.desktop;
    return v ? { ...m, ...v, variants: undefined } : m;
  });
}

export const MODELS: Model[] = [
  // ⚠️ DEPRECATI (ordine del 2026-07-16, eval-fluenza fallita: 83%/10% difetti — sostituiti dalla
  // nuova fila di candidati via models.json). Restano nel fallback SOLO perché gli utenti esistenti
  // li hanno installati e attivi: senza queste entry la UI mostrerebbe etichette/vision sbagliate.
  { id: "1.7b-it", lang: "it", flag: "🇮🇹", icon: "✨", size: "Bilanciato", tag: "Liara 1.7B", gb: "1,0 GB", status: "deprecated",
    sub: "1.7B · più sveglio sugli strumenti e nel dialogo. Buon equilibrio velocità/capacità.",
    file: "liara-1.7b-it.gguf", url: `${M}/liara-1.7b-it.gguf`, dialect: "chatml",
    sha: "f2e93470ee8837e20c6825159acf7e990ffaf2ce3034989199a1ffcbab29b2cc", bytes: 1107408992 },
  { id: "4b-it", lang: "it", flag: "🇮🇹", icon: "🚀", size: "Avanzato", tag: "Liara 4B", gb: "2,5 GB", status: "deprecated", ramMinGb: 10,
    sub: "4B · più capace e preciso. ⚠️ Richiede almeno 12 GB di RAM (telefoni top): su telefoni più piccoli si blocca.",
    file: "liara-4b-it.gguf", url: `${M}/liara-4b-it.gguf`, dialect: "chatml",
    sha: "6bcb8a29841435ef7825495f1c2c7a62a67c39873d8fa8f8fdc1486b69f9e7a7", bytes: 2497279136 },
  { id: "gemma4-e4b", lang: "it", flag: "🇪🇺", icon: "💎", size: "Google Edge", tag: "Gemma E4B", gb: "5,3 GB", ramMinGb: 8, dialect: "gemma",
    sub: "Gemma 4 E4B (Google) affinato sul nostro dominio · function calling e visione NATIVI (foto/documenti 📎). ⚠️ ~5 GB: richiede 8 GB+ di RAM.",
    file: "gemma-4-e4b-it.gguf", url: `${M}/gemma-4-e4b-it.gguf`,
    sha: "55a9e1f1198b0f29ed3af753f725f52785a33805634f90dd03d11c762eba1c8d", bytes: 5335290656,
    // Vision NATIVO: il modello stesso vede le immagini col suo mmproj (niente Qwen-VL companion).
    // Scaricato su APK E desktop → il 📎 funziona anche su Android con Gemma.
    mmprojNative: { file: "mmproj-gemma-4-e4b-f16.gguf", sha: "ddf46c21d7078e95338cfc22306b19b276a29a5ad089023449dd54d4b6170a51", bytes: 990372672 } },
  { id: "gemma4-12b", lang: "it", flag: "🇪🇺", icon: "💠", size: "Mac / desktop", tag: "Gemma 12B", gb: "7,1 GB", desktopOnly: true, ramMinGb: 16, dialect: "gemma",
    sub: "Gemma 4 12B (Google) · il più capace, per Mac/desktop. Visione nativa (foto/documenti 📎). ⚠️ solo desktop: richiede 16 GB+ di RAM.",
    file: "gemma-4-12b-it.gguf", url: `${M}/gemma-4-12b-it.gguf`,
    sha: "43fec98c5102b1c446b4ddd0a9439f1db3a2e1f2e0b8cd143ce1ea619a9403d6", bytes: 7121860000,
    mmprojNative: { file: "mmproj-gemma-4-12b-f16.gguf", sha: "2e269f906eb15169ee9ce880ea649bd6d42d4964c21f8ede10d0d0efc738bcbb", bytes: 175115840 } },
];

// VISIONE: solo i modelli con `mmprojNative` vedono le immagini, col proprio proiettore nativo. Il vecchio
// companion Qwen2.5-VL-3B separato è stato RIMOSSO (2026-07-12): i liara-1.7b/4b sono `qwen3` testo-only,
// verificato dall'architettura nel GGUF. Un futuro VL nativo si aggiunge con `mmprojNative` in models.json.

const CACHE_KEY = "liara_models_json";

/// Una entry del models.json remoto è usabile se ha tutto ciò che serve a scaricare e verificare.
function validModel(m: unknown): m is Model {
  const x = m as Record<string, unknown>;
  return !!x && typeof x.id === "string" && typeof x.file === "string" && typeof x.url === "string"
    && typeof x.sha === "string" && x.sha.length === 64 && typeof x.bytes === "number" && x.bytes > 0
    && typeof x.size === "string" && typeof x.gb === "string" && typeof x.sub === "string";
}

/// Applica i default alle entry remote (il server può omettere i campi puramente estetici).
function withDefaults(m: Model): Model {
  return { ...m, lang: m.lang || "it", flag: m.flag || "🇮🇹", icon: m.icon || "✨", tag: m.tag || m.size };
}

export function parseCatalog(json: string): Model[] | null {
  try {
    const arr = JSON.parse(json);
    if (!Array.isArray(arr)) return null;
    // "coming" = annunciato senza file: MAI offerto finché il curatore non pubblica url+sha validi.
    const live = arr.filter((m) => m?.status !== "coming").filter(validModel).map(withDefaults);
    return live.length > 0 ? live : null;
  } catch { return null; }
}

/// Catalogo iniziale SINCRONO per il primo render: ultima copia buona dal cache, altrimenti il fallback.
export function initialCatalog(): Model[] {
  try {
    const cached = parseCatalog(localStorage.getItem(CACHE_KEY) || "");
    if (cached) return mergeCatalog(cached);
  } catch { /* */ }
  return MODELS;
}

/// Fusione server+fallback: il server COMANDA (ordina e definisce l'offerta); le entry del fallback
/// assenti dal server sopravvivono come "deprecated" così un modello installato resta riconoscibile
/// (etichetta, vision, delete) anche dopo che il server l'ha tolto dall'offerta.
function mergeCatalog(server: Model[]): Model[] {
  const ids = new Set(server.map((m) => m.id));
  const legacy = MODELS.filter((m) => !ids.has(m.id)).map((m) => ({ ...m, status: "deprecated" as const }));
  return [...server, ...legacy];
}

/// Scarica il catalogo dinamico dal server (fetch_models → models.json su NHA), lo valida e lo cachea.
/// In QUALSIASI caso di errore ritorna null: il chiamante resta sul catalogo che ha già (cache o fallback).
export async function loadCatalog(invoke: (cmd: string) => Promise<string>): Promise<Model[] | null> {
  try {
    const json = await invoke("fetch_models");
    const server = parseCatalog(json);
    if (!server) return null;
    try { localStorage.setItem(CACHE_KEY, json); } catch { /* */ }
    return mergeCatalog(server);
  } catch { return null; }
}
