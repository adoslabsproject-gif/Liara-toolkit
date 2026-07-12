import { describe, it, expect } from "vitest";
import { activePath, toMsg, childrenOf, chainTo } from "./tree";
import type { Node } from "./constants";

const n = (id: string, parentId: string, role: "user" | "assistant", content = ""): Node =>
  ({ id, parentId, role, content });

// Albero di prova:  ROOT → u1 → a1 → u2 → a2 ;  u1 ha anche un fratello di a1 = a1b (versione alternativa)
function tree() {
  const nodes: Record<string, Node> = {
    u1: n("u1", "", "user", "ciao"),
    a1: n("a1", "u1", "assistant", "risposta A"),
    a1b: n("a1b", "u1", "assistant", "risposta B"),
    u2: n("u2", "a1", "user", "e poi?"),
    a2: n("a2", "u2", "assistant", "fine"),
  };
  const activeChild: Record<string, string> = { "": "u1", u1: "a1", a1: "u2", u2: "a2" };
  return { nodes, activeChild };
}

describe("activePath", () => {
  it("segue il ramo attivo dalla radice alle foglie", () => {
    const { nodes, activeChild } = tree();
    expect(activePath(nodes, activeChild).map((x) => x.id)).toEqual(["u1", "a1", "u2", "a2"]);
  });
  it("cambiando activeChild segue l'altra versione", () => {
    const { nodes } = tree();
    const path = activePath(nodes, { "": "u1", u1: "a1b" });
    expect(path.map((x) => x.id)).toEqual(["u1", "a1b"]); // il ramo di a1b non prosegue
  });
  it("vuoto se non c'è nessun nodo dalla radice", () => {
    expect(activePath({}, {})).toEqual([]);
  });
  it("SUPERIORE all'originale: un ciclo NON manda in loop infinito", () => {
    // dato corrotto: a1 → u2 → a1 (ciclo). L'originale con `while(true)` si sarebbe bloccato.
    const nodes: Record<string, Node> = { u1: n("u1", "", "user"), a1: n("a1", "u1", "assistant"), u2: n("u2", "a1", "user") };
    const cyclic = { "": "u1", u1: "a1", a1: "u2", u2: "a1" };
    const path = activePath(nodes, cyclic);
    expect(path.map((x) => x.id)).toEqual(["u1", "a1", "u2"]); // si ferma, non esplode
  });
});

describe("chainTo", () => {
  it("ricostruisce la catena radice→id", () => {
    const { nodes } = tree();
    expect(chainTo(nodes, "a2").map((x) => x.id)).toEqual(["u1", "a1", "u2", "a2"]);
    expect(chainTo(nodes, "a1b").map((x) => x.id)).toEqual(["u1", "a1b"]);
  });
  it("id inesistente → vuoto", () => {
    expect(chainTo({}, "boh")).toEqual([]);
  });
});

describe("childrenOf", () => {
  it("trova i fratelli (le versioni) di un nodo", () => {
    const { nodes } = tree();
    expect(childrenOf(nodes, "u1").map((x) => x.id).sort()).toEqual(["a1", "a1b"]);
    expect(childrenOf(nodes, "a1").map((x) => x.id)).toEqual(["u2"]);
  });
});

describe("toMsg", () => {
  it("l'utente passa invariato", () => {
    expect(toMsg(n("u", "", "user", "domanda"))).toEqual({ role: "user", content: "domanda" });
  });
  it("dall'assistant strippa il <think> chiuso", () => {
    const m = toMsg(n("a", "u", "assistant", "<think>ragiono</think>\n\nEcco la risposta."));
    expect(m).toEqual({ role: "assistant", content: "Ecco la risposta." });
  });
  it("dall'assistant strippa anche un <think> APERTO in coda (risposta troncata)", () => {
    const m = toMsg(n("a", "u", "assistant", "Parte visibile <think>ragionamento troncato"));
    expect(m.content).toBe("Parte visibile");
  });
});
