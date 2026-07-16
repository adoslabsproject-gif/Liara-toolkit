// Sottosistema EMAIL: stato + handler + polling, incapsulati in un hook. Prima erano 14 useState e
// 16 funzioni dentro App(); ora vivono qui e il drawer (EmailDrawer.tsx) riceve l'oggetto ritornato.
// Comportamento IDENTICO all'originale (estrazione verbatim) — refactor a comportamento invariato.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { haptic } from "./audio";
import { EMAIL_PROVIDERS, PROVIDER_HELP } from "./constants";

export type Mail = { id: number; sender: string; subject: string; date: string; seen: boolean };
export type OpenMail = { id: number; sender: string; subject: string; date: string; body: string };
export type Compose = { to: string; subject: string; body: string };

export function useEmail() {
  const [showEmail, setShowEmail] = useState(false);
  const [showCfg, setShowCfg] = useState(false);
  const [emailCfg, setEmailCfg] = useState<Record<string, string>>({});
  const [emails, setEmails] = useState<Mail[]>([]);
  const [openMail, setOpenMail] = useState<OpenMail | null>(null);
  const [mailStatus, setMailStatus] = useState("");
  const [mailLoading, setMailLoading] = useState(false); // true durante lo scarico IMAP → spinner nel drawer
  const [provider, setProvider] = useState("");
  const [unread, setUnread] = useState(0);
  const [folder, setFolder] = useState("INBOX");
  const [showPw, setShowPw] = useState(false);
  const [mailHelp, setMailHelp] = useState(false);
  const [compose, setCompose] = useState<Compose | null>(null);
  const [sendStatus, setSendStatus] = useState("");
  const [cfgMsg, setCfgMsg] = useState("");

  async function refreshEmails(f = folder) {
    const list = await invoke<Mail[]>("email_list_folder", { folder: f });
    setEmails(list);
    // unread badge always reflects INBOX
    const inbox = f === "INBOX" ? list : await invoke<Mail[]>("email_list_folder", { folder: "INBOX" });
    setUnread(inbox.filter((m) => !m.seen).length);
  }
  async function openEmail() {
    const cfg = await invoke<Record<string, string>>("email_get_config");
    // auto-fill SMTP for known providers if missing (configs saved before SMTP existed)
    if (cfg.imap_host && !cfg.smtp_host) {
      const match = Object.values(EMAIL_PROVIDERS).find((p) => p.imap_host === cfg.imap_host);
      if (match) {
        cfg.smtp_host = match.smtp_host;
        cfg.smtp_port = match.smtp_port;
        await invoke("email_set_config", { config: cfg });
      }
    }
    setEmailCfg(cfg);
    await refreshEmails();
    setOpenMail(null); setMailStatus(""); setShowCfg(false); setShowEmail(true);
  }
  const setCfg = (k: string, v: string) => setEmailCfg((c) => ({ ...c, [k]: v }));
  function pickProvider(name: string) {
    setProvider(name);
    const p = EMAIL_PROVIDERS[name];
    if (p) setEmailCfg((c) => ({ ...c, imap_host: p.imap_host, imap_port: p.imap_port, smtp_host: p.smtp_host, smtp_port: p.smtp_port }));
    if (PROVIDER_HELP[name]) setMailHelp(true); // show the provider-specific sign-in help upfront
  }
  async function switchFolder(f: string) {
    setFolder(f);
    setOpenMail(null);
    haptic(12);
    await refreshEmails(f);
  }
  async function restoreMail(id: number) {
    await invoke("email_restore", { id });
    await refreshEmails();
    haptic(20);
  }
  async function purgeMail(id: number) {
    await invoke("email_purge", { id });
    await refreshEmails();
  }
  async function emptyTrash() {
    await invoke("email_purge", { id: 0 });
    await refreshEmails();
    haptic([20, 40, 20]);
  }
  async function saveCfg() {
    await invoke("email_set_config", { config: emailCfg });
    haptic(40);
    setCfgMsg(t("✓ Configurazione salvata", "✓ Configuration saved"));
    setTimeout(() => setCfgMsg(""), 2200);
  }
  async function fetchEmails() {
    setMailStatus(t("Salvo la configurazione e scarico…", "Saving the configuration and downloading…"));
    setMailHelp(false);
    setMailLoading(true); // avvia lo spinner: lo scarico IMAP dura secondi, altrimenti sembra fermo
    try {
      await invoke("email_set_config", { config: emailCfg }); // usa sempre i valori del form
      const n = await invoke<number>("email_fetch");
      setMailStatus(n > 0 ? t(`${n} nuove email scaricate.`, `${n} new emails downloaded.`) : t("Nessuna nuova email.", "No new emails."));
      await refreshEmails();
    } catch (e) {
      const msg = String(e);
      if (/password required|application-specific|185833|AUTHENTICATIONFAILED|Invalid credentials|login fail/i.test(msg)) {
        setMailStatus(t("⚠️ Credenziali rifiutate da Gmail: serve una password per le app, non quella normale.", "⚠️ Gmail rejected the credentials: you need an app password, not your normal one."));
        setMailHelp(true);
      } else {
        setMailStatus(t("Errore: ", "Error: ") + msg);
      }
    } finally {
      setMailLoading(false); // ferma lo spinner sia in successo che in errore
    }
  }
  async function readMail(id: number) {
    setOpenMail(await invoke("email_get", { id }));
    await refreshEmails();
  }
  async function delMail(id: number, e: React.MouseEvent) {
    e.stopPropagation();
    await invoke("email_delete", { id });
    await refreshEmails();
    if (openMail?.id === id) setOpenMail(null);
  }
  function startCompose() { setCompose({ to: "", subject: "", body: "" }); setSendStatus(""); }
  function startReply() {
    if (!openMail) return;
    const m = openMail.sender.match(/<([^>]+)>/);
    const to = (m ? m[1] : openMail.sender).trim();
    const subj = /^re:/i.test(openMail.subject) ? openMail.subject : `Re: ${openMail.subject}`;
    setCompose({ to, subject: subj, body: "" });
    setSendStatus("");
  }
  async function sendCompose() {
    if (!compose) return;
    if (!compose.to.trim()) { setSendStatus(t("Inserisci un destinatario.", "Enter a recipient.")); return; }
    setSendStatus(t("Invio in corso…", "Sending…"));
    try {
      await invoke("email_send", { to: compose.to.trim(), subject: compose.subject, body: compose.body });
      haptic([30, 50, 30]);
      setSendStatus(t("✓ Inviata!", "✓ Sent!"));
      setTimeout(() => { setCompose(null); setSendStatus(""); }, 1200);
    } catch (e) {
      const msg = String(e);
      if (/SMTP/i.test(msg)) setSendStatus(t("⚠️ Manca il server SMTP. Apri ⚙️ Configurazione, scegli il provider (compila SMTP) e premi Salva.", "⚠️ The SMTP server is missing. Open ⚙️ Configuration, pick your provider (it fills in SMTP) and press Save."));
      else setSendStatus(t("Errore: ", "Error: ") + msg);
    }
  }

  // polling: ogni 3 minuti scarica le nuove (se configurata). Comportamento verbatim dall'originale.
  useEffect(() => {
    const tick = async () => {
      try {
        const cfg = await invoke<Record<string, string>>("email_get_config");
        if (cfg.imap_host && cfg.email && cfg.password) {
          await invoke("email_fetch");
          await refreshEmails();
        }
      } catch { /* offline o non configurata */ }
    };
    tick();
    const id = setInterval(tick, 180000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return {
    showEmail, setShowEmail, showCfg, setShowCfg, emailCfg, emails, openMail, setOpenMail,
    mailStatus, mailLoading, provider, unread, folder, showPw, setShowPw, mailHelp, compose, setCompose, sendStatus, cfgMsg,
    setCfg, openEmail, pickProvider, refreshEmails, switchFolder, restoreMail, purgeMail, emptyTrash,
    saveCfg, fetchEmails, readMail, delMail, startCompose, startReply, sendCompose,
  };
}

export type EmailApi = ReturnType<typeof useEmail>;
