// Drawer "Modello AI": elenco modelli scaricabili/attivabili, con tag ↻ Usa / ⬇ Scarica / IN USO,
// eliminazione e progresso download. Riceve l'API di useModelDownload e `onBack`. JSX verbatim.
import { t } from "./i18n";
import type { ModelDownloadApi } from "./useModelDownload";

export function ModelDrawer({ md, cloud, onCloud, onBack }: { md: ModelDownloadApi; cloud: boolean; onCloud: (on: boolean) => void; onBack: () => void }) {
  return (
    <div className="drawer-overlay" onClick={() => md.setShowModel(false)}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><button className="ghost back" onClick={onBack}>←</button><h2>🧠 {t("Modello AI", "AI model")}</h2><button className="ghost" onClick={() => md.setShowModel(false)}>✕</button></div>
        <p className="load-hint" style={{ margin: "0 4px 10px" }}>{t("Cambiando modello l'app si ", "Switching model ")}<b>{t("riavvia", "restarts the app")}</b>{t(" per liberare memoria e GPU. Puoi tenerli entrambi scaricati e alternarli.", " to free memory and GPU. You can keep both downloaded and switch between them.")}</p>
        {/* Modalità cloud: il 32B via API (nessun download). Attivarla = i dati escono dal dispositivo (consenso). */}
        <button className={`menurow ${cloud ? "active" : ""}`} onClick={() => onCloud(!cloud)}>
          <span className="menuico">☁️ ✨</span>
          <span className="menuname">{t("Liara Cloud (32B)", "Liara Cloud (32B)")}<br /><small style={{ color: "var(--mut)" }}>{t("Il più capace · via internet · niente da scaricare. I dati escono dal dispositivo.", "The most capable · over the internet · nothing to download. Data leaves the device.")}</small></span>
          <span className="menutag">{cloud ? t("IN USO", "IN USE") : t("☁️ Attiva", "☁️ Enable")}</span>
        </button>
        {md.visibleModels.map((m) => (
          <button key={m.id} className={`menurow ${!cloud && m.id === md.modelId ? "active" : ""}`} onClick={() => { if (cloud) onCloud(false); md.chooseModel(m); }} disabled={!!md.dl}>
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
        {md.dl && <div style={{ margin: "12px 4px 0" }}><div className="dl-bar"><div className="dl-fill" style={{ width: `${md.dl.total ? Math.round(md.dl.done / md.dl.total * 100) : 0}%` }} /></div><div className="load-hint">{md.dl.label ? `${md.dl.label} — ` : ""}{t("Scarico…", "Downloading…")} {(md.dl.done / 1e9).toFixed(2)} / {(md.dl.total / 1e9).toFixed(2)} GB</div></div>}
        {md.dlErr && <div className="load-hint" style={{ color: "#e88" }}>⚠️ {md.dlErr}</div>}
      </div>
    </div>
  );
}
