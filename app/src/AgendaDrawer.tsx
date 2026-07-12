// Drawer Agenda: form nuovo evento + lista eventi. Riceve l'API di useAgenda e `onBack`. JSX verbatim.
import { t } from "./i18n";
import type { AgendaApi } from "./useAgenda";

export function AgendaDrawer({ agenda, onBack }: { agenda: AgendaApi; onBack: () => void }) {
  const { setShowAgenda, events, evTitle, setEvTitle, evWhen, setEvWhen, evNotes, setEvNotes, addEvent, removeEvent } = agenda;
  return (
    <div className="drawer-overlay" onClick={() => setShowAgenda(false)}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><button className="ghost back" onClick={onBack}>←</button><h2>📅 {t("Agenda", "Calendar")}</h2><button className="ghost" onClick={() => setShowAgenda(false)}>✕</button></div>
        <div className="pgroup">
          <h3>{t("Nuovo evento", "New event")}</h3>
          <input placeholder={t("Titolo", "Title")} value={evTitle} onChange={(e) => setEvTitle(e.target.value)} />
          <input placeholder={t("Data e ora (es. 2026-06-27 15:00)", "Date and time (e.g. 2026-06-27 15:00)")} value={evWhen} onChange={(e) => setEvWhen(e.target.value)} />
          <input placeholder={t("Note (opzionale)", "Notes (optional)")} value={evNotes} onChange={(e) => setEvNotes(e.target.value)} />
          <button className="send-sm" onClick={addEvent}>＋ {t("Aggiungi evento", "Add event")}</button>
        </div>
        <div className="pgroup">
          <h3>{t("Eventi", "Events")}</h3>
          {events.length === 0 && <p className="hint">{t("Nessun evento. Aggiungine uno qui o chiedi a Liara.", "No events. Add one here or ask Liara.")}</p>}
          {events.map((ev) => (
            <div key={ev.id} className="mailrow">
              <div className="mailrow-main">
                <span className="mailsubj">{ev.title}</span>
                <span className="mailsender">{ev.when_str}{ev.notes ? ` · ${ev.notes}` : ""}</span>
              </div>
              <button className="convdel" title={t("Elimina", "Delete")} onClick={() => removeEvent(ev.id)}>🗑</button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
