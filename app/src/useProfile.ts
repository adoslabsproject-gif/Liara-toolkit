// Sottosistema PROFILO: il profilo strutturato ("Su di me") + i fatti che Liara impara da sola.
// Stato + handler incapsulati in un hook. `setFacts` è esposto perché anche il listener globale
// "memory-updated" (in App) aggiorna la lista dei fatti. Comportamento verbatim dall'originale.
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { haptic } from "./audio";

export function useProfile() {
  const [showProfile, setShowProfile] = useState(false);
  const [profile, setProfile] = useState<Record<string, string>>({});
  const [editFact, setEditFact] = useState<{ i: number; v: string } | null>(null);
  const [facts, setFacts] = useState<string[]>([]);
  const [newFact, setNewFact] = useState("");

  async function openProfile() {
    const entries = await invoke<[string, string][]>("get_profile");
    const map: Record<string, string> = {};
    entries.forEach(([k, v]) => (map[k] = v));
    setProfile(map);
    setFacts(await invoke<string[]>("memory_facts"));
    setShowProfile(true);
  }
  const saveField = (key: string, value: string) => invoke("set_profile", { key, value });
  async function addManualFact() {
    const t = newFact.trim();
    if (!t) return;
    await invoke("add_fact", { text: t });
    setNewFact("");
    setFacts(await invoke<string[]>("memory_facts"));
  }
  async function forgetFacts() {
    await invoke("forget_all");
    setFacts([]);
  }
  async function deleteFact(text: string) {
    await invoke("delete_fact", { text });
    setFacts(await invoke<string[]>("memory_facts"));
    haptic(20);
  }
  async function saveEditFact(oldText: string, newText: string) {
    const t = newText.trim();
    if (t && t !== oldText) {
      await invoke("delete_fact", { text: oldText });
      await invoke("add_fact", { text: t });
    }
    setEditFact(null);
    setFacts(await invoke<string[]>("memory_facts"));
  }

  return {
    showProfile, setShowProfile, profile, setProfile, editFact, setEditFact, facts, setFacts, newFact, setNewFact,
    openProfile, saveField, addManualFact, forgetFacts, deleteFact, saveEditFact,
  };
}

export type ProfileApi = ReturnType<typeof useProfile>;
