// Drawer Tema: griglia di temi selezionabili.
import { t } from "./i18n";
import { THEMES } from "./constants";
import { haptic } from "./audio";

export function ThemeDrawer({ theme, setTheme, onBack, onClose }: {
  theme: string; setTheme: (id: string) => void; onBack: () => void; onClose: () => void;
}) {
  return (
    <div className="drawer-overlay" onClick={onClose}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><button className="ghost back" onClick={onBack}>←</button><h2>🎨 {t("Tema", "Theme")}</h2><button className="ghost" onClick={onClose}>✕</button></div>
        <div className="themegrid">
          {THEMES.map((th) => (
            <button key={th.id} className={`themecard ${theme === th.id ? "active" : ""}`} onClick={() => { setTheme(th.id); haptic(20); }}>
              <span className="themeswatch" style={{ background: th.c }} />
              <span className="themename">{t(th.name, th.en)}</span>
              {theme === th.id && <span className="themecheck">✓</span>}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
