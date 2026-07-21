// Sottosistema RUBRICA: contatti importati nello store cifrato di Liara + flusso "Sincronizza
// rubrica" (lettura della rubrica di sistema via backend, solo Android). Stato + handler in un hook.
// Gli SMS vivono in un menù separato (useSms.ts): qui c'è SOLO la rubrica.
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { haptic } from "./audio";

export type Contact = { id: number; name: string; number: string; customized: boolean };
// Contatto della rubrica DI SISTEMA annotato dal backend: imported = già nello store; customized =
// modificato nell'app (non va re-importato, ripristinerebbe il numero di sistema).
export type SysContact = { name: string; number: string; imported: boolean; customized: boolean };

export function useContacts() {
  const [showContacts, setShowContacts] = useState(false);
  const [imported, setImported] = useState<Contact[]>([]);
  const [sys, setSys] = useState<SysContact[] | null>(null); // null = non ancora sincronizzato
  const [sel, setSel] = useState<Set<string>>(new Set()); // numeri selezionati per l'import
  const [syncing, setSyncing] = useState(false);
  const [err, setErr] = useState("");
  // modifica in-linea di un contatto della rubrica dell'app
  const [editId, setEditId] = useState<number | null>(null);
  const [editName, setEditName] = useState("");
  const [editNumber, setEditNumber] = useState("");

  async function refreshImported() {
    setImported(await invoke<Contact[]>("contacts_list").catch(() => []));
  }
  async function openContacts() {
    setErr("");
    setSys(null);
    setSel(new Set());
    setEditId(null);
    await refreshImported();
    setShowContacts(true);
  }
  async function syncNow() {
    if (syncing) return;
    setErr("");
    setSyncing(true);
    try {
      const list = await invoke<SysContact[]>("contacts_sync");
      setSys(list);
      // preseleziona SOLO i nuovi (né importati né personalizzati): un tap su "Importa" acquisisce
      // le novità; i già-importati/personalizzati non sono selezionabili.
      setSel(new Set(list.filter((c) => !c.imported).map((c) => c.number)));
      haptic(20);
    } catch (e) {
      setErr(String(e));
    } finally {
      setSyncing(false);
    }
  }
  function toggleSel(number: string) {
    setSel((s) => {
      const n = new Set(s);
      if (n.has(number)) n.delete(number); else n.add(number);
      return n;
    });
  }
  async function importSel() {
    if (!sys || sel.size === 0) return;
    const items = sys.filter((c) => sel.has(c.number)).map((c) => [c.name, c.number]);
    try {
      await invoke<number>("contacts_import", { items });
      haptic(40);
      await refreshImported();
      // gli appena-importati diventano "importati" → escono dalla lista dei nuovi (che si svuota)
      setSys((s) => s && s.map((c) => (sel.has(c.number) ? { ...c, imported: true } : c)));
      setSel(new Set());
    } catch (e) {
      setErr(String(e));
    }
  }
  function startEdit(c: Contact) {
    setEditId(c.id);
    setEditName(c.name);
    setEditNumber(c.number);
  }
  function cancelEdit() {
    setEditId(null);
  }
  async function saveEdit() {
    if (editId == null) return;
    const name = editName.trim();
    const number = editNumber.trim();
    if (!name || !number) return;
    try {
      await invoke("contacts_update", { id: editId, name, number });
      haptic(30);
      setEditId(null);
      await refreshImported();
    } catch (e) {
      setErr(String(e));
    }
  }
  async function removeContact(id: number) {
    await invoke("contacts_delete", { id }).catch(() => {});
    if (editId === id) setEditId(null);
    await refreshImported();
  }

  return {
    showContacts, setShowContacts, imported, sys, sel, syncing, err,
    openContacts, syncNow, toggleSel, importSel, removeContact,
    editId, editName, setEditName, editNumber, setEditNumber, startEdit, saveEdit, cancelEdit,
  };
}

export type ContactsApi = ReturnType<typeof useContacts>;
