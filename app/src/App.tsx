import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t, useLang } from "./i18n";
import { ROOT, TOOL_LABELS } from "./constants";
import type { Node, Msg } from "./constants";
import { haptic, speak, stopSpeak, flushSpeak, startRecAndroid, stopRecAndroid, setAndroid, getAndroid } from "./audio";
import { isRich, cleanForSpeech, fileIcon } from "./text";
import { AssistantBody } from "./markdown";
import { activePath, toMsg, childrenOf as treeChildrenOf, chainTo as treeChainTo } from "./tree";
import { useEmail } from "./useEmail";
import { EmailDrawer } from "./EmailDrawer";
import { useProfile } from "./useProfile";
import { ProfileDrawer } from "./ProfileDrawer";
import { useModelDownload } from "./useModelDownload";
import { useAgenda } from "./useAgenda";
import { AgendaDrawer } from "./AgendaDrawer";
import { LoadOverlays } from "./LoadOverlays";
import { ModelDrawer } from "./ModelDrawer";
import { ConsentModal } from "./ConsentModal";
import { ExitModal } from "./ExitModal";
import { ThemeDrawer } from "./ThemeDrawer";
import { PermsDrawer } from "./PermsDrawer";
import { ChatsDrawer } from "./ChatsDrawer";
import { MenuDrawer } from "./MenuDrawer";
import "./App.css";

export default function App() {
  const [nodes, setNodes] = useState<Record<string, Node>>({});
  const [activeChild, setActiveChild] = useState<Record<string, string>>({});
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [initializing, setInitializing] = useState(true); // overlay bloccante finché il modello non è in RAM
  const [status, setStatus] = useState("");
  const [firstHint, setFirstHint] = useState(() => { try { return !localStorage.getItem("liara-hint"); } catch { return true; } });
  useLang(); // rende l'intera UI reattiva ai cambi di lingua (switch istantaneo)
  const [toolUsed, setToolUsed] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const prof = useProfile(); // profilo "Su di me" + fatti appresi (useProfile.ts)
  const [showChats, setShowChats] = useState(false);
  const [convs, setConvs] = useState<[string, string, number][]>([]);
  const [autoSpeak, setAutoSpeak] = useState(false);
  const autoSpeakRef = useRef(false);
  const speakBuf = useRef("");
  const fileRef = useRef<HTMLInputElement>(null);
  // Piattaforma: lo userAgent è INAFFIDABILE sulla WebView Tauri Android (spesso senza "Android") →
  // il 12B desktop-only finiva nell'APK. Verità dal BACKEND (device_caps, #[cfg(target_os)]). Parte dal
  // guess userAgent e si corregge appena device_caps risponde (istantaneo).
  const [isAndroid, setIsAndroid] = useState(getAndroid());
  useEffect(() => {
    invoke<{ android?: boolean }>("device_caps")
      .then((c) => { if (typeof c.android === "boolean") { setIsAndroid(c.android); setAndroid(c.android); } })
      .catch(() => {});
  }, []);
  const md = useModelDownload(isAndroid, initializing, setInitializing); // modello/download (useModelDownload.ts)
  // Ragionamento (thinking di Qwen3): ACCESO di default (2026-07-06). Il LoRA v6 (attuale) USA il
  // ragionamento per chiamare i tool correttamente — senza, i tool non partono o vengono usati male.
  // (Era OFF per il v4, addestrato col blocco <think> vuoto; superato dal v6.) Chiave bumped a _v3 così i
  // device col vecchio _v2="0" ripartono dal nuovo default ON. Chi lo vuole spento lo toggla dal menu.
  const [thinking, setThinking] = useState(() => { try { const v = localStorage.getItem("liara_thinking_v3"); return v === null ? true : v === "1"; } catch { return true; } });
  useEffect(() => { invoke("set_thinking", { on: thinking }).catch(() => {}); try { localStorage.setItem("liara_thinking_v3", thinking ? "1" : "0"); } catch {} }, [thinking]);
  // Modalità cloud: i turni vanno al 32B (Qwen3-VL) via API invece che al modello locale. I tool si
  // eseguono comunque IN LOCALE (memoria/sensori/file on-device). ⚠️ i dati escono dal dispositivo →
  // si attiva solo dopo consenso esplicito. OFF di default (Liara è on-device). Vedi commands/remote.rs.
  const [cloudMode, setCloudMode] = useState(() => { try { return localStorage.getItem("liara_cloud") === "1"; } catch { return false; } });
  useEffect(() => { try { localStorage.setItem("liara_cloud", cloudMode ? "1" : "0"); } catch {} }, [cloudMode]);
  // Consenso cloud: modale IN-APP (window.confirm NON funziona nella WebView di Tauri → ritornava
  // sempre false, il cloud restava OFF). Attivandola i dati escono dal dispositivo. Unico per menu+selettore.
  const [cloudAsk, setCloudAsk] = useState(false);
  const toggleCloud = (on: boolean) => {
    if (!on) {
      // Spegni il cloud → torna al modello locale. RIAVVIO (come il cambio modello): così l'engine
      // locale riparte pulito e non resta "bloccato" lo stato precedente. La modalità è in localStorage.
      setCloudMode(false);
      md.setSwitchTo(t("Modello locale", "Local model"));
      haptic(20);
      return;
    }
    setCloudAsk(true); // apre il modale di consenso; l'attivazione vera avviene sul bottone "Attiva"
  };
  const [attachments, setAttachments] = useState<{ name: string; icon: string }[]>([]);
  const [image, setImage] = useState<string | null>(null); // data URL of an attached image → vision
  const [camOpen, setCamOpen] = useState(false); // fotocamera aperta (cattura → stessa pipeline visione)
  const camVideoRef = useRef<HTMLVideoElement>(null);
  const camStreamRef = useRef<MediaStream | null>(null);
  useEffect(() => () => { camStreamRef.current?.getTracks().forEach((tr) => tr.stop()); }, []); // stop camera su unmount
  // Aggancia lo stream al <video> QUANDO è montato (camOpen true). Il preview nero su WKWebView/macOS
  // nasce dall'assegnare srcObject troppo presto o dal non chiamare play(): qui aspettiamo il mount reale
  // e forziamo play() sia subito sia su 'loadedmetadata' (alcune WebView renderizzano solo dopo i metadati).
  useEffect(() => {
    if (!camOpen) return;
    const v = camVideoRef.current;
    const s = camStreamRef.current;
    if (!v || !s) return;
    v.srcObject = s;
    v.muted = true; // proprietà (non solo attributo) → autoplay inline consentito senza gesto utente
    const play = () => { v.play().catch(() => {}); };
    v.addEventListener("loadedmetadata", play);
    play();
    return () => v.removeEventListener("loadedmetadata", play);
  }, [camOpen]);
  const [speaking, setSpeaking] = useState(false);
  const [listening, setListening] = useState(false);
  const [consentReq, setConsentReq] = useState<{ tool: string; action: string } | null>(null);
  const [showPerms, setShowPerms] = useState(false);
  const [showMenu, setShowMenu] = useState(false);
  const [perms, setPerms] = useState<[string, string, string][]>([]);
  const [theme, setTheme] = useState(() => localStorage.getItem("liara_theme") || "");
  const [showTheme, setShowTheme] = useState(false);
  const agenda = useAgenda(); // agenda/calendario (useAgenda.ts)
  const email = useEmail(); // sottosistema email: stato + handler + polling (useEmail.ts)
  const streamTarget = useRef<string>("");
  const stoppedRef = useRef(false); // true dopo Stop → ignora i token ancora in arrivo (stop immediato a schermo)
  // Batching dei token in streaming: accumula in un ref e riversa UNA volta per frame (rAF) invece di
  // un setNodes per token → re-render da centinaia/risposta a ~60/s. Flush garantito su "done"/Stop.
  const pendingTok = useRef("");
  const rafTok = useRef<number | null>(null);
  const convId = useRef<string>("");
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickBottom = useRef(true);
  const [navHint, setNavHint] = useState("");
  const [confirmExit, setConfirmExit] = useState(false);

  useEffect(() => {
    const subs = [
      listen<string>("token", (e) => {
        if (stoppedRef.current) return; // Stop premuto: non aggiungere altri token
        if (!streamTarget.current) return;
        pendingTok.current += e.payload; // accumula; il flush avviene una volta per frame
        if (rafTok.current == null) {
          rafTok.current = requestAnimationFrame(() => {
            rafTok.current = null;
            const id = streamTarget.current;
            const chunk = pendingTok.current;
            pendingTok.current = "";
            if (id && chunk) setNodes((nd) => (nd[id] ? { ...nd, [id]: { ...nd[id], content: nd[id].content + chunk } } : nd));
          });
        }
        if (autoSpeakRef.current) { speakBuf.current += e.payload; if (flushSpeak(speakBuf)) setSpeaking(true); }
      }),
      listen<string>("status", (e) => {
        const p = e.payload;
        if (p === "ready") { setInitializing(false); md.setNeedDownload(false); setStatus(""); return; }
        // FIX race "click troppo presto sul download" (owner 2026-07-07): NON sblocchiamo la UI all'istante
        // di need-download. Teniamo l'overlay di caricamento ancora ~1,8s così il WebView (che all'avvio è
        // sotto pressione — "tile memory limits exceeded") e il backend si STABILIZZANO prima che l'utente
        // possa toccare "Scarica". Un tap immediato faceva morire l'app. Prepariamo la schermata (md.needDownload)
        // ma la lasciamo coperta dall'overlay bloccante finché non è tutto pronto.
        if (p === "need-download") { md.setNeedDownload(true); setTimeout(() => setInitializing(false), 1800); return; }
        if (p.startsWith("error:")) { setInitializing(false); setStatus("⚠️ " + p.slice(6)); return; }
        setStatus(
          p === "loading-model" ? t("Carico il modello locale…", "Loading the local model…")
          : p === "loading-vision" ? t("Carico il modello visione (~1.8GB, solo la prima volta)…", "Loading the vision model (~1.8GB, first time only)…")
          : p === "vision-look" ? t("👁️ Guardo l'immagine…", "👁️ Looking at the image…")
          : "");
      }),
      listen<{ downloaded?: number; total?: number; done?: boolean }>("download-progress", (e) => {
        const { downloaded = 0, total = 0, done } = e.payload || {};
        md.setDl((prev) => ({ done: downloaded, total, label: prev?.label }));
        if (done) { md.setDl(null); md.setNeedDownload(false); setInitializing(true); invoke("warmup").catch(() => {}); }
      }),
      listen<string>("done", () => {
        // flush SINCRONO degli ultimi token in coda prima di azzerare il target (o si perdono)
        if (rafTok.current != null) { cancelAnimationFrame(rafTok.current); rafTok.current = null; }
        const id = streamTarget.current; const chunk = pendingTok.current; pendingTok.current = "";
        if (id && chunk) setNodes((nd) => (nd[id] ? { ...nd, [id]: { ...nd[id], content: nd[id].content + chunk } } : nd));
        setBusy(false); setStatus(""); setToolUsed(""); streamTarget.current = ""; haptic(12);
        if (autoSpeakRef.current && speakBuf.current.trim()) { speak(speakBuf.current.trim()); speakBuf.current = ""; setSpeaking(true); }
      }),
      listen<{ name: string; args: string }>("tool", (e) => { setToolUsed(e.payload.name); haptic(15); }),
      listen<{ to: string; subject: string; body: string }>("compose", (e) => {
        email.setCompose({ to: e.payload.to, subject: e.payload.subject, body: e.payload.body });
        email.setShowEmail(true);
      }),
      listen("memory-updated", async () => { prof.setFacts(await invoke<string[]>("memory_facts")); haptic([10, 30, 10]); }),
      listen<{ tool: string; action: string }>("consent-request", (e) => { setConsentReq(e.payload); haptic([20, 40, 20]); }),
      listen("tts-idle", () => setSpeaking(false)),
    ];
    return () => { subs.forEach((p) => p.then((f) => f())); };
  }, []);

  // Tastiera: lega l'altezza dell'app al visualViewport → quando compare la tastiera l'app si rimpicciolisce
  // e il campo di input resta SEMPRE visibile sopra di essa (robusto sulle WebView Android, dove adjustResize
  // da solo a volte non basta).
  useEffect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const apply = () => document.documentElement.style.setProperty("--app-h", `${vv.height}px`);
    vv.addEventListener("resize", apply);
    vv.addEventListener("scroll", apply);
    apply();
    return () => { vv.removeEventListener("resize", apply); vv.removeEventListener("scroll", apply); };
  }, []);

  // AUDIO on-demand: i modelli voce (TTS/STT, 351MB) NON sono nell'app (APK/DMG leggeri) — si scaricano
  // da GitHub SOLO al primo uso della voce, poi si estraggono in models/audio. Idempotente (audio_present).
  const AUDIO_ZIP = {
    url: "https://github.com/adoslabsproject-gif/nothumanallowed/releases/download/liara-app-1.3/liara-audio.zip",
    sha: "ad035283a4224ee95899dbe415b34e1565ff2c33db183f3988763fb3d61b9770",
    bytes: 351188393,
  };
  const ensureAudio = async (): Promise<boolean> => {
    if (await invoke<boolean>("audio_present").catch(() => false)) return true;
    try {
      md.setDl({ done: 0, total: AUDIO_ZIP.bytes, label: t("Voce (una volta sola)", "Voice (one-time)") });
      await invoke("download_model", { url: AUDIO_ZIP.url, sha256: AUDIO_ZIP.sha, bytes: AUDIO_ZIP.bytes, filename: "liara-audio.zip" });
      md.setDl({ done: AUDIO_ZIP.bytes, total: AUDIO_ZIP.bytes, label: t("Estraggo la voce…", "Extracting voice…") });
      await invoke("extract_audio");
      md.setDl(null);
      return true;
    } catch (e) {
      md.setDl(null);
      setStatus(t("Download voce non riuscito: ", "Voice download failed: ") + String(e));
      setTimeout(() => setStatus(""), 3500);
      return false;
    }
  };

  // Il warmup (caricamento del modello) è guidato dal BACKEND: parte da solo in un thread Rust dopo un
  // ritardo (vedi lib.rs setup). NON lo lanciamo qui: la WebView Android è in uno stato in cui Chromium
  // throttla i timer JS → un setTimeout dal frontend non parte e il modello non si caricherebbe mai.
  // Il frontend si limita ad ASCOLTARE gli eventi "status" (loading-model → ready/error), gestiti
  // nell'effect dei listener; l'overlay `initializing` blocca l'input finché non arriva "ready".

  // --- Android: tasto INDIETRO hardware ---
  // Ad ogni render, layersRef elenca le chiusure dei pannelli APERTI (il più interno per ULTIMO).
  const layersRef = useRef<(() => void)[]>([]);
  layersRef.current = ([
    showMenu && (() => setShowMenu(false)),
    showTheme && (() => setShowTheme(false)),
    showPerms && (() => setShowPerms(false)),
    prof.showProfile && (() => prof.setShowProfile(false)),
    showChats && (() => setShowChats(false)),
    agenda.showAgenda && (() => agenda.setShowAgenda(false)),
    email.showEmail && (() => email.setShowEmail(false)),
    email.showCfg && (() => email.setShowCfg(false)),
    email.compose && (() => email.setCompose(null)),
    email.openMail && (() => email.setOpenMail(null)),
    consentReq && (() => setConsentReq(null)),
    confirmExit && (() => setConfirmExit(false)),
  ].filter(Boolean) as (() => void)[]);
  useEffect(() => {
    history.pushState({ liara: true }, ""); // seed: la 1ª pressione genera popstate, non esce subito
    const onPop = () => {
      const layers = layersRef.current;
      if (layers.length) layers[layers.length - 1](); // chiudi il pannello in cima
      else setConfirmExit(true);                       // sei nella chat → chiedi conferma uscita
      history.pushState({ liara: true }, "");          // resta dentro l'app
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const path = useMemo(() => activePath(nodes, activeChild), [nodes, activeChild]);

  // Stima grezza dei token del contesto attivo (~4 caratteri per token) → avvisa PRIMA del degrado,
  // così l'utente può riassumere o ripartire invece di subire il "rotten context" (contesto degradato).
  // #17 FIX: allineato all'n_ctx REALE (Android 4096 prudenziale, desktop 32768) — prima era 2048/8192.
  const ctxLimit = isAndroid ? 4096 : 32768;
  const ctxUsedFrac = useMemo(() => {
    const chars = path.reduce((s, n) => s + (n.content?.length || 0), 0);
    // Occupazione FISSA del prompt (system minimo + tool selezionati per intento).
    // 2026-07-03: era 3000 (prompt lungo, 24 tool) → col prompt compatto è ~400, e
    // teneva il banner "conversazione lunga" al ~73% GIÀ dal primo messaggio. Ora ~400
    // → il banner scatta davvero quando la conversazione è lunga.
    const FIXED_TOKENS = 400;
    return (chars / 4 + FIXED_TOKENS) / ctxLimit;
  }, [path, ctxLimit]);

  useEffect(() => {
    const el = scrollRef.current;
    // segui il flusso SOLO se l'utente è agganciato al fondo; scroll istantaneo = niente tremolio
    if (el && stickBottom.current) el.scrollTop = el.scrollHeight;
  }, [path, status]);

  // apply + persist the selected color theme
  useEffect(() => {
    if (theme) document.documentElement.dataset.theme = theme;
    else delete document.documentElement.dataset.theme;
    localStorage.setItem("liara_theme", theme);
  }, [theme]);

  // load the conversation list once
  useEffect(() => { refreshConvs(); }, []);

  // device GPS → backend (Android = real GPS; if denied/unavailable, backend falls back to IP)
  useEffect(() => {
    if (!navigator.geolocation) return;
    navigator.geolocation.getCurrentPosition(
      (pos) => { invoke("set_gps", { latitude: pos.coords.latitude, longitude: pos.coords.longitude }).catch(() => {}); },
      () => { /* permesso negato → resta IP/manuale */ },
      { enableHighAccuracy: true, timeout: 10000, maximumAge: 600000 }
    );
  }, []);

  // autosave the active conversation after each completed turn
  useEffect(() => {
    if (busy || !convId.current || Object.keys(nodes).length === 0) return;
    const t = setTimeout(() => {
      const first = Object.values(nodes).find((n) => n.role === "user");
      const title = (first?.content || "Nuova chat").slice(0, 60);
      invoke("save_conversation", { id: convId.current, title, data: JSON.stringify({ nodes, activeChild }) }).then(refreshConvs);
    }, 500);
    return () => clearTimeout(t);
  }, [nodes, activeChild, busy]);

  // Lo storico passato al modello NON deve contenere il <think> (reasoning) dei turni passati: ingolfa il
  // contesto (spesso più lungo della risposta) e non serve. Teniamo solo la risposta finale; il reasoning
  // resta visibile nel bubble (UI), non nel prompt. È il primo pilastro dell'anti-"rotten context".
  // logica dell'albero (pura, testata in tree.ts); wrapper sottili col `nodes` corrente.
  const childrenOf = (pid: string) => treeChildrenOf(nodes, pid);
  const chainTo = (id: string): Node[] => treeChainTo(nodes, id);

  async function run(messages: Msg[], assistantId: string) {
    setBusy(true);
    streamTarget.current = assistantId;
    speakBuf.current = "";
    if (autoSpeakRef.current) stopSpeak(); // clear any leftover speech before the new turn
    try {
      // Cloud → il 32B via API (stessa firma + stessi eventi token/done/tool del locale, streaming
      // identico); i tool_call che il 32B restituisce vengono eseguiti in LOCALE dal backend.
      await invoke(cloudMode ? "remote_generate" : "generate", { messages });
    } catch (err) {
      setNodes((nd) => (nd[assistantId] ? { ...nd, [assistantId]: { ...nd[assistantId], content: "⚠️ " + String(err) } } : nd));
      setBusy(false);
      setStatus("");
    }
  }

  // Vision: answer about an attached image (Qwen2.5-VL). Reuses the token/done streaming.
  async function runVision(imageB64: string, prompt: string, assistantId: string) {
    setBusy(true);
    streamTarget.current = assistantId;
    speakBuf.current = "";
    if (autoSpeakRef.current) stopSpeak();
    try {
      await invoke("describe_image", { imageB64, prompt });
    } catch (err) {
      setNodes((nd) => (nd[assistantId] ? { ...nd, [assistantId]: { ...nd[assistantId], content: "⚠️ " + String(err) } } : nd));
      setBusy(false);
      setStatus("");
    }
  }

  function send() {
    sendText(input.trim());
  }
  function sendText(text: string) {
    if ((!text && !image) || busy || initializing) return; // niente invii prima che il modello sia pronto
    stoppedRef.current = false; // nuovo turno: riabilita lo streaming dei token
    haptic(18);
    setInput("");
    if (!convId.current) convId.current = crypto.randomUUID();
    const parent = path.length ? path[path.length - 1].id : ROOT;
    const uid = crypto.randomUUID();
    const aid = crypto.randomUUID();
    const shown = text || (image ? t("🖼️ (immagine)", "🖼️ (image)") : "");
    const userNode: Node = { id: uid, parentId: parent, role: "user", content: shown };
    // 🔴 FIX CRASH CONVERSAZIONE (2026-07-04): il contesto accumulato faceva esplodere il prefill
    // (fino a 100s misurati) → il prompt sfondava n_ctx (4096) → llama.cpp fa throw → Rust non può
    // catturare l'eccezione C++ → abort() dell'app ("Rust cannot catch foreign exceptions"). E un
    // contesto marcio degradava anche le RISPOSTE (allucinazioni). Finestra scorrevole: teniamo solo
    // gli ultimi messaggi entro un budget di caratteri (~1500 token); con system+tool compatti il
    // contesto resta ben sotto n_ctx, con margine per la risposta. Memoria lunga → "Riassumi e continua".
    const CTX_CHAR_BUDGET = 6000;
    const windowed: Node[] = [];
    let acc = 0;
    for (let i = path.length - 1; i >= 0; i--) {
      acc += (path[i].content?.length || 0) + 16; // +16 ≈ overhead token di ruolo/separatori
      if (acc > CTX_CHAR_BUDGET && windowed.length > 0) break;
      windowed.unshift(path[i]);
    }
    const msgs = [...windowed.map(toMsg), toMsg(userNode)];
    setNodes((nd) => ({ ...nd, [uid]: userNode, [aid]: { id: aid, parentId: uid, role: "assistant", content: "" } }));
    setActiveChild((ac) => ({ ...ac, [parent]: uid, [uid]: aid }));
    if (image) {
      const img = image;
      setImage(null);
      setAttachments([]);
      runVision(img, text, aid);
    } else {
      run(msgs, aid);
    }
  }

  function regenerate(a: Node) {
    if (busy) return;
    const userId = a.parentId;
    const msgs = chainTo(userId).map(toMsg);
    const aid = crypto.randomUUID();
    setNodes((nd) => ({ ...nd, [aid]: { id: aid, parentId: userId, role: "assistant", content: "" } }));
    setActiveChild((ac) => ({ ...ac, [userId]: aid }));
    run(msgs, aid);
  }

  function submitEdit(orig: Node) {
    const text = editText.trim();
    if (!text || busy) return;
    setEditing(null);
    const parent = orig.parentId;
    const base = parent === ROOT ? [] : chainTo(parent);
    const uid = crypto.randomUUID();
    const aid = crypto.randomUUID();
    const userNode: Node = { id: uid, parentId: parent, role: "user", content: text };
    const msgs = [...base.map(toMsg), toMsg(userNode)];
    setNodes((nd) => ({ ...nd, [uid]: userNode, [aid]: { id: aid, parentId: uid, role: "assistant", content: "" } }));
    setActiveChild((ac) => ({ ...ac, [parent]: uid, [uid]: aid }));
    run(msgs, aid);
  }

  function nav(n: Node, dir: number) {
    if (busy) {
      setNavHint(t("Attendi la fine della risposta per cambiare versione", "Wait for the reply to finish before switching version"));
      setTimeout(() => setNavHint(""), 2500);
      return;
    }
    const sibs = childrenOf(n.parentId);
    const idx = sibs.findIndex((s) => s.id === n.id);
    const next = sibs[idx + dir];
    if (next) setActiveChild((ac) => ({ ...ac, [n.parentId]: next.id }));
  }

  // Anti-"rotten context": riassume la conversazione (backend, slot ausiliario) e riparte da capo con
  // il riassunto come memoria → il filo del discorso resta, il contesto torna pulito. Meglio del taglio cieco.
  async function summarizeAndContinue() {
    if (busy || path.length < 2) return;
    const msgs = path.map(toMsg);
    setBusy(true);
    try {
      const summary = await invoke<string>("summarize_conversation", { messages: msgs });
      setBusy(false);
      newChat();
      const sid = crypto.randomUUID();
      setNodes({ [sid]: { id: sid, parentId: ROOT, role: "assistant", content: `📝 *${t("Riassunto della conversazione precedente", "Summary of the previous conversation")}:*\n\n${summary}` } });
      setActiveChild({ [ROOT]: sid });
    } catch {
      setBusy(false);
      newChat(); // fallback: riparti pulito comunque
    }
  }

  function newChat() {
    if (busy) return;
    haptic(15);
    setNodes({}); setActiveChild({}); setEditing(null);
    convId.current = "";
    setShowChats(false);
    setAttachments([]);
    setImage(null);
  }


  async function refreshConvs() {
    setConvs(await invoke<[string, string, number][]>("list_conversations"));
  }
  async function loadConv(id: string) {
    const data = await invoke<string | null>("load_conversation", { id });
    if (!data) return;
    try {
      const parsed = JSON.parse(data);
      setNodes(parsed.nodes || {});
      setActiveChild(parsed.activeChild || {});
      convId.current = id;
      setShowChats(false);
    } catch { /* ignore corrupt */ }
  }
  async function deleteConv(id: string, e: React.MouseEvent) {
    e.stopPropagation();
    await invoke("delete_conversation", { id });
    if (convId.current === id) newChat();
    refreshConvs();
  }

  // --- Fotocamera: cattura un frame e lo manda alla STESSA pipeline visione dell'allegato 🖼️ ---
  // Via webview MediaDevices → funziona su Android/Mac/Windows senza codice nativo. La foto diventa
  // `image` (data URL) esattamente come un allegato: il modello VL la analizza al prossimo messaggio.
  async function openCamera() {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: { ideal: "environment" } }, audio: false, // camera posteriore quando c'è
      });
      camStreamRef.current = stream;
      setCamOpen(true); // il <video> viene montato dal render; lo stream lo aggancia l'useEffect qui sotto
      haptic(20);
    } catch {
      setStatus(t("Fotocamera non disponibile o permesso negato", "Camera unavailable or permission denied"));
      setTimeout(() => setStatus(""), 3000);
    }
  }
  function closeCamera() {
    camStreamRef.current?.getTracks().forEach((tr) => tr.stop()); // rilascia la fotocamera (LED off)
    camStreamRef.current = null;
    setCamOpen(false);
  }
  function capturePhoto() {
    const video = camVideoRef.current;
    if (!video) return;
    const MAX = 512; // stessa soglia dell'allegato: immagine piccola = niente OOM su telefono
    let w = video.videoWidth, h = video.videoHeight;
    if (!w || !h) return;
    if (w > MAX || h > MAX) { const s = MAX / Math.max(w, h); w = Math.round(w * s); h = Math.round(h * s); }
    const canvas = document.createElement("canvas");
    canvas.width = w; canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) { setStatus(t("Immagine non leggibile", "Image can't be read")); return; }
    ctx.drawImage(video, 0, 0, w, h);
    setImage(canvas.toDataURL("image/jpeg", 0.85)); // stesso formato dell'allegato
    setAttachments([{ name: "foto.jpg", icon: "📷" }]);
    closeCamera();
    haptic([20, 40, 20]);
    setStatus(t("📷 Foto pronta — scrivi una domanda (o invia) e la analizzo", "📷 Photo ready — type a question (or send) and I'll analyse it"));
    setTimeout(() => setStatus(""), 3500);
  }

  async function handleFile(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    // images → vision (Qwen2.5-VL); only one at a time.
    if (file.type.startsWith("image/")) {
      // Ridimensiona a max 1024px PRIMA di inviare: una foto da 12MP genera migliaia di token
      // immagine → la RAM del telefono esplode (OOM) e l'app crasha. 1024px basta per descriverla
      // bene e regge la memoria. Riencode in JPEG q0.85 per ridurre anche la dimensione del base64.
      const url = URL.createObjectURL(file);
      const img = new Image();
      img.onload = () => {
        URL.revokeObjectURL(url);
        const MAX = 512; // telefono: encoder immagine su CPU + RAM stretta → immagine piccola = niente OOM
        let w = img.naturalWidth || img.width;
        let h = img.naturalHeight || img.height;
        if (w > MAX || h > MAX) {
          const s = MAX / Math.max(w, h);
          w = Math.round(w * s);
          h = Math.round(h * s);
        }
        const canvas = document.createElement("canvas");
        canvas.width = w;
        canvas.height = h;
        const ctx = canvas.getContext("2d");
        if (!ctx) { setStatus(t("Immagine non leggibile", "Image can't be read")); return; }
        ctx.drawImage(img, 0, 0, w, h);
        setImage(canvas.toDataURL("image/jpeg", 0.85));
        setAttachments([{ name: file.name, icon: "🖼️" }]);
        haptic([20, 40, 20]);
        setStatus(t("🖼️ Immagine pronta — scrivi una domanda (o invia) e la analizzo", "🖼️ Image ready — type a question (or send) and I'll analyse it"));
        setTimeout(() => setStatus(""), 3500);
      };
      img.onerror = () => { URL.revokeObjectURL(url); setStatus(t("Immagine non leggibile", "Image can't be read")); };
      img.src = url;
      return;
    }
    // PDFs: send the raw file (base64) — the backend extracts the text and indexes it
    if (file.name.toLowerCase().endsWith(".pdf")) {
      setAttachments((a) => [...a, { name: file.name, icon: "📕" }]);
      haptic([20, 40, 20]);
      setStatus(t(`Estraggo e indicizzo «${file.name}»…`, `Extracting and indexing "${file.name}"…`));
      const reader = new FileReader();
      reader.onload = async () => {
        try {
          const n = await invoke<number>("ingest_document", { name: file.name, text: String(reader.result) });
          setStatus(t(`✅ «${file.name}» indicizzato (${n} parti) — chiedimi pure del PDF`, `✅ "${file.name}" indexed (${n} parts) — feel free to ask me about the PDF`));
          setTimeout(() => setStatus(""), 3500);
        } catch (err) {
          setStatus(t("Errore PDF: ", "PDF error: ") + String(err));
          setTimeout(() => setStatus(""), 4000);
        }
      };
      reader.readAsDataURL(file);
      return;
    }
    if (attachments.length >= 5) {
      setStatus(t("Massimo 5 allegati per messaggio", "Up to 5 attachments per message"));
      setTimeout(() => setStatus(""), 2500);
      return;
    }
    // show the chip immediately, then index in the background
    setAttachments((a) => [...a, { name: file.name, icon: fileIcon(file.name) }]);
    haptic([20, 40, 20]);
    setStatus(t(`Indicizzo «${file.name}»…`, `Indexing "${file.name}"…`));
    try {
      const text = await file.text();
      const n = await invoke<number>("ingest_document", { name: file.name, text });
      setStatus(t(`✅ «${file.name}» indicizzato (${n} parti)`, `✅ "${file.name}" indexed (${n} parts)`));
      setTimeout(() => setStatus(""), 3000);
    } catch (err) {
      setStatus(t("Errore indicizzazione: ", "Indexing error: ") + String(err));
      setTimeout(() => setStatus(""), 3500);
    }
  }

  async function openPerms() {
    setPerms(await invoke("permissions"));
    setShowPerms(true);
  }


  return (
    <div className="app">
      <header className="top">
        <div className="brand">
          <button className="icon" title={t("Conversazioni", "Conversations")} onClick={() => { refreshConvs(); setShowChats(true); }}>☰</button>
          <button className="icon" title={t("Nuova chat", "New chat")} onClick={newChat}>✚</button>
          <span className="dot" /><span className="name">Liara</span><small>Personal AI</small>
        </div>
        <div className="topbtns">
          <button className="icon" title={t("Menu", "Menu")} onClick={() => setShowMenu(true)}>⚙️</button>
        </div>
      </header>

      <div className="chat" ref={scrollRef} onScroll={(e) => {
        const el = e.currentTarget;
        stickBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
      }}>
        {md.outdated && (
          <div className="firstrun-hint" style={{ borderColor: "#4ade80" }}>
            <span>🆕 {t("È disponibile una versione migliorata del modello (più precisa sugli strumenti). Scaricala per aggiornare.", "An improved version of the model is available (better at tools). Download it to update.")}</span>
            <button title={t("Aggiorna", "Update")} onClick={() => { md.setOutdated(false); md.startDownload(md.activeModel, true); }}>{t("Aggiorna", "Update")}</button>
          </div>
        )}
        {firstHint && (
          <div className="firstrun-hint">
            <span>💡 {t("Le ", "The ")}<b>{t("prime", "first")}</b>{t(" risposte sono più lente: la GPU si sta preparando (compila i kernel una volta sola). Dopo i primi messaggi ", " replies are slower: the GPU is warming up (it compiles the kernels once). After the first few messages it ")}<b>{t("accelera", "speeds up")}</b>{t(". Anche lo Stop può tardare nei primissimi.", ". Even Stop may lag on the very first ones.")}</span>
            <button title={t("Ho capito", "Got it")} onClick={() => { setFirstHint(false); try { localStorage.setItem("liara-hint", "1"); } catch { /* */ } }}>×</button>
          </div>
        )}
        {path.length === 0 && (
          <div className="empty">
            <h1>{t("Ciao 👋", "Hi 👋")}</h1>
            <p className="lead">{t("Sono ", "I'm ")}<b>Liara</b>{t(", la tua assistente personale.", ", your personal assistant.")}</p>
            <p className="sub">{t("Giro ", "I run ")}<b>{t("solo sul tuo dispositivo", "entirely on your device")}</b>{t(" — privata e offline.", " — private and offline.")}<br />{t("Scrivimi qualcosa, o raccontami di te.", "Write me something, or tell me about yourself.")}</p>
          </div>
        )}

        {path.map((n) => {
          const sibs = childrenOf(n.parentId);
          const idx = sibs.findIndex((s) => s.id === n.id);
          const streaming = busy && streamTarget.current === n.id;
          return (
            <div key={n.id} className={`row ${n.role}`}>
              <div className="msg">
                {editing === n.id ? (
                  <div className="editbox">
                    <textarea value={editText} autoFocus onChange={(e) => setEditText(e.target.value)} />
                    <div className="editbtns">
                      <button className="send-sm" onClick={() => submitEdit(n)}>{t("Salva & invia", "Save & send")}</button>
                      <button className="ghost" onClick={() => setEditing(null)}>{t("Annulla", "Cancel")}</button>
                    </div>
                  </div>
                ) : (
                  <div className="bubble">
                    {n.content
                      ? (n.role === "assistant" ? <AssistantBody text={n.content} /> : n.content)
                      : streaming
                        ? (toolUsed
                            ? <span className="toolinline">🔧 {TOOL_LABELS[toolUsed] ? t(TOOL_LABELS[toolUsed][0], TOOL_LABELS[toolUsed][1]) : toolUsed}<span className="dots"><i /><i /><i /></span></span>
                            : <span className="thinking"><i /><i /><i /></span>)
                        : ""}
                  </div>
                )}
                <div className="msgbar">
                  {sibs.length > 1 && (
                    <span className="nav">
                      <button onClick={() => nav(n, -1)} disabled={idx === 0}>‹</button>
                      {idx + 1}/{sibs.length}
                      <button onClick={() => nav(n, 1)} disabled={idx === sibs.length - 1}>›</button>
                    </span>
                  )}
                  {n.role === "user" && !busy && editing !== n.id && (
                    <button className="act" onClick={() => { setEditing(n.id); setEditText(n.content); }}>✎ {t("Modifica", "Edit")}</button>
                  )}
                  {n.role === "assistant" && !busy && n.content && (
                    <>
                      <button className="act" onClick={() => regenerate(n)}>↻ {t("Rigenera", "Regenerate")}</button>
                      <button className="act" onClick={() => navigator.clipboard.writeText(n.content)}>⧉ {t("Copia", "Copy")}</button>
                      {!isRich(n.content) && (
                        <button className="act" onClick={async () => { if (await ensureAudio()) { speak(cleanForSpeech(n.content)); setSpeaking(true); } }}>🔊 {t("Ascolta", "Listen")}</button>
                      )}
                    </>
                  )}
                </div>
              </div>
            </div>
          );
        })}
        {status && <div className="status">{status}</div>}
      </div>
      <LoadOverlays md={md} initializing={initializing} status={status} />

      {navHint && <div className="navhint">{navHint}</div>}
      {speaking && (
        <button className="stopvoice" onClick={() => { stopSpeak(); setSpeaking(false); haptic(20); }}>⏹ {t("Ferma voce", "Stop voice")}</button>
      )}
      {ctxUsedFrac >= 0.72 && !busy && path.length >= 2 && (
        <div className="ctxbanner">
          <span className="ctxbanner-txt">💬 {t("Conversazione lunga: per risposte più lucide riparti da un riassunto o da una nuova chat.", "Long conversation: for sharper replies, restart from a summary or a new chat.")}</span>
          <div className="ctxbanner-btns">
            <button className="ctxbanner-btn" onClick={summarizeAndContinue}>📝 {t("Riassumi e continua", "Summarize & continue")}</button>
            <button className="ctxbanner-btn ghostbtn" onClick={newChat}>✨ {t("Nuova", "New")}</button>
          </div>
        </div>
      )}
      {attachments.length > 0 && (
        <div className="attachbar">
          {attachments.map((a, i) => (
            <span key={i} className="chip" title={a.name}>
              <span className="chipico">{a.icon}</span>
              <span className="chipname">{a.name}</span>
              <button onClick={() => { setAttachments((x) => x.filter((_, j) => j !== i)); setImage(null); }}>✕</button>
            </span>
          ))}
        </div>
      )}
      {listening && (
        <div className="rec-bar">
          <span className="rec-dot" /> {t("Sto ascoltando…", "Listening…")} <b>{t("rilascia per trascrivere", "release to transcribe")}</b>
        </div>
      )}
      <div className="composer">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }}
          placeholder={busy ? t("Liara sta rispondendo… premi ■ per fermarla", "Liara is replying… press ■ to stop her") : t("Scrivi un messaggio…  (Invio per inviare)", "Type a message…  (Enter to send)")}
          rows={1}
          disabled={busy || initializing}
        />
        <input ref={fileRef} type="file" accept="image/*,.pdf,.txt,.md,.csv,.json,.log,.rs,.py,.js,.ts" style={{ display: "none" }} onChange={handleFile} />
        {!busy && md.hasVision && (
          <button className="send attach" title={t("Allega documento (lo indicizzo per risponderti)", "Attach a document (I'll index it to answer you)")} onClick={() => fileRef.current?.click()}>📎</button>
        )}
        {!busy && md.hasVision && (
          <button className="send attach" title={t("Scatta una foto e la analizzo", "Take a photo and I'll analyse it")} onClick={openCamera}>📷</button>
        )}
        {!busy && (() => {
          const micStart = async () => {
            if (!(await ensureAudio())) return; // scarica+estrae la voce al PRIMO uso (poi è locale)
            setListening(true); haptic(30);
            stopSpeak(); setSpeaking(false); // barge-in
            if (isAndroid) {
              try { await startRecAndroid(); } catch { setListening(false); setStatus(t("Microfono non disponibile", "Microphone not available")); setTimeout(() => setStatus(""), 2500); }
            } else {
              try { await invoke("stt_start"); } catch { setListening(false); }
            }
          };
          const micStop = async () => {
            if (!listening) return;
            setListening(false); haptic(20);
            try {
              let txt = "";
              if (isAndroid) {
                const { pcm, rate } = stopRecAndroid();
                setStatus(t("Trascrivo…", "Transcribing…"));
                txt = await invoke<string>("stt_transcribe", { pcm, rate });
                setStatus("");
              } else {
                txt = await invoke<string>("stt_stop");
              }
              if (txt && txt.trim()) setInput((p) => (p ? p + " " : "") + txt.trim());
            } catch (e) { setStatus(t("Trascrizione non riuscita", "Transcription failed")); setTimeout(() => setStatus(""), 2500); }
          };
          // HOLD-TO-TALK (entrambe le piattaforme): tieni premuto per dettare, RILASCIA per trascrivere.
          // Su Android la WebView a volte perde il pointerup (il dito si sposta di un pixel) → la
          // registrazione restava "premuta". Fix: catturiamo il puntatore (setPointerCapture) e fermiamo
          // su pointerup E su pointercancel/lostpointercapture → il rilascio interrompe SEMPRE.
          return (
            <button
              className={`send mic${listening ? " rec" : ""}`}
              title={t("Tieni premuto per dettare, rilascia per trascrivere", "Hold to dictate, release to transcribe")}
              onPointerDown={(e) => {
                e.preventDefault();
                try { e.currentTarget.setPointerCapture(e.pointerId); } catch { /* ok */ }
                if (!listening) micStart();
              }}
              onPointerUp={(e) => { e.preventDefault(); micStop(); }}
              onPointerCancel={() => micStop()}
              onLostPointerCapture={() => micStop()}
              onContextMenu={(e) => e.preventDefault()}
            >{listening ? "🔴" : "🎤"}</button>
          );
        })()}
        {busy ? (
          <button className="send stop" title={t("Ferma", "Stop")} onClick={() => { stoppedRef.current = true; if (rafTok.current != null) { cancelAnimationFrame(rafTok.current); rafTok.current = null; } pendingTok.current = ""; invoke("stop_generation").catch(() => {}); setBusy(false); setStatus(""); setToolUsed(""); haptic(35); }}>■</button>
        ) : (
          <button className="send" onClick={send} disabled={!input.trim()}>➤</button>
        )}
      </div>

      {showChats && (
        <ChatsDrawer convs={convs} activeId={convId.current} onNew={newChat} onLoad={loadConv} onDelete={deleteConv} onClose={() => setShowChats(false)} />
      )}

      {consentReq && <ConsentModal req={consentReq} onClose={() => setConsentReq(null)} />}
      {cloudAsk && (
        <div className="modal-overlay">
          <div className="consent">
            <div className="consent-icon">☁️</div>
            <h3>{t("Modalità Cloud — Liara 32B", "Cloud mode — Liara 32B")}</h3>
            <p className="consent-action">{t(
              "Molto più capace, ma i tuoi messaggi e i dati che l'assistente legge (file, memoria, posizione, foto) vengono inviati al server. NON è più tutto sul dispositivo.",
              "Far more capable, but your messages and the data the assistant reads (files, memory, location, photos) are sent to the server. It's no longer fully on-device.")}</p>
            <div className="consent-btns">
              <button className="ghost" onClick={() => { setCloudAsk(false); haptic(12); }}>{t("Annulla", "Cancel")}</button>
              <button className="send-sm" onClick={() => { setCloudMode(true); setCloudAsk(false); md.setSwitchTo(t("Liara Cloud 32B ☁️", "Liara Cloud 32B ☁️")); haptic(20); }}>{t("☁️ Attiva cloud", "☁️ Enable cloud")}</button>
            </div>
          </div>
        </div>
      )}

      {camOpen && (
        <div className="modal-overlay" onClick={closeCamera}>
          <div className="cammodal" onClick={(e) => e.stopPropagation()}>
            <video ref={camVideoRef} className="camview" playsInline muted autoPlay />
            <div className="consent-btns">
              <button className="ghost" onClick={closeCamera}>{t("Annulla", "Cancel")}</button>
              <button className="send-sm" onClick={capturePhoto}>📷 {t("Scatta", "Capture")}</button>
            </div>
          </div>
        </div>
      )}

      {confirmExit && <ExitModal onStay={() => setConfirmExit(false)} />}

      {showPerms && (
        <PermsDrawer perms={perms} setPerms={setPerms} onBack={() => { setShowPerms(false); setShowMenu(true); }} onClose={() => setShowPerms(false)} />
      )}

      {showMenu && (
        <MenuDrawer
          theme={theme}
          emailUnread={email.unread}
          modelTag={cloudMode ? "☁️ Cloud 32B" : `${md.activeModel.id.includes("gemma") ? "Gemma" : "Liara"} ${md.activeModel.id.includes("12b") ? "12B" : md.activeModel.id.includes("e4b") ? "E4B" : md.activeModel.id.includes("4b") ? "4B" : "1.7B"} ${md.activeModel.flag}`}
          autoSpeak={autoSpeak}
          thinking={thinking}
          cloud={cloudMode}
          onClose={() => setShowMenu(false)}
          onProfile={() => { setShowMenu(false); prof.openProfile(); }}
          onEmail={() => { setShowMenu(false); email.openEmail(); }}
          onAgenda={() => { setShowMenu(false); agenda.openAgenda(); }}
          onPerms={() => { setShowMenu(false); openPerms(); }}
          onTheme={() => { setShowMenu(false); setShowTheme(true); }}
          onModel={() => { setShowMenu(false); md.openModelDrawer(); }}
          onToggleVoice={() => { const v = !autoSpeak; setAutoSpeak(v); autoSpeakRef.current = v; if (!v) stopSpeak(); else haptic(20); }}
          onToggleThinking={() => { setThinking((v) => !v); haptic(20); }}
          onToggleCloud={() => toggleCloud(!cloudMode)}
        />
      )}

      {showTheme && (
        <ThemeDrawer theme={theme} setTheme={setTheme} onBack={() => { setShowTheme(false); setShowMenu(true); }} onClose={() => setShowTheme(false)} />
      )}

      {md.showModel && (
        <ModelDrawer md={md} cloud={cloudMode} onCloud={toggleCloud} onBack={() => { md.setShowModel(false); setShowMenu(true); }} />
      )}

      {agenda.showAgenda && (
        <AgendaDrawer agenda={agenda} onBack={() => { agenda.setShowAgenda(false); setShowMenu(true); }} />
      )}

      {email.showEmail && (
        <EmailDrawer email={email} onBack={() => { email.setShowEmail(false); setShowMenu(true); }} />
      )}

      {prof.showProfile && (
        <ProfileDrawer profile={prof} onBack={() => { prof.setShowProfile(false); setShowMenu(true); }} />
      )}
    </div>
  );
}
