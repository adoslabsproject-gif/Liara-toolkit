//! The ReAct tool loop: the model thinks, calls tools, observes, and answers.
use super::format::{format_chat, Dialect, Message};
use super::parse::{extract_tool_call, strip_markers};
use super::stream::StreamRouter;
use crate::core::engine::{Engine, GenOptions};
use crate::core::tools::ToolRegistry;

/// Gli eventi che l'agente emette verso la UI, come UN unico oggetto invece di 4 closure sciolte
/// (review round-3 #4). Prima `run_agent` aveva 10 argomenti (`too_many_arguments`) e i due call-site
/// — generate.rs e vision.rs — duplicavano lo stesso wiring; ora entrambi passano un solo `&mut dyn
/// AgentSink` (l'impl condivisa `WindowSink` vive in `commands/`, che conosce Tauri; il core resta puro).
pub trait AgentSink {
    /// Un pezzo di risposta in streaming (già ripulito da tool-call/reasoning nascosti).
    fn on_token(&mut self, piece: &str);
    /// Un tool sta per essere eseguito (nome + argomenti JSON).
    fn on_tool(&mut self, name: &str, args: &str);
    /// Il risultato (cappato) di un tool appena eseguito.
    fn on_tool_result(&mut self, name: &str, result: &str);
    /// Consenso per un tool sensibile: `true` = concesso. Bloccante (chiede all'utente se serve).
    fn on_consent(&mut self, tool: &str, action: &str) -> bool;
}

/// Costruisce il blocco `<tool_call>` del tool-forcing con `serde` (review round-3 #2): il JSON è
/// corretto-per-costruzione (escaping garantito), niente più `format!` + strip dei caratteri a mano
/// — una località come `Reggio nell'Emilia` o con virgolette non può più rompere il JSON.
fn forced_call(name: &str, args: serde_json::Value) -> String {
    let call = serde_json::json!({ "name": name, "arguments": args });
    format!("<tool_call>\n{call}\n</tool_call>")
}

/// Il risultato di un tool iniettato nel prompt, nel formato NATIVO del dialetto. Qwen:
/// `<tool_response>…`. Mistral: `[TOOL_RESULTS]…[/TOOL_RESULTS]`. Gemma/Cohere: testo neutro —
/// imiterebbero un marker estraneo e lo leakerebbero nella risposta.
fn tool_resp(result: &str, dialect: Dialect) -> String {
    match dialect {
        Dialect::Qwen => format!("<tool_response>\n{result}\n</tool_response>"),
        Dialect::Mistral => format!("[TOOL_RESULTS]{result}[/TOOL_RESULTS]"),
        Dialect::Gemma | Dialect::Cohere => format!("Risultato dello strumento:\n{result}"),
    }
}

/// Come registrare nel contesto (turno assistant) il tool-call appena emesso, nel formato che QUEL
/// modello emette davvero (train==runtime). Qwen: `<tool_call>{…}`. Mistral: `[TOOL_CALLS][{…}]`.
/// Gemma/Cohere: una nota in linguaggio naturale — se rivedessero un blocco di un altro dialetto nel
/// PROPRIO turno lo imiterebbero, tornando a leakare i marker; la frase dà lo stesso grounding.
fn assistant_toolcall(name: &str, args: &serde_json::Value, dialect: Dialect) -> String {
    match dialect {
        Dialect::Qwen => format!("<tool_call>\n{{\"name\": \"{name}\", \"arguments\": {args}}}\n</tool_call>"),
        Dialect::Mistral => format!("[TOOL_CALLS][{{\"name\": \"{name}\", \"arguments\": {args}}}]"),
        Dialect::Gemma | Dialect::Cohere => format!("Ho usato lo strumento {name} con argomenti {args}."),
    }
}

/// Stop di generazione durante il loop tool: l'EOS ATOMICO del dialetto + il chiusura del blocco
/// tool-call (dove esiste come marker), così la generazione si ferma subito dopo la chiamata.
fn gen_stops(dialect: Dialect) -> Vec<String> {
    match dialect {
        Dialect::Qwen => vec!["</tool_call>".into(), "<|im_end|>".into()],
        // `<turn|>` è il vecchio marker Gemma: lasciato come stop DIFENSIVO finché i Gemma non sono
        // ri-allenati sul nativo, così un modello vecchio almeno termina (non papiro) nella transizione.
        Dialect::Gemma => vec!["<tool_call|>".into(), "</tool_call>".into(), "<end_of_turn>".into(), "<turn|>".into()],
        Dialect::Mistral => vec!["</s>".into()],
        Dialect::Cohere => vec!["<|END_OF_TURN_TOKEN|>".into()],
    }
}

/// Stop per la risposta FINALE: solo l'EOS atomico del dialetto (niente tool → niente chiusure tool-call).
fn final_stops(dialect: Dialect) -> Vec<String> {
    match dialect {
        Dialect::Qwen => vec!["<|im_end|>".into()],
        Dialect::Gemma => vec!["<end_of_turn>".into(), "<turn|>".into()], // <turn|> difensivo (transizione)
        Dialect::Mistral => vec!["</s>".into()],
        Dialect::Cohere => vec!["<|END_OF_TURN_TOKEN|>".into()],
    }
}

/// ReAct loop: the model thinks, optionally calls tools (Qwen native format),
/// observes results, and answers. Streams the answer; surfaces tool steps.
#[allow(clippy::too_many_arguments)]
/// Ultimi 3 messaggi utente concatenati (minuscoli): è il testo su cui si scelgono le FAMIGLIE di
/// tool da mettere nel prompt. Vedi il commento nel corpo di run_agent (fix "e domani?").
fn routing_window(messages: &[Message]) -> String {
    let mut recent: Vec<&str> = messages
        .iter()
        .rev()
        .filter(|m| m.role == "user")
        .take(3)
        .map(|m| m.content.as_str())
        .collect();
    recent.reverse();
    recent.join("\n").to_lowercase()
}

pub fn run_agent(
    engine: &dyn Engine,
    registry: &ToolRegistry,
    base_system: &str,
    messages: &[Message],
    thinking: bool,
    max_tokens: usize, // budget risposta (regolabile dall'utente: Breve/Media/Lunga/Massima per dispositivo)
    temperature: f32,  // creatività (regolabile dall'utente PER MODELLO locale; il cloud non passa di qui)
    cancel: &std::sync::atomic::AtomicBool,
    sink: &mut dyn AgentSink,
) -> anyhow::Result<String> {
    let mut convo: Vec<Message> = messages.to_vec();
    // Gemma parla un dialetto diverso da Qwen (formato prompt + token di stop). Lo determiniamo UNA volta.
    let dialect = engine.dialect();
    // the user's latest request: drives tool routing + the selection guard
    let user_request: String = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.to_lowercase())
        .unwrap_or_default();
    // FINESTRA DI ROUTING (fix "e domani?"): la selezione delle famiglie tool guarda gli ULTIMI 3
    // messaggi utente, non solo l'ultimo. Nei follow-up ellittici ("e domani?", "sempre modena")
    // l'intento vive nei turni precedenti: con la sola ultima frase la famiglia (es. weather)
    // SPARIVA dal prompt e il modello girava a vuoto senza poter richiamare il tool. Il forcing
    // e le guardie d'intento restano sull'ULTIMO messaggio (forzare su intenti vecchi = errori).
    let routing_window = routing_window(messages);
    // raw (non-lowercased) latest user message → URL extraction for tool-forcing
    let user_msg: String = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();
    let forced_url = extract_url(&user_msg);
    // se NON c'è un URL ma c'è chiaro intento di ricerca → forziamo web_search (il modello piccolo spesso
    // "scrive" la chiamata come testo invece di eseguirla → niente risultati → allucina le notizie).
    let forced_search = if forced_url.is_none() { forced_search_query(&user_request, messages) } else { None };
    // stesso problema col METEO: il modello piccolo (specie E4B) spesso risponde a parole invece di
    // chiamare `weather`. Se l'intento meteo è chiaro, forziamo NOI il tool con la località estratta
    // (Some("") = nessuna città → posizione IP). Affidabilità ~100% indipendente dal modello.
    let forced_weather = if forced_url.is_none() && forced_search.is_none() {
        weather_query(&user_request)
            .or_else(|| weather_followup_city(&routing_window, &user_request))
    } else {
        None
    };
    let mut forced_used = false;

    // always give the model the current date/time so it can resolve relative dates,
    // and include ONLY the tools relevant to the request (smaller prompt, better choice)
    let now = registry.execute("datetime", &serde_json::json!({})).unwrap_or_default();
    let base = format!("Data e ora correnti: {now}.\n{base_system}");
    let system = if registry.is_empty() {
        base
    } else {
        format!("{}{}", base, registry.prompt_block_for(&routing_window))
    };
    // GBNF: forces a valid tool-call JSON once the model emits <tool_call> (lazy in the engine).
    // SOLO Qwen: Gemma emette il SUO formato nativo (`<|tool_call>call:…`), su cui la grammar Qwen
    // (ancorata a `<tool_call>` senza pipe) non scatterebbe comunque — la teniamo spenta per non
    // rischiare interferenze e lasciare Gemma libero di produrre il dialetto che il parser capisce.
    // ✅ GBNF RIATTIVATA per default (2026-07-14): il crash "foreign exception" era il `throw` su dead-end
    // in llama-grammar.cpp ("Unexpected empty grammar stack") che attraversava l'FFI → SIGABRT. Ora quel
    // throw è reso NON-FATALE nel vendored (su dead-end la grammatica smette di vincolare → sampling libero,
    // il parser recupera il JSON): niente più crash. Con la grammatica ON il tool-call è deterministicamente
    // ben formato. Kill-switch runtime SENZA rebuild: `LIARA_GBNF=0` la disattiva. (SOLO Qwen: Gemma usa il
    // suo dialetto nativo, su cui la grammar ancorata a `<tool_call>` non scatterebbe → la lasciamo libera.)
    let gbnf_on = std::env::var("LIARA_GBNF").map(|v| v != "0").unwrap_or(true);
    // 🔑 GRAMMATICA = SOTTOINSIEME ROUTED (non i 30 tool): il modello vede E può chiamare SOLO i
    // pochi strumenti pertinenti (stesso set del prompt, stessa `routing_window`). Un nome inventato
    // o fuori-contesto è impossibile da emettere → il crollo degli errori di tool-calling. È la
    // singola leva con più resa, tutta a costo-zero di training.
    // GBNF SOLO per ChatML/Qwen: Gemma/Mistral/Cohere emettono il loro tool-call nativo (non
    // `<tool_call>` su cui la grammar è ancorata) e si affidano al training + all'EOS atomico.
    let grammar = (!registry.is_empty() && dialect == Dialect::Qwen && gbnf_on)
        .then(|| registry.tool_call_grammar_for(&routing_window));

    for _ in 0..5 {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        // `raw` è l'output grezzo dell'iterazione: o il tool-call iniettato (forcing), o ciò che il
        // modello ha generato (via StreamRouter). Assegnato in OGNI ramo → nessun valore iniziale morto.
        let raw: String;
        // TOOL-FORCING: se l'utente ha nominato un URL/dominio, web_fetch è OBBLIGATORIO. Non deleghiamo
        // la decisione a un modello (specie se piccolo): alla 1ª iterazione iniettiamo NOI il tool-call,
        // e il resto del loop lo processa come se l'avesse emesso il modello (consenso, esecuzione,
        // risposta). Affidabilità ~100% sui casi inequivocabili, indipendente dalla bravura del modello.
        let can_force = !forced_used && !registry.is_empty();
        if can_force && forced_url.is_some() {
            forced_used = true;
            raw = forced_call("web_fetch", serde_json::json!({ "url": forced_url.as_deref().unwrap_or("") }));
        } else if can_force && forced_search.is_some() {
            forced_used = true;
            raw = forced_call("web_search", serde_json::json!({ "query": forced_search.as_deref().unwrap_or("").trim() }));
        } else if can_force && forced_weather.is_some() {
            forced_used = true;
            let city = forced_weather.as_deref().unwrap_or("").trim();
            // città estratta → weather{location}; vuota → weather{} (il tool usa la posizione IP)
            let args = if city.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::json!({ "location": city })
            };
            raw = forced_call("weather", args);
        } else {
            let prompt = format_chat(&system, &convo, thinking, dialect);
            let opts = GenOptions {
                // #2 FIX: budget più ampio (era 700) così, col reasoning ON, il <think> non affama la
                // risposta. Il papiro-loop è già contenuto dalla repeat/frequency penalty (llama.rs
                // build_sampler, finestra 256 + freq/presence 0.4). Un cap SEPARATO sul <think> (stop a
                // </think> + reset del budget) sarebbe l'ideale ma richiede due fasi nello streaming;
                // la coppia penalty+budget rende il papiro raro. Il budget è ora REGOLABILE dall'utente
                // (preset Breve/Media/Lunga/Massima, con max per-dispositivo) → passato da generate().
                max_tokens,
                temperature,
                stop: gen_stops(dialect),
                grammar: grammar.clone(),
                ..Default::default()
            };
            // Router di streaming testabile (review round-3 #4): nasconde tool-call e thinking-channel,
            // trattiene le code-parziali di marker, emette solo la prosa buona. Vedi `stream.rs`.
            let mut router = StreamRouter::new();
            engine.generate(&prompt, &opts, cancel, &mut |piece| {
                let out = router.push(piece);
                if !out.is_empty() {
                    sink.on_token(&out);
                }
            })?;
            let tail = router.finish();
            if !tail.is_empty() {
                sink.on_token(&tail);
            }
            raw = router.into_raw();
        }

        if let Some((mut name, args)) = extract_tool_call(&raw) {
            // intent guard: a weak model often confuses received vs sent email
            if name == "email_recent"
                && ["inviat", "spedit", "mandat"].iter().any(|h| user_request.contains(h))
            {
                name = "email_sent".to_string();
            } else if name == "email_sent"
                && ["ricevut", "in arrivo", "arrivata"].iter().any(|h| user_request.contains(h))
            {
                name = "email_recent".to_string();
            }
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            // consent gate: sensitive tools need the user's OK (argument-aware), enforced here
            if registry.is_sensitive(&name) {
                let action = registry.consent_action(&name, &args);
                if !sink.on_consent(&name, &action) {
                    let result = format!("Permesso negato dall'utente: {action}.");
                    sink.on_tool_result(&name, &result);
                    convo.push(Message {
                        role: "assistant".into(),
                        content: assistant_toolcall(&name, &args, dialect),
                    });
                    convo.push(Message {
                        role: "user".into(),
                        content: tool_resp(&result, dialect),
                    });
                    continue;
                }
            }
            sink.on_tool(&name, &args.to_string());
            let result = cap_tool_result(
                registry.execute(&name, &args).unwrap_or_else(|e| format!("Errore: {e}")),
            );
            sink.on_tool_result(&name, &result);
            convo.push(Message {
                role: "assistant".into(),
                content: assistant_toolcall(&name, &args, dialect),
            });
            convo.push(Message {
                role: "user".into(),
                content: tool_resp(&result, dialect),
            });
            continue;
        }
        return Ok(strip_markers(&raw));
    }
    // out of tool steps: force a plain answer (no tools, no technical message)
    let prompt = format_chat(base_system, &convo, thinking, dialect);
    let opts = GenOptions {
        max_tokens: 400,
        temperature,
        stop: final_stops(dialect),
        ..Default::default()
    };
    let mut out = String::new();
    engine.generate(&prompt, &opts, cancel, &mut |p| {
        out.push_str(p);
        sink.on_token(p);
    })?;
    Ok(strip_markers(&out))
}

/// Cap sul risultato tool prima che entri nel contesto (anti-overflow). DEVE stare
/// SOPRA i budget interni dei tool web: web_fetch rende fino a ~4100 char e
/// web_search ~900 di risultati + ~2600 di "📄 contenuto del 1° risultato".
/// FIX (review 2026-07-02 #6): il vecchio cap 1500 troncava via PROPRIO il
/// contenuto della pagina allegato per l'anti-allucinazione → feature morta
/// all'arrivo (il modello tornava a inventare).
const MAX_TOOL_RESULT_CHARS: usize = 6000;

fn cap_tool_result(result: String) -> String {
    if result.chars().count() > MAX_TOOL_RESULT_CHARS {
        result.chars().take(MAX_TOOL_RESULT_CHARS).collect::<String>() + "\n…(troncato)"
    } else {
        result
    }
}

/// Estrae il primo URL o dominio nudo dal testo dell'utente (per il tool-forcing di web_fetch).
/// Riconosce `http(s)://...` e domini tipo `esempio.com` con un TLD noto. `None` se non c'è nulla
/// di chiaramente navigabile (così non forziamo web_fetch quando l'utente non intende un sito).
fn extract_url(text: &str) -> Option<String> {
    const TLDS: &[&str] = &[
        ".com", ".it", ".org", ".net", ".io", ".dev", ".ai", ".co", ".eu", ".info", ".me", ".app",
        ".gov", ".edu", ".tv", ".news", ".shop", ".store", ".xyz", ".cloud", ".online",
    ];
    for tok in text.split(char::is_whitespace) {
        // trim punteggiatura ai bordi, ma tieni i caratteri validi di un URL
        let t = tok.trim_matches(|c: char| {
            !c.is_alphanumeric() && !matches!(c, '/' | ':' | '.' | '-' | '_' | '~' | '?' | '=' | '&' | '%' | '#')
        });
        if t.is_empty() || t.contains('@') {
            continue; // salta vuoti ed email
        }
        if t.starts_with("http://") || t.starts_with("https://") {
            return Some(t.to_string());
        }
        let lower = t.to_lowercase();
        if let Some(end) = TLDS.iter().find_map(|tld| lower.find(tld).map(|p| p + tld.len())) {
            // #14 FIX: il TLD deve CHIUDERE l'host — dopo di esso solo fine-token o '/' ':' '?' '#'.
            // Se segue '.' o una lettera (es. "note.io.txt", "logo.ai.png", "relazione.io.bak") è un
            // file locale, non un dominio → non forziamo web_fetch bypassando fs_read.
            let closes = matches!(
                lower.as_bytes().get(end).copied(),
                None | Some(b'/') | Some(b':') | Some(b'?') | Some(b'#')
            );
            if closes {
                let candidate = &t[..end.min(t.len())];
                if let Some(dot) = candidate.find('.') {
                    if dot > 0 {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Rileva l'intento di ricerca web e restituisce la query (None se non è una ricerca). Forziamo
/// web_search su questi pattern: il modello locale spesso "scrive" la chiamata invece di eseguirla.
fn search_query(req_lower: &str) -> Option<String> {
    // #3 FIX: se l'intento è LOCALE (email/agenda/file/note) NON forziamo web_search — "cerca l'email di
    // Mario" deve andare a email_search, non a DuckDuckGo. Il forcing web resta per le ricerche vere.
    const LOCAL: &[&str] = &[
        "email", "mail", "posta", "casella", "appuntament", "agenda", "evento", "calendario", "impegno",
        "scadenz", "riunion", "file", "cartella", "document", "appunt", "annota", "i miei appunt",
    ];
    if LOCAL.iter().any(|k| req_lower.contains(k)) {
        return None;
    }
    const TRIGGERS: &[&str] = &[
        "cerca", "notizie", "ultime notiz", "che succede", "cosa è successo", "cosa e successo",
        "novità", "novita", "in rete", "sul web", "su internet", "online", "aggiornament", "trova",
    ];
    if !TRIGGERS.iter().any(|t| req_lower.contains(t)) {
        return None;
    }
    let mut q = req_lower.to_string();
    // NB: i pattern PIÙ LUNGHI prima — "cerca" da solo mangiava "cercare" lasciando "re" nella
    // query ("non puoi cercare zeli…" → "non puoi re zeli…" → risultati spazzatura, caso 17/07).
    for p in [
        "non puoi cercare", "puoi cercare", "riesci a cercare", "cerca in rete", "cerca sul web",
        "cerca su internet", "cerca online", "cerca per me", "cercare", "cercami", "cercale",
        "cercalo", "cerca", "e vedere i risultati", "vedere i risultati", "i risultati",
        "puoi vedere", "vedere", "puoi darmi", "darmi", "dammi", "in rete", "sul web",
        "su internet", "trova", "dimmi", "non puoi", "puoi ",
    ] {
        q = q.replace(p, " ");
    }
    let q = q.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(if q.chars().count() < 3 { req_lower.trim().to_string() } else { q })
}

/// Rileva l'intento METEO e ne estrae la località. `Some("Modena")` = meteo di una città precisa;
/// `Some("")` = meteo senza città → il tool usa la posizione IP; `None` = non è una richiesta meteo.
/// Rimuove i trigger e le stopword (temporali/cortesia) lasciando la sola località, come `search_query`.
/// Trigger dell'intento meteo, condivisi dal forcing diretto (weather_query) e dal forcing
/// slot-filling sul follow-up (weather_followup_city). Allineati alle keyword del router.
const WEATHER_TRIGGERS: &[&str] = &[
    "che tempo", "tempo fa", "il tempo", "tempo domani", "meteo", "temperatura", "previsioni",
    "pioggia", "pioverà", "piovera", "piove", "gradi ci sono", "gradi fa", "fa freddo", "fa caldo",
    "c'è il sole",
];

fn weather_query(req_lower: &str) -> Option<String> {
    if !WEATHER_TRIGGERS.iter().any(|t| req_lower.contains(t)) {
        return None;
    }
    // 1) ROBUSTO: la città è ciò che segue una preposizione di luogo ("a/ad Milano"), anche con
    //    parole in mezzo ("che tempo fa nel weekend a Milano" → "milano"). "a"/"ad" dopo un trigger
    //    meteo introducono quasi sempre un luogo — molto più affidabile del "sottrai e spera".
    for prep in [" a ", " ad "] {
        if let Some(i) = req_lower.rfind(prep) {
            let city = clean_city(&req_lower[i + prep.len()..]);
            if city.chars().count() >= 2 {
                return Some(city);
            }
        }
    }
    // 2) FALLBACK (frasi senza preposizione): sottrai trigger e stopword, tieni ciò che resta.
    let mut q = req_lower.to_string();
    // NB: i pattern più lunghi PRIMA (così "che tempo fa" sparte prima di "che ")
    for p in [
        "che tempo fa", "che tempo", "tempo fa", "il tempo", "tempo domani", "il meteo", "meteo",
        "temperatura", "previsioni del tempo", "previsioni meteo", "previsioni",
        "quanti gradi ci sono", "quanti gradi", "gradi ci sono", "gradi fa", "fa freddo",
        "fa caldo", "c'è il sole", "pioverà", "piovera", "pioggia", "piove", "ho bisogno del",
        "ho bisogno di", "mi serve il", "mi serve", "puoi dirmi", "potresti dirmi", "dirmi",
        "dimmi", "mi dici", "sapere", "vorrei", "puoi ", "qual è", "qual e", "com'è", "come è",
        "che ", " di ", " in ", " su ", " per ",
    ] {
        q = q.replace(p, " ");
    }
    let city = clean_city(&q);
    Some(if city.chars().count() < 2 { String::new() } else { city })
}

/// Query di ricerca fatta SOLO di pronomi/particelle/interrogativi ("perchè ?? le" da "Perchè??
/// cercale in rete"): non c'è un oggetto da cercare — l'oggetto vive nel turno precedente (anafora).
fn degenerate_search_query(q: &str) -> bool {
    const STOP: &[&str] = &[
        "le", "lo", "la", "li", "gli", "l", "quelle", "quelli", "quella", "quello", "questo",
        "questa", "queste", "questi", "perché", "perchè", "perche", "cosa", "che", "come", "mai",
        "ora", "adesso", "oggi", "domani", "subito", "allora", "si", "sì", "no", "ma", "poi",
        "pure", "anche", "ancora", "dai", "su",
    ];
    q.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .all(|w| w.chars().count() <= 2 || STOP.contains(&w))
}

/// Forcing web con risoluzione dell'ANAFORA (caso reale 17/07): "puoi darmi le notizie di oggi?"
/// → il modello nega → "Perchè?? cercale in rete" → la query ripulita è un moncone di pronomi →
/// si riusa l'oggetto del turno utente PRECEDENTE ("le notizie di oggi"), non la spazzatura.
fn forced_search_query(user_request: &str, messages: &[Message]) -> Option<String> {
    let q = search_query(user_request)?;
    if !degenerate_search_query(&q) {
        return Some(q);
    }
    messages
        .iter()
        .rev()
        .filter(|m| m.role == "user")
        .nth(1)
        .and_then(|m| search_query(&m.content.to_lowercase()))
        .filter(|p| !degenerate_search_query(p))
        .or(Some(q))
}

/// Forcing SLOT-FILLING del meteo (caso reale 17/07): nei turni recenti c'era una richiesta meteo
/// e l'utente risponde SOLO col luogo ("modena") — l'ultima frase non ha trigger, weather_query
/// tace, e il modello piccolo richiede la città all'infinito pur avendola appena ricevuta. Se
/// l'ultima frase è corta, non è un convenevole e non è un altro comando, la trattiamo come la
/// città e forziamo NOI il meteo.
fn weather_followup_city(window_lower: &str, last_lower: &str) -> Option<String> {
    let l = last_lower.trim();
    // il trigger deve stare nei turni PRECEDENTI (se è nell'ultima frase ci pensa weather_query)
    let before = window_lower.strip_suffix(l).unwrap_or(window_lower);
    if !WEATHER_TRIGGERS.iter().any(|t| before.contains(t)) {
        return None;
    }
    if l.is_empty() || l.chars().count() > 30 || l.split_whitespace().count() > 3 {
        return None;
    }
    const NON_LUOGHI: &[&str] = &[
        "grazie", "ok", "okay", "va bene", "si", "sì", "no", "perfetto", "ciao", "certo",
        "niente", "boh", "forse", "non lo so", "aspetta", "lascia stare", "grazie mille",
    ];
    if NON_LUOGHI.iter().any(|a| l == *a) {
        return None;
    }
    // un altro comando non è un luogo ("leggi le email" dopo una domanda meteo)
    const ALTRI_INTENTI: &[&str] =
        &["email", "mail", "nota", "note", "agenda", "appunt", "chiama", "cerca", "file", "sms"];
    if ALTRI_INTENTI.iter().any(|w| l.contains(w)) {
        return None;
    }
    let city = clean_city(l);
    (city.chars().count() >= 2).then_some(city)
}

/// Ripulisce la coda estratta da una città: toglie parole di tempo, articoli/preposizioni residui e
/// punteggiatura, lasciando solo il nome del luogo (eventualmente multi-parola, es. "reggio emilia").
fn clean_city(s: &str) -> String {
    let mut c = format!(" {} ", s.replace(['?', '!', '.', ','], " "));
    for w in [
        " oggi ", " domani ", " dopodomani ", " stasera ", " stamattina ", " stanotte ",
        " adesso ", " ora ", " in questo momento ", " questo momento ", " questo ", " momento ",
        " nel weekend ", " weekend ", " questa settimana ", " la prossima settimana ",
        " prossima settimana ", " il ", " lo ", " la ", " i ", " le ", " per ", " nel ", " nella ",
    ] {
        c = c.replace(w, " ");
    }
    c.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{cap_tool_result, extract_url, weather_followup_city, weather_query, MAX_TOOL_RESULT_CHARS};

    #[test]
    fn search_query_pulita_senza_monconi() {
        // NESSUNA whitelist: la subtraction toglie cortesia/comando e la query è ciò che RESTA,
        // qualunque sia il soggetto. ("zeli" qui sotto è solo il caso reale che aprì il bug:
        // "cerca" mangiava "cercare" lasciando "re" nella query.)
        let q = super::search_query("non puoi cercare zeli in rete e vedere i risultati?").unwrap();
        assert!(q.contains("zeli"), "query: {q}");
        assert!(!q.contains(" re "), "moncone 're' nella query: {q}");
        // soggetti qualsiasi: resta l'oggetto della ricerca, pulito
        let q = super::search_query("cerca in rete la storia della ferrari").unwrap();
        assert!(q.contains("ferrari"), "query: {q}");
        let q = super::search_query("puoi darmi le notizie di oggi?").unwrap();
        assert!(q.contains("notizie"), "query: {q}");
        let q = super::search_query("trova una ricetta della carbonara").unwrap();
        assert!(q.contains("carbonara"), "query: {q}");
    }

    #[test]
    fn anafora_riusa_l_oggetto_del_turno_prima() {
        use super::{degenerate_search_query, forced_search_query};
        use crate::core::agent::format::Message;
        // "perchè ?? le" = solo pronomi/interrogativi → degenerata; un soggetto vero no
        assert!(degenerate_search_query("perchè le"));
        assert!(!degenerate_search_query("notizie di oggi"));
        assert!(!degenerate_search_query("storia della ferrari"));
        // caso reale: "Perchè?? cercale in rete" dopo "puoi darmi le notizie di oggi?" →
        // la query forzata deve essere l'OGGETTO del turno prima, non "perchè le"
        let msgs = vec![
            Message { role: "user".into(), content: "puoi darmi le notizie di oggi?".into() },
            Message { role: "assistant".into(), content: "non posso…".into() },
            Message { role: "user".into(), content: "Perchè?? cercale in rete".into() },
        ];
        let q = forced_search_query("perchè?? cercale in rete", &msgs).unwrap();
        assert!(q.contains("notizie"), "query anaforica: {q}");
    }

    #[test]
    fn weather_forme_naturali_forzano_il_tool() {
        // caso reale 17/07: "puoi dirmi il tempo per domani?" non innescava il forcing
        assert_eq!(weather_query("puoi dirmi il tempo per domani?").as_deref(), Some("")); // "" = posizione IP
        assert_eq!(weather_query("il tempo a modena domani?").as_deref(), Some("modena"));
        assert_eq!(weather_query("com'è il tempo a bari").as_deref(), Some("bari"));
    }

    #[test]
    fn weather_followup_slot_filling() {
        let win = "puoi dirmi il tempo per domani?\nmodena";
        // l'utente risponde SOLO col luogo dopo una domanda meteo → forziamo weather(luogo)
        assert_eq!(weather_followup_city(win, "modena").as_deref(), Some("modena"));
        assert_eq!(weather_followup_city("che tempo fa?\nreggio emilia", "reggio emilia").as_deref(), Some("reggio emilia"));
        // convenevoli e altri comandi NON sono luoghi
        assert_eq!(weather_followup_city("che tempo fa?\ngrazie", "grazie"), None);
        assert_eq!(weather_followup_city("che tempo fa?\nleggi le email", "leggi le email"), None);
        // senza intento meteo nei turni PRECEDENTI, niente forcing
        assert_eq!(weather_followup_city("ciao come stai?\nmodena", "modena"), None);
        assert_eq!(weather_followup_city("il tempo per domani?", "il tempo per domani?"), None); // trigger solo nell'ultima → ci pensa weather_query
    }

    #[test]
    fn weather_estrae_citta() {
        assert_eq!(weather_query("che tempo fa a modena oggi?").as_deref(), Some("modena"));
        assert_eq!(weather_query("ho bisogno del meteo a modena").as_deref(), Some("modena"));
        assert_eq!(weather_query("previsioni per roma domani").as_deref(), Some("roma"));
    }

    #[test]
    fn weather_citta_dopo_parole_di_mezzo() {
        // #2 ANTI-REGRESSIONE: il caso che prima SBAGLIAVA ("sottrai e spera" lasciava "weekend milano").
        // Ora la città è ciò che segue "a"/"ad" → robusto anche con parole di tempo in mezzo.
        assert_eq!(weather_query("che tempo fa nel weekend a milano").as_deref(), Some("milano"));
        assert_eq!(weather_query("previsioni meteo domani ad ancona").as_deref(), Some("ancona"));
        assert_eq!(weather_query("meteo a modena questa settimana").as_deref(), Some("modena"));
    }

    #[test]
    fn weather_senza_citta_usa_posizione_ip() {
        // meteo SENZA località → Some("") = il tool weather userà la posizione IP
        assert_eq!(weather_query("che tempo fa oggi?").as_deref(), Some(""));
        assert_eq!(weather_query("mi serve il meteo").as_deref(), Some(""));
    }

    #[test]
    fn weather_ignora_non_meteo() {
        // ANTI-REGRESSIONE: nessun trigger meteo → None (non forziamo weather a sproposito)
        assert!(weather_query("mandami l'email a mario").is_none());
        assert!(weather_query("che ore sono").is_none());
        assert!(weather_query("apri il file relazione").is_none());
    }

    #[test]
    fn weather_citta_multiparola() {
        assert_eq!(weather_query("meteo a reggio emilia").as_deref(), Some("reggio emilia"));
    }

    #[test]
    fn finds_bare_domain_and_url() {
        assert_eq!(extract_url("Vai su automazionezeli.com e dimmi che sito è").as_deref(), Some("automazionezeli.com"));
        assert_eq!(extract_url("apri https://example.org/path ora").as_deref(), Some("https://example.org/path"));
        assert_eq!(extract_url("scrivi a mario@test.com").as_deref(), None); // email, non navigazione
        assert_eq!(extract_url("che ore sono?"), None);
    }

    #[test]
    fn cap_lascia_passare_un_output_web_search_completo() {
        // ANTI-REGRESSIONE #6: un output della taglia di web_search (risultati +
        // contenuto del 1° risultato ≈ 3700 char) deve arrivare INTERO al modello.
        // Col vecchio cap 1500 questo test è ROSSO (mutation-verify).
        let payload = "R".repeat(3700);
        assert_eq!(cap_tool_result(payload.clone()), payload);
        // e web_fetch pieno (~4100) pure
        let fetch = "F".repeat(4100);
        assert_eq!(cap_tool_result(fetch.clone()), fetch);
    }

    #[test]
    fn cap_tronca_oltre_il_limite_con_marcatore() {
        let huge = "X".repeat(MAX_TOOL_RESULT_CHARS + 500);
        let capped = cap_tool_result(huge);
        assert!(capped.ends_with("…(troncato)"));
        assert!(capped.chars().count() <= MAX_TOOL_RESULT_CHARS + 20);
    }
}

/// Test d'INTEGRAZIONE del ReAct loop (review round-4): prima `run_agent` — il cuore
/// dell'orchestrazione — non aveva copertura. Con un `Engine` finto (output prescritti) esercitiamo
/// il ciclo completo: risposta semplice, chiamata+esecuzione+grounding di un tool, e il gate di
/// consenso (negato → non esegue, concesso → esegue). Hermetico: solo tool locali, nessuna rete.
#[cfg(test)]
mod integration {
    use super::{routing_window, run_agent, AgentSink, Message};
    use crate::core::calendar::Calendar;
    use crate::core::crypto::Crypto;
    use crate::core::email::EmailStore;
    use crate::core::engine::{Engine, GenOptions};
    use crate::core::memory::Memory;
    use crate::core::tools::ToolRegistry;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    /// Engine finto: a ogni `generate` restituisce (ed emette) il prossimo output prescritto.
    struct FakeEngine {
        scripted: Mutex<Vec<String>>,
    }
    impl FakeEngine {
        fn new(outs: &[&str]) -> Self {
            Self { scripted: Mutex::new(outs.iter().map(|s| s.to_string()).collect()) }
        }
    }
    impl Engine for FakeEngine {
        fn id(&self) -> &str {
            "fake"
        }
        fn generate(&self, _p: &str, _o: &GenOptions, _c: &AtomicBool, on_token: &mut dyn FnMut(&str)) -> anyhow::Result<String> {
            let mut s = self.scripted.lock().unwrap();
            let out = if s.is_empty() { String::new() } else { s.remove(0) };
            on_token(&out);
            Ok(out)
        }
        fn embed(&self, _t: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![0.0])
        }
    }

    #[derive(Default)]
    struct RecSink {
        tokens: String,
        tools: Vec<String>,
        consent: Vec<String>,
        allow: bool,
    }
    impl AgentSink for RecSink {
        fn on_token(&mut self, p: &str) {
            self.tokens.push_str(p);
        }
        fn on_tool(&mut self, n: &str, _a: &str) {
            self.tools.push(n.to_string());
        }
        fn on_tool_result(&mut self, _n: &str, _r: &str) {}
        fn on_consent(&mut self, tool: &str, _action: &str) -> bool {
            self.consent.push(tool.to_string());
            self.allow
        }
    }

    fn registry() -> ToolRegistry {
        let crypto = Arc::new(Crypto::from_key(&[4u8; 32]));
        let mem = Arc::new(Memory::open(":memory:", crypto.clone()).unwrap());
        let email = Arc::new(EmailStore::open(":memory:", crypto.clone()).unwrap());
        let cal = Arc::new(Calendar::open(":memory:", crypto).unwrap());
        ToolRegistry::build(email, Arc::new(Mutex::new(None)), cal, mem)
    }

    fn drive(eng: &FakeEngine, user: &str, sink: &mut RecSink) -> String {
        let never = AtomicBool::new(false);
        let msgs = vec![Message { role: "user".into(), content: user.into() }];
        run_agent(eng, &registry(), "sys", &msgs, false, 1024, 0.7, &never, sink).unwrap()
    }

    #[test]
    fn follow_up_ellittico_tiene_il_tool_nel_prompt() {
        // 🔴 caso reale (2.6B, 17/07): "che tempo farà a modena?" → ok; "e domani?" → il router
        // guardava SOLO l'ultima frase → weather SPARIVA dal prompt → il modello girava a vuoto
        // ("dimmi la città e cerco") senza poter chiamare il tool. La finestra di routing sui
        // 3 messaggi utente recenti deve tenere la famiglia attiva.
        let msgs = vec![
            Message { role: "user".into(), content: "che tempo farà a Modena oggi?".into() },
            Message { role: "assistant".into(), content: "A Modena oggi 34 gradi, sereno.".into() },
            Message { role: "user".into(), content: "e domani?".into() },
        ];
        let win = routing_window(&msgs);
        assert!(registry().prompt_block_for(&win).contains("\"weather\""),
            "il follow-up ellittico deve mantenere weather nel prompt");
        // e con la finestra piena di altro, il meteo di 4+ turni fa decade (niente prompt eterno)
        let old = vec![
            Message { role: "user".into(), content: "che tempo fa?".into() },
            Message { role: "user".into(), content: "leggi le email".into() },
            Message { role: "user".into(), content: "che appuntamenti ho".into() },
            Message { role: "user".into(), content: "raccontami una storia".into() },
        ];
        assert!(!registry().prompt_block_for(&routing_window(&old)).contains("\"weather\""));
    }

    #[test]
    fn risposta_semplice_senza_tool() {
        let eng = FakeEngine::new(&["Ciao! Come stai?"]);
        let mut sink = RecSink::default();
        let ans = drive(&eng, "ciao", &mut sink);
        assert_eq!(ans, "Ciao! Come stai?");
        assert!(sink.tools.is_empty(), "un saluto non deve chiamare tool");
    }

    #[test]
    fn ciclo_react_esegue_il_tool_e_risponde() {
        let eng = FakeEngine::new(&[
            "<tool_call>\n{\"name\": \"calculator\", \"arguments\": {\"expression\": \"2+2\"}}\n</tool_call>",
            "Fa 4.",
        ]);
        let mut sink = RecSink::default();
        let ans = drive(&eng, "quanto fa 2+2", &mut sink);
        assert_eq!(sink.tools, vec!["calculator"], "il tool va eseguito");
        assert_eq!(ans, "Fa 4.", "poi il modello risponde col risultato");
    }

    #[test]
    fn consenso_negato_non_esegue_il_tool() {
        // fs_delete è sensibile → passa dal gate. Negato → NON eseguito, il modello prosegue.
        let eng = FakeEngine::new(&[
            "<tool_call>\n{\"name\": \"fs_delete\", \"arguments\": {\"path\": \"~/x.txt\"}}\n</tool_call>",
            "Ok, non ho eliminato nulla.",
        ]);
        let mut sink = RecSink { allow: false, ..Default::default() };
        let ans = drive(&eng, "elimina il file x", &mut sink);
        assert_eq!(sink.consent, vec!["fs_delete"], "deve chiedere il consenso");
        assert!(sink.tools.is_empty(), "consenso NEGATO → tool NON eseguito");
        assert_eq!(ans, "Ok, non ho eliminato nulla.");
    }

    #[test]
    fn consenso_concesso_esegue_il_tool() {
        let eng = FakeEngine::new(&[
            "<tool_call>\n{\"name\": \"fs_list\", \"arguments\": {\"path\": \"~/__inesistente_test_zeli__\"}}\n</tool_call>",
            "Fatto.",
        ]);
        let mut sink = RecSink { allow: true, ..Default::default() };
        let ans = drive(&eng, "elenca i file nella cartella x", &mut sink);
        assert_eq!(sink.consent, vec!["fs_list"], "deve chiedere il consenso");
        assert_eq!(sink.tools, vec!["fs_list"], "consenso CONCESSO → tool eseguito (errore dir gestito)");
        assert_eq!(ans, "Fatto.");
    }
}
