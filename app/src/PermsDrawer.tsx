// Drawer Permessi: per ogni strumento sensibile, Consenti / Chiedi / Nega (persistito lato server).
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { haptic } from "./audio";

export type Perm = [string, string, string]; // [tool, descrizione, stato]

export function PermsDrawer({ perms, setPerms, onBack, onClose }: {
  perms: Perm[]; setPerms: React.Dispatch<React.SetStateAction<Perm[]>>; onBack: () => void; onClose: () => void;
}) {
  return (
    <div className="drawer-overlay" onClick={onClose}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><button className="ghost back" onClick={onBack}>←</button><h2>🔐 {t("Permessi", "Permissions")}</h2><button className="ghost" onClick={onClose}>✕</button></div>
        <p className="hint">{t('Controlla cosa può fare Liara. "Chiedi" = te lo chiede ogni volta; revocabile quando vuoi.', 'Control what Liara can do. "Ask" = she asks every time; revocable anytime.')}</p>
        {perms.map(([tool, desc, st]) => (
          <div key={tool} className="mailrow">
            <div className="mailrow-main"><span className="mailsubj">{tool}</span><span className="mailsender">{desc}</span></div>
            <select className="permsel" value={st} onChange={(e) => { invoke("set_permission", { tool, value: e.target.value }); setPerms((p) => p.map((r) => (r[0] === tool ? [r[0], r[1], e.target.value] : r))); haptic(15); }}>
              <option value="allow">{t("Consenti", "Allow")}</option>
              <option value="ask">{t("Chiedi", "Ask")}</option>
              <option value="deny">{t("Nega", "Deny")}</option>
            </select>
          </div>
        ))}
      </div>
    </div>
  );
}
