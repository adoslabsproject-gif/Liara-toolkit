// Drawer "Su di me": campi del profilo strutturato + i fatti appresi (modifica/aggiungi/dimentica).
// Riceve l'API di useProfile e `onBack` (torna al menu). JSX estratto verbatim da App.tsx.
import { t } from "./i18n";
import { PROFILE_GROUPS } from "./constants";
import type { ProfileApi } from "./useProfile";

export function ProfileDrawer({ profile: p, onBack }: { profile: ProfileApi; onBack: () => void }) {
  const {
    setShowProfile, profile, setProfile, editFact, setEditFact, facts, newFact, setNewFact,
    saveField, addManualFact, forgetFacts, deleteFact, saveEditFact,
  } = p;
  return (
    <div className="drawer-overlay" onClick={() => setShowProfile(false)}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head">
          <button className="ghost back" onClick={onBack}>←</button>
          <h2>👤 {t("Su di me", "About me")}</h2>
          <button className="ghost" onClick={() => setShowProfile(false)}>✕</button>
        </div>
        <p className="hint">{t("Tutto è ", "Everything is ")}<b>{t("facoltativo", "optional")}</b>{t(". Più Liara ti conosce, meglio ti aiuta. Resta solo sul tuo dispositivo.", ". The more Liara knows you, the better she helps. It stays only on your device.")}</p>

        {PROFILE_GROUPS.map((g) => (
          <div className="pgroup" key={g.title}>
            <h3>{t(g.title, g.titleEn)}</h3>
            {g.fields.map(([key, label]) =>
              key === "Note" ? (
                <textarea key={key} placeholder={t("Qualsiasi cosa Liara dovrebbe sapere…", "Anything Liara should know…")} value={profile[key] || ""}
                  onChange={(e) => setProfile((pr) => ({ ...pr, [key]: e.target.value }))}
                  onBlur={(e) => saveField(key, e.target.value)} />
              ) : (
                <input key={key} placeholder={t(key, label)} value={profile[key] || ""}
                  onChange={(e) => setProfile((pr) => ({ ...pr, [key]: e.target.value }))}
                  onBlur={(e) => saveField(key, e.target.value)} />
              )
            )}
          </div>
        ))}

        <div className="pgroup">
          <h3>🧠 {t("Cosa Liara ha imparato da sola", "What Liara has learned on her own")}</h3>
          {facts.length === 0 && <p className="hint">{t("Niente ancora — emergerà chiacchierando.", "Nothing yet — it'll emerge as you chat.")}</p>}
          {facts.map((f, i) => editFact?.i === i ? (
            <div className="factrow" key={i}>
              <input className="factedit" value={editFact.v} autoFocus
                onChange={(e) => setEditFact({ i, v: e.target.value })}
                onKeyDown={(e) => { if (e.key === "Enter") saveEditFact(f, editFact.v); if (e.key === "Escape") setEditFact(null); }} />
              <button className="act" title={t("Salva", "Save")} onClick={() => saveEditFact(f, editFact.v)}>✓</button>
              <button className="act" title={t("Annulla", "Cancel")} onClick={() => setEditFact(null)}>✕</button>
            </div>
          ) : (
            <div className="factrow" key={i}>
              <span className="facttext">• {f}</span>
              <button className="act" title={t("Modifica", "Edit")} onClick={() => setEditFact({ i, v: f })}>✎</button>
              <button className="act" title={t("Elimina", "Delete")} onClick={() => deleteFact(f)}>🗑️</button>
            </div>
          ))}
          <div className="addfact">
            <input placeholder={t("Aggiungi un dettaglio…", "Add a detail…")} value={newFact}
              onChange={(e) => setNewFact(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") addManualFact(); }} />
            <button className="send-sm" onClick={addManualFact}>+</button>
          </div>
          {facts.length > 0 && <button className="ghost forget" onClick={forgetFacts}>{t("Dimentica i dettagli appresi", "Forget the learned details")}</button>}
        </div>
      </div>
    </div>
  );
}
