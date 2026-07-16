// PeerHub — la chat Liara↔Liara completa: rubrica dei QR accettati, mostra/scansiona QR, e il chatter
// E2E per ogni contatto. La CRITTOGRAFIA è in Rust (peer_seal/open); qui c'è solo UI + trasporto (peer.ts).
//
// NB: su Android la rubrica sarà quella NATIVA del telefono (Fase A, bridge contatti) e il QR vi si
// innesta sopra; per ora e su desktop la rubrica è la lista dei QR accettati (peer_list), stesso flusso.
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import QRCode from "qrcode";
import jsQR from "jsqr";
import { t } from "./i18n";
import {
  Contact, ChatMsg, NetStatus, PeerRequest, Task, getStatus, connect,
  subscribe, sendTo, sendDoc, loadHistory, unreadCount, setActiveChat, clearHistory,
  sendInvite, pendingRequests, acceptRequest, rejectRequest,
  getTask, setTask, sendTask, materialsText,
} from "./peer";

const DOC_MAX = 120 * 1024; // cap del testo condiviso: sta sotto il limite 256KB/messaggio del relay

const QR_PREFIX = "liara:"; // marchia i nostri QR → non confondiamo un QR qualsiasi con un ID Liara

export function PeerHub({ onClose }: { onClose: () => void }) {
  const [myId, setMyId] = useState("");
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [status, setStatus] = useState<NetStatus>(getStatus());
  const [view, setView] = useState<"list" | "qr" | "scan" | "paste" | "chat">("list");
  const [chatWith, setChatWith] = useState<Contact | null>(null);
  const [, force] = useState(0);

  const refresh = () => invoke<Contact[]>("peer_list").then(setContacts).catch(() => {});
  useEffect(() => {
    invoke<string>("peer_identity").then(setMyId).catch(() => {});
    refresh();
    connect();
    return subscribe(() => { setStatus(getStatus()); force((n) => n + 1); });
  }, []);

  const openChat = (c: Contact) => { setChatWith(c); setActiveChat(c.id); setView("chat"); };
  const backToList = () => { setActiveChat(null); setChatWith(null); setView("list"); refresh(); };

  return (
    <div className="drawer-overlay" onClick={onClose}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        {view === "chat" && chatWith ? (
          <ChatView contact={chatWith} onBack={backToList} />
        ) : view === "qr" ? (
          <MyQR myId={myId} onBack={() => setView("list")} />
        ) : view === "scan" ? (
          <Scanner onBack={() => setView("list")} onAdded={() => { refresh(); setView("list"); }} />
        ) : view === "paste" ? (
          <PasteAdd onBack={() => setView("list")} onAdded={() => { refresh(); setView("list"); }} />
        ) : (
          <>
            <div className="drawer-head">
              <h2>💬 {t("Liara Chat", "Liara Chat")} · {status === "on" ? "🟢" : status === "connecting" ? "🟡" : "⚪️"}</h2>
              <button className="ghost" onClick={onClose}>✕</button>
            </div>

            <div className="peer-actions">
              <button className="peer-act" onClick={() => setView("qr")}>🔳 {t("Il mio QR", "My QR")}</button>
              <button className="peer-act" onClick={() => setView("scan")}>📷 {t("Scansiona", "Scan")}</button>
              <button className="peer-act" onClick={() => setView("paste")}>📋 {t("Incolla ID", "Paste ID")}</button>
            </div>

            {pendingRequests().length > 0 && (
              <div className="pgroup">
                <h3>🔔 {t("Richieste", "Requests")}</h3>
                {pendingRequests().map((r) => (
                  <RequestRow key={r.id} req={r} onDone={refresh} />
                ))}
              </div>
            )}

            <div className="pgroup">
              <h3>{t("Rubrica", "Contacts")}</h3>
              {contacts.length === 0 && <p className="hint">{t("Nessun contatto. Scansiona il QR di un altro Liara.", "No contacts. Scan another Liara's QR.")}</p>}
              {contacts.map((c) => (
                <ContactRow key={c.id} c={c} onOpen={openChat} onChanged={refresh} />
              ))}
            </div>

          </>
        )}
      </div>
    </div>
  );
}

// ---- Il mio QR -----------------------------------------------------------
function MyQR({ myId, onBack }: { myId: string; onBack: () => void }) {
  const [dataUrl, setDataUrl] = useState("");
  useEffect(() => {
    if (!myId) return;
    QRCode.toDataURL(QR_PREFIX + myId, { width: 260, margin: 1 }).then(setDataUrl).catch(() => {});
  }, [myId]);
  return (
    <>
      <div className="drawer-head"><h2>🔳 {t("Il mio QR", "My QR")}</h2><button className="ghost" onClick={onBack}>‹ {t("Indietro", "Back")}</button></div>
      <p className="hint">{t("Fai scansionare questo QR all'altra persona per collegare i vostri Liara.", "Have the other person scan this QR to connect your Liaras.")}</p>
      <div className="qr-wrap">{dataUrl ? <img src={dataUrl} alt="QR" width={260} height={260} /> : <span className="hint">…</span>}</div>
      <button className="peer-act" style={{ width: "100%" }} disabled={!dataUrl} onClick={async () => {
        // Condividi QR (immagine) + ID via share sheet di sistema (WhatsApp/Telegram/…). dataURL→File
        // senza fetch (evita la CSP connect-src). Fallback: copia l'ID.
        const text = t(`Aggiungimi su Liara! Apri Liara → Aggiungi → Incolla ID e incolla questo:\n${myId}`,
                       `Add me on Liara! Open Liara → Add → Paste ID and paste this:\n${myId}`);
        try {
          const b64 = dataUrl.split(",")[1] || "";
          const bin = atob(b64);
          const arr = new Uint8Array(bin.length);
          for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
          const file = new File([arr], "liara-qr.png", { type: "image/png" });
          const data: Record<string, unknown> = { title: "Liara", text };
          const nav = navigator as Navigator & { canShare?: (d: unknown) => boolean };
          if (nav.canShare?.({ files: [file] })) data.files = [file];
          await (navigator as Navigator & { share: (d: unknown) => Promise<void> }).share(data);
        } catch { navigator.clipboard?.writeText(myId).catch(() => {}); }
      }}>📤 {t("Condividi il mio QR / ID", "Share my QR / ID")}</button>
      <div className="pgroup">
        <h3>{t("Oppure copia l'ID", "Or copy the ID")}</h3>
        <div className="netid" onClick={() => navigator.clipboard?.writeText(myId).catch(() => {})} title={t("Tocca per copiare", "Tap to copy")}>{myId || "…"}</div>
      </div>
    </>
  );
}

// ---- Scanner QR (fotocamera + jsQR) --------------------------------------
function Scanner({ onBack, onAdded }: { onBack: () => void; onAdded: () => void }) {
  // Su Android la WebView gira con hardwareAccelerated=false (per evitare un crash) → il <video>
  // getUserMedia resta NERO. Quindi su Android si SCATTA una foto del QR e la si decodifica con jsQR;
  // su desktop resta il video live che funziona.
  const isAndroid = /android/i.test(navigator.userAgent);
  const videoRef = useRef<HTMLVideoElement>(null);
  const [err, setErr] = useState("");
  const [scanned, setScanned] = useState<string | null>(null);
  const [name, setName] = useState("");
  const streamRef = useRef<MediaStream | null>(null);
  const rafRef = useRef<number>(0);

  useEffect(() => {
    if (isAndroid) return; // niente video live su Android (nero): si usa la foto
    let cancelled = false;
    (async () => {
      try {
        // facingMode IDEAL (non exact): su alcuni WebView "environment" esatto fallisce → nero.
        const stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: { ideal: "environment" } }, audio: false });
        if (cancelled) { stream.getTracks().forEach((t) => t.stop()); return; }
        streamRef.current = stream;
        const v = videoRef.current;
        if (!v) return;
        v.srcObject = stream;
        v.setAttribute("playsinline", "true");
        v.muted = true;
        // play() può rifiutare per autoplay policy: riprova su loadedmetadata e ignora l'errore.
        v.onloadedmetadata = () => { v.play().catch(() => {}); };
        try { await v.play(); } catch { /* riprova via onloadedmetadata */ }
        const canvas = document.createElement("canvas");
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        const tick = () => {
          if (cancelled || !ctx || !videoRef.current) return;
          const vid = videoRef.current;
          if (vid.readyState === vid.HAVE_ENOUGH_DATA) {
            canvas.width = vid.videoWidth; canvas.height = vid.videoHeight;
            ctx.drawImage(vid, 0, 0, canvas.width, canvas.height);
            const img = ctx.getImageData(0, 0, canvas.width, canvas.height);
            const code = jsQR(img.data, img.width, img.height);
            if (code && code.data.startsWith(QR_PREFIX)) {
              setScanned(code.data.slice(QR_PREFIX.length).trim());
              stopCamera();
              return;
            }
          }
          rafRef.current = requestAnimationFrame(tick);
        };
        rafRef.current = requestAnimationFrame(tick);
      } catch (e) {
        setErr(t("Fotocamera non disponibile o permesso negato.", "Camera unavailable or permission denied.") + ` (${e})`);
      }
    })();
    const stopCamera = () => {
      cancelAnimationFrame(rafRef.current);
      streamRef.current?.getTracks().forEach((tr) => tr.stop());
      streamRef.current = null;
    };
    return () => { cancelled = true; stopCamera(); };
  }, []);

  const confirmAdd = async () => {
    if (!scanned) return;
    try {
      await invoke("peer_add", { id: scanned, name: name.trim(), added: Date.now() });
      await sendInvite(scanned); // manda la richiesta d'amicizia all'altro
      onAdded();
    } catch (e) { setErr(String(e)); }
  };

  return (
    <>
      <div className="drawer-head"><h2>📷 {t("Scansiona QR", "Scan QR")}</h2><button className="ghost" onClick={onBack}>‹ {t("Indietro", "Back")}</button></div>
      {err && <p className="hint err">{err}</p>}
      {!scanned ? (
        isAndroid ? (
          <div className="pgroup">
            <p className="hint">{t("Apri la fotocamera e inquadra il QR dell'altro Liara: lo riconosco al volo.", "Open the camera and point at the other Liara's QR: I'll recognize it instantly.")}</p>
            <button className="peer-act" onClick={async () => {
              setErr("");
              try {
                const text = await invoke<string>("scan_qr");
                const scannedId = extractId(text);
                if (scannedId) setScanned(scannedId);
                else setErr(t("QR non valido (non è un ID Liara).", "Invalid QR (not a Liara ID)."));
              } catch (e) { setErr(String(e)); }
            }}>📷 {t("Apri lettore QR", "Open QR scanner")}</button>
          </div>
        ) : (
          <>
            <p className="hint">{t("Inquadra il QR dell'altro Liara.", "Point at the other Liara's QR.")}</p>
            <div className="qr-wrap"><video ref={videoRef} autoPlay playsInline muted
              style={{ width: "100%", minHeight: 260, background: "#000", objectFit: "cover", borderRadius: 12 }} /></div>
          </>
        )
      ) : (
        <div className="pgroup">
          <p className="hint">{t("Trovato! Dai un nome a questo contatto:", "Found! Name this contact:")}</p>
          <div className="netid">{scanned.slice(0, 20)}…</div>
          <input autoFocus placeholder={t("Nome (es. Marco)", "Name (e.g. Marco)")} value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") confirmAdd(); }} />
          <button className="peer-act" onClick={async () => {
            try { const [n] = await invoke<[string, string]>("pick_contact"); if (n) setName(n); }
            catch { /* annullato o non su Android */ }
          }}>📇 {t("Scegli dai contatti del telefono", "Pick from phone contacts")}</button>
          <button className="send-sm" onClick={confirmAdd}>{t("Aggiungi alla rubrica", "Add to contacts")}</button>
        </div>
      )}
    </>
  );
}

// Estrae un ID Liara (chiave X25519 = 43 char base64url) da testo grezzo: gestisce l'ID nudo,
// "liara:<id>", o l'intero testo di condivisione che lo contiene.
function extractId(raw: string): string | null {
  if (!raw) return null;
  const s = raw.trim();
  const body = s.startsWith(QR_PREFIX) ? s.slice(QR_PREFIX.length) : s;
  const m = body.match(/[A-Za-z0-9_-]{43}/);
  return m ? m[0] : null;
}

// ---- Aggiungi incollando l'ID (per chi riceve il QR/ID via chat) ---------
function PasteAdd({ onBack, onAdded }: { onBack: () => void; onAdded: () => void }) {
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [err, setErr] = useState("");
  useEffect(() => {
    // prefill dall'appunti se contiene un ID (l'utente ha appena copiato quello condiviso)
    navigator.clipboard?.readText?.().then((txt) => { const m = extractId(txt); if (m) setId(m); }).catch(() => {});
  }, []);
  const add = async () => {
    const clean = extractId(id) || id.trim();
    if (!clean) { setErr(t("Incolla un ID valido.", "Paste a valid ID.")); return; }
    try {
      await invoke("peer_add", { id: clean, name: name.trim(), added: Date.now() });
      await sendInvite(clean); // manda la richiesta d'amicizia
      onAdded();
    } catch (e) { setErr(String(e)); }
  };
  return (
    <>
      <div className="drawer-head"><h2>📋 {t("Aggiungi con ID", "Add by ID")}</h2><button className="ghost" onClick={onBack}>‹ {t("Indietro", "Back")}</button></div>
      <p className="hint">{t("Incolla l'ID che ti ha condiviso l'altra persona (WhatsApp, Telegram…).", "Paste the ID the other person shared (WhatsApp, Telegram…).")}</p>
      {err && <p className="hint err">{err}</p>}
      <div className="pgroup">
        <input autoFocus placeholder={t("Incolla qui l'ID", "Paste the ID here")} value={id} onChange={(e) => setId(e.target.value)} />
        <input placeholder={t("Nome (es. Marco)", "Name (e.g. Marco)")} value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") add(); }} />
        <button className="send-sm" onClick={add} disabled={!id.trim()}>{t("Aggiungi alla rubrica", "Add to contacts")}</button>
      </div>
    </>
  );
}

// ---- Riga contatto in rubrica: apri chat, RINOMINA (inline), ELIMINA (doppio tap di conferma) --------
function ContactRow({ c, onOpen, onChanged }: { c: Contact; onOpen: (c: Contact) => void; onChanged: () => void }) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(c.name || "");
  const [confirmDel, setConfirmDel] = useState(false);
  const un = unreadCount(c.id);
  const save = () => invoke("peer_add", { id: c.id, name: name.trim(), added: c.added || Date.now() })
    .then(() => { setEditing(false); onChanged(); }).catch(() => {});
  const del = () => invoke("peer_remove", { id: c.id }).then(onChanged).catch(() => {});
  if (editing) {
    return (
      <div className="peer-req">
        <span className="peer-av">{(name || "?").slice(0, 1).toUpperCase()}</span>
        <input className="req-name" autoFocus value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") save(); }} />
        <button className="reqbtn ok" title={t("Salva", "Save")} onClick={save}>✓</button>
        <button className="reqbtn no" title={t("Annulla", "Cancel")} onClick={() => setEditing(false)}>✕</button>
      </div>
    );
  }
  return (
    <div className="peer-row-wrap">
      <button className="peer-row" onClick={() => onOpen(c)}>
        <span className="peer-av">{(c.name || "?").slice(0, 1).toUpperCase()}</span>
        <span className="peer-name">{c.name || c.id.slice(0, 12) + "…"}</span>
        {un > 0 && <span className="peer-badge">{un}</span>}
      </button>
      <button className="peer-ico" title={t("Rinomina", "Rename")} onClick={() => { setName(c.name || ""); setEditing(true); }}>✏️</button>
      {confirmDel
        ? <button className="peer-ico del" title={t("Tocca di nuovo per eliminare", "Tap again to delete")} onClick={del}>⚠️</button>
        : <button className="peer-ico" title={t("Elimina", "Delete")} onClick={() => setConfirmDel(true)}>🗑</button>}
    </div>
  );
}

// ---- Riga richiesta d'amicizia: scegli TU il nome da dare in rubrica, poi accetti --------
function RequestRow({ req, onDone }: { req: PeerRequest; onDone: () => void }) {
  const [name, setName] = useState(req.name || "");
  const [busy, setBusy] = useState(false);
  return (
    <div className="peer-req">
      <span className="peer-av">{(name || "?").slice(0, 1).toUpperCase()}</span>
      <input className="req-name" placeholder={t("Nome da dare in rubrica", "Name for contacts")}
        value={name} onChange={(e) => setName(e.target.value)} />
      <button className="reqbtn ok" title={t("Accetta", "Accept")} disabled={busy}
        onClick={async () => { setBusy(true); await acceptRequest(req.id, name); onDone(); }}>✓</button>
      <button className="reqbtn no" title={t("Rifiuta", "Reject")}
        onClick={() => { rejectRequest(req.id); onDone(); }}>✕</button>
    </div>
  );
}

// ---- Chat 1:1 ------------------------------------------------------------
function ChatView({ contact, onBack }: { contact: Contact; onBack: () => void }) {
  const [msgs, setMsgs] = useState<ChatMsg[]>([]);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);

  const reload = () => loadHistory(contact.id).then(setMsgs).catch(() => {});
  useEffect(() => {
    reload();
    setActiveChat(contact.id);
    return subscribe(reload); // nuovo messaggio in arrivo → ricarica
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contact.id]);
  useEffect(() => { endRef.current?.scrollIntoView({ behavior: "smooth" }); }, [msgs]);

  // ── COORDINAZIONE AI (task-driven): i due Liara lavorano insieme a un OBIETTIVO coi MATERIALI. ──
  const [aiMode, setAiMode] = useState(false);
  const [aiThinking, setAiThinking] = useState(false);
  const [showTask, setShowTask] = useState(false);
  const [task, setTaskState] = useState<Task>(() => getTask(contact.id));
  const replyingRef = useRef(false);
  const aiHistory = (m: ChatMsg[]) => m.filter((x) => !x.doc).map((x) => [x.dir === "me" ? "me" : "peer", x.text] as [string, string]);
  // ogni risposta del mio Liara è guidata dall'obiettivo + materiali del task
  const genReply = (history: [string, string][]) =>
    invoke<string>("liara_reply", { history, goal: task.goal || null, materials: materialsText(task) || null });
  useEffect(() => {
    if (!aiMode || replyingRef.current) return;
    if (msgs.length === 0) return;
    if (msgs[msgs.length - 1].dir !== "peer") return; // rispondo solo a un messaggio dell'ALTRO Liara
    if (aiHistory(msgs).length > 40) return; // cap anti-runaway
    replyingRef.current = true; setAiThinking(true);
    genReply(aiHistory(msgs))
      .then((reply) => sendTo(contact.id, reply))
      .then(() => reload())
      .catch(() => {})
      .finally(() => { replyingRef.current = false; setAiThinking(false); });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [msgs, aiMode]);
  // Avvia la coordinazione: salva il task, lo sincronizza col peer, attiva l'AI e fa partire il mio Liara.
  const startCoord = async () => {
    setTask(contact.id, task);
    await sendTask(contact.id, task);
    setShowTask(false); setAiMode(true);
    if (replyingRef.current) return;
    replyingRef.current = true; setAiThinking(true);
    try {
      const opener: [string, string][] = [["peer", task.goal
        ? `Ciao! Sono il Liara di un'altra persona. Coordiniamoci su questo obiettivo: ${task.goal}`
        : "Ciao! Sono il Liara di un'altra persona: presentati e conosciamoci."]];
      const reply = await genReply(opener);
      await sendTo(contact.id, reply); reload();
    } catch { /* */ } finally { replyingRef.current = false; setAiThinking(false); }
  };
  // gestione materiali della galleria
  const addDocMaterial = (file: File) => {
    // i PDF non sono leggibili come testo dal browser: passano dall'estrattore Rust (stesso percorso della RAG)
    const isPdf = /\.pdf$/i.test(file.name);
    const r = new FileReader();
    r.onload = async () => {
      let content = String(r.result || "");
      if (isPdf) {
        try {
          content = await invoke<string>("extract_doc_text", { name: file.name, data: content });
        } catch {
          content = ""; // PDF senza testo estraibile (es. scansione): non allegare spazzatura binaria
        }
      }
      content = content.slice(0, DOC_MAX);
      if (!content.trim()) return;
      setTaskState((t) => ({ ...t, materials: [...t.materials, { kind: "doc", name: file.name, content }] }));
    };
    if (isPdf) r.readAsDataURL(file); else r.readAsText(file);
  };
  const addPhotoMaterial = (file: File) => {
    const r = new FileReader();
    r.onload = () => setTaskState((t) => ({ ...t, materials: [...t.materials, { kind: "photo", name: file.name, content: String(r.result || "") }] }));
    r.readAsDataURL(file);
  };
  const removeMaterial = (i: number) => setTaskState((t) => ({ ...t, materials: t.materials.filter((_, k) => k !== i) }));
  const taskDocRef = useRef<HTMLInputElement>(null);
  const taskPhotoRef = useRef<HTMLInputElement>(null);

  const send = async () => {
    const body = text.trim();
    if (!body || sending) return;
    setSending(true);
    const ok = await sendTo(contact.id, body);
    setSending(false);
    if (ok) { setText(""); reload(); }
  };

  const wipe = () => { clearHistory(contact.id); setMsgs([]); };

  const docRef = useRef<HTMLInputElement>(null);
  const [ingested, setIngested] = useState<Record<string, "busy" | "done">>({});
  const [note, setNote] = useState("");

  // Allega un documento: leggi il testo, verifica il tetto, invialo E2E → il Liara dell'altro lo leggerà.
  const attachDoc = (file: File) => {
    if (file.size > DOC_MAX) { setNote(t("Documento troppo grande (max ~120KB di testo).", "Document too big (max ~120KB text).")); return; }
    const reader = new FileReader();
    reader.onload = async () => {
      const txt = String(reader.result || "").slice(0, DOC_MAX);
      if (!txt.trim()) { setNote(t("Documento vuoto o non testuale.", "Empty or non-text document.")); return; }
      setSending(true);
      const ok = await sendDoc(contact.id, file.name, txt);
      setSending(false);
      if (ok) { setNote(""); reload(); } else setNote(t("Invio non riuscito.", "Send failed."));
    };
    reader.readAsText(file);
  };

  // Ricevuto un documento → ingest nella RAG locale col MODELLO: da qui il mio Liara può discuterne.
  const ingest = async (key: string, name: string, text: string) => {
    setIngested((s) => ({ ...s, [key]: "busy" }));
    try {
      await invoke<number>("ingest_document", { name, text });
      setIngested((s) => ({ ...s, [key]: "done" }));
    } catch { setIngested((s) => { const c = { ...s }; delete c[key]; return c; }); setNote(t("Lettura non riuscita.", "Reading failed.")); }
  };

  return (
    <>
      <div className="drawer-head">
        <button className="ghost" onClick={onBack}>‹</button>
        <h2 style={{ flex: 1, textAlign: "center" }}>{contact.name || contact.id.slice(0, 12) + "…"}</h2>
        <button className={`ghost ${aiMode || showTask ? "aion" : ""}`} title={t("Fai coordinare i Liara su un obiettivo", "Let the Liaras coordinate on a goal")} onClick={() => setShowTask((v) => !v)}>🤖</button>
        <button className="ghost" title={t("Svuota chat", "Clear chat")} onClick={wipe}>🗑</button>
      </div>
      {showTask && (
        <div className="task-panel">
          <p className="hint">🎯 {t("Dai un OBIETTIVO ai vostri Liara e allega i materiali: si coordineranno da soli.", "Give your Liaras a GOAL and attach materials: they'll coordinate on their own.")}</p>
          <textarea className="task-goal" rows={2} placeholder={t("Obiettivo (es. organizzare una riunione la prossima settimana)", "Goal (e.g. schedule a meeting next week)")}
            value={task.goal} onChange={(e) => setTaskState((tk) => ({ ...tk, goal: e.target.value }))} />
          {task.materials.length > 0 && (
            <div className="task-materials">
              {task.materials.map((m, i) => (
                <span key={i} className="material-chip">{m.kind === "photo" ? "🖼" : "📄"} {m.name.length > 16 ? m.name.slice(0, 16) + "…" : m.name}<button onClick={() => removeMaterial(i)}>✕</button></span>
              ))}
            </div>
          )}
          <div className="peer-actions">
            <button className="peer-act" onClick={() => taskDocRef.current?.click()}>📄 {t("Documento", "Document")}</button>
            <button className="peer-act" onClick={() => taskPhotoRef.current?.click()}>🖼 {t("Foto", "Photo")}</button>
          </div>
          <input ref={taskDocRef} type="file" accept=".txt,.md,.csv,.json,.log,.rs,.py,.js,.ts,.html,.xml,.yaml,.yml" style={{ display: "none" }}
            onChange={(e) => { const f = e.target.files?.[0]; if (f) addDocMaterial(f); e.currentTarget.value = ""; }} />
          <input ref={taskPhotoRef} type="file" accept="image/*" style={{ display: "none" }}
            onChange={(e) => { const f = e.target.files?.[0]; if (f) addPhotoMaterial(f); e.currentTarget.value = ""; }} />
          <button className="send-sm" onClick={startCoord}>🚀 {t("Avvia coordinamento", "Start coordination")}</button>
        </div>
      )}
      {aiMode && !showTask && (
        <div className="ai-banner">
          🤖 {aiThinking ? t("il tuo Liara sta scrivendo…", "your Liara is typing…") : t("Coordinazione AI attiva: i Liara lavorano all'obiettivo.", "AI coordination on: the Liaras are working on the goal.")}
          <button className="ghost aistop" onClick={() => setAiMode(false)}>{t("Ferma", "Stop")}</button>
        </div>
      )}
      <div className="chat-scroll">
        {msgs.length === 0 && <p className="hint">🔒 {t("Chat cifrata end-to-end. Scrivi il primo messaggio.", "End-to-end encrypted chat. Send the first message.")}</p>}
        {msgs.map((m, i) => {
          if (m.doc) {
            const key = `${i}-${m.ts}`;
            const st = ingested[key];
            return (
              <div key={i} className={`bubble ${m.dir} docbub`}>
                <div className="docname">📄 {m.doc.name}</div>
                {m.dir === "peer" && (
                  st === "done"
                    ? <div className="dochint">✓ {t("Liara l'ha letto", "Liara read it")}</div>
                    : <button className="docbtn" disabled={st === "busy"} onClick={() => ingest(key, m.doc!.name, m.doc!.text)}>
                        {st === "busy" ? t("Leggo…", "Reading…") : `🧠 ${t("Fallo leggere a Liara", "Let Liara read it")}`}
                      </button>
                )}
                {m.dir === "me" && <div className="dochint">{t("inviato", "sent")}</div>}
              </div>
            );
          }
          return <div key={i} className={`bubble ${m.dir}`}>{m.text}</div>;
        })}
        <div ref={endRef} />
      </div>
      {note && <p className="hint err" style={{ padding: "0 4px" }}>{note}</p>}
      <div className="peer-composer">
        <button className="ctool" title={t("Allega documento", "Attach document")} onClick={() => docRef.current?.click()} disabled={sending}>📎</button>
        <input ref={docRef} type="file" accept=".txt,.md,.csv,.json,.log,.rs,.py,.js,.ts,.html,.xml,.yaml,.yml" style={{ display: "none" }}
          onChange={(e) => { const f = e.target.files?.[0]; if (f) attachDoc(f); e.currentTarget.value = ""; }} />
        <input className="peer-input" placeholder={t("Messaggio…", "Message…")} value={text} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") send(); }} />
        <button className="csend" onClick={send} disabled={!text.trim() || sending}>➤</button>
      </div>
    </>
  );
}
