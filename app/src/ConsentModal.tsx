// Modale di consenso: quando l'agente vuole usare uno strumento sensibile, l'utente sceglie
// Nega / Solo stavolta / Consenti sempre. Il backend attende la risposta (ConsentGate).
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { haptic } from "./audio";

export function ConsentModal({ req, onClose }: { req: { tool: string; action: string }; onClose: () => void }) {
  const respond = (allow: boolean, remember: boolean, h: number | number[]) => {
    invoke("consent_respond", { allow, remember, tool: req.tool });
    onClose();
    haptic(h);
  };
  return (
    <div className="modal-overlay">
      <div className="consent">
        <div className="consent-icon">🔐</div>
        <h3>{t("Liara chiede il permesso", "Liara is asking for permission")}</h3>
        <p className="consent-action">{t("Vuole ", "Wants to ")}<b>{req.action}</b></p>
        <div className="consent-btns">
          <button className="ghost" onClick={() => respond(false, false, 15)}>{t("Nega", "Deny")}</button>
          <button className="send-sm alt" onClick={() => respond(true, false, 20)}>{t("Solo stavolta", "Just this once")}</button>
          <button className="send-sm" onClick={() => respond(true, true, 20)}>{t("Consenti sempre", "Always allow")}</button>
        </div>
      </div>
    </div>
  );
}
