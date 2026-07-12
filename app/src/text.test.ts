import { describe, it, expect } from "vitest";
import { isRich, cleanForSpeech, fileIcon, takeSentences } from "./text";

describe("isRich", () => {
  it("riconosce chart, tabelle e HTML come 'ricchi'", () => {
    expect(isRich("```chart\n{}\n```")).toBe(true);
    expect(isRich("| a | b |\n| 1 | 2 |")).toBe(true);
    expect(isRich("ecco <table><tr><td>x</td></tr></table>")).toBe(true);
    expect(isRich("una risposta <b>in grassetto</b>")).toBe(true);
  });
  it("la prosa semplice NON è ricca (si può leggere ad alta voce)", () => {
    expect(isRich("Ciao, come stai oggi?")).toBe(false);
    expect(isRich("Sono le 15:30 di lunedì.")).toBe(false);
    // un '<' che non apre un tag non deve falsare
    expect(isRich("3 < 5 e 5 > 3")).toBe(false);
  });
});

describe("cleanForSpeech", () => {
  it("rimuove code/chart, HTML, tabelle e marcatori markdown", () => {
    expect(cleanForSpeech("**ciao** _mondo_")).toBe("ciao mondo");
    expect(cleanForSpeech("prima ```js\ncode\n``` dopo")).toBe("prima dopo");
    expect(cleanForSpeech("testo <span>x</span> fine")).toBe("testo x fine");
    expect(cleanForSpeech("# Titolo\n> citazione")).toBe("Titolo citazione");
  });
  it("collassa gli spazi e trimma", () => {
    expect(cleanForSpeech("   a    b   ")).toBe("a b");
    expect(cleanForSpeech("")).toBe("");
  });
});

describe("fileIcon", () => {
  it("mappa le estensioni comuni", () => {
    expect(fileIcon("relazione.pdf")).toBe("📕");
    expect(fileIcon("dati.CSV")).toBe("📊"); // case-insensitive
    expect(fileIcon("config.json")).toBe("🗂️");
    expect(fileIcon("main.rs")).toBe("💻");
    expect(fileIcon("foto.JPEG")).toBe("🖼️");
    expect(fileIcon("qualcosa.xyz")).toBe("📄"); // sconosciuto → generico
    expect(fileIcon("senza-estensione")).toBe("📄");
  });
});

describe("takeSentences (streaming TTS, parte pura)", () => {
  it("estrae le frasi complete e tiene la coda parziale", () => {
    const r = takeSentences("Ciao. Come stai? Io b");
    expect(r.sentences).toEqual(["Ciao.", "Come stai?"]);
    expect(r.rest).toBe(" Io b"); // la coda non terminata resta
  });
  it("nessuna frase completa → tutto resta nel resto", () => {
    const r = takeSentences("sto ancora scrivendo");
    expect(r.sentences).toEqual([]);
    expect(r.rest).toBe("sto ancora scrivendo");
  });
  it("spezza sui newline; i terminatori consecutivi restano nella stessa frase (comportamento reale)", () => {
    // CARATTERIZZAZIONE: il regex `[.!?\n]+` è greedy → "\n.\n" viene assorbito con la frase
    // precedente. È il comportamento STORICO di flushSpeak (estratto verbatim, zero regressione).
    const r = takeSentences("Riga uno\n.\nRiga due!");
    expect(r.sentences).toEqual(["Riga uno\n.", "Riga due!"]);
    expect(r.rest).toBe("");
  });

  it("un terminatore isolato con spazi attorno non produce una frase di 1 char", () => {
    // "a . b." → "a ." (len>1, tenuta) + "b." ; nessun frammento di lunghezza 1
    const r = takeSentences("a . b.");
    expect(r.sentences).toEqual(["a .", "b."]);
    expect(r.rest).toBe("");
  });
  it("è idempotente sul resto: ri-processare il resto non produce frasi", () => {
    const first = takeSentences("Frase intera. resto parziale");
    const second = takeSentences(first.rest);
    expect(first.sentences).toEqual(["Frase intera."]);
    expect(second.sentences).toEqual([]);
    expect(second.rest).toBe(" resto parziale");
  });
});
