// Drawer Messaggi SMS (menù separato dalla rubrica): consenso + "Sincronizza SMS" (permesso
// READ_SMS, solo Android). Tiene una copia cifrata locale per i tool sms_recent/sms_search.
import { t } from "./i18n";
import type { SmsApi } from "./useSms";

// "20/07 10:30" — data compatta; l'anno solo se diverso da quello corrente.
function fmtTs(ts: number): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  const dm = `${String(d.getDate()).padStart(2, "0")}/${String(d.getMonth() + 1).padStart(2, "0")}`;
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  return d.getFullYear() === now.getFullYear() ? `${dm} ${hm}` : `${dm}/${d.getFullYear()} ${hm}`;
}

export function SmsDrawer({ sms, onBack }: { sms: SmsApi; onBack: () => void }) {
  const { setShowSms, count, syncing, msg, list, syncNow } = sms;
  return (
    <div className="drawer-overlay" onClick={() => setShowSms(false)}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><button className="ghost back" onClick={onBack}>←</button><h2>📩 {t("Messaggi SMS", "SMS messages")}</h2><button className="ghost" onClick={() => setShowSms(false)}>✕</button></div>
        <div className="pgroup">
          <h3>{t("Sincronizza dal telefono", "Sync from phone")}{count > 0 ? ` (${count})` : ""}</h3>
          <p className="hint">{t(
            "Con la sincronizzazione Liara legge gli SMS del telefono e ne tiene una copia cifrata su questo dispositivo (mai inviata a server). Servono per “leggi gli ultimi sms” o “cosa mi ha scritto Marco?”.",
            "When syncing, Liara reads the phone's SMS and keeps an encrypted copy on this device (never sent to any server). They power “read my latest texts” or “what did Marco text me?”."
          )}</p>
          <button className="send-sm" onClick={syncNow} disabled={syncing}>{syncing ? t("Leggo gli SMS…", "Reading SMS…") : "🔄 " + t("Sincronizza SMS", "Sync SMS")}</button>
          {msg && <p className="hint">{msg === "ok" ? t("Nessun messaggio nuovo.", "No new messages.") : msg.startsWith("+") ? t(`${msg.slice(1)} nuovi messaggi importati.`, `${msg.slice(1)} new messages imported.`) : msg}</p>}
        </div>

        <div className="pgroup">
          <h3>{t("Messaggi", "Messages")}{list.length > 0 ? ` (${list.length})` : ""}</h3>
          {list.length === 0 && <p className="hint">{t("Nessun messaggio. Premi “Sincronizza SMS” per importarli dal telefono.", "No messages. Tap “Sync SMS” to import them from the phone.")}</p>}
          {list.map((m, i) => (
            <div key={i} className={"mailrow smsrow" + (m.kind === "out" ? " out" : "")}>
              <div className="mailrow-main">
                <span className="mailsubj">{m.kind === "out" ? "→ " : ""}{m.who}<span className="smsdate"> · {fmtTs(m.ts)}</span></span>
                <span className="smsbody">{m.body}</span>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
