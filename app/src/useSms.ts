// Sottosistema SMS: copia locale cifrata dei messaggi + "Sincronizza SMS" (permesso READ_SMS,
// solo Android). Menù separato dalla rubrica (l'owner li ha voluti distinti). Stato + handler.
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { haptic } from "./audio";

export type SmsMsg = { who: string; body: string; ts: number; kind: string };

export function useSms() {
  const [showSms, setShowSms] = useState(false);
  const [count, setCount] = useState(0);
  const [syncing, setSyncing] = useState(false);
  const [msg, setMsg] = useState(""); // esito ultimo sync (o errore)
  const [list, setList] = useState<SmsMsg[]>([]); // messaggi da mostrare

  async function refreshList() {
    setList(await invoke<SmsMsg[]>("sms_list", { limit: 200 }).catch(() => []));
  }
  async function openSms() {
    setMsg("");
    setCount(await invoke<number>("sms_count").catch(() => 0));
    await refreshList();
    setShowSms(true);
  }
  async function syncNow() {
    if (syncing) return;
    setMsg("");
    setSyncing(true);
    try {
      const [nuovi, totale] = await invoke<[number, number]>("sms_sync");
      setCount(totale);
      setMsg(nuovi > 0 ? `+${nuovi}` : "ok");
      await refreshList();
      haptic(20);
    } catch (e) {
      setMsg("⚠️ " + String(e));
    } finally {
      setSyncing(false);
    }
  }

  return { showSms, setShowSms, count, syncing, msg, list, openSms, syncNow };
}

export type SmsApi = ReturnType<typeof useSms>;
