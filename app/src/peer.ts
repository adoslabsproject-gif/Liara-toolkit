// Trasporto della chat peer (Liara↔Liara), lato frontend.
//
// - CRITTOGRAFIA in Rust: `peer_seal`/`peer_open` (la chiave privata non lascia mai il core).
// - TRASPORTO qui: una singola connessione WebSocket al relay, VIVA a livello App (non solo quando il
//   drawer è aperto) → i messaggi arrivano anche mentre navighi, con badge "non letti".
// - STORICO cifrato a riposo GRATIS: salviamo il CIPHERTEXT (già E2E) in localStorage. La box NaCl è
//   simmetrica sul segreto condiviso, quindi `peer_open(peerId, payload)` apre sia i messaggi ricevuti
//   sia quelli che ho inviato io → un solo formato per entrambe le direzioni.
import { invoke } from "@tauri-apps/api/core";

export type Contact = { id: string; name: string; added: number };
// Stato di consegna, stile messaggistica: sent = partito/in coda (✓), delivered = il relay l'ha
// consegnato al destinatario online (✓✓), read = il destinatario ha aperto la chat (✓✓ blu).
// Vale solo per i MIEI messaggi (dir === "me").
export type MsgStatus = "sent" | "delivered" | "read";
export type ChatMsg = { dir: "me" | "peer"; text: string; ts: number; id: string; status?: MsgStatus; doc?: { name: string; text: string }; photo?: { name: string; dataUrl: string } };
type Stored = { dir: "me" | "peer"; payload: string; ts: number; id: string; status?: MsgStatus };
export type NetStatus = "off" | "connecting" | "on";

// Un messaggio-documento viaggia nello STESSO canale cifrato di un testo, marcato da un sentinella
// (carattere di controllo improbabile in un messaggio umano). Formato plaintext:
//   DOC<nome><testo estratto>
// Così i due Liara possono leggere/discutere lo stesso documento. Limite: il relay accetta 256KB per
// messaggio → il testo è capato lato UI (vedi PeerHub). Retro-compatibile: i messaggi senza sentinella
// restano testo normale.
const DOC_SENTINEL = "DOC";
// Foto: come i documenti ma il "corpo" è un dataURL immagine (già RIDIMENSIONATO lato UI per stare
// sotto i 256KB del relay → niente CDN). Sentinella distinta → la mostriamo come IMMAGINE apribile.
const PHOTO_SENTINEL = String.fromCharCode(1) + "IMG" + String.fromCharCode(1);
export function makePhotoPlaintext(name: string, dataUrl: string): string {
  return PHOTO_SENTINEL + name + String.fromCharCode(1) + dataUrl;
}
export function makeDocPlaintext(name: string, text: string): string {
  return DOC_SENTINEL + name + "" + text;
}
function parsePlaintext(raw: string): { text: string; doc?: { name: string; text: string }; photo?: { name: string; dataUrl: string } } {
  if (raw.startsWith(PHOTO_SENTINEL)) {
    const rest = raw.slice(PHOTO_SENTINEL.length);
    const i = rest.indexOf(String.fromCharCode(1));
    const name = i >= 0 ? rest.slice(0, i) : "foto";
    const dataUrl = i >= 0 ? rest.slice(i + 1) : "";
    return { text: `🖼 ${name}`, photo: { name, dataUrl } };
  }
  if (raw.startsWith(DOC_SENTINEL)) {
    const rest = raw.slice(DOC_SENTINEL.length);
    const i = rest.indexOf("");
    const name = i >= 0 ? rest.slice(0, i) : "documento";
    const text = i >= 0 ? rest.slice(i + 1) : "";
    return { text: `📄 ${name}`, doc: { name, text } };
  }
  return { text: raw };
}

const RELAY_KEY = "liara_relay";
const HIST_KEY = (id: string) => `liara_hist_${id}`;
const UNREAD_KEY = "liara_unread";
const TASK_KEY = (id: string) => `liara_task_${id}`;

// ── Task-driven peer-AI: un OBIETTIVO + MATERIALI (doc/foto) su cui i due Liara lavorano insieme. ──
export type Material = { kind: "doc" | "photo"; name: string; content: string }; // doc: testo; photo: dataURL
export type Task = { goal: string; materials: Material[] };
const EMPTY_TASK: Task = { goal: "", materials: [] };
export function getTask(id: string): Task {
  try { return JSON.parse(localStorage.getItem(TASK_KEY(id)) || "null") || EMPTY_TASK; } catch { return EMPTY_TASK; }
}
export function setTask(id: string, task: Task) { try { localStorage.setItem(TASK_KEY(id), JSON.stringify(task)); } catch { /* */ } }
/// Testo dei materiali per il modello: i documenti col testo, le foto solo segnalate (il testo non le "vede").
export function materialsText(task: Task): string {
  return task.materials.map((m) => m.kind === "doc" ? `[Documento: ${m.name}]\n${m.content}` : `[Foto condivisa: ${m.name}]`).join("\n\n");
}
// messaggio di CONTROLLO che sincronizza il task al peer (sentinella con char di controllo)
const TASK_SENTINEL = String.fromCharCode(1) + "TASK" + String.fromCharCode(1);
const REQ_KEY = "liara_requests"; // richieste d'amicizia in arrivo, da accettare
const NAME_KEY = "liara_display_name"; // il mio nome mostrato agli altri negli inviti

// Messaggi di CONTROLLO (sealed E2E come i normali): invito d'amicizia e accettazione. Sentinella
// improbabile in un testo umano.
const INVITE_SENTINEL = "INV";
const ACCEPT_SENTINEL = "ACK";

export type PeerRequest = { id: string; name: string; ts: number };

export function setDisplayName(name: string) { try { localStorage.setItem(NAME_KEY, name); } catch { /* */ } }
function myDisplayName(): string { try { return localStorage.getItem(NAME_KEY) || ""; } catch { return ""; } }

function loadRequests(): PeerRequest[] {
  try { return JSON.parse(localStorage.getItem(REQ_KEY) || "[]"); } catch { return []; }
}
function saveRequests(r: PeerRequest[]) { try { localStorage.setItem(REQ_KEY, JSON.stringify(r)); } catch { /* */ } }
export function pendingRequests(): PeerRequest[] { return loadRequests(); }
export function pendingCount(): number { return loadRequests().length; }
function addRequest(id: string, name: string, ts: number) {
  const r = loadRequests();
  if (!r.some((x) => x.id === id)) { r.push({ id, name, ts }); saveRequests(r); }
}
function dropRequest(id: string) { saveRequests(loadRequests().filter((x) => x.id !== id)); }
/// Toglie richieste/non-letti riferiti a SE STESSI (residui di test con un solo device) → niente campanella fantasma.
function cleanupSelf() {
  const reqs = loadRequests().filter((r) => r.id && r.id !== myId);
  if (reqs.length !== loadRequests().length) saveRequests(reqs);
  const u = loadUnread();
  if (u[myId]) { delete u[myId]; saveUnread(u); }
}

// Relay OSPITATO da noi (stesso server del cloud Liara, dietro Cloudflare+TLS): l'utente NON deve
// configurare nulla. Resta un override in localStorage solo per test/self-hosting avanzato.
const DEFAULT_RELAY = "wss://liara.nothumanallowed.com/relay";

export function getRelay(): string {
  try { return localStorage.getItem(RELAY_KEY) || DEFAULT_RELAY; } catch { return DEFAULT_RELAY; }
}
export function setRelay(url: string) {
  try { localStorage.setItem(RELAY_KEY, url.trim()); } catch { /* */ }
}

// ---- storico (ciphertext) ------------------------------------------------
function loadStored(id: string): Stored[] {
  try { return JSON.parse(localStorage.getItem(HIST_KEY(id)) || "[]"); } catch { return []; }
}
function pushStored(id: string, m: Stored) {
  const all = loadStored(id);
  all.push(m);
  // tetto anti-crescita: teniamo gli ultimi 500 messaggi per contatto
  const trimmed = all.slice(-500);
  try { localStorage.setItem(HIST_KEY(id), JSON.stringify(trimmed)); } catch { /* */ }
}
export function clearHistory(id: string) {
  try { localStorage.removeItem(HIST_KEY(id)); } catch { /* */ }
}

/// Decifra tutto lo storico di un contatto per mostrarlo. I payload illeggibili (chiave cambiata)
/// vengono saltati, non fanno crashare la vista.
export async function loadHistory(id: string): Promise<ChatMsg[]> {
  const stored = loadStored(id);
  const out: ChatMsg[] = [];
  for (const s of stored) {
    try {
      const raw = await invoke<string>("peer_open", { peerId: id, payload: s.payload });
      // MAI mostrare i messaggi di CONTROLLO in chat (READ/INVITE/ACCEPT/TASK o futuri sconosciuti):
      // iniziano tutti col char \001 (SOH), improbabile in un testo umano. L'unico control "visibile"
      // è il DOC (allegato). Filtro qui → sparisce anche la spazzatura già finita nello storico da un
      // client vecchio (es. "☒READ☒[...]") e regge il disallineamento di versione tra i due Liara.
      if (raw.charCodeAt(0) === 1 && !raw.startsWith(DOC_SENTINEL)) continue;
      const p = parsePlaintext(raw);
      out.push({ dir: s.dir, text: p.text, ts: s.ts, id: s.id, status: s.status, doc: p.doc, photo: p.photo });
    } catch { /* payload non apribile → salta */ }
  }
  return out;
}

// ---- non letti -----------------------------------------------------------
function loadUnread(): Record<string, number> {
  try { return JSON.parse(localStorage.getItem(UNREAD_KEY) || "{}"); } catch { return {}; }
}
function saveUnread(u: Record<string, number>) {
  try { localStorage.setItem(UNREAD_KEY, JSON.stringify(u)); } catch { /* */ }
}
export function unreadCount(id: string): number { return loadUnread()[id] || 0; }
export function totalUnread(): number { return Object.values(loadUnread()).reduce((a, b) => a + b, 0); }
export function clearUnread(id: string) { const u = loadUnread(); delete u[id]; saveUnread(u); emit(); }

// ---- stato messaggi (spunte) + ricevute di lettura -----------------------
const READ_SENTINEL = String.fromCharCode(1) + "READ" + String.fromCharCode(1);
const READHW_KEY = "liara_readhw"; // ts dell'ultimo msg del contatto per cui ho già mandato la ricevuta

// id breve per-messaggio: lega l'ack del relay e la ricevuta di lettura AL messaggio giusto.
function genMsgId(): string {
  return Math.random().toString(36).slice(2, 10) + Date.now().toString(36).slice(-4);
}

// Aggiorna lo stato di un mio messaggio (per id). Non declassa MAI (read > delivered > sent): un ack
// tardivo non deve spegnere una spunta di lettura già arrivata.
const STATUS_RANK: Record<MsgStatus, number> = { sent: 0, delivered: 1, read: 2 };
function setMsgStatus(peerId: string, id: string, status: MsgStatus) {
  const all = loadStored(peerId);
  let changed = false;
  for (const m of all) {
    if (m.dir === "me" && m.id === id && STATUS_RANK[status] > STATUS_RANK[m.status ?? "sent"]) {
      m.status = status; changed = true;
    }
  }
  if (changed) { try { localStorage.setItem(HIST_KEY(peerId), JSON.stringify(all)); } catch { /* */ } emit(); }
}

function loadReadHw(): Record<string, number> {
  try { return JSON.parse(localStorage.getItem(READHW_KEY) || "{}"); } catch { return {}; }
}
// Manda al contatto la ricevuta di lettura per i SUOI messaggi non ancora confermati (ts > highwater).
// Chiamata quando apro la sua chat (setActiveChat) e quando arriva un nuovo suo msg mentre la guardo.
async function sendReadReceipts(peerId: string): Promise<void> {
  const hw = loadReadHw();
  const since = hw[peerId] || 0;
  const fresh = loadStored(peerId).filter((m) => m.dir === "peer" && m.ts > since && m.id);
  if (fresh.length === 0) return;
  const ids = fresh.map((m) => m.id);
  const ok = await sendControl(peerId, READ_SENTINEL + JSON.stringify(ids)).catch(() => false);
  if (ok) {
    hw[peerId] = Math.max(since, ...fresh.map((m) => m.ts));
    try { localStorage.setItem(READHW_KEY, JSON.stringify(hw)); } catch { /* */ }
  }
}

// ---- pub/sub -------------------------------------------------------------
type Listener = () => void;
const listeners = new Set<Listener>();
export function subscribe(fn: Listener): () => void { listeners.add(fn); return () => listeners.delete(fn); }
function emit() { listeners.forEach((l) => { try { l(); } catch { /* */ } }); }

// ---- connessione singleton ----------------------------------------------
let ws: WebSocket | null = null;
let myId = "";
let status: NetStatus = "off";
let activeChat: string | null = null; // se sto guardando la chat di questo id, non conta come "non letto"

export function getStatus(): NetStatus { return status; }
export function setActiveChat(id: string | null) { activeChat = id; if (id) { clearUnread(id); sendReadReceipts(id).catch(() => {}); } }

/// Apre (o riapre) la connessione al relay e registra il mio ID. Idempotente.
export async function connect(): Promise<void> {
  const relay = getRelay();
  if (!relay) return;
  if (!myId) { try { myId = await invoke<string>("peer_identity"); } catch { return; } }
  cleanupSelf(); // rimuovi residui di test: richieste/non-letti riferiti al MIO stesso id
  try { ws?.close(); } catch { /* */ }
  status = "connecting"; emit();
  const sock = new WebSocket(relay);
  ws = sock;
  sock.onopen = () => sock.send(JSON.stringify({ type: "register", id: myId }));
  sock.onmessage = async (e) => {
    let m: { type?: string; from?: string; to?: string; body?: unknown; id?: string; delivered?: boolean };
    try { m = JSON.parse(e.data); } catch { return; }
    if (m.type === "registered") { status = "on"; emit(); return; }
    // ACK del relay per un MIO messaggio: delivered=true → consegnato (✓✓), false → solo accodato (✓)
    if (m.type === "ack" && typeof m.to === "string" && typeof m.id === "string") {
      setMsgStatus(m.to, m.id, m.delivered ? "delivered" : "sent");
      return;
    }
    if (m.type === "msg" && typeof m.from === "string" && typeof m.body === "string") {
      const from = m.from, payload = m.body as string, ts = Date.now();
      const msgId = typeof m.id === "string" && m.id ? m.id : genMsgId();
      if (from === myId) return; // ignora i messaggi/inviti da SE STESSI (test con un solo device)
      // Decifra per CLASSIFICARE (invito / accetta / messaggio). Non apribile → scarta.
      let text: string;
      try { text = await invoke<string>("peer_open", { peerId: from, payload }); } catch { return; }
      if (text.startsWith(INVITE_SENTINEL)) {
        addRequest(from, text.slice(INVITE_SENTINEL.length), ts); // richiesta d'amicizia in arrivo
        emit();
        return;
      }
      if (text.startsWith(ACCEPT_SENTINEL)) {
        // il peer ha accettato → assicurati che sia nei tuoi contatti (l'avevi invitato tu)
        invoke("peer_add", { id: from, name: text.slice(ACCEPT_SENTINEL.length), added: ts }).catch(() => {});
        emit();
        return;
      }
      if (text.startsWith(TASK_SENTINEL)) {
        // il peer ha impostato l'OBIETTIVO + materiali della coordinazione → salvalo per il mio Liara
        try { setTask(from, JSON.parse(text.slice(TASK_SENTINEL.length))); } catch { /* */ }
        emit();
        return;
      }
      if (text.startsWith(READ_SENTINEL)) {
        // il peer ha LETTO i miei messaggi → segna "read" (✓✓ blu) quelli con gli id indicati
        try { (JSON.parse(text.slice(READ_SENTINEL.length)) as string[]).forEach((id) => setMsgStatus(from, id, "read")); } catch { /* */ }
        return;
      }
      // messaggio normale → conserva il ciphertext (cifrato a riposo) + segna non letto
      pushStored(from, { dir: "peer", payload, ts, id: msgId });
      if (activeChat !== from) {
        const u = loadUnread(); u[from] = (u[from] || 0) + 1; saveUnread(u);
      } else {
        sendReadReceipts(from).catch(() => {}); // sto guardando la chat → conferma subito la lettura
      }
      emit();
    }
  };
  sock.onclose = () => { if (ws === sock) { status = "off"; emit(); } };
  sock.onerror = () => { status = "off"; emit(); };
}

export function disconnect() { try { ws?.close(); } catch { /* */ } ws = null; status = "off"; emit(); }

/// Cifra un plaintext qualsiasi (testo o documento) e lo invia, salvando il ciphertext nello storico.
async function sealSend(peerId: string, plaintext: string): Promise<boolean> {
  let payload: string;
  try { payload = await invoke<string>("peer_seal", { peerId, text: plaintext }); } catch { return false; }
  if (!ws || ws.readyState !== WebSocket.OPEN) { await connect(); }
  if (!ws || ws.readyState !== WebSocket.OPEN) return false;
  // id per-messaggio: il relay lo rimanda nell'ack (✓✓ consegnato) e il peer nella ricevuta (✓✓ letto)
  const id = genMsgId();
  ws.send(JSON.stringify({ type: "send", to: peerId, body: payload, id }));
  pushStored(peerId, { dir: "me", payload, ts: Date.now(), id, status: "sent" });
  emit();
  return true;
}

/// Invia un messaggio di testo a un contatto.
export async function sendTo(peerId: string, text: string): Promise<boolean> {
  const body = text.trim();
  if (!body) return false;
  return sealSend(peerId, body);
}

/// Invia un DOCUMENTO (nome + testo estratto) sullo stesso canale E2E → il Liara dell'altro potrà leggerlo.
export async function sendDoc(peerId: string, name: string, text: string): Promise<boolean> {
  return sealSend(peerId, makeDocPlaintext(name, text));
}

/// Invia una FOTO (nome + dataURL già ridimensionato) sullo stesso canale E2E → il peer la vede.
export async function sendPhoto(peerId: string, name: string, dataUrl: string): Promise<boolean> {
  return sealSend(peerId, makePhotoPlaintext(name, dataUrl));
}

/// Invia un messaggio di CONTROLLO (invito/accetta) — sealed E2E ma SENZA finire nello storico chat.
async function sendControl(peerId: string, plaintext: string): Promise<boolean> {
  let payload: string;
  try { payload = await invoke<string>("peer_seal", { peerId, text: plaintext }); } catch { return false; }
  if (!ws || ws.readyState !== WebSocket.OPEN) { await connect(); }
  if (!ws || ws.readyState !== WebSocket.OPEN) return false;
  ws.send(JSON.stringify({ type: "send", to: peerId, body: payload }));
  return true;
}

/// Manda una richiesta d'amicizia al peer (col mio nome). L'altro la vede da accettare.
export async function sendInvite(peerId: string): Promise<boolean> {
  return sendControl(peerId, INVITE_SENTINEL + myDisplayName());
}

/// Accetta una richiesta in arrivo: aggiunge il contatto e avvisa l'altro (ACK). Poi la toglie dalle pendenti.
export async function acceptRequest(id: string, name: string): Promise<void> {
  await invoke("peer_add", { id, name: name.trim(), added: Date.now() }).catch(() => {});
  await sendControl(id, ACCEPT_SENTINEL + myDisplayName());
  dropRequest(id);
  emit();
}

export function rejectRequest(id: string): void { dropRequest(id); emit(); }

/// Sincronizza l'OBIETTIVO + materiali col peer (solo goal + documenti-testo: le foto/dataURL possono
/// superare il limite 256KB del relay → restano locali). Così entrambi i Liara lavorano allo stesso compito.
export async function sendTask(peerId: string, task: Task): Promise<boolean> {
  const light: Task = { goal: task.goal, materials: task.materials.filter((m) => m.kind === "doc") };
  return sendControl(peerId, TASK_SENTINEL + JSON.stringify(light));
}
