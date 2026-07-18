// Overlay a schermo intero: caricamento modello, first-run download (scelta modello + progresso),
// e "riavvia per applicare lo switch". Estratti da App.tsx; ricevono l'API di useModelDownload.
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import type { ModelDownloadApi } from "./useModelDownload";

export function LoadOverlays({ md, initializing, settling, status, onUseCloud }: { md: ModelDownloadApi; initializing: boolean; settling: boolean; status: string; onUseCloud: () => void }) {
  return (
    <>
      {(initializing || settling || status.startsWith("Carico il modello") || status.startsWith("Loading the local model")) && (
        <div className="load-overlay">
          <div className="load-spinner" />
          <div className="load-title">{t("Avvio Liara…", "Starting Liara…")}</div>
          <div className="load-sub">{status || t("Mi sto assestando…", "Getting ready…")}</div>
          <div className="load-hint">{t("Un istante: preparo tutto. Sono pronta appena sparisce questa schermata.", "One moment: getting everything ready. I'm ready as soon as this screen disappears.")}</div>
        </div>
      )}
      {md.needDownload && !settling && (
        <div className="load-overlay">
          <div className="load-title">{t("Benvenuto in Liara 👋", "Welcome to Liara 👋")}</div>
          <div className="load-sub">{t("Scegli il modello da scaricare (una volta sola — lo cambi quando vuoi dalle Impostazioni):", "Choose the model to download (just once — you can change it anytime in Settings):")}</div>
          {md.dl ? (
            <div style={{ width: "82%", maxWidth: 340, textAlign: "center" }}>
              <div className="dl-bar"><div className="dl-fill" style={{ width: `${md.dl.total ? Math.min(100, Math.round((md.dl.done / md.dl.total) * 100)) : 0}%` }} /></div>
              <div className="load-hint">{md.dl.label ? `${md.dl.label}: ` : ""}{(md.dl.done / 1e9).toFixed(2)} / {(md.dl.total / 1e9).toFixed(2)} GB · {md.dl.total ? Math.round((md.dl.done / md.dl.total) * 100) : 0}%</div>
              <button className="dl-btn dl-retry" onClick={() => { md.dlCancelRef.current = true; invoke("cancel_download").catch(() => {}); }}>{t("Annulla download", "Cancel download")}</button>
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 12, width: "86%", maxWidth: 360 }}>
              {/* Opzione cloud: parti SUBITO senza scaricare nulla (il 32B gira sul server). I dati escono
                  dal dispositivo → l'attivazione passa dal consenso (onUseCloud apre il modale). */}
              <button className="dl-btn dl-cloud" style={{ margin: 0, textAlign: "left", lineHeight: 1.35 }} onClick={onUseCloud}>
                ☁️ <b>{t("Liara Cloud (32B)", "Liara Cloud (32B)")}</b> · {t("subito, niente da scaricare", "start now, nothing to download")}<br />
                <small style={{ fontWeight: 400, opacity: .9 }}>{t("La più capace · via internet · i dati escono dal dispositivo", "The most capable · over the internet · data leaves the device")}</small>
              </button>
              {md.visibleModels.map((m) => (
                <button key={m.id} className="dl-btn" style={{ margin: 0, textAlign: "left", lineHeight: 1.35 }} onClick={() => md.startDownload(m)}>
                  {m.flag} {m.icon} <b>{m.size}</b> · ~{m.gb}<br />
                  <small style={{ fontWeight: 400, opacity: .9 }}>{m.sub}</small>
                </button>
              ))}
            </div>
          )}
          {md.dlErr && <div className="load-hint" style={{ color: "#e88" }}>⚠️ {md.dlErr} <button className="dl-btn dl-retry" onClick={() => md.startDownload()}>{t("Riprova", "Retry")}</button></div>}
          <div className="load-hint">{t("Consigliato Wi-Fi. Se si interrompe, riprende da dove era. 🔒 Niente lascia il telefono.", "Wi-Fi recommended. If it stops, it resumes where it left off. 🔒 Nothing leaves your phone.")}</div>
        </div>
      )}
      {md.switchTo && (
        <div className="load-overlay">
          <div className="load-title">✅ {md.switchTo} {t("pronto", "ready")}</div>
          <div className="load-sub">{t("Per usare questo modello, riavvia Liara — così memoria e GPU ripartono pulite.", "To use this model, restart Liara — so memory and GPU start fresh.")}</div>
          <button className="dl-btn" onClick={() => invoke("exit_app")}>{t("Chiudi Liara ora", "Close Liara now")}</button>
          {/* ANNULLA (cambio idea): chiude l'overlay senza chiudere l'app. Il modello scelto è già
              salvato → si applica al prossimo riavvio naturale; la sessione attuale continua com'è. */}
          <button className="dl-btn" style={{ background: "transparent", border: "1px solid var(--line)", color: "var(--txt)" }} onClick={() => md.setSwitchTo(null)}>{t("Annulla", "Cancel")}</button>
          <div className="load-hint">{t("Poi riaprila dall'icona: caricherà il modello scelto.", "Then reopen it from the icon: it'll load the chosen model.")}</div>
        </div>
      )}
    </>
  );
}
