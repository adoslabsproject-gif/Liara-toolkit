// Drawer Rubrica: consenso + "Sincronizza rubrica" (solo Android) → mostra SOLO i contatti NUOVI da
// importare con selezione multipla e un bottone di conferma STICKY (sempre visibile, niente scroll
// fino in fondo). Sotto, la rubrica dell'app: ogni contatto si modifica (→ "personalizzato", non più
// sovrascritto dal sync) o si elimina.
import { t } from "./i18n";
import type { ContactsApi } from "./useContacts";

export function ContactsDrawer({ contacts, onBack }: { contacts: ContactsApi; onBack: () => void }) {
  const {
    setShowContacts, imported, sys, sel, syncing, err, syncNow, toggleSel, importSel, removeContact,
    editId, editName, setEditName, editNumber, setEditNumber, startEdit, saveEdit, cancelEdit,
  } = contacts;

  // dal risultato del sync isoliamo i NUOVI (importabili). I già-importati/personalizzati non si
  // mostrano qui: sono nella rubrica dell'app sotto → lista corta, niente scroll infinito.
  const nuovi = sys ? sys.filter((c) => !c.imported) : [];
  const giaPresenti = sys ? sys.length - nuovi.length : 0;

  return (
    <div className="drawer-overlay" onClick={() => setShowContacts(false)}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><button className="ghost back" onClick={onBack}>←</button><h2>👥 {t("Rubrica", "Contacts")}</h2><button className="ghost" onClick={() => setShowContacts(false)}>✕</button></div>

        <div className="pgroup">
          <h3>{t("Sincronizza dal telefono", "Sync from phone")}</h3>
          <p className="hint">{t(
            "Liara legge la rubrica del telefono e ti fa scegliere quali contatti importare. Nome e numero restano cifrati su questo dispositivo (mai inviati a server) e servono per “chiama Marco” o “scrivi a Marco”.",
            "Liara reads the phone's contacts and lets you choose which ones to import. Names and numbers stay encrypted on this device (never sent to any server) and power “call Marco” or “text Marco”."
          )}</p>
          <button className="send-sm" onClick={syncNow} disabled={syncing}>{syncing ? t("Leggo la rubrica…", "Reading contacts…") : "🔄 " + t("Sincronizza rubrica", "Sync contacts")}</button>
          {err && <p className="hint">⚠️ {err}</p>}
        </div>

        {sys && nuovi.length === 0 && (
          <div className="pgroup">
            <p className="hint">{t("✓ Niente da sincronizzare: la rubrica è aggiornata.", "✓ Nothing to sync: contacts are up to date.")}
              {giaPresenti > 0 && " " + t(`(${giaPresenti} già in rubrica)`, `(${giaPresenti} already in contacts)`)}</p>
          </div>
        )}

        {sys && nuovi.length > 0 && (
          <div className="pgroup synclist">
            <h3>{t("Nuovi da importare", "New to import")} ({nuovi.length}){giaPresenti > 0 && <span className="hint inline"> · {t(`${giaPresenti} già in rubrica`, `${giaPresenti} already in contacts`)}</span>}</h3>
            <div className="pickscroll">
              {nuovi.map((c) => (
                <label key={c.number} className="mailrow">
                  <input type="checkbox" checked={sel.has(c.number)} onChange={() => toggleSel(c.number)} />
                  <div className="mailrow-main">
                    <span className="mailsubj">{c.name || c.number}</span>
                    <span className="mailsender">{c.number}</span>
                  </div>
                </label>
              ))}
            </div>
            {/* barra sticky: sempre visibile, non serve scorrere fino in fondo */}
            <div className="stickybar">
              <button className="send-sm" onClick={importSel} disabled={sel.size === 0}>
                ＋ {t("Importa selezionati", "Import selected")} ({sel.size})
              </button>
            </div>
          </div>
        )}

        <div className="pgroup">
          <h3>{t("Rubrica di Liara", "Liara's contacts")}{imported.length > 0 ? ` (${imported.length})` : ""}</h3>
          {imported.length === 0 && <p className="hint">{t("Nessun contatto importato.", "No contacts imported yet.")}</p>}
          {imported.map((c) =>
            editId === c.id ? (
              <div key={c.id} className="editrow">
                <input value={editName} onChange={(e) => setEditName(e.target.value)} placeholder={t("Nome", "Name")} />
                <input value={editNumber} onChange={(e) => setEditNumber(e.target.value)} placeholder={t("Numero", "Number")} inputMode="tel" />
                <div className="editrow-actions">
                  <button className="send-sm" onClick={saveEdit}>{t("Salva", "Save")}</button>
                  <button className="send-sm alt" onClick={cancelEdit}>{t("Annulla", "Cancel")}</button>
                </div>
              </div>
            ) : (
              <div key={c.id} className="mailrow">
                <div className="mailrow-main">
                  <span className="mailsubj">{c.name}{c.customized && <span className="tag-custom">{t("personalizzato", "custom")}</span>}</span>
                  <span className="mailsender">{c.number}</span>
                </div>
                <button className="convdel" title={t("Modifica", "Edit")} onClick={() => startEdit(c)}>✏️</button>
                <button className="convdel" title={t("Rimuovi", "Remove")} onClick={() => removeContact(c.id)}>🗑</button>
              </div>
            )
          )}
        </div>
      </div>
    </div>
  );
}
