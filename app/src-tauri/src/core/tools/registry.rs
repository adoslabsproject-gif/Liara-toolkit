//! Holds the available tools, renders the Qwen tool prompt, and dispatches calls.
use super::builtin::{
    Calculator, CalendarAdd, CalendarDelete, CalendarList, CalendarSearch, DateTime, EmailDraft,
    EmailRecent, EmailReply, EmailSearch, EmailSend, EmailSent, FsDelete, FsList, FsMove, FsRead, FsSearch,
    FsWrite, NoteAdd, NoteList, NoteSearch, PeerAsk, PeerConnect, PeerProposeSlot, PhoneCall,
    SetLocation, SmsSend, Weather, WebFetch, WebSearch,
};
use super::{PendingCompose, Tool};
use crate::core::calendar::Calendar;
use crate::core::email::EmailStore;
use crate::core::memory::Memory;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use super::ToolSpec;

/// Versione COMPATTA della spec di un tool per il prompt (2026-07-03). Distilla il
/// catalogo da ~2326 a ~800 token per stare sotto la soglia di crash del prefill
/// mobile (GPU Adreno). Rimuove il peso ridondante SENZA perdere ciò che serve al
/// tool-calling (nome + parametri + required):
///   - description del TOOL: solo la prima frase (il "cosa fa" in una riga)
///   - description di ogni PARAMETRO: RIMOSSA (il modello impara i param dagli
///     esempi di training, non dalla prosa dello schema)
/// ⚠️ USATA SIA da catalog_json (export → dataset) SIA da render (runtime): il CODICE le tiene
/// identiche. MA vale solo se `tools_catalog.json` è rigenerato da `dump_tools`: il file committato
/// oggi è la versione FULL (stale) → drift reale finché non si riesporta (vedi `prompt_block_for`).
fn compact_tool_value(s: &ToolSpec) -> Value {
    // descrizione ULTRA-CORTA: prime ~40 char (il "cosa fa" in poche parole). I nomi
    // sono già parlanti (email_recent, calendar_add) e il modello impara i dettagli
    // dagli ESEMPI di training, non dalla prosa. char-safe (mai spezzare UTF-8).
    let first = s.description.split_once(". ").map(|(a, _)| a).unwrap_or(&s.description);
    let desc: String = first.trim_end_matches('.').chars().take(40).collect();
    // parameters: togli le "description" annidate dentro properties.* (peso ridondante:
    // il tipo + required bastano al tool-calling; il significato è nel nome del param)
    let mut params = s.parameters.clone();
    if let Some(props) = params.get_mut("properties").and_then(|p| p.as_object_mut()) {
        for (_k, v) in props.iter_mut() {
            if let Some(obj) = v.as_object_mut() {
                obj.remove("description");
            }
        }
    }
    json!({ "name": s.name, "description": desc, "parameters": params })
}

/// Categoria di un tool dal suo nome. FONTE UNICA della classificazione (runtime + il
/// gen del dataset la replica). Le stringhe DEVONO restare stabili (sono la chiave del
/// gate di equivalenza).
fn tool_category(name: &str) -> &'static str {
    if name.starts_with("email_") {
        "email"
    } else if name.starts_with("calendar_") {
        "calendar"
    } else if name.starts_with("fs_") {
        "files"
    } else if name.starts_with("note_") {
        "notes"
    } else if name == "web_fetch" || name == "web_search" {
        "web"
    } else if name == "weather" || name == "set_location" {
        "weather"
    } else if name.starts_with("peer_") {
        "peer"
    } else if name == "phone_call" || name == "sms_send" {
        "phone"
    } else {
        "core" // datetime, calculator
    }
}

/// Categorie di tool da includere nel prompt per una richiesta. "core" SEMPRE; le altre
/// per keyword generose (meglio includere un tool in più che perderlo — es. "chi mi ha
/// scritto" DEVE dare email).
///
/// 🔴 DRIFT NOTO (review round-3, aperto): questa selezione per-intento NON è replicata dal
/// generatore del dataset — combine_v3/v4 mettono il catalogo COMPLETO (tutti 24) in ogni
/// esempio. Quindi il blocco <tools> di training ≠ runtime (numero di tool E formato spec:
/// il training usa quello FULL, il runtime quello compatto). Decisione al revisore: o il
/// generatore emette compatto+selezionato, o si toglie la selezione a runtime (tutti-24
/// compatti ovunque, prefill ~1.5k < soglia crash ~3k). Finché non deciso: mismatch reale.
fn selected_categories(request: &str) -> Vec<&'static str> {
    let r = request.to_lowercase();
    let has = |ks: &[&str]| ks.iter().any(|k| r.contains(k));
    let mut cats = vec!["core"];
    if has(&["email", "mail", "posta", "casella", "inbox", "scritto", "messaggi", "rispondi",
             "risposta", "invia", "inviat", "manda", "scrivi a", "scrivere a", "mittente"]) {
        cats.push("email");
    }
    if has(&["agenda", "appuntament", "event", "calendar", "impegn", "ricordami", "promemoria",
             "scadenz", "riunion", "in programma", "che ho da fare"]) {
        cats.push("calendar");
    }
    if has(&["file", "cartella", "directory", "document", "download", "scarica il", "crea un file",
             "scrivi su", "salva nel file", "elimina il", "cancella il", "sposta", "rinomina"]) {
        cats.push("files");
    }
    if has(&["appunt", "annota", "segnati", "prendi nota", "nota che", "i miei appunt"]) {
        cats.push("notes");
    }
    // peer (chat AI↔AI): quando l'utente vuole collegare/presentare/coordinare col Liara di un altro.
    if has(&["il liara di", "collega", "presenta", "conosci il", "coordina con", "combina con",
             "l'altro liara", "senti il liara", "peer", "invita "]) {
        cats.push("peer");
    }
    // phone: quando l'utente vuole chiamare qualcuno o mandare un SMS (hand-off all'app di sistema).
    if has(&["chiama", "telefona", "chiamare", "telefonare", "fai una chiamata", "componi il numero",
             "sms", "messaggino", "manda un messaggio", "scrivi un sms", "manda un sms", "texta"]) {
        cats.push("phone");
    }
    // web: GENEROSO (meglio un tool in più che il modello che INVENTA per mancanza dello strumento).
    // Oltre alle forme di "cerca", copre le RICERCHE LOCALI/FATTUALI comuni ("trovami un meccanico
    // della zona", "numero del ristorante", "dove...", "quanto costa", "orari", "chi ha vinto") che
    // prima NON attivavano web → il modello, senza web_search, si inventava nomi/numeri/luoghi.
    if has(&["cerca", "notiz", "cerc", "http", "www", ".com", ".it", "sito", "web", "internet",
             "online", "in rete", "prezzo", "quotazion", "aggiornament", "novità", "novita",
             "trova", "trovami", "dove ", "dov'è", "dove si", "vicin", "in zona", "della zona",
             "qui vicino", "numero di", "numero del", "numero della", "indirizzo", "recension",
             "miglior", "consigli", "quanto costa", "quanto cost", "quanto viene", "orari",
             "a che ora apre", "aperto", "quanti abitant", "chi ha vinto", "chi è ", "cos'è",
             "come si ", "significa", "ristorant", "pizzeri", "negozio", "meccanic", "idraulic",
             "elettricist", "farmaci", "farmacia", "dottore", "medico", "ospedale", "hotel",
             "voli", "treno", "treni", "ricetta"]) {
        cats.push("web");
    }
    // weather include set_location: "dove sono/mia posizione/imposta città" → il modello sa dove sei.
    if has(&["meteo", "tempo fa", "che tempo", "temperatura", "previsioni", "pioggia", "pioverà",
             "dove sono", "mia posizione", "la mia città", "imposta la città", "dove mi trovo",
             "gradi", "farà caldo", "farà freddo", "clima"]) {
        cats.push("weather");
    }
    cats
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

/// #4: valida gli argomenti di un tool contro il suo JSON-Schema. Controlla (a) che TUTTI i campi
/// `required` siano presenti e non-null, (b) che i tipi dei campi presenti siano coerenti. Il check
/// di tipo è LENIENTE di proposito (un modello piccolo emette spesso i numeri come stringa): accetta
/// le coercizioni ovvie e blocca solo i mismatch grossolani (es. un oggetto dove serve una stringa).
fn validate_args(schema: &Value, args: &Value) -> Result<()> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow!("gli argomenti devono essere un oggetto JSON"))?;
    if let Some(req) = schema.get("required").and_then(|v| v.as_array()) {
        for field in req.iter().filter_map(|v| v.as_str()) {
            let present = obj.get(field).map(|v| !v.is_null()).unwrap_or(false);
            if !present {
                return Err(anyhow!("manca l'argomento obbligatorio '{field}'"));
            }
        }
    }
    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(ty) = props.get(k).and_then(|p| p.get("type")).and_then(|t| t.as_str()) {
                if !type_matches(ty, v) {
                    return Err(anyhow!("l'argomento '{k}' deve essere di tipo {ty}"));
                }
            }
        }
    }
    Ok(())
}

/// Coerenza di tipo LENIENTE (vedi `validate_args`): gli scalari sono intercambiabili con la stringa,
/// un numero può arrivare come stringa numerica, un bool come "true"/"false". Blocca solo object/array
/// dove serve uno scalare e viceversa. Tipo sconosciuto → non blocca.
fn type_matches(ty: &str, v: &Value) -> bool {
    match ty {
        "string" => v.is_string() || v.is_number() || v.is_boolean(),
        "integer" | "number" => {
            v.is_number() || v.as_str().map(|s| s.trim().parse::<f64>().is_ok()).unwrap_or(false)
        }
        "boolean" => v.is_boolean() || matches!(v.as_str(), Some("true") | Some("false")),
        "object" => v.is_object(),
        "array" => v.is_array(),
        _ => true,
    }
}

/// GBNF dell'oggetto `arguments` di UN tool dal suo JSON-Schema: i campi REQUIRED (nell'ordine
/// dello schema, tipati) e poi eventuali membri opzionali. Nessun required → `object` generico.
/// Ora è VIVO e usato da `tool_call_grammar` (review round-3 #1): prima era dead code, e la grammar
/// vincolava solo il nome + un JSON qualsiasi → si potevano emettere argomenti vuoti per un tool che
/// li richiede (es. `calendar_add` senza `title`/`when`) e il tool falliva a runtime.
fn args_object_grammar(schema: &Value) -> String {
    let required: Vec<String> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let Some(props) = schema.get("properties").and_then(|v| v.as_object()) else {
        return "object".to_string();
    };
    if required.is_empty() {
        return "object".to_string();
    }
    let mut parts = Vec::new();
    for (i, field) in required.iter().enumerate() {
        let ty = props.get(field).and_then(|p| p.get("type")).and_then(|t| t.as_str()).unwrap_or("string");
        let tg = match ty {
            "integer" | "number" => "number",
            "boolean" => r#"( "true" | "false" )"#,
            _ => "string",
        };
        let sep = if i == 0 { "" } else { r#" space "," space "# };
        parts.push(format!(r#"{sep}"\"{field}\"" space ":" space {tg}"#));
    }
    format!(r#""{{" space {} ( space "," space member )* space "}}""#, parts.concat())
}

impl ToolRegistry {
    /// Build the registry, wiring tools to the resources they need.
    pub fn build(email: Arc<EmailStore>, pending: PendingCompose, cal: Arc<Calendar>, mem: Arc<Memory>) -> Self {
        Self {
            tools: vec![
                Box::new(DateTime),
                Box::new(Calculator),
                Box::new(WebFetch),
                Box::new(WebSearch),
                Box::new(Weather { mem: mem.clone() }),
                Box::new(SetLocation { mem: mem.clone() }),
                Box::new(EmailRecent { store: email.clone() }),
                Box::new(EmailSent { store: email.clone() }),
                Box::new(EmailSearch { store: email.clone() }),
                Box::new(EmailReply { store: email.clone(), pending: pending.clone() }),
                Box::new(EmailDraft { pending: pending.clone() }),
                Box::new(EmailSend { store: email, pending }),
                Box::new(CalendarAdd { cal: cal.clone() }),
                Box::new(CalendarList { cal: cal.clone() }),
                Box::new(CalendarSearch { cal: cal.clone() }),
                Box::new(CalendarDelete { cal }),
                Box::new(FsList),
                Box::new(FsRead),
                Box::new(FsSearch),
                Box::new(FsWrite),
                Box::new(FsMove),
                Box::new(FsDelete),
                Box::new(NoteAdd { mem: mem.clone() }),
                Box::new(NoteList { mem: mem.clone() }),
                Box::new(NoteSearch { mem }),
                // Canale peer (chat AI↔AI) — SPEC congelata per il dataset; execute stub finché E2E+AI (M2/M3).
                Box::new(PeerConnect),
                Box::new(PeerAsk),
                Box::new(PeerProposeSlot),
                // Telefono: hand-off all'app di sistema (chiamata/SMS), nessun permesso pericoloso.
                Box::new(PhoneCall),
                Box::new(SmsSend),
            ],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Append dynamically-discovered tools (e.g. from MCP servers).
    pub fn add_dynamic(&mut self, extra: Vec<Box<dyn Tool>>) {
        self.tools.extend(extra);
    }

    /// The REAL tool catalog as JSON — single source of truth for the LoRA dataset (anti-drift).
    pub fn catalog_json(&self) -> String {
        let arr: Vec<Value> =
            self.tools.iter().map(|t| compact_tool_value(&t.spec())).collect();
        serde_json::to_string_pretty(&Value::Array(arr)).unwrap_or_default()
    }

    /// Tool in formato OpenAI (`[{type:"function", function:{name,description,parameters}}]`) per la
    /// modalità "Liara via API" (32B cloud, `/liara/chat`): il server ha tool-calling nativo hermes →
    /// gli passiamo gli schemi e riceviamo i `tool_call` da eseguire in LOCALE (tool on-device). Stessa
    /// `compact_tool_value` del catalogo/training → zero drift tra locale e cloud.
    pub fn openai_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| json!({ "type": "function", "function": compact_tool_value(&t.spec()) }))
            .collect()
    }

    /// GBNF (lazy, dopo `<tool_call>`) che vincola la chiamata a `{"name": <tool>, "arguments": <args>}
    /// </tool_call>` — PER-TOOL (review round-3 #1): ogni nome è accoppiato al SUO oggetto arguments
    /// (required tipati e nell'ordine dello schema, poi opzionali), non a un JSON generico. Così un
    /// tool-call è deterministicamente corretto in formato: `calendar_add` NON può uscire senza
    /// `title`/`when`. ⚠️ L'ordine dei required è quello dello schema → il dataset deve emettere gli
    /// argomenti in quest'ordine (li costruisce come mappe ordinate, quindi coerente).
    pub fn tool_call_grammar(&self) -> String {
        let mut g = String::new();
        // root: name-object accoppiati, uno per tool → ( call0 | call1 | … )
        let calls: Vec<String> = (0..self.tools.len()).map(|i| format!("call{i}")).collect();
        g.push_str(r#"root ::= space "{" space "\"name\"" space ":" space ( "#);
        g.push_str(&calls.join(" | "));
        g.push_str(r#" ) space "}" space "</tool_call>""#);
        g.push('\n');
        // callN: "<name>" , "arguments": <args-object-di-quel-tool>
        for (i, t) in self.tools.iter().enumerate() {
            let s = t.spec();
            let args = args_object_grammar(&s.parameters);
            g.push_str(&format!(
                "call{i} ::= \"\\\"{}\\\"\" space \",\" space \"\\\"arguments\\\"\" space \":\" space {args}\n",
                s.name
            ));
        }
        // regole comuni
        g.push_str(r#"object ::= "{" space ( member ( space "," space member )* )? space "}""#);
        g.push('\n');
        g.push_str(r#"member ::= string space ":" space value"#);
        g.push('\n');
        g.push_str(r#"array ::= "[" space ( value ( space "," space value )* )? space "]""#);
        g.push('\n');
        g.push_str(r#"value ::= object | array | string | number | "true" | "false" | "null""#);
        g.push('\n');
        // #1 FIX (CRITICAL): escludiamo i control char grezzi (0x00-0x1F). serde_json li rifiuta dentro
        // le stringhe JSON, quindi con newline grezzi (email body / note / fs_write multi-riga) il tool_call
        // non veniva parsato -> tool MAI eseguito. Cosi il modello e' forzato a emettere \n ESCAPATO.
        g.push_str(r#"string ::= "\"" ( [^"\\\x00-\x1F] | "\\" . )* "\"""#);
        g.push('\n');
        g.push_str(r#"number ::= "-"? ( "0" | [1-9] [0-9]* ) ( "." [0-9]+ )? ( [eE] [-+]? [0-9]+ )?"#);
        g.push('\n');
        g.push_str(r#"space ::= [ \t\n]*"#);
        g.push('\n');
        g
    }

    pub fn find(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .map(|b| b.as_ref())
            .find(|t| t.spec().name == name)
    }

    pub fn execute(&self, name: &str, args: &Value) -> Result<String> {
        match self.find(name) {
            Some(t) => {
                // #4: validazione degli argomenti contro lo schema PRIMA del dispatch — per OGNI
                // dialetto (Qwen/Gemma/MCP/forcing), non solo quello coperto dalla grammatica GBNF.
                // Un errore chiaro torna al modello nel loop ReAct, che ritenta; meglio di un tool
                // che fallisce a metà o produce risultati muti su un required mancante.
                validate_args(&t.spec().parameters, args)
                    .map_err(|e| anyhow!("Argomenti non validi per {name}: {e}"))?;
                t.execute(args)
            }
            None => Err(anyhow!("tool sconosciuto: {name}")),
        }
    }

    pub fn is_sensitive(&self, name: &str) -> bool {
        self.find(name).map(|t| t.sensitive()).unwrap_or(false)
    }

    pub fn consent_action(&self, name: &str, args: &Value) -> String {
        self.find(name).map(|t| t.consent_action(args)).unwrap_or_else(|| name.to_string())
    }

    /// (name, description) for every sensitive tool — for the permissions UI.
    pub fn sensitive_tools(&self) -> Vec<(String, String)> {
        self.tools
            .iter()
            .filter(|t| t.sensitive())
            .map(|t| {
                let s = t.spec();
                (s.name, s.description)
            })
            .collect()
    }

    /// Render only the tools relevant to the user's request.
    ///
    /// 🔴 SELEZIONE PER INTENTO (2026-07-03): passare TUTTI i 24 tool a ogni turno faceva
    /// un prefill ~3000 token che CRASHA la GPU Adreno mobile (eccezione OpenCL). Ora solo
    /// i CORE (datetime, calculator) sono sempre presenti; le altre famiglie entrano per
    /// keyword. Per una conversazione ("ciao") → 2 tool → prompt ~300 tok → niente crash.
    /// Per "leggi le email" → core+email → ~700 tok → sotto la soglia (~960).
    ///
    /// ⚠️ DRIFT col training (vedi `selected_categories`): oggi il dataset NON replica questa
    /// selezione (mette tutti-24), quindi train ≠ runtime sul blocco <tools>. In attesa della
    /// decisione del revisore (compattare/selezionare il generatore, o togliere la selezione qui).
    pub fn prompt_block_for(&self, request: &str) -> String {
        let cats = selected_categories(request);
        let selected: Vec<&Box<dyn Tool>> = self
            .tools
            .iter()
            .filter(|t| cats.contains(&tool_category(&t.spec().name)))
            .collect();
        self.render(&selected)
    }

    /// Qwen2.5 native tool-calling system block (Hermes-style) for the given tools.
    fn render(&self, tools: &[&Box<dyn Tool>]) -> String {
        let mut tools_json = String::new();
        for t in tools {
            // compact_tool_value = STESSA compressione dell'export per il training
            // (catalog_json) → il blocco <tools> a runtime è byte-identico a quello
            // che il LoRA ha visto in addestramento. Zero drift.
            let entry = json!({ "type": "function", "function": compact_tool_value(&t.spec()) });
            tools_json.push_str(&entry.to_string());
            tools_json.push('\n');
        }
        // EXACT Qwen2.5 tool template (so training via the tokenizer chat_template
        // and our inference produce the identical prompt — the LoRA aligns perfectly).
        format!(
            "\n\n# Tools\n\nYou may call one or more functions to assist with the user query.\n\n\
You are provided with function signatures within <tools></tools> XML tags:\n<tools>\n{tools_json}</tools>\n\n\
For each function call, return a json object with function name and arguments within <tool_call></tool_call> XML tags:\n<tool_call>\n{{\"name\": <function-name>, \"arguments\": <args-json-object>}}\n</tool_call>"
        )
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn tool_category_classifica_tutte_le_famiglie() {
        assert_eq!(tool_category("datetime"), "core");
        assert_eq!(tool_category("calculator"), "core");
        assert_eq!(tool_category("email_recent"), "email");
        assert_eq!(tool_category("calendar_add"), "calendar");
        assert_eq!(tool_category("fs_read"), "files");
        assert_eq!(tool_category("note_add"), "notes");
        assert_eq!(tool_category("web_fetch"), "web");
        assert_eq!(tool_category("web_search"), "web");
        assert_eq!(tool_category("weather"), "weather");
        assert_eq!(tool_category("set_location"), "weather");
    }

    #[test]
    fn conversazione_porta_solo_core() {
        // 🔴 è l'invariante ANTI-CRASH: "ciao" non deve trascinare 24 tool nel prompt.
        let c = selected_categories("ciao, come stai?");
        assert_eq!(c, vec!["core"], "una conversazione deve avere SOLO i core");
    }

    #[test]
    fn intenti_attivano_la_famiglia_giusta() {
        assert!(selected_categories("leggi le email").contains(&"email"));
        assert!(selected_categories("chi mi ha scritto oggi").contains(&"email")); // implicito
        assert!(selected_categories("che appuntamenti ho").contains(&"calendar"));
        assert!(selected_categories("ricordami di chiamare").contains(&"calendar"));
        assert!(selected_categories("cerca le notizie online").contains(&"web"));
        assert!(selected_categories("apri esempio.com").contains(&"web"));
        assert!(selected_categories("che tempo fa a Roma").contains(&"weather"));
        assert!(selected_categories("elenca i file nella cartella").contains(&"files"));
        assert!(selected_categories("prendi nota della spesa").contains(&"notes"));
    }

    #[test]
    fn core_sempre_presente() {
        for msg in ["", "ciao", "leggi le email", "che tempo fa"] {
            assert!(selected_categories(msg).contains(&"core"), "core deve esserci per {msg:?}");
        }
    }

    #[test]
    fn compact_tool_value_toglie_le_description_dei_parametri() {
        let spec = ToolSpec {
            name: "x".into(),
            description: "Prima frase lunghissima da accorciare per forza. Seconda frase.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "q": { "type": "string", "description": "da rimuovere" } },
                "required": ["q"]
            }),
        };
        let v = compact_tool_value(&spec);
        let s = v.to_string();
        assert!(!s.contains("da rimuovere"), "la description del param NON deve restare");
        assert!(!s.contains("Seconda frase"), "solo la prima frase della description tool");
        assert!(s.contains("\"required\":[\"q\"]"), "required preservato (serve al tool-calling)");
        assert!(s.contains("\"q\""), "il nome del parametro resta");
    }

    // ── #1: grammatica GBNF per-tool (i required sono forzati) ─────────────────────────────
    #[test]
    fn args_object_grammar_forza_i_required_tipati() {
        // due required string, IN ORDINE
        let sch = serde_json::json!({
            "type": "object",
            "properties": { "title": { "type": "string" }, "when": { "type": "string" } },
            "required": ["title", "when"]
        });
        let g = args_object_grammar(&sch);
        assert!(g.contains(r#""\"title\"""#) && g.contains(r#""\"when\"""#), "i required devono comparire");
        assert!(g.find("title").unwrap() < g.find("when").unwrap(), "ordine dello schema preservato");
        // nessun required → object generico (accetta {} o qualsiasi)
        let none = serde_json::json!({
            "type": "object", "properties": { "count": { "type": "integer" } }, "required": []
        });
        assert_eq!(args_object_grammar(&none), "object");
        // un required integer usa la regola `number`, non `string`
        let num = serde_json::json!({
            "type": "object", "properties": { "id": { "type": "integer" } }, "required": ["id"]
        });
        assert!(args_object_grammar(&num).contains("space number"), "integer → number");
    }

    #[test]
    fn tool_call_grammar_accoppia_nome_e_argomenti() {
        use crate::core::calendar::Calendar;
        use crate::core::crypto::Crypto;
        use crate::core::email::EmailStore;
        use crate::core::memory::Memory;
        use std::sync::{Arc, Mutex};
        let crypto = Arc::new(Crypto::from_key(&[9u8; 32]));
        let mem = Arc::new(Memory::open(":memory:", crypto.clone()).unwrap());
        let email = Arc::new(EmailStore::open(":memory:", crypto.clone()).unwrap());
        let cal = Arc::new(Calendar::open(":memory:", crypto).unwrap());
        let pending = Arc::new(Mutex::new(None));
        let g = ToolRegistry::build(email, pending, cal, mem).tool_call_grammar();

        // ANTI-REGRESSIONE #1: la regola di calendar_add DEVE richiedere title+when (non object generico),
        // così il modello non può emettere {"name":"calendar_add","arguments":{}} → tool che fallisce.
        let add = g.lines().find(|l| l.contains(r#"\"calendar_add\""#)).expect("regola calendar_add");
        assert!(add.contains(r#"\"title\""#) && add.contains(r#"\"when\""#), "calendar_add forza title+when");
        // datetime (zero argomenti) → object generico
        let dt = g.lines().find(|l| l.contains(r#"\"datetime\""#)).expect("regola datetime");
        assert!(dt.trim_end().ends_with("object"), "datetime → arguments object generico");
    }

    // ── #4: validazione argomenti per-schema (tutti i dialetti, prima del dispatch) ────────
    #[test]
    fn validate_args_richiede_i_required() {
        let sch = serde_json::json!({
            "type": "object",
            "properties": { "title": { "type": "string" }, "when": { "type": "string" } },
            "required": ["title", "when"]
        });
        // manca 'when' → errore che nomina il campo
        let err = validate_args(&sch, &serde_json::json!({ "title": "Dentista" })).unwrap_err();
        assert!(err.to_string().contains("when"), "l'errore deve nominare il required mancante");
        // required null = mancante
        assert!(validate_args(&sch, &serde_json::json!({ "title": "x", "when": null })).is_err());
        // tutti presenti → ok
        assert!(validate_args(&sch, &serde_json::json!({ "title": "x", "when": "domani 15:00" })).is_ok());
    }

    #[test]
    fn validate_args_tipi_lenienti_ma_non_grossolani() {
        let sch = serde_json::json!({
            "type": "object",
            "properties": { "count": { "type": "integer" } },
            "required": []
        });
        // numero come stringa → accettato (i modelli piccoli lo fanno)
        assert!(validate_args(&sch, &serde_json::json!({ "count": "3" })).is_ok());
        assert!(validate_args(&sch, &serde_json::json!({ "count": 3 })).is_ok());
        // un oggetto dove serve un integer → rifiutato
        assert!(validate_args(&sch, &serde_json::json!({ "count": { "x": 1 } })).is_err());
    }

    #[test]
    fn validate_args_via_registry_blocca_calendar_add_vuoto() {
        // ANTI-REGRESSIONE #1/#4: il buco originale — calendar_add senza title/when — ora è bloccato
        // a runtime (oltre che dalla grammatica), quindi vale ANCHE per Gemma e per l'MCP.
        use crate::core::calendar::Calendar;
        use crate::core::crypto::Crypto;
        use crate::core::email::EmailStore;
        use crate::core::memory::Memory;
        use std::sync::{Arc, Mutex};
        let crypto = Arc::new(Crypto::from_key(&[8u8; 32]));
        let mem = Arc::new(Memory::open(":memory:", crypto.clone()).unwrap());
        let email = Arc::new(EmailStore::open(":memory:", crypto.clone()).unwrap());
        let cal = Arc::new(Calendar::open(":memory:", crypto).unwrap());
        let reg = ToolRegistry::build(email, Arc::new(Mutex::new(None)), cal, mem);
        let err = reg.execute("calendar_add", &serde_json::json!({})).unwrap_err();
        assert!(err.to_string().contains("obbligatorio"), "calendar_add vuoto deve fallire con errore chiaro");
        // datetime non ha required → passa con {}
        assert!(reg.execute("datetime", &serde_json::json!({})).is_ok());
    }
}
