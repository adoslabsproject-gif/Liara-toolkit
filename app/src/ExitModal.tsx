// Modale "vuoi uscire?" (tasto indietro Android nella schermata chat).
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { haptic } from "./audio";

export function ExitModal({ onStay }: { onStay: () => void }) {
  return (
    <div className="modal-overlay">
      <div className="consent">
        <div className="consent-icon">👋</div>
        <h3>{t("Vuoi uscire da Liara?", "Leave Liara?")}</h3>
        <p className="consent-action">{t("L'app si chiuderà completamente.", "The app will close completely.")}</p>
        <div className="consent-btns">
          <button className="ghost" onClick={() => { onStay(); haptic(12); }}>{t("Resta", "Stay")}</button>
          <button className="send-sm" onClick={() => { haptic(20); invoke("exit_app"); }}>{t("Sì, esci", "Yes, exit")}</button>
        </div>
      </div>
    </div>
  );
}
