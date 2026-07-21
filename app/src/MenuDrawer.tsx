// Drawer Menu (hub delle impostazioni): profilo, email, agenda, permessi, tema, modello, voce
// automatica, ragionamento, cloud e consenso al salvataggio anonimo. Presentazionale: riceve
// callback e valori, nessuna logica propria.
import { t } from "./i18n";
import { MENU_ICONS } from "./constants";

export function MenuDrawer(props: {
  theme: string;
  emailUnread: number;
  modelTag: string;
  autoSpeak: boolean;
  thinking: boolean;
  cloud: boolean;
  trainConsent: boolean;
  onClose: () => void;
  onProfile: () => void;
  onEmail: () => void;
  onAgenda: () => void;
  isAndroid: boolean;
  onContacts: () => void;
  onSms: () => void;
  onPerms: () => void;
  onTheme: () => void;
  onModel: () => void;
  onToggleVoice: () => void;
  voiceSid: number;
  onVoice: () => void;
  respLen: string;
  onRespLen: () => void;
  onToggleThinking: () => void;
  onToggleCloud: () => void;
  onToggleTrain: () => void;
  onNet: () => void;
  chatNotif: number;
}) {
  const mi = MENU_ICONS[props.theme] || MENU_ICONS[""];
  return (
    <div className="drawer-overlay" onClick={props.onClose}>
      <div className="drawer menudrawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><h2>{mi.theme} {t("Menu", "Menu")}</h2><button className="ghost" onClick={props.onClose}>✕</button></div>
        <button className="menurow" onClick={props.onProfile}><span className="menuico">{mi.profile}</span><span className="menuname">{t("Su di me", "About me")}</span></button>
        <button className="menurow" onClick={props.onEmail}><span className="menuico">{mi.email}</span><span className="menuname">{t("Email", "Email")}</span>{props.emailUnread > 0 && <span className="badge">{props.emailUnread > 9 ? "9+" : props.emailUnread}</span>}</button>
        <button className="menurow" onClick={props.onAgenda}><span className="menuico">{mi.agenda}</span><span className="menuname">{t("Agenda", "Calendar")}</span></button>
        {/* Rubrica e SMS: SOLO su Android (desktop/Mac non hanno rubrica né SMS di sistema). */}
        {props.isAndroid && <button className="menurow" onClick={props.onContacts}><span className="menuico">👥</span><span className="menuname">{t("Rubrica", "Contacts")}</span></button>}
        {props.isAndroid && <button className="menurow" onClick={props.onSms}><span className="menuico">📩</span><span className="menuname">{t("Messaggi SMS", "SMS messages")}</span></button>}
        <button className="menurow" onClick={props.onPerms}><span className="menuico">{mi.perms}</span><span className="menuname">{t("Permessi", "Permissions")}</span></button>
        <button className="menurow" onClick={props.onTheme}><span className="menuico">{mi.theme}</span><span className="menuname">{t("Tema", "Theme")}</span></button>
        <button className="menurow" onClick={props.onModel}><span className="menuico">🧠</span><span className="menuname">{t("Modello AI", "AI model")}</span><span className="menutag">{props.modelTag}</span></button>
        <button className="menurow" onClick={props.onToggleVoice}><span className="menuico">{props.autoSpeak ? mi.voiceOn : mi.voiceOff}</span><span className="menuname">{t("Voce automatica", "Auto voice")}</span><span className="menutag">{props.autoSpeak ? "ON" : "OFF"}</span></button>
        <button className="menurow" onClick={props.onVoice}><span className="menuico">🗣️</span><span className="menuname">{t("Voce di Liara", "Liara's voice")}</span><span className="menutag">{props.voiceSid === 36 ? t("Nicola (M)", "Nicola (M)") : t("Sara (F)", "Sara (F)")}</span></button>
        <button className="menurow" onClick={props.onRespLen}><span className="menuico">📏</span><span className="menuname">{t("Lunghezza risposte", "Response length")}</span><span className="menutag">{{ breve: t("Breve", "Short"), media: t("Media", "Medium"), lunga: t("Lunga", "Long"), massima: t("Massima", "Max") }[props.respLen] || props.respLen}</span></button>
        <button className="menurow" onClick={props.onToggleThinking}><span className="menuico">💭</span><span className="menuname">{t("Ragionamento", "Reasoning")}</span><span className="menutag">{props.thinking ? t("Attivo", "On") : t("Spento", "Off")}</span></button>
        <button className="menurow" onClick={props.onToggleCloud}><span className="menuico">☁️</span><span className="menuname">{t("Liara Cloud (24B)", "Liara Cloud (24B)")}</span><span className="menutag">{props.cloud ? "ON" : "OFF"}</span></button>
        {/* Consenso revocabile al salvataggio anonimo delle conversazioni (vale solo in cloud). OFF di default. */}
        <button className="menurow" onClick={props.onToggleTrain}><span className="menuico">🔬</span><span className="menuname">{t("Migliora Liara (anonimo)", "Improve Liara (anon.)")}</span><span className="menutag">{props.trainConsent ? "ON" : "OFF"}</span></button>
        <button className="menurow" onClick={props.onNet}><span className="menuico">💬</span><span className="menuname">{t("Liara Chat", "Liara Chat")}</span>{props.chatNotif > 0 && <span className="badge">{props.chatNotif > 9 ? "9+" : props.chatNotif}</span>}</button>
      </div>
    </div>
  );
}
