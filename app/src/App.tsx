import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t, useLang } from "./i18n";
import { ROOT, TOOL_LABELS } from "./constants";
import type { Node, Msg } from "./constants";
import { haptic, speak, stopSpeak, flushSpeak, startRecAndroid, stopRecAndroid, setAndroid, getAndroid, setOnTtsIdle } from "./audio";
import { defaultTemp, localTemp } from "./models";
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
import { useContacts } from "./useContacts";
import { ContactsDrawer } from "./ContactsDrawer";
import { useSms } from "./useSms";
import { SmsDrawer } from "./SmsDrawer";
import { LoadOverlays } from "./LoadOverlays";
import { ModelDrawer } from "./ModelDrawer";
import { ConsentModal } from "./ConsentModal";
import { ExitModal } from "./ExitModal";
import { ThemeDrawer } from "./ThemeDrawer";
import { PermsDrawer } from "./PermsDrawer";
import { ChatsDrawer } from "./ChatsDrawer";
import { MenuDrawer } from "./MenuDrawer";
import { PeerHub } from "./PeerHub";
import { connect as peerConnect, subscribe as peerSubscribe, totalUnread, pendingCount } from "./peer";
import "./App.css";

export default function App() {
  const [nodes, setNodes] = useState<Record<string, Node>>({});
  const [activeChild, setActiveChild] = useState<Record<string, string>>({});
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [initializing, setInitializing] = useState(true); // overlay bloccante finché il modello non è in RAM
  const [status, setStatus] = useState("");
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
  // Settling (solo Android): la WebView, appena appare la chat, resta "bloccata" ~10s mentre si assesta
  // sotto pressione di memoria (layout + tile). Invece di mostrare una chat che sembra freezata, teniamo
  // l'animazione di caricamento per qualche secondo — così dà l'idea di stare caricando (non bloccata) e
  // l'utente non tocca troppo presto (il tap-precoce fa morire il renderer).
  const [settling, setSettling] = useState(getAndroid());
  useEffect(() => { if (!settling) return; const tm = setTimeout(() => setSettling(false), 6500); return () => clearTimeout(tm); }, []);
  // Android: quando la TTS della WebView finisce (coda vuota), nascondi il pulsante "Ferma voce". Su desktop
  // ci pensa l'evento backend "tts-idle"; su Android il backend non lo emette (voce nella WebView) → serve questo.
  useEffect(() => { setOnTtsIdle(() => setSpeaking(false)); }, []);
  // Chat peer: apri la connessione al relay all'avvio → i messaggi arrivano anche col drawer chiuso.
  // La campanella mostra non-letti + richieste d'amicizia in arrivo (aggiornati via subscribe).
  const [chatNotif, setChatNotif] = useState(0);
  useEffect(() => {
    peerConnect().catch(() => {});
    const upd = () => setChatNotif(totalUnread() + pendingCount());
    upd();
    return peerSubscribe(upd);
  }, []);
  // Voce Kokoro scelta (35 = Sara F, 36 = Nicola M): caricata all'avvio, cambiabile dal menu.
  const [voiceSid, setVoiceSid] = useState(35);
  useEffect(() => { invoke<number>("get_tts_voice").then(setVoiceSid).catch(() => {}); }, []);
  const toggleVoice = () => {
    const next = voiceSid === 36 ? 35 : 36;
    setVoiceSid(next);
    invoke("set_tts_voice", { sid: next }).catch(() => {});
    haptic(20);
  };
  // Lunghezza risposte (preset per-dispositivo): il budget max_tokens dipende da DOVE gira il cervello —
  // cloud 24B generoso (contesto ~40k), desktop medio (~8k), mobile conservativo (piccolo, anti-papiro/OOM).
  const [respLen, setRespLen] = useState<"breve" | "media" | "lunga" | "massima">(() => {
    try { return (localStorage.getItem("liara_resp_len") as "breve" | "media" | "lunga" | "massima") || "media"; } catch { return "media"; }
  });
  const cycleRespLen = () => {
    const order = ["breve", "media", "lunga", "massima"] as const;
    const next = order[(order.indexOf(respLen) + 1) % order.length];
    setRespLen(next);
    try { localStorage.setItem("liara_resp_len", next); } catch { /* */ }
    haptic(20);
  };
  // Icona della lunghezza risposte: cambia a ogni preset ("riempimento" crescente) → feedback visivo immediato.
  const RESP_META = {
    breve: { icon: "◔", label: t("Breve", "Short") },
    media: { icon: "◑", label: t("Media", "Medium") },
    lunga: { icon: "◕", label: t("Lunga", "Long") },
    massima: { icon: "●", label: t("Massima", "Max") },
  } as const;
  useEffect(() => {
    invoke<{ android?: boolean }>("device_caps")
      .then((c) => { if (typeof c.android === "boolean") { setIsAndroid(c.android); setAndroid(c.android); } })
      .catch(() => {});
  }, []);
  const md = useModelDownload(isAndroid, initializing, setInitializing); // modello/download (useModelDownload.ts)
  // Creatività (temperatura) PER MODELLO locale: stato reattivo (mirror di localStorage) così l'icona
  // nel composer + il popover si aggiornano subito e `send` la passa senza rileggere. Il cloud non usa temp.
  const [temp, setTemp] = useState(() => localTemp(md.activeModel));
  const [showTempPop, setShowTempPop] = useState(false);
  useEffect(() => { setTemp(localTemp(md.activeModel)); setShowTempPop(false); }, [md.activeModel.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const saveTemp = (v: number) => {
    setTemp(v);
    try { localStorage.setItem(`liara_temp:${md.activeModel.id}`, String(v)); } catch { /* */ }
  };
  const resetTemp = () => {
    try { localStorage.removeItem(`liara_temp:${md.activeModel.id}`); } catch { /* */ }
    setTemp(defaultTemp(md.activeModel));
    haptic(15);
  };
  // Ragionamento (thinking di Qwen3): ACCESO di default (2026-07-06). Il LoRA v6 (attuale) USA il
  // ragionamento per chiamare i tool correttamente — senza, i tool non partono o vengono usati male.
  // (Era OFF per il v4, addestrato col blocco <think> vuoto; superato dal v6.) Chiave bumped a _v3 così i
  // device col vecchio _v2="0" ripartono dal nuovo default ON. Chi lo vuole spento lo toggla dal menu.
  const [thinking, setThinking] = useState(() => { try { const v = localStorage.getItem("liara_thinking_v3"); return v === null ? true : v === "1"; } catch { return true; } });
  useEffect(() => { invoke("set_thinking", { on: thinking }).catch(() => {}); try { localStorage.setItem("liara_thinking_v3", thinking ? "1" : "0"); } catch {} }, [thinking]);
  // Modalità cloud: i turni vanno al 24B (Qwen3-VL) via API invece che al modello locale. I tool si
  // eseguono comunque IN LOCALE (memoria/sensori/file on-device). ⚠️ i dati escono dal dispositivo →
  // si attiva solo dopo consenso esplicito. OFF di default (Liara è on-device). Vedi commands/remote.rs.
  const [cloudMode, setCloudMode] = useState(() => { try { return localStorage.getItem("liara_cloud") === "1"; } catch { return false; } });
  useEffect(() => {
    try { localStorage.setItem("liara_cloud", cloudMode ? "1" : "0"); } catch {}
    // Scrive il flag lato backend: in cloud il boot NON carica il modello locale (risparmia RAM e calore).
    invoke("set_cloud_active", { on: cloudMode }).catch(() => {});
  }, [cloudMode]);
  // Auto-grow del composer (stile Claude): la textarea cresce col contenuto fino a un max, poi scrolla.
  // Su input="" (dopo l'invio) torna a una riga. Vedi .composer-box in App.css.
  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = Math.min(ta.scrollHeight, 200) + "px";
  }, [input]);
  // Consenso SEPARATO e opt-in al salvataggio anonimo delle conversazioni (dataset di miglioramento). OFF di
  // default (privacy-first, come da promessa "nessuna telemetria"). Se ON, remote_generate manda l'header
  // x-liara-training:allow → SOLO allora il server salva la conversazione anonimizzata (PII redatta). È
  // DISTINTO dal consenso cloud (quello = "i dati vanno al server per rispondere"; questo = "…e potete anche
  // conservarli per migliorare Liara"). Revocabile quando si vuole. Vedi commands/remote.rs.
  const [trainConsent, setTrainConsent] = useState(() => { try { return localStorage.getItem("liara_train") === "1"; } catch { return false; } });
  useEffect(() => { try { localStorage.setItem("liara_train", trainConsent ? "1" : "0"); } catch {} }, [trainConsent]);
  // Cloud attivo (scelto all'avvio o già salvato da una sessione precedente) → NON serve il modello locale:
  // smonta la schermata di download e l'overlay di caricamento, così l'app è subito usabile via API. Copre
  // sia l'attivazione dal bottone d'avvio sia il rilancio con cloud già scelto (il backend emette comunque
  // need-download non sapendo del cloud). Vedi commands/remote.rs.
  useEffect(() => {
    if (cloudMode && (md.needDownload || initializing)) { md.setNeedDownload(false); setInitializing(false); }
  }, [cloudMode, md.needDownload, initializing]);
  // Saluto di stato dal server cloud (GET /v1/hello via backend cloud_hello): mostra un messaggio come
  // PRIMO turno SOLO se il server lo abilita (__liara_hello) — es. avviso che il 24B è temporaneamente
  // sostituito (dataset) o è tornato. Parte SOLO in modalità cloud (in locale NON si contatta il server:
  // promessa on-device) e SOLO su chat vuota (non sovrascrive una conversazione). Ogni errore/timeout →
  // nessun messaggio (fail-safe). Vedi commands/remote.rs::cloud_hello.
  const helloFired = useRef(false); // il saluto server va mostrato una volta sola per sessione
  useEffect(() => {
    if (!cloudMode || helloFired.current) return; // solo in cloud (mai on-device), una volta
    helloFired.current = true;
    invoke<string | null>("cloud_hello").then((content) => {
      if (!content) return; // __liara_hello !== true o errore/timeout → niente messaggio
      const sid = crypto.randomUUID();
      setNodes((nd) => (Object.keys(nd).length ? nd : { [sid]: { id: sid, parentId: ROOT, role: "assistant", content } }));
      setActiveChild((ac) => (Object.keys(ac).length ? ac : { [ROOT]: sid }));
    }).catch(() => { /* silenzioso: nessun saluto, nessun disturbo */ });
  }, [cloudMode]);
  // Consenso cloud: modale IN-APP (window.confirm NON funziona nella WebView di Tauri → ritornava
  // sempre false, il cloud restava OFF). Attivandola i dati escono dal dispositivo. Unico per menu+selettore.
  const [cloudAsk, setCloudAsk] = useState(false);
  const toggleCloud = (on: boolean, silent = false) => {
    if (!on) {
      // Spegni il cloud → torna al modello locale. `silent` = spegnimento SENZA overlay di riavvio
      // (usato quando si sceglie un modello da scaricare dal cloud: lì il warmup carica in-place e non
      // va chiusa l'app). Altrimenti mostra l'overlay "riavvia per applicare". Modalità in localStorage.
      setCloudMode(false);
      if (!silent) md.setSwitchTo(t("Modello locale", "Local model"));
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
  const camInputRef = useRef<HTMLInputElement>(null); // Android: <input capture> → fotocamera NATIVA (la WebView software non disegna il <video>)
  const taRef = useRef<HTMLTextAreaElement>(null); // textarea del composer: auto-grow stile Claude
  const [viewImage, setViewImage] = useState<string | null>(null); // lightbox: immagine allegata aperta a schermo intero
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
  const [voiceBusy, setVoiceBusy] = useState(false); // download+estrazione voce in corso → banner + mic bloccato
  const [consentReq, setConsentReq] = useState<{ tool: string; action: string } | null>(null);
  const [showPerms, setShowPerms] = useState(false);
  const [showMenu, setShowMenu] = useState(false);
  const [showNet, setShowNet] = useState(false); // drawer "Rete" (chat AI↔AI, M1)
  const [perms, setPerms] = useState<[string, string, string][]>([]);
  const [theme, setTheme] = useState(() => localStorage.getItem("liara_theme") || "");
  const [showTheme, setShowTheme] = useState(false);
  const agenda = useAgenda(); // agenda/calendario (useAgenda.ts)
  const contacts = useContacts(); // rubrica cifrata + sincronizzazione dal telefono (useContacts.ts)
  const sms = useSms(); // SMS: copia locale cifrata, menù separato (useSms.ts)
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
        // A fine download: warmup dell'LLM locale SOLO se NON siamo in cloud. In cloud non c'è LLM locale
        // (es. scaricando i modelli VOCE, questo stesso evento scattava e warmup cercava il gguf → "modello
        // non trovato"). Leggo localStorage (non lo stato: il listener è montato una volta, cloudMode sarebbe stale).
        if (done) {
          md.setDl(null);
          if (localStorage.getItem("liara_cloud") !== "1") { md.setNeedDownload(false); setInitializing(true); invoke("warmup").catch(() => {}); }
        }
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

  // AUDIO on-demand: i modelli voce (Kokoro TTS + whisper-small STT + silero VAD, ~592MB) NON sono
  // nell'app (APK/DMG leggeri) — si scaricano da GitHub SOLO al primo uso della voce, poi si estraggono
  // in models/audio. Idempotente (audio_present guarda il lexicon Kokoro). Asset "v3": include lexicon+dict,
  // OBBLIGATORI per il Kokoro multi-lingua (senza, sherpa ABORTA all'ascolto). La v2 era rotta.
  const AUDIO_ZIP = {
    url: "https://github.com/adoslabsproject-gif/nothumanallowed/releases/download/liara-app-1.3/liara-audio-v3.zip",
    sha: "c430ac7a52fc55e3a3e4fe026f118e6e7d9f8e51c29b5e3760cae111c5556fd7",
    bytes: 592149872,
  };
  const ensureAudio = async (): Promise<boolean> => {
    if (await invoke<boolean>("audio_present").catch(() => false)) return true;
    if (voiceBusy) return false; // già in download → non avviarne un secondo
    setVoiceBusy(true); // banner permanente + mic bloccato finché non è tutto pronto
    try {
      md.setDl({ done: 0, total: AUDIO_ZIP.bytes, label: t("Voce (una volta sola)", "Voice (one-time)") });
      await invoke("download_model", { url: AUDIO_ZIP.url, sha256: AUDIO_ZIP.sha, bytes: AUDIO_ZIP.bytes, filename: "liara-audio-v3.zip" });
      md.setDl({ done: AUDIO_ZIP.bytes, total: AUDIO_ZIP.bytes, label: t("Estraggo la voce…", "Extracting voice…") });
      await invoke("extract_audio");
      md.setDl(null);
      return true;
    } catch (e) {
      md.setDl(null);
      setStatus(t("Download voce non riuscito: ", "Voice download failed: ") + String(e));
      setTimeout(() => setStatus(""), 3500);
      return false;
    } finally {
      setVoiceBusy(false);
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
    contacts.showContacts && (() => contacts.setShowContacts(false)),
    sms.showSms && (() => sms.setShowSms(false)),
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

  // device GPS → backend (Android = real GPS; if denied/unavailable, backend falls back to IP).
  // ⚠️ DIFFERITA (non all'avvio): chiedere il permesso posizione al mount faceva apparire il dialog di
  // sistema (GrantPermissionsActivity) DURANTE l'assestamento della WebView → Liara perdeva il foreground
  // ("va in background"/ridotta a icona all'avvio). La chiediamo SOLO quando l'app è su e assestata
  // (!initializing && !settling) e con un piccolo ritardo → il dialog cade fuori dalla finestra fragile.
  // Denied → resta IP/manuale (nessun disturbo). Una sola volta per sessione.
  const gpsAskedRef = useRef(false);
  useEffect(() => {
    if (gpsAskedRef.current || initializing || settling || !navigator.geolocation) return;
    gpsAskedRef.current = true;
    const id = setTimeout(() => {
      navigator.geolocation.getCurrentPosition(
        (pos) => { invoke("set_gps", { latitude: pos.coords.latitude, longitude: pos.coords.longitude }).catch(() => {}); },
        () => { /* permesso negato → resta IP/manuale */ },
        { enableHighAccuracy: true, timeout: 10000, maximumAge: 600000 }
      );
    }, 3500);
    return () => clearTimeout(id);
  }, [initializing, settling]);

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

  async function run(messages: Msg[], assistantId: string, image?: string | null) {
    setBusy(true);
    streamTarget.current = assistantId;
    speakBuf.current = "";
    if (autoSpeakRef.current) stopSpeak(); // clear any leftover speech before the new turn
    // 🔴 FIX "Liara non disponibile dopo Stop": uno Stop lascia in cronologia un turno assistant
    // VUOTO (la risposta interrotta). Inviarlo al cloud fa rispondere al server "momentaneamente
    // non disponibile" (rifiuta il turno assistant vuoto). Filtriamo i turni assistant senza testo
    // PRIMA di inviare: due user consecutivi il modello li gestisce, un assistant vuoto no.
    messages = messages.filter((m) => !(m.role === "assistant" && !(m.content ?? "").trim()));
    try {
      // Cloud → il 24B via API (stessa firma + stessi eventi token/done/tool del locale, streaming
      // identico); i tool_call che il 24B restituisce vengono eseguiti in LOCALE dal backend. `image`
      // (data URL) va al 24B (Qwen3-VL) per la visione cloud — il generate locale non la usa (visione
      // locale = describe_image, flusso a parte).
      // `train`: consenso al salvataggio anonimo (opt-in). Letto FRESH da localStorage per non catturare uno
      // stato stale — se assente/false, il backend NON manda l'header e il server non salva nulla.
      // budget risposta = preset scelto × dispositivo (il backend fa comunque un clamp di sicurezza)
      const RESP_TOKENS = {
        cloud: { breve: 512, media: 2048, lunga: 6144, massima: 16384 },
        desktop: { breve: 384, media: 1024, lunga: 3072, massima: 8192 },
        mobile: { breve: 256, media: 768, lunga: 1536, massima: 2048 },
      } as const;
      const respMode = cloudMode ? "cloud" : isAndroid ? "mobile" : "desktop";
      const maxTokens = RESP_TOKENS[respMode][respLen];
      if (cloudMode) await invoke("remote_generate", { messages, image: image ?? null, train: localStorage.getItem("liara_train") === "1", conversationId: convId.current, maxTokens });
      else await invoke("generate", { messages, maxTokens, temperature: temp });
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
      await invoke("describe_image", { imageB64, prompt, temperature: temp });
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
    // niente invii finché il modello non è PRONTO: in LOCALE serve scaricato E caricato (non needDownload,
    // non initializing); in cloud basta la modalità attiva. + mai durante busy/settling. (Bug: durante la
    // schermata di DOWNLOAD initializing/settling erano false → la textarea restava attiva → Invio da
    // tastiera → generate su engine NON caricato → crash. Ora bloccato.)
    // `settling` (6,5s anti-crash) serve solo al modello LOCALE (stabilizza GPU/WebView); in CLOUD non
    // c'è nulla da stabilizzare → non bloccare l'invio, altrimenti "premo invia e non succede niente".
    if ((!text && !image) || busy || initializing || (settling && !cloudMode) || (!cloudMode && md.needDownload)) return;
    stoppedRef.current = false; // nuovo turno: riabilita lo streaming dei token
    haptic(18);
    setInput("");
    if (!convId.current) convId.current = crypto.randomUUID();
    const parent = path.length ? path[path.length - 1].id : ROOT;
    const uid = crypto.randomUUID();
    const aid = crypto.randomUUID();
    const userNode: Node = { id: uid, parentId: parent, role: "user", content: text, image: image || undefined };
    // 🔴 FIX CRASH CONVERSAZIONE (2026-07-04): il contesto accumulato faceva esplodere il prefill
    // (fino a 100s misurati) → il prompt sfondava n_ctx (4096) → llama.cpp fa throw → Rust non può
    // catturare l'eccezione C++ → abort() dell'app ("Rust cannot catch foreign exceptions"). E un
    // contesto marcio degradava anche le RISPOSTE (allucinazioni). Finestra scorrevole: teniamo solo
    // gli ultimi messaggi entro un budget di caratteri. Memoria lunga → "Riassumi e continua".
    // ⚙️ BUDGET SCALATO per capacità di contesto (2026-07-14): prima era 6000 UNIFORME → anche il cloud
    // 24B e il desktop (n_ctx 32768) ricevevano solo ~1500 token di storia → "non ricordi il primo
    // messaggio". Ora: CLOUD (24B, ~40960 ctx) e DESKTOP-locale (32768) ricevono ~tutta la conversazione;
    // ANDROID-locale resta stretto perché n_ctx=4096 col prompt fisso ~3016 token è al limite (anti-crash).
    const CTX_CHAR_BUDGET = cloudMode ? 80000 : isAndroid ? 6000 : 60000;
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
      // Cloud → l'immagine va al 24B (Qwen3-VL) dentro run/remote_generate. Locale → visione on-device
      // (describe_image col Gemma), flusso separato.
      if (cloudMode) run(msgs, aid, img);
      else runVision(img, text, aid);
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
    // images → vision. Riconosci l'immagine anche se il MIME è vuoto (alcune camere Android restituiscono
    // type="" ) usando l'estensione del nome — altrimenti finiva nel ramo documento e non arrivava alla visione.
    if (file.type.startsWith("image/") || /\.(jpe?g|png|webp|heic|heif|gif|bmp)$/i.test(file.name)) {
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

  // Nome breve del modello in uso, per l'header e il menu (cloud o locale). Dal catalogo (tag), non
  // da euristiche sul nome: coi modelli dinamici (models.json) l'id non è più prevedibile.
  const modelName = cloudMode ? "Cloud 24B" : md.activeModel.tag;

  return (
    <div className="app">
      <header className="top">
        <div className="brand">
          <button className="icon" title={t("Conversazioni", "Conversations")} onClick={() => { refreshConvs(); setShowChats(true); }}>☰</button>
          <button className="icon" title={t("Nuova chat", "New chat")} onClick={newChat}>✚</button>
          <span className="dot" /><span className="name">Liara</span><small>Personal AI</small>
          <button className="modelbadge" title={t("Cambia modello", "Change model")} onClick={() => md.openModelDrawer()}>{modelName}</button>
        </div>
        <div className="topbtns">
          <button className="icon bell" title={t("Liara Chat", "Liara Chat")} onClick={() => setShowNet(true)}>
            🔔{chatNotif > 0 && <span className="notifdot">{chatNotif > 9 ? "9+" : chatNotif}</span>}
          </button>
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
                    {n.image && <img src={n.image} className="bubimg" alt={t("allegato", "attachment")} onClick={() => setViewImage(n.image!)} />}
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
      <LoadOverlays md={md} initializing={initializing} settling={settling} status={status} onUseCloud={() => setCloudAsk(true)} />

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
      {/* #7: durante il download dei driver voce (351 MB, una volta sola) mostra un banner PERMANENTE con
          progressbar; il mic resta bloccato (disabled) finché non è tutto scaricato ed estratto. */}
      {voiceBusy && (
        <div className="voice-dl">
          <div className="voice-dl-head">🎙️ <b>{md.dl?.label || t("Scarico la voce…", "Downloading voice…")}</b>
            <span className="voice-dl-pct">{md.dl?.total ? Math.round((md.dl.done / md.dl.total) * 100) : 0}%</span>
          </div>
          <div className="dl-bar"><div className="dl-fill" style={{ width: `${md.dl?.total ? Math.min(100, Math.round((md.dl.done / md.dl.total) * 100)) : 0}%` }} /></div>
          <div className="load-hint">{((md.dl?.done ?? 0) / 1e9).toFixed(2)} / {((md.dl?.total ?? AUDIO_ZIP.bytes) / 1e9).toFixed(2)} GB · {t("il microfono si sblocca quando è pronto", "the mic unlocks when ready")}</div>
        </div>
      )}
      <div className="composer">
        {/* Popover creatività: backdrop full-screen che chiude al tap fuori; il pannello ferma la propagazione. */}
        {showTempPop && !cloudMode && (
          <>
            <div className="pop-backdrop" onClick={() => setShowTempPop(false)} />
            <div className="temp-pop" onClick={(e) => e.stopPropagation()}>
              <div className="temp-pop-head">
                🌡️ <b>{md.activeModel.tag}</b> · {t("creatività", "creativity")} <b>{temp.toFixed(2)}</b>
                {Math.abs(temp - defaultTemp(md.activeModel)) > 0.001 && (
                  <span className="temp-reset" role="button" onClick={resetTemp}>↺ {defaultTemp(md.activeModel).toFixed(2)}</span>
                )}
              </div>
              <input type="range" min={0.1} max={1.5} step={0.05} value={temp} onChange={(e) => saveTemp(parseFloat(e.target.value))} />
              <div className="temp-pop-legend"><span>{t("preciso", "precise")}</span><span>{t("creativo", "creative")}</span></div>
            </div>
          </>
        )}
        <div className={`composer-box ${busy ? "busy" : ""}`}>
        <textarea
          ref={taRef}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }}
          placeholder={busy ? t("Liara sta rispondendo…", "Liara is replying…") : t("Scrivi un messaggio…", "Type a message…")}
          rows={1}
          disabled={busy || initializing || (settling && !cloudMode) || (!cloudMode && md.needDownload)}
        />
        <div className="composer-actions"><div className="composer-tools">
        <input ref={fileRef} type="file" accept="image/*,.pdf,.txt,.md,.csv,.json,.log,.rs,.py,.js,.ts" style={{ display: "none" }} onChange={handleFile} />
        {/* Android: fotocamera NATIVA (capture) → apre l'app camera del telefono e restituisce la foto come
            file (poi handleFile la tratta come un'immagine allegata). Evita il <video> getUserMedia che sulla
            WebView software (hardwareAccelerated=false) resta NERO. Su desktop invece si usa il preview live. */}
        <input ref={camInputRef} type="file" accept="image/*" capture="environment" style={{ display: "none" }} onChange={handleFile} />
        {!busy && (md.hasVision || cloudMode) && (
          <button className="ctool" title={t("Allega documento (lo indicizzo per risponderti)", "Attach a document (I'll index it to answer you)")} onClick={() => fileRef.current?.click()}>📎</button>
        )}
        {!busy && (md.hasVision || cloudMode) && (
          <button className="ctool" title={t("Scatta una foto e la analizzo", "Take a photo and I'll analyse it")} onClick={() => isAndroid ? camInputRef.current?.click() : openCamera()}>📷</button>
        )}
        {!busy && (() => {
          const micStart = async () => {
            if (voiceBusy) return; // voce in download → mic BLOCCATO finché non è pronto
            // Voce non ancora presente? avvia il download (banner permanente) e NON registrare adesso:
            // sarebbe assurdo tenere premuto per minuti. A download finito il mic si sblocca e si ritocca.
            const ready = await invoke<boolean>("audio_present").catch(() => false);
            if (!ready) { ensureAudio(); return; }
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
              className={`ctool${listening ? " rec" : ""}`}
              disabled={voiceBusy}
              title={voiceBusy ? t("Scarico la voce…", "Downloading voice…") : t("Tieni premuto per dettare, rilascia per trascrivere", "Hold to dictate, release to transcribe")}
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
        {/* Lunghezza risposte: icona che cambia a ogni preset (◔◑◕●) + ciclo al tap. Vale cloud e locale. */}
        {!busy && (
          <button className="ctool" title={`${t("Lunghezza risposte", "Response length")}: ${RESP_META[respLen].label}`} onClick={cycleRespLen}>{RESP_META[respLen].icon}</button>
        )}
        {/* Creatività (temperatura): SOLO modello locale. Tap → popover slider; tap fuori → chiude. */}
        {!busy && !cloudMode && (
          <button className={`ctool${showTempPop ? " on" : ""}`} title={t("Creatività (temperatura)", "Creativity (temperature)")} onClick={() => { setShowTempPop((v) => !v); haptic(15); }}>🌡️</button>
        )}
        </div>
        {busy ? (
          <button className="csend stop" title={t("Ferma", "Stop")} onClick={() => { stoppedRef.current = true; if (rafTok.current != null) { cancelAnimationFrame(rafTok.current); rafTok.current = null; } pendingTok.current = ""; invoke("stop_generation").catch(() => {}); setBusy(false); setStatus(""); setToolUsed(""); haptic(35); }}>■</button>
        ) : (
          <button className="csend" onClick={send} disabled={!input.trim()}>➤</button>
        )}
        </div>
        </div>
      </div>

      {showChats && (
        <ChatsDrawer convs={convs} activeId={convId.current} onNew={newChat} onLoad={loadConv} onDelete={deleteConv} onClose={() => setShowChats(false)} />
      )}

      {consentReq && <ConsentModal req={consentReq} onClose={() => setConsentReq(null)} />}
      {cloudAsk && (
        <div className="modal-overlay">
          <div className="consent">
            <div className="consent-icon">☁️</div>
            <h3>{t("Modalità Cloud — Liara 24B", "Cloud mode — Liara 24B")}</h3>
            <p className="consent-action">{t(
              "Molto più capace, ma i tuoi messaggi e i dati che l'assistente legge (file, memoria, posizione, foto) vengono inviati al server. NON è più tutto sul dispositivo.",
              "Far more capable, but your messages and the data the assistant reads (files, memory, location, photos) are sent to the server. It's no longer fully on-device.")}</p>
            {/* Consenso SEPARATO al salvataggio: spento di default. Solo se spuntato parte l'header
                x-liara-training:allow → il server conserva la conversazione anonimizzata (PII redatta). */}
            <label className="train-opt">
              <input type="checkbox" checked={trainConsent} onChange={(e) => { setTrainConsent(e.target.checked); haptic(10); }} />
              <span>{t(
                "Aiuta a migliorare Liara: salva le mie conversazioni in forma anonima (dati personali rimossi). Facoltativo, disattivabile quando vuoi.",
                "Help improve Liara: save my conversations anonymously (personal data removed). Optional, you can turn it off anytime.")}</span>
            </label>
            <div className="consent-btns">
              <button className="ghost" onClick={() => { setCloudAsk(false); haptic(12); }}>{t("Annulla", "Cancel")}</button>
              <button className="send-sm" onClick={() => {
                setCloudAsk(false);
                // Se un engine LOCALE è già caricato (non siamo sulla schermata di download né in avvio),
                // attivare il cloud "sul posto" lo lascia APPESO → la chat si impalla sull'ultima risposta.
                // Come per il cambio tra modelli LOCALI (chooseModel), va rilasciato con un RIAVVIO pulito:
                // persistiamo il cloud e mostriamo l'overlay "riavvia per applicare" (bottone esplicito
                // "Chiudi Liara ora", NON un exit a sorpresa). Al riavvio l'app riparte in cloud, engine
                // locale scaricato. Se invece NESSUN locale è caricato (cloud scelto all'avvio) → istantaneo.
                const localLoaded = !cloudMode && !md.needDownload && !initializing;
                setCloudMode(true); // persiste liara_cloud=1 (come setModelId nel cambio tra locali)
                if (localLoaded) md.setSwitchTo(t("Liara Cloud (24B)", "Liara Cloud (24B)"));
                haptic(20);
              }}>{t("☁️ Attiva cloud", "☁️ Enable cloud")}</button>
            </div>
          </div>
        </div>
      )}

      {viewImage && (
        <div className="modal-overlay" onClick={() => setViewImage(null)}>
          <img src={viewImage} className="lightbox-img" alt={t("allegato", "attachment")} onClick={(e) => e.stopPropagation()} />
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
          modelTag={cloudMode ? "☁️ Cloud 24B" : `${md.activeModel.tag} ${md.activeModel.flag}`}
          autoSpeak={autoSpeak}
          thinking={thinking}
          cloud={cloudMode}
          trainConsent={trainConsent}
          onClose={() => setShowMenu(false)}
          onProfile={() => { setShowMenu(false); prof.openProfile(); }}
          onEmail={() => { setShowMenu(false); email.openEmail(); }}
          onAgenda={() => { setShowMenu(false); agenda.openAgenda(); }}
          isAndroid={isAndroid}
          onContacts={() => { setShowMenu(false); contacts.openContacts(); }}
          onSms={() => { setShowMenu(false); sms.openSms(); }}
          onPerms={() => { setShowMenu(false); openPerms(); }}
          onTheme={() => { setShowMenu(false); setShowTheme(true); }}
          onModel={() => { setShowMenu(false); md.openModelDrawer(); }}
          onToggleVoice={() => { const v = !autoSpeak; setAutoSpeak(v); autoSpeakRef.current = v; if (!v) stopSpeak(); else haptic(20); }}
          voiceSid={voiceSid}
          onVoice={toggleVoice}
          respLen={respLen}
          onRespLen={cycleRespLen}
          onToggleThinking={() => { setThinking((v) => !v); haptic(20); }}
          onToggleCloud={() => toggleCloud(!cloudMode)}
          onToggleTrain={() => { setTrainConsent((v) => !v); haptic(20); }}
          onNet={() => { setShowMenu(false); setShowNet(true); }}
          chatNotif={chatNotif}
        />
      )}

      {showNet && <PeerHub onClose={() => setShowNet(false)} />}

      {showTheme && (
        <ThemeDrawer theme={theme} setTheme={setTheme} onBack={() => { setShowTheme(false); setShowMenu(true); }} onClose={() => setShowTheme(false)} />
      )}

      {md.showModel && (
        <ModelDrawer md={md} cloud={cloudMode} onCloud={toggleCloud} onBack={() => { md.setShowModel(false); setShowMenu(true); }} />
      )}

      {agenda.showAgenda && (
        <AgendaDrawer agenda={agenda} onBack={() => { agenda.setShowAgenda(false); setShowMenu(true); }} />
      )}

      {contacts.showContacts && (
        <ContactsDrawer contacts={contacts} onBack={() => { contacts.setShowContacts(false); setShowMenu(true); }} />
      )}

      {sms.showSms && (
        <SmsDrawer sms={sms} onBack={() => { sms.setShowSms(false); setShowMenu(true); }} />
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
