// Funzioni PURE su testo (nessun React, nessun DOM, nessun Tauri) — estratte da App.tsx così sono
// unità testabili in isolamento. Migliori dell'originale: `takeSentences` era annegata dentro
// `flushSpeak` (effetto collaterale `speak`), ora è pura e coperta da test.

/// Un messaggio è "ricco" se rende un grafico / tabella / HTML → leggerlo ad alta voce è assurdo.
export function isRich(text: string): boolean {
  return /```chart/.test(text) || /<\/?[a-z][a-z0-9]*[\s/>]/i.test(text) || /\n?\s*\|.*\|.*\|/.test(text);
}

/// Toglie blocchi codice/chart, HTML, sintassi tabelle e marcatori markdown → sola prosa pronunciabile.
export function cleanForSpeech(text: string): string {
  return text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/^\s*\|.*\|\s*$/gm, " ")
    .replace(/[*_#`>|]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

/// Icona per un allegato in base all'estensione del nome file.
export function fileIcon(name: string): string {
  const ext = name.split(".").pop()?.toLowerCase() || "";
  if (ext === "pdf") return "📕";
  if (["csv", "xls", "xlsx"].includes(ext)) return "📊";
  if (ext === "json") return "🗂️";
  if (["rs", "py", "js", "ts", "go", "java", "c", "cpp", "html", "css", "sh"].includes(ext)) return "💻";
  if (["png", "jpg", "jpeg", "gif", "webp", "svg"].includes(ext)) return "🖼️";
  return "📄";
}

/// Streaming TTS (parte PURA): dal buffer estrae le frasi COMPLETE (terminate da . ! ? o newline) e
/// restituisce quelle da pronunciare + il resto (la coda parziale non ancora terminata). Le frasi di
/// 1 solo carattere (es. un "." isolato) sono scartate. `flushSpeak` (in audio.ts) ci mette solo la
/// pronuncia sopra → la logica di taglio è testabile senza toccare l'audio.
export function takeSentences(text: string): { sentences: string[]; rest: string } {
  const re = /[^.!?\n]*[.!?\n]+/g;
  const sentences: string[] = [];
  let m: RegExpExecArray | null;
  let lastIdx = 0;
  while ((m = re.exec(text)) !== null) {
    const s = m[0].trim();
    if (s.length > 1) sentences.push(s);
    lastIdx = re.lastIndex;
  }
  return { sentences, rest: text.slice(lastIdx) };
}
