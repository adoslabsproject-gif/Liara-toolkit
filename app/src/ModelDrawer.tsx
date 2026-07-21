// Drawer "Modello AI": elenco modelli scaricabili/attivabili, con tag ↻ Usa / ⬇ Scarica / IN USO,
// eliminazione e progresso download. La creatività (temperatura) NON è più qui: sta nel composer
// (icona 🌡️ → popover slider), più a portata di mano — vedi App.tsx.
import { t } from "./i18n";
import type { ModelDownloadApi } from "./useModelDownload";

export function ModelDrawer({ md, cloud, onCloud, onBack }: { md: ModelDownloadApi; cloud: boolean; onCloud: (on: boolean, silent?: boolean) => void; onBack: () => void }) {
  return (
    <div className="drawer-overlay" onClick={() => md.setShowModel(false)}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><button className="ghost back" onClick={onBack}>←</button><h2>🧠 {t("Modello AI", "AI model")}</h2><button className="ghost" onClick={() => md.setShowModel(false)}>✕</button></div>
        <p className="load-hint" style={{ margin: "0 4px 10px" }}>{t("Cambiando modello l'app si ", "Switching model ")}<b>{t("riavvia", "restarts the app")}</b>{t(" per liberare memoria e GPU. Puoi tenerli entrambi scaricati e alternarli.", " to free memory and GPU. You can keep both downloaded and switch between them.")}</p>
        {/* Progresso download IN ALTO (sticky): durante lo scarico è la prima cosa visibile, sopra i modelli. */}
        {md.dl && (
          <div className="dl-top">
            <div className="dl-top-head">⬇️ <b>{md.dl.label || t("Scarico il modello…", "Downloading the model…")}</b>
              <span className="dl-top-pct">{md.dl.total ? Math.round(md.dl.done / md.dl.total * 100) : 0}%</span></div>
            <div className="dl-bar"><div className="dl-fill" style={{ width: `${md.dl.total ? Math.round(md.dl.done / md.dl.total * 100) : 0}%` }} /></div>
            <div className="load-hint" style={{ margin: "4px 0 0" }}>{(md.dl.done / 1e9).toFixed(2)} / {(md.dl.total / 1e9).toFixed(2)} GB</div>
          </div>
        )}
        {md.dlErr && <div className="load-hint" style={{ color: "#e88", margin: "0 4px 8px" }}>⚠️ {md.dlErr}</div>}
        {/* Modalità cloud: il 24B via API (nessun download). Attivarla = i dati escono dal dispositivo (consenso). */}
        <button className={`menurow ${cloud ? "active" : ""}`} onClick={() => onCloud(!cloud)}>
          <span className="menuico">☁️ ✨</span>
          <span className="menuname">{t("Liara Cloud (24B)", "Liara Cloud (24B)")}<br /><small style={{ color: "var(--mut)" }}>{t("Il più capace · via internet · niente da scaricare. I dati escono dal dispositivo.", "The most capable · over the internet · nothing to download. Data leaves the device.")}</small></span>
          <span className="menutag">{cloud ? t("IN USO", "IN USE") : t("☁️ Attiva", "☁️ Enable")}</span>
        </button>
        {md.visibleModels.map((m) => (
          <button key={m.id} className={`menurow ${!cloud && m.id === md.modelId ? "active" : ""}`} onClick={() => md.chooseModel(m, cloud, () => onCloud(false, true))} disabled={!!md.dl || (!cloud && m.id === md.modelId)}>
            <span className="menuico">{m.flag} {m.icon}</span>
            <span className="menuname">{m.size}<br /><small style={{ color: "var(--mut)" }}>{m.sub} · {m.gb}</small></span>
            <span className="menutag">{!cloud && m.id === md.modelId ? t("IN USO", "IN USE") : (md.modelsPresent[m.file] ? t("↻ Usa", "↻ Use") : t("⬇ Scarica", "⬇ Download"))}</span>
            {md.modelsPresent[m.file] && m.id !== md.modelId && (
              <span className="convdel" role="button" title={t("Elimina modello", "Delete model")}
                onClick={(e) => { e.stopPropagation(); md.deleteModelFiles(m); }}
                style={{ marginLeft: 8, cursor: "pointer" }}>🗑</span>
            )}
          </button>
        ))}
      </div>
    </div>
  );
}
