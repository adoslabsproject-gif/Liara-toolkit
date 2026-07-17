// Test del catalogo dinamico: il parser deve accettare SOLO entry complete e scartare le "coming",
// e resolveVariants deve appiattire il quant giusto per device. È il path del first-run: se il
// parser lascia passare una entry senza sha, il download parte e fallisce la verifica.
import { describe, expect, it } from "vitest";
import { defaultTemp, parseCatalog, resolveVariants } from "./models";
import type { Model } from "./models";

const ok = {
  id: "x", file: "x.gguf", url: "https://h/x.gguf", sha: "a".repeat(64), bytes: 10,
  size: "Test", gb: "1 GB", sub: "sub",
};

describe("parseCatalog", () => {
  it("accetta una entry completa e applica i default estetici", () => {
    const c = parseCatalog(JSON.stringify([ok]))!;
    expect(c).toHaveLength(1);
    expect(c[0].tag).toBe("Test"); // default: tag = size
    expect(c[0].flag).toBe("🇮🇹");
  });
  it("scarta le entry 'coming' (annunciate senza file)", () => {
    const coming = { ...ok, id: "y", url: "", sha: "", status: "coming" };
    const c = parseCatalog(JSON.stringify([ok, coming]))!;
    expect(c.map((m) => m.id)).toEqual(["x"]);
  });
  it("scarta sha malformati e bytes mancanti", () => {
    expect(parseCatalog(JSON.stringify([{ ...ok, sha: "corto" }]))).toBeNull();
    expect(parseCatalog(JSON.stringify([{ ...ok, bytes: 0 }]))).toBeNull();
  });
  it("ritorna null su JSON rotto o vuoto (si resta sul catalogo che c'è)", () => {
    expect(parseCatalog("{non-json")).toBeNull();
    expect(parseCatalog("[]")).toBeNull();
  });
});

describe("defaultTemp", () => {
  const base: Model = { ...ok, lang: "it", flag: "🇮🇹", icon: "✨", tag: "T" };
  it("piccoli precisi (0.35), grandi conversazionali (0.7)", () => {
    expect(defaultTemp({ ...base, bytes: 1_246_252_832 })).toBe(0.35); // 1.2B
    expect(defaultTemp({ ...base, bytes: 5_335_290_656 })).toBe(0.7); // E4B
  });
  it("tempDefault dal catalogo vince sull'euristica, ma solo se sensato", () => {
    expect(defaultTemp({ ...base, bytes: 1e9, tempDefault: 0.5 })).toBe(0.5);
    expect(defaultTemp({ ...base, bytes: 1e9, tempDefault: 9 })).toBe(0.35); // fuori range → euristica
  });
});

describe("resolveVariants", () => {
  const m: Model = {
    ...ok, lang: "it", flag: "🇮🇹", icon: "✨", tag: "Test",
    variants: {
      mobile: { quant: "q4km", file: "x-q4km.gguf", url: "https://h/x-q4km.gguf", sha: "b".repeat(64), bytes: 5 },
      desktop: { quant: "q6k", file: "x-q6k.gguf", url: "https://h/x-q6k.gguf", sha: "c".repeat(64), bytes: 8 },
    },
  };
  it("su Android sceglie la variante mobile", () => {
    const r = resolveVariants([m], true)[0];
    expect(r.file).toBe("x-q4km.gguf");
    expect(r.variants).toBeUndefined();
  });
  it("su desktop sceglie la variante desktop", () => {
    expect(resolveVariants([m], false)[0].file).toBe("x-q6k.gguf");
  });
  it("senza varianti lascia il default intatto", () => {
    expect(resolveVariants([{ ...m, variants: undefined }], true)[0].file).toBe("x.gguf");
  });
});
