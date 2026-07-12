// Sottosistema MODELLO/DOWNLOAD: selezione modello, first-run download (con retry+resume), switch
// modello, eliminazione file, rilevamento versione obsoleta. È il path più critico dell'app (se si
// rompe, non parte al primo avvio) → estratto verbatim, comportamento IDENTICO. Dipendenze esterne
// (stato app-level) passate come argomenti: `isAndroid`, `initializing`, `setInitializing`.
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t, setLang } from "./i18n";
import { M, MODELS } from "./models";
import type { Model } from "./models";

export type Dl = { done: number; total: number; label?: string };

export function useModelDownload(isAndroid: boolean, initializing: boolean, setInitializing: (v: boolean) => void) {
  const [needDownload, setNeedDownload] = useState(false);
  const [dl, setDl] = useState<Dl | null>(null);
  const [dlErr, setDlErr] = useState("");
  const [modelId, setModelId] = useState(() => { try { return localStorage.getItem("liara_model") || "1.7b-it"; } catch { return "1.7b-it"; } });
  const [showModel, setShowModel] = useState(false);
  const [modelsPresent, setModelsPresent] = useState<Record<string, boolean>>({});
  const [switchTo, setSwitchTo] = useState<string | null>(null);
  const [outdated, setOutdated] = useState(false);
  const dlCancelRef = useRef(false);

  const activeModel = MODELS.find((m) => m.id === modelId) || MODELS[0];
  // Vision disponibile (→ mostra 📎): SOLO i modelli con visione nativa (Gemma 4, che ha il proprio
  // mmproj), su APK e desktop. I Qwen (1.7b/4b) sono testo-only → niente 📎 (il vecchio companion 3B è rimosso).
  const hasVision = !!activeModel.mmprojNative;
  // Il 12B è desktop-only: su Android (telefono) non ci sta (7 GB, serve 16 GB RAM) → nascosto dal selettore.
  const visibleModels = MODELS.filter((m) => !m.desktopOnly || !isAndroid);

  // Versioning: se il file scaricato ha uno SHA diverso da quello atteso → c'è un modello nuovo su HF.
  useEffect(() => {
    if (initializing || needDownload || !activeModel.sha) { setOutdated(false); return; }
    invoke<boolean>("model_outdated", { filename: activeModel.file, sha256: activeModel.sha })
      .then((o) => setOutdated(!!o)).catch(() => setOutdated(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initializing, needDownload, modelId]);

  // Download robusto: RETRY automatico (fino a 6 tentativi). Il backend riprende SEMPRE da dove era.
  const startDownload = async (model: Model = activeModel, restart = false) => {
    // Proteggi i telefoni deboli: il 4B (pesante) vuole RAM ≥12GB e GPU robusta.
    if (model.id.includes("4b")) {
      const caps = await invoke<{ ram_gb: number; weak_gpu: boolean }>("device_caps").catch(() => ({ ram_gb: 16, weak_gpu: false }));
      if (caps.ram_gb < 10 || caps.weak_gpu) {
        setDlErr(t(
          `Il tuo telefono (${caps.ram_gb} GB RAM) non regge il modello Avanzato (4B): si bloccherebbe. Usa il Bilanciato (1.7B) 👍`,
          `Your phone (${caps.ram_gb} GB RAM) can't handle the Advanced (4B): it would freeze. Use the Balanced (1.7B) 👍`
        ));
        return;
      }
    }
    setDlErr(""); setDl({ done: 0, total: model.bytes, label: t("Modello", "Model") }); dlCancelRef.current = false;
    for (let attempt = 1; attempt <= 6; attempt++) {
      try {
        await invoke("download_model", { url: model.url, sha256: model.sha, bytes: model.bytes, filename: model.file });
        // Vision NATIVA: solo Gemma (mmprojNative) → scarica il SUO mmproj (APK+desktop). I Qwen sono
        // testo-only: nessun download visione (il companion Qwen-VL 3B è stato rimosso).
        if (model.mmprojNative) {
          const mp = model.mmprojNative;
          setDl({ done: 0, total: mp.bytes, label: t("Visione (foto/documenti)", "Vision (photos/docs)") });
          await invoke("download_model", { url: `${M}/${mp.file}`, sha256: mp.sha, bytes: mp.bytes, filename: mp.file });
        }
        await invoke("set_active_model", { filename: model.file });
        try { localStorage.setItem("liara_model", model.id); } catch { /* */ }
        setModelId(model.id); setLang(model.lang as "it" | "en"); // l'interfaccia segue la lingua scelta
        setDl(null);
        if (restart) { await invoke("exit_app"); return; } // switch: riavvio pulito (RAM/GPU liberate)
        setNeedDownload(false); setInitializing(true); await invoke("warmup");
        return;
      } catch (err) {
        if (dlCancelRef.current) { setDl(null); setDlErr(""); return; } // annullato dall'utente
        if (attempt < 6) {
          setDlErr(t(`Connessione interrotta, riprovo… (${attempt}/6)`, `Connection lost, retrying… (${attempt}/6)`));
          await new Promise((r) => setTimeout(r, 2500));
          continue;
        }
        setDl(null);
        setDlErr(t("Download non riuscito. Controlla la connessione (consigliato Wi-Fi) e riprova.",
                   "Download failed. Check your connection (Wi-Fi recommended) and try again."));
      }
    }
  };

  // Cambia modello dal selettore: se presente → riavvia per usarlo; se manca → scarica (con retry) e riavvia.
  const chooseModel = async (m: Model) => {
    const present = await invoke<boolean>("model_present", { filename: m.file }).catch(() => false);
    // Gemma (mmprojNative): la VISTA è in un file SEPARATO. Se il modello c'è ma il mmproj no, Gemma
    // parte "senza occhi" → verifichiamo ENTRAMBI così un modello scaricato prima del fix recupera il mmproj.
    const mm = m.mmprojNative;
    const mmMissing = !!mm && !(await invoke<boolean>("model_present", { filename: mm.file }).catch(() => false));
    if (!present || mmMissing) { startDownload(m, true); return; }
    if (m.id === modelId) return;
    await invoke("set_active_model", { filename: m.file }).catch(() => {});
    try { localStorage.setItem("liara_model", m.id); } catch { /* */ }
    setModelId(m.id); setLang(m.lang as "it" | "en"); // l'interfaccia segue la lingua scelta
    setSwitchTo(m.size); // overlay "riavvia per applicare" invece di chiudere a sorpresa
  };

  // Elimina i file di un modello SCARICATO (GGUF + eventuale mmproj nativo Gemma) per liberare spazio.
  const deleteModelFiles = async (m: Model) => {
    const mm = m.mmprojNative;
    const files = mm ? [m.file, mm.file] : [m.file];
    const ok = window.confirm(t(
      `Eliminare il modello "${m.size}"? Libera ${m.gb}. Potrai riscaricarlo quando vuoi.`,
      `Delete the "${m.size}" model? Frees ${m.gb}. You can re-download it anytime.`));
    if (!ok) return;
    try {
      await invoke("delete_model", { files });
      setModelsPresent((p) => ({ ...p, [m.file]: false }));
    } catch (e) {
      setDlErr(t("Eliminazione non riuscita: ", "Deletion failed: ") + String(e));
    }
  };

  // Apre il drawer modelli caricando prima quali sono presenti su disco (per il tag ↻ Usa / ⬇ Scarica).
  const openModelDrawer = async () => {
    const p: Record<string, boolean> = {};
    for (const m of MODELS) p[m.file] = await invoke<boolean>("model_present", { filename: m.file }).catch(() => false);
    setModelsPresent(p);
    setShowModel(true);
  };

  return {
    needDownload, setNeedDownload, dl, setDl, dlErr, modelId, showModel, setShowModel, modelsPresent,
    switchTo, setSwitchTo, outdated, setOutdated, dlCancelRef, activeModel, hasVision, visibleModels,
    startDownload, chooseModel, deleteModelFiles, openModelDrawer,
  };
}

export type ModelDownloadApi = ReturnType<typeof useModelDownload>;
