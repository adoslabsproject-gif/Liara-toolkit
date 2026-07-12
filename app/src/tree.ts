// Albero della conversazione (nodi + ramo attivo) — logica PURA estratta da App.tsx, così è
// testabile e riusabile. Migliore dell'originale: `activePath`/`chainTo` avevano un `while` senza
// guardia → un eventuale ciclo nei parentId (dato corrotto) li faceva loopare all'infinito e
// bloccare la UI. Qui un set `visited` interrompe con grazia. Comportamento IDENTICO sugli alberi
// validi (aciclici): nessuna regressione.
import type { Node, Msg } from "./constants";
import { ROOT } from "./constants";

/// Il ramo ATTIVO della conversazione: dalla radice, seguendo `activeChild`, la catena di nodi mostrati.
export function activePath(nodes: Record<string, Node>, activeChild: Record<string, string>): Node[] {
  const out: Node[] = [];
  const seen = new Set<string>();
  let parent = ROOT;
  for (;;) {
    const childId = activeChild[parent];
    if (!childId || !nodes[childId] || seen.has(childId)) break; // fine ramo o ciclo → stop
    seen.add(childId);
    out.push(nodes[childId]);
    parent = childId;
  }
  return out;
}

/// Converte un nodo nel messaggio per il modello. Dall'assistant STRIPPA il <think> (reasoning):
/// non deve rientrare nello storico (ingolfa il contesto). Gestisce anche un <think> APERTO in coda
/// (Stop premuto / max_tokens senza </think>). È il primo pilastro dell'anti-"rotten context".
export function toMsg(n: Node): Msg {
  return {
    role: n.role,
    content: n.role === "assistant"
      ? n.content.replace(/<think>[\s\S]*?<\/think>\s*/g, "").replace(/<think>[\s\S]*$/, "").trim()
      : n.content,
  };
}

/// I figli diretti di un nodo (le "versioni" fratelli, per la navigazione ‹ 1/N ›).
export function childrenOf(nodes: Record<string, Node>, pid: string): Node[] {
  return Object.values(nodes).filter((n) => n.parentId === pid);
}

/// La catena dalla radice fino a `id` incluso (per ricostruire il contesto di un ramo).
export function chainTo(nodes: Record<string, Node>, id: string): Node[] {
  const c: Node[] = [];
  const seen = new Set<string>();
  let cur: string | undefined = id;
  while (cur && nodes[cur] && !seen.has(cur)) {
    seen.add(cur);
    c.unshift(nodes[cur]);
    cur = nodes[cur].parentId;
  }
  return c;
}
