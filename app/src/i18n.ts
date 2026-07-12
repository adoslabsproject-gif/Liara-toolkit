// ─────────────────────────────────────────────────────────────────────────────
// i18n di Liara — minimale, reattivo, zero dipendenze.
//
// Filosofia: traduzioni INLINE e leggibili → ogni stringa è `t("Italiano", "English")`,
// niente file di chiavi da tenere sincronizzato, il testo originale è lì sotto gli occhi.
// La lingua è globale e reattiva: `setLang()` notifica i componenti (via useLang) che
// ri-renderizzano all'istante, senza riavviare l'app. Persistita in localStorage.
// ─────────────────────────────────────────────────────────────────────────────
import { useEffect, useState } from "react";

export type Lang = "it" | "en";

let _lang: Lang = (() => {
  try {
    const s = localStorage.getItem("liara_lang");
    if (s === "it" || s === "en") return s;
    // primo avvio: deduci dalla lingua del telefono (it → italiano, tutto il resto → inglese)
    return (navigator.language || "it").toLowerCase().startsWith("it") ? "it" : "en";
  } catch {
    return "it";
  }
})();

const listeners = new Set<() => void>();

export function getLang(): Lang {
  return _lang;
}

export function setLang(l: Lang): void {
  if (l === _lang) return;
  _lang = l;
  try { localStorage.setItem("liara_lang", l); } catch { /* */ }
  document.documentElement.lang = l;
  listeners.forEach((f) => f());
}

/** Traduzione inline: ritorna la stringa nella lingua attiva. */
export function t(it: string, en: string): string {
  return _lang === "en" ? en : it;
}

/** Hook React: rende il componente reattivo ai cambi di lingua e restituisce la lingua attiva. */
export function useLang(): Lang {
  const [, force] = useState(0);
  useEffect(() => {
    const f = () => force((x) => x + 1);
    listeners.add(f);
    return () => { listeners.delete(f); };
  }, []);
  return _lang;
}
