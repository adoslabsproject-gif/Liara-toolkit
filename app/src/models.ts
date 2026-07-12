// Catalogo dei modelli scaricabili (config, non logica): id, file, URL, SHA256, dimensioni, e — per
// Gemma — l'mmproj della visione nativa. Estratto da App.tsx e TIPIZZATO esplicitamente, così `Model`
// ha `mmprojNative?`/`desktopOnly?` opzionali e spariscono i cast `as { mmprojNative?… }` sparsi.
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
  gb: string;
  sub: string;
  file: string;
  url: string;
  sha: string;
  bytes: number;
  desktopOnly?: boolean;
  mmprojNative?: { file: string; sha: string; bytes: number };
};

export const MODELS: Model[] = [
  { id: "1.7b-it", lang: "it", flag: "🇮🇹", icon: "✨", size: "Bilanciato", gb: "1,0 GB", sub: "1.7B · più sveglio sugli strumenti e nel dialogo. Buon equilibrio velocità/capacità.",
    file: "liara-1.7b-it.gguf", url: `${M}/liara-1.7b-it.gguf`,
    sha: "f2e93470ee8837e20c6825159acf7e990ffaf2ce3034989199a1ffcbab29b2cc", bytes: 1107408992 },
  { id: "4b-it", lang: "it", flag: "🇮🇹", icon: "🚀", size: "Avanzato", gb: "2,5 GB", sub: "4B · più capace e preciso. ⚠️ Richiede almeno 12 GB di RAM (telefoni top): su telefoni più piccoli si blocca.",
    file: "liara-4b-it.gguf", url: `${M}/liara-4b-it.gguf`,
    sha: "6bcb8a29841435ef7825495f1c2c7a62a67c39873d8fa8f8fdc1486b69f9e7a7", bytes: 2497279136 },
  { id: "gemma4-e4b", lang: "it", flag: "🇪🇺", icon: "💎", size: "Google Edge", gb: "5,3 GB", sub: "Gemma 4 E4B (Google) affinato sul nostro dominio · function calling e visione NATIVI (foto/documenti 📎). ⚠️ ~5 GB: richiede 8 GB+ di RAM.",
    file: "gemma-4-e4b-it.gguf", url: `${M}/gemma-4-e4b-it.gguf`,
    sha: "55a9e1f1198b0f29ed3af753f725f52785a33805634f90dd03d11c762eba1c8d", bytes: 5335290656,
    // Vision NATIVO: il modello stesso vede le immagini col suo mmproj (niente Qwen-VL companion).
    // Scaricato su APK E desktop → il 📎 funziona anche su Android con Gemma.
    mmprojNative: { file: "mmproj-gemma-4-e4b-f16.gguf", sha: "ddf46c21d7078e95338cfc22306b19b276a29a5ad089023449dd54d4b6170a51", bytes: 990372672 } },
  { id: "gemma4-12b", lang: "it", flag: "🇪🇺", icon: "💠", size: "Mac / desktop", gb: "7,1 GB", desktopOnly: true, sub: "Gemma 4 12B (Google) · il più capace, per Mac/desktop. Visione nativa (foto/documenti 📎). ⚠️ solo desktop: richiede 16 GB+ di RAM.",
    file: "gemma-4-12b-it.gguf", url: `${M}/gemma-4-12b-it.gguf`,
    sha: "43fec98c5102b1c446b4ddd0a9439f1db3a2e1f2e0b8cd143ce1ea619a9403d6", bytes: 7121860000,
    mmprojNative: { file: "mmproj-gemma-4-12b-f16.gguf", sha: "2e269f906eb15169ee9ce880ea649bd6d42d4964c21f8ede10d0d0efc738bcbb", bytes: 175115840 } },
  // NB: modelli in inglese (1.7b-en / 4b-en) RIMOSSI il 2026-07-04: non esistono ancora su HF.
  // Il modello italiano è addestrato SOLO su dataset italiano → in inglese risponderebbe male.
  // Verranno riaggiunti dopo aver addestrato i LoRA inglesi dedicati (dataset tradotto + SFT+DPO).
];

// VISIONE: solo i Gemma 4 (mmprojNative) vedono le immagini, col proprio proiettore nativo. Il vecchio
// companion Qwen2.5-VL-3B separato è stato RIMOSSO (2026-07-12): i liara-1.7b/4b sono `qwen3` testo-only,
// verificato dall'architettura nel GGUF. Un futuro Qwen-VL nativo si aggiungerà con `mmprojNative`, come Gemma.
