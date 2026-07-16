// Drawer "Rete" — MILESTONE 1 della chat AI↔AI: prova il TUBO tra Mac e Android.
// Mostra il MIO ID (chiave/identità di rete), l'URL del relay (configurabile per il test LAN), il peer ID,
// e permette di connettersi + inviare/ricevere un messaggio. NIENTE E2E ancora (viaggia dentro wss/ws):
// serve solo a verificare che il transport Mac↔Android funzioni. Il Milestone 2 aggiunge la crittografia.
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";

type Line = { from: "me" | "peer" | "sys"; text: string };

export function NetDrawer({ onClose }: { onClose: () => void }) {
  const [myId, setMyId] = useState("");
  const [relayUrl, setRelayUrl] = useState(() => { try { return localStorage.getItem("liara_relay") || ""; } catch { return ""; } });
  const [peerId, setPeerId] = useState(() => { try { return localStorage.getItem("liara_peer") || ""; } catch { return ""; } });
  const [status, setStatus] = useState<"off" | "connecting" | "on">("off");
  const [lines, setLines] = useState<Line[]>([]);
  const [msg, setMsg] = useState("");
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => { invoke<string>("my_network_id").then(setMyId).catch(() => {}); }, []);
  useEffect(() => () => { wsRef.current?.close(); }, []); // chiudi il socket alla chiusura del drawer

  const log = (from: Line["from"], text: string) => setLines((l) => [...l.slice(-80), { from, text }]);

  const connect = () => {
    if (!relayUrl.trim() || !myId) return;
    try { wsRef.current?.close(); } catch { /* ok */ }
    try { localStorage.setItem("liara_relay", relayUrl.trim()); } catch { /* */ }
    setStatus("connecting");
    const ws = new WebSocket(relayUrl.trim());
    wsRef.current = ws;
    ws.onopen = () => { ws.send(JSON.stringify({ type: "register", id: myId })); };
    ws.onmessage = (e) => {
      let m: { type?: string; from?: string; body?: unknown; delivered?: boolean };
      try { m = JSON.parse(e.data); } catch { return; }
      if (m.type === "registered") { setStatus("on"); log("sys", t("connesso al relay ✅", "connected to relay ✅")); }
      else if (m.type === "msg") log("peer", `${(m.from || "").slice(0, 8)}…: ${String(m.body)}`);
      else if (m.type === "ack") log("sys", m.delivered ? t("consegnato", "delivered") : t("in coda (peer offline)", "queued (peer offline)"));
    };
    ws.onclose = () => { setStatus("off"); log("sys", t("disconnesso", "disconnected")); };
    ws.onerror = () => { setStatus("off"); log("sys", t("errore di connessione", "connection error")); };
  };

  const sendMsg = () => {
    const text = msg.trim();
    const ws = wsRef.current;
    if (!text || !peerId.trim() || !ws || ws.readyState !== WebSocket.OPEN) return;
    try { localStorage.setItem("liara_peer", peerId.trim()); } catch { /* */ }
    ws.send(JSON.stringify({ type: "send", to: peerId.trim(), body: text }));
    log("me", text);
    setMsg("");
  };

  return (
    <div className="drawer-overlay" onClick={onClose}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head"><h2>🔗 {t("Rete (beta)", "Network (beta)")}</h2><button className="ghost" onClick={onClose}>✕</button></div>
        <p className="hint">{t("Prova di connessione Liara↔Liara. Condividi il tuo ID con l'altro, incolla il suo, connettiti al relay e scrivi.", "Liara↔Liara connection test. Share your ID, paste theirs, connect to the relay and type.")}</p>

        <div className="pgroup">
          <h3>{t("Il mio ID", "My ID")}</h3>
          <div className="netid" onClick={() => navigator.clipboard?.writeText(myId).catch(() => {})} title={t("Tocca per copiare", "Tap to copy")}>{myId || "…"}</div>
        </div>

        <div className="pgroup">
          <h3>{t("Relay", "Relay")} · {status === "on" ? "🟢" : status === "connecting" ? "🟡" : "⚪️"}</h3>
          <input placeholder="ws://192.168.x.x:8790" value={relayUrl} onChange={(e) => setRelayUrl(e.target.value)} />
          <input placeholder={t("ID dell'altro Liara", "The other Liara's ID")} value={peerId} onChange={(e) => setPeerId(e.target.value)} />
          <button className="send-sm" onClick={connect} disabled={!relayUrl.trim() || !myId}>{status === "on" ? t("Riconnetti", "Reconnect") : t("Connetti", "Connect")}</button>
        </div>

        <div className="pgroup">
          <div className="netlog">
            {lines.length === 0 && <p className="hint">{t("Nessun messaggio.", "No messages.")}</p>}
            {lines.map((l, i) => <div key={i} className={`netline ${l.from}`}>{l.from === "sys" ? <i>{l.text}</i> : l.text}</div>)}
          </div>
          <div className="addfact">
            <input placeholder={t("Scrivi…", "Type…")} value={msg} onChange={(e) => setMsg(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") sendMsg(); }} disabled={status !== "on"} />
            <button className="send-sm" onClick={sendMsg} disabled={status !== "on" || !msg.trim() || !peerId.trim()}>➤</button>
          </div>
        </div>
      </div>
    </div>
  );
}
