// Sottosistema AGENDA: eventi del calendario + form nuovo evento. Stato + handler in un hook.
// Comportamento verbatim dall'originale.
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { haptic } from "./audio";

export type Event = { id: number; title: string; when_str: string; notes: string };

export function useAgenda() {
  const [showAgenda, setShowAgenda] = useState(false);
  const [events, setEvents] = useState<Event[]>([]);
  const [evTitle, setEvTitle] = useState("");
  const [evWhen, setEvWhen] = useState("");
  const [evNotes, setEvNotes] = useState("");

  async function openAgenda() {
    setEvents(await invoke("calendar_events"));
    setShowAgenda(true);
  }
  async function addEvent() {
    if (!evTitle.trim() || !evWhen.trim()) return;
    await invoke("calendar_create", { title: evTitle, when: evWhen, notes: evNotes });
    haptic(40);
    setEvTitle(""); setEvWhen(""); setEvNotes("");
    setEvents(await invoke("calendar_events"));
  }
  async function removeEvent(id: number) {
    await invoke("calendar_remove", { id });
    setEvents(await invoke("calendar_events"));
  }

  return { showAgenda, setShowAgenda, events, evTitle, setEvTitle, evWhen, setEvWhen, evNotes, setEvNotes, openAgenda, addEvent, removeEvent };
}

export type AgendaApi = ReturnType<typeof useAgenda>;
