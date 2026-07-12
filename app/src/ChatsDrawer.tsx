// Drawer Conversazioni: nuova chat + elenco delle chat salvate (caricamento/eliminazione).
// La logica (newChat/loadConv/deleteConv) resta nel core (tocca nodes/activeChild) → passata come callback.
import { t } from "./i18n";

export function ChatsDrawer({ convs, activeId, onNew, onLoad, onDelete, onClose }: {
  convs: [string, string, number][];
  activeId: string;
  onNew: () => void;
  onLoad: (id: string) => void;
  onDelete: (id: string, e: React.MouseEvent) => void;
  onClose: () => void;
}) {
  return (
    <div className="drawer-overlay left" onClick={onClose}>
      <div className="drawer ldrawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><h2>🗂️ {t("Conversazioni", "Conversations")}</h2><button className="ghost" onClick={onClose}>✕</button></div>
        <button className="ghost newconv" onClick={onNew}>✚ {t("Nuova chat", "New chat")}</button>
        {convs.length === 0 && <p className="hint">{t("Nessuna conversazione salvata ancora.", "No saved conversations yet.")}</p>}
        {convs.map(([id, title]) => (
          <div key={id} className={`convrow ${id === activeId ? "active" : ""}`} onClick={() => onLoad(id)}>
            <span className="convtitle">{title}</span>
            <button className="convdel" title={t("Elimina", "Delete")} onClick={(e) => onDelete(id, e)}>🗑</button>
          </div>
        ))}
      </div>
    </div>
  );
}
