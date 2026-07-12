// Drawer Menu (hub delle impostazioni): profilo, email, agenda, permessi, tema, modello, voce
// automatica, ragionamento. Presentazionale: riceve callback e valori, nessuna logica propria.
import { t } from "./i18n";
import { MENU_ICONS } from "./constants";

export function MenuDrawer(props: {
  theme: string;
  emailUnread: number;
  modelTag: string;
  autoSpeak: boolean;
  thinking: boolean;
  cloud: boolean;
  onClose: () => void;
  onProfile: () => void;
  onEmail: () => void;
  onAgenda: () => void;
  onPerms: () => void;
  onTheme: () => void;
  onModel: () => void;
  onToggleVoice: () => void;
  onToggleThinking: () => void;
  onToggleCloud: () => void;
}) {
  const mi = MENU_ICONS[props.theme] || MENU_ICONS[""];
  return (
    <div className="drawer-overlay" onClick={props.onClose}>
      <div className="drawer menudrawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><h2>{mi.theme} {t("Menu", "Menu")}</h2><button className="ghost" onClick={props.onClose}>✕</button></div>
        <button className="menurow" onClick={props.onProfile}><span className="menuico">{mi.profile}</span><span className="menuname">{t("Su di me", "About me")}</span></button>
        <button className="menurow" onClick={props.onEmail}><span className="menuico">{mi.email}</span><span className="menuname">{t("Email", "Email")}</span>{props.emailUnread > 0 && <span className="badge">{props.emailUnread > 9 ? "9+" : props.emailUnread}</span>}</button>
        <button className="menurow" onClick={props.onAgenda}><span className="menuico">{mi.agenda}</span><span className="menuname">{t("Agenda", "Calendar")}</span></button>
        <button className="menurow" onClick={props.onPerms}><span className="menuico">{mi.perms}</span><span className="menuname">{t("Permessi", "Permissions")}</span></button>
        <button className="menurow" onClick={props.onTheme}><span className="menuico">{mi.theme}</span><span className="menuname">{t("Tema", "Theme")}</span></button>
        <button className="menurow" onClick={props.onModel}><span className="menuico">🧠</span><span className="menuname">{t("Modello AI", "AI model")}</span><span className="menutag">{props.modelTag}</span></button>
        <button className="menurow" onClick={props.onToggleVoice}><span className="menuico">{props.autoSpeak ? mi.voiceOn : mi.voiceOff}</span><span className="menuname">{t("Voce automatica", "Auto voice")}</span><span className="menutag">{props.autoSpeak ? "ON" : "OFF"}</span></button>
        <button className="menurow" onClick={props.onToggleThinking}><span className="menuico">💭</span><span className="menuname">{t("Ragionamento", "Reasoning")}</span><span className="menutag">{props.thinking ? t("Attivo", "On") : t("Spento", "Off")}</span></button>
        <button className="menurow" onClick={props.onToggleCloud}><span className="menuico">☁️</span><span className="menuname">{t("Liara Cloud (32B)", "Liara Cloud (32B)")}</span><span className="menutag">{props.cloud ? "ON" : "OFF"}</span></button>
      </div>
    </div>
  );
}
