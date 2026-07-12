// Drawer Email: configurazione account, cartelle (INBOX/SENT/TRASH), lettura, composizione, invio.
// Riceve l'API dell'hook useEmail e un callback `onBack` (torna al menu). JSX estratto verbatim da App.tsx.
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "./i18n";
import { EMAIL_PROVIDERS, PROVIDER_HELP } from "./constants";
import type { EmailApi } from "./useEmail";

export function EmailDrawer({ email, onBack }: { email: EmailApi; onBack: () => void }) {
  const {
    setShowEmail, showCfg, setShowCfg, provider, pickProvider, emailCfg, setCfg, showPw, setShowPw,
    saveCfg, cfgMsg, compose, setCompose, sendCompose, sendStatus, fetchEmails, startCompose, folder,
    switchFolder, unread, mailStatus, mailHelp, openMail, setOpenMail, startReply, emails, emptyTrash,
    readMail, restoreMail, purgeMail, delMail,
  } = email;
  return (
    <div className="drawer-overlay" onClick={() => setShowEmail(false)}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><button className="ghost back" onClick={onBack}>←</button><h2>✉️ {t("Email", "Email")}</h2><button className="ghost" onClick={() => setShowEmail(false)}>✕</button></div>

        <button className="ghost" onClick={() => setShowCfg((s) => !s)}>{showCfg ? t("▲ Nascondi configurazione", "▲ Hide configuration") : t("⚙️ Configurazione account", "⚙️ Account configuration")}</button>
        {showCfg && (
          <div className="pgroup">
            <select className="psel" value={provider} onChange={(e) => pickProvider(e.target.value)}>
              <option value="">{t("— Scegli il provider —", "— Choose your provider —")}</option>
              {Object.keys(EMAIL_PROVIDERS).map((k) => <option key={k} value={k}>{k}</option>)}
              <option value="__custom">{t("Altro (manuale)", "Other (manual)")}</option>
            </select>
            <p className="hint">
              {provider && EMAIL_PROVIDERS[provider]?.app_pw
                ? <>⚠️ <b>{provider}</b>{t(" richiede una ", " requires an ")}<b>{t("password per le app", "app password")}</b>{t(" (non quella normale dell'account).", " (not your normal account password).")}</>
                : <>{t("IMAP manuale, ", "Manual IMAP, ")}<b>{t("nessun OAuth", "no OAuth")}</b>{t(". Scegli un provider o inserisci host/porta a mano.", ". Pick a provider or enter host/port manually.")}</>}
            </p>
            <input placeholder={t("Email", "Email")} value={emailCfg.email || ""} onChange={(e) => setCfg("email", e.target.value)} />
            <div className="pwfield">
              <input placeholder={t("Password (o password per le app)", "Password (or app password)")} type={showPw ? "text" : "password"} value={emailCfg.password || ""} onChange={(e) => setCfg("password", e.target.value)} />
              <button className="pweye" type="button" title={showPw ? t("Nascondi", "Hide") : t("Mostra", "Show")} onClick={() => setShowPw((s) => !s)}>{showPw ? "🙈" : "👁️"}</button>
            </div>
            <input placeholder={t("Host IMAP (ricezione)", "IMAP host (incoming)")} value={emailCfg.imap_host || ""} onChange={(e) => setCfg("imap_host", e.target.value)} />
            <input placeholder={t("Porta IMAP (993)", "IMAP port (993)")} value={emailCfg.imap_port || ""} onChange={(e) => setCfg("imap_port", e.target.value)} />
            <input placeholder={t("Host SMTP (invio)", "SMTP host (outgoing)")} value={emailCfg.smtp_host || ""} onChange={(e) => setCfg("smtp_host", e.target.value)} />
            <input placeholder={t("Porta SMTP (465)", "SMTP port (465)")} value={emailCfg.smtp_port || ""} onChange={(e) => setCfg("smtp_port", e.target.value)} />
            <button className="send-sm" onClick={saveCfg}>{t("Salva configurazione", "Save configuration")}</button>
            {cfgMsg && <p className="savedmsg">{cfgMsg}</p>}
          </div>
        )}

        {compose ? (
          <div className="pgroup">
            <button className="ghost" onClick={() => setCompose(null)}>‹ {t("Annulla", "Cancel")}</button>
            <input placeholder={t("A: destinatario@email.com", "To: recipient@email.com")} value={compose.to} onChange={(e) => setCompose({ ...compose, to: e.target.value })} />
            <input placeholder={t("Oggetto", "Subject")} value={compose.subject} onChange={(e) => setCompose({ ...compose, subject: e.target.value })} />
            <textarea className="composebody" placeholder={t("Scrivi il messaggio…", "Write your message…")} value={compose.body} onChange={(e) => setCompose({ ...compose, body: e.target.value })} />
            <button className="send-sm" onClick={sendCompose}>📤 {t("Invia", "Send")}</button>
            {sendStatus && <p className="hint">{sendStatus}</p>}
          </div>
        ) : (
          <>
            <div className="emailbar">
              <button className="ghost" onClick={fetchEmails}>⬇︎ {t("Scarica", "Download")}</button>
              <button className="ghost" onClick={startCompose}>✍️ {t("Scrivi", "Compose")}</button>
            </div>
            <div className="folders">
              {([["INBOX", "📥", t("In arrivo", "Inbox")], ["SENT", "📤", t("Inviate", "Sent")], ["TRASH", "🗑️", t("Cestino", "Trash")]] as const).map(([f, ic, label]) => (
                <button key={f} className={`folderbtn ${folder === f ? "active" : ""}`} onClick={() => switchFolder(f)}>
                  <span className="folderico">{ic}</span><span className="foldername">{label}</span>
                  {f === "INBOX" && unread > 0 && <span className="badge">{unread > 9 ? "9+" : unread}</span>}
                </button>
              ))}
            </div>
            {mailStatus && <p className="hint">{mailStatus}</p>}
            {mailHelp && (() => {
              const h = PROVIDER_HELP[provider] || { title: "Configurazione email", titleEn: "Email configuration", note: "Usa la password della casella e assicurati che l'accesso IMAP sia abilitato.", noteEn: "Use your mailbox password and make sure IMAP access is enabled." };
              return (
                <div className="helpbox">
                  <b>{t(h.title, h.titleEn)}</b>
                  <p className="hint">{t(h.note, h.noteEn)}</p>
                  {h.link && <a className="link" onClick={() => openUrl(h.link!)}>{t(h.linkText!, h.linkTextEn!)}</a>}
                  <div className="hint">{t("Poi premi 👁️ per controllare, ", "Then tap 👁️ to check, ")}<b>{t("Salva", "Save")}</b> → <b>{t("Scarica", "Download")}</b>.</div>
                </div>
              );
            })()}
            {openMail ? (
              <div className="pgroup">
                <button className="ghost" onClick={() => setOpenMail(null)}>‹ {t("Indietro", "Back")}</button>
                <div className="mailhead"><b>{openMail.subject || t("(senza oggetto)", "(no subject)")}</b><div className="hint">{openMail.sender} · {openMail.date}</div></div>
                <div className="mailbody">{openMail.body}</div>
                <button className="send-sm" onClick={startReply}>↩️ {t("Rispondi", "Reply")}</button>
              </div>
            ) : (
              <div className="pgroup">
                {folder === "TRASH" && emails.length > 0 && (
                  <button className="ghost forget" onClick={emptyTrash}>🗑️ {t("Svuota cestino", "Empty trash")}</button>
                )}
                {emails.length === 0 && (
                  <p className="hint">{
                    folder === "TRASH" ? t("Cestino vuoto.", "Trash is empty.")
                    : folder === "SENT" ? t("Nessuna email inviata.", "No sent emails.")
                    : t("Nessuna email. Configura l'account e premi «Scarica».", "No emails. Set up your account and press \"Download\".")
                  }</p>
                )}
                {emails.map((m) => (
                  <div key={m.id} className={`mailrow ${m.seen ? "" : "unseen"}`} onClick={() => readMail(m.id)}>
                    <div className="mailrow-main">
                      <span className="mailsender">{m.sender}</span>
                      <span className="mailsubj">{m.subject || t("(senza oggetto)", "(no subject)")}</span>
                    </div>
                    {folder === "TRASH" ? (
                      <>
                        <button className="convdel" title={t("Ripristina", "Restore")} onClick={(e) => { e.stopPropagation(); restoreMail(m.id); }}>♻️</button>
                        <button className="convdel" title={t("Elimina definitivamente", "Delete permanently")} onClick={(e) => { e.stopPropagation(); purgeMail(m.id); }}>✕</button>
                      </>
                    ) : (
                      <button className="convdel" title={t("Sposta nel cestino", "Move to trash")} onClick={(e) => delMail(m.id, e)}>🗑</button>
                    )}
                  </div>
                ))}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
