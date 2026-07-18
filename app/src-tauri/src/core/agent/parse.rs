//! Parsing model output: tool-call extraction, marker stripping, fact arrays.
use serde_json::Value;

/// Extract the matched `{...}` JSON object starting at the first `{` in `s`.
fn first_json_object(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for i in start..s.len() {
        let c = bytes[i] as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(s[start..=i].to_string());
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// If the model emitted a tool call, return (name, arguments). Prova PRIMA il formato Qwen
/// (`<tool_call>{"name":…}`, usato anche dal tool-forcing), POI il nativo Gemma
/// (`<|tool_call>call:NAME{…}`). Così un unico punto di estrazione copre entrambi i dialetti.
pub(super) fn extract_tool_call(raw: &str) -> Option<(String, Value)> {
    extract_tool_call_qwen(raw)
        .or_else(|| extract_tool_call_mistral(raw))
        .or_else(|| extract_tool_call_gemma(raw))
}

/// Mistral (Nemo/Small): `[TOOL_CALLS][{"name":…,"arguments":{…}}]` (array JSON prefissato). Porto
/// esatto di `_parse_mistral_toolcall` del training: scansiona l'array bilanciato (ignorando le
/// parentesi dentro le stringhe) e legge la PRIMA call → (name, args). train==runtime.
fn extract_tool_call_mistral(raw: &str) -> Option<(String, Value)> {
    let i = raw.find("[TOOL_CALLS]")?;
    let after = raw[i + "[TOOL_CALLS]".len()..].trim_start();
    let arr = first_json_array(after)?;
    let v: Value = serde_json::from_str(&arr).ok()?;
    let first = v.as_array()?.first()?;
    let name = first.get("name")?.as_str()?.to_string();
    let args = first.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({}));
    Some((name, args))
}

/// Primo array `[…]` bilanciato in `s` (gemello di `first_json_object`, ignora `[`/`]` dentro le stringhe).
fn first_json_array(s: &str) -> Option<String> {
    if !s.starts_with('[') {
        return None;
    }
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, c) in s.char_indices() {
        if in_str {
            if esc { esc = false } else if c == '\\' { esc = true } else if c == '"' { in_str = false }
            continue;
        }
        match c {
            '"' => in_str = true,
            '[' => depth += 1,
            ']' => { depth -= 1; if depth == 0 { return Some(s[..=i].to_string()); } }
            _ => {}
        }
    }
    None
}

/// Qwen/ChatML: `<tool_call>{"name": "...", "arguments": {...}}`.
fn extract_tool_call_qwen(raw: &str) -> Option<(String, Value)> {
    let pos = raw.find("<tool_call>")?;
    let obj = first_json_object(&raw[pos + "<tool_call>".len()..])?;
    let v: Value = serde_json::from_str(&obj).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let args = v.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({}));
    Some((name, args))
}

/// Gemma 4 emette il tool-call nel SUO formato nativo (dal chat template Unsloth del GGUF),
/// che NON è JSON: `<|tool_call>call:NAME{key:<|"|>val<|"|>,key2:123}<tool_call|>`.
/// Le stringhe sono delimitate da `<|"|>…<|"|>` (non da virgolette); numeri e bool sono nudi.
/// Estrae (name, args) convertendo gli argomenti in un oggetto JSON standard, così il resto
/// del loop (esecuzione, consenso, grounding) resta invariato tra Qwen e Gemma.
fn extract_tool_call_gemma(raw: &str) -> Option<(String, Value)> {
    let pos = raw.find("<|tool_call>")?;
    let after = &raw[pos + "<|tool_call>".len()..];
    let after = after.trim_start().strip_prefix("call:").unwrap_or(after);
    let brace = after.find('{')?;
    let name = after[..brace].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let body = gemma_braced_body(&after[brace..])?;
    Some((name, parse_gemma_args(&body)))
}

/// The Gemma string delimiter (opens AND closes, like a quote): `<|"|>`.
const GSTR: &str = "<|\"|>";

/// Estrae `{…}` bilanciato partendo da `s` (che inizia con `{`), ignorando graffe dentro le
/// stringhe `<|"|>…<|"|>`. UTF-8 safe (itera per char, non per byte).
fn gemma_braced_body(s: &str) -> Option<String> {
    let (mut depth, mut in_str) = (0i32, false);
    let mut out = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix(GSTR) {
            in_str = !in_str;
            out.push_str(GSTR);
            rest = r;
            continue;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        if !in_str {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    return Some(out);
                }
            }
        }
        rest = &rest[ch.len_utf8()..];
    }
    None
}

/// Parsa il corpo `{key:<|"|>val<|"|>,key2:123}` in un oggetto JSON.
fn parse_gemma_args(body: &str) -> Value {
    let inner = body.trim().trim_start_matches('{').trim_end_matches('}');
    let mut map = serde_json::Map::new();
    for pair in split_gemma_pairs(inner) {
        if let Some((k, v)) = pair.split_once(':') {
            let key = k.trim().to_string();
            if !key.is_empty() {
                map.insert(key, parse_gemma_value(v.trim()));
            }
        }
    }
    Value::Object(map)
}

/// Divide `key:val,key2:val2` sulle virgole di TOP-LEVEL, senza spezzare dentro `<|"|>…<|"|>`
/// (una stringa può contenere virgole). UTF-8 safe.
fn split_gemma_pairs(inner: &str) -> Vec<String> {
    let mut pairs = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut rest = inner;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix(GSTR) {
            in_str = !in_str;
            cur.push_str(GSTR);
            rest = r;
            continue;
        }
        let ch = rest.chars().next().unwrap();
        if ch == ',' && !in_str {
            pairs.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
        rest = &rest[ch.len_utf8()..];
    }
    if !cur.trim().is_empty() {
        pairs.push(cur);
    }
    pairs
}

/// Converte un valore Gemma: `<|"|>testo<|"|>` → stringa; `true`/`false` → bool; numero → number;
/// altrimenti (fallback prudente) stringa grezza.
fn parse_gemma_value(v: &str) -> Value {
    if let Some(inner) = v.strip_prefix(GSTR).and_then(|x| x.strip_suffix(GSTR)) {
        return Value::String(inner.to_string());
    }
    match v {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(n) = v.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(f) = v.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(v.to_string())
}

pub(super) fn strip_markers(s: &str) -> String {
    let mut t = strip_channel_thinking(s);
    // fine turno per dialetto (EOS atomici): Qwen <|im_end|>, Gemma nativo <end_of_turn> (+ vecchio
    // <turn|> difensivo), Mistral </s>, Cohere <|END_OF_TURN_TOKEN|>. E mai leakare un blocco tool-call:
    // Qwen `<tool_call`, Gemma `<|tool_call`, Mistral `[TOOL_CALLS`. Tronca alla PRIMA occorrenza.
    for marker in [
        "<|im_end|>", "<end_of_turn>", "<turn|>", "</s>", "<|END_OF_TURN_TOKEN|>",
        "<|tool_call", "<tool_call", "[TOOL_CALLS",
    ] {
        if let Some(i) = t.find(marker) {
            t.truncate(i);
        }
    }
    t.trim().to_string()
}

/// Gemma 12B ragiona in un "canale pensiero": `<|channel>thought\n…\n<channel|>[risposta]`. È il suo
/// reasoning interno — va rimosso dalla risposta (l'utente vuole il risultato, non il ragionamento sui
/// tool). Rimuove il blocco fino a `<channel|>` incluso; se il canale è aperto ma mai chiuso (risposta
/// troncata) taglia da lì in poi (è tutto ragionamento).
pub(super) fn strip_channel_thinking(s: &str) -> String {
    let mut t = s.to_string();
    while let Some(start) = t.find("<|channel>") {
        if let Some(rel) = t[start..].find("<channel|>") {
            let end = start + rel + "<channel|>".len();
            t.replace_range(start..end, "");
        } else {
            t.truncate(start);
            break;
        }
    }
    t
}

/// Length of the trailing portion of `s` that could be the start of a tool-call marker —
/// Qwen `<tool_call` OR Gemma `<|tool_call` — held back from streaming until we know whether
/// it becomes a tool call. Returns the LONGEST partial match across both markers.
pub(super) fn toolcall_prefix_tail(s: &str) -> usize {
    let mut best = 0;
    for marker in ["<|tool_call", "<tool_call", "[TOOL_CALLS"] {
        let max = marker.len().min(s.len());
        for i in (1..=max).rev() {
            if s.ends_with(&marker[..i]) {
                best = best.max(i);
                break;
            }
        }
    }
    best
}

/// Lenient parse: pull the first JSON array of strings out of the model output.
pub fn parse_facts(out: &str) -> Vec<String> {
    let (start, end) = match (out.find('['), out.rfind(']')) {
        (Some(a), Some(b)) if b > a => (a, b),
        _ => return vec![],
    };
    match serde_json::from_str::<Vec<String>>(&out[start..=end]) {
        Ok(v) => v
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| s.len() > 2)
            .collect(),
        Err(_) => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_tool_call() {
        let raw = "Certo!\n<tool_call>\n{\"name\": \"datetime\", \"arguments\": {}}\n</tool_call>";
        let (name, _) = extract_tool_call(raw).expect("tool call");
        assert_eq!(name, "datetime");
    }

    #[test]
    fn extracts_tool_call_after_preamble() {
        let raw = "<Sleeper/>\n<tool_call>{\"name\":\"calculator\",\"arguments\":{\"expression\":\"2+2\"}}</tool_call>";
        let (name, args) = extract_tool_call(raw).expect("tool call");
        assert_eq!(name, "calculator");
        assert_eq!(args.get("expression").unwrap(), "2+2");
    }

    #[test]
    fn no_tool_call_in_plain_text() {
        assert!(extract_tool_call("Ciao, come stai?").is_none());
    }

    #[test]
    fn json_object_balances_braces_and_strings() {
        assert_eq!(first_json_object("x {\"a\": {\"b\": 1}} y").unwrap(), "{\"a\": {\"b\": 1}}");
        assert_eq!(first_json_object("{\"s\":\"}\"}").unwrap(), "{\"s\":\"}\"}");
    }

    #[test]
    fn strip_markers_removes_toolcall_and_imend() {
        assert_eq!(strip_markers("ciao<|im_end|>"), "ciao");
        assert_eq!(strip_markers("ciao <tool_call>{...}"), "ciao");
    }

    #[test]
    fn prefix_tail_detects_partial_marker() {
        assert_eq!(toolcall_prefix_tail("hello <too"), 4);
        assert_eq!(toolcall_prefix_tail("hello"), 0);
        assert_eq!(toolcall_prefix_tail("x <tool_call"), 10);
    }

    #[test]
    fn parses_fact_array() {
        let facts = parse_facts("Ecco: [\"Si chiama Marco\", \"Ama il jazz\"]");
        assert_eq!(facts, vec!["Si chiama Marco", "Ama il jazz"]);
    }

    // ── Gemma 4 tool-call nativo (formato Unsloth del GGUF) ────────────────────────────────
    #[test]
    fn extracts_gemma_native_tool_call() {
        // È ESATTAMENTE ciò che l'utente ha visto grezzo in chat prima del fix.
        let raw = "<|tool_call>call:weather{location:<|\"|>Modena<|\"|>}<tool_call|>";
        let (name, args) = extract_tool_call(raw).expect("gemma tool call");
        assert_eq!(name, "weather");
        assert_eq!(args.get("location").unwrap(), "Modena");
    }

    #[test]
    fn extracts_gemma_tool_call_multi_arg_and_types() {
        // più argomenti + tipi non-stringa (numero, bool) + stringa con virgola interna
        let raw = "<|tool_call>call:book{title:<|\"|>Uno, due<|\"|>,pages:320,ebook:true}<tool_call|>";
        let (name, args) = extract_tool_call(raw).expect("gemma tool call");
        assert_eq!(name, "book");
        assert_eq!(args.get("title").unwrap(), "Uno, due"); // virgola NON spezza la stringa
        assert_eq!(args.get("pages").unwrap(), 320); // numero, non stringa
        assert_eq!(args.get("ebook").unwrap(), true); // bool
    }

    #[test]
    fn extracts_gemma_tool_call_after_prose() {
        // Gemma a volte antepone una frase; l'estrazione deve trovarlo comunque.
        let raw = "Controllo subito.\n<|tool_call>call:datetime{}<tool_call|>";
        let (name, args) = extract_tool_call(raw).expect("gemma tool call");
        assert_eq!(name, "datetime");
        assert!(args.as_object().unwrap().is_empty());
    }

    #[test]
    fn gemma_string_with_unicode_is_utf8_safe() {
        // ANTI-REGRESSIONE: il parser itera per char, non per byte — un accento non deve
        // spezzare lo slicing (col vecchio `bytes[i] as char` questo panica/corrompe).
        let raw = "<|tool_call>call:note{text:<|\"|>caffè però àèìòù<|\"|>}<tool_call|>";
        let (name, args) = extract_tool_call(raw).expect("gemma tool call");
        assert_eq!(name, "note");
        assert_eq!(args.get("text").unwrap(), "caffè però àèìòù");
    }

    // ── Mistral tool-call nativo [TOOL_CALLS][{…}] ──────────────────────────────────────────
    #[test]
    fn extracts_mistral_tool_call() {
        let raw = "[TOOL_CALLS][{\"name\": \"weather\", \"arguments\": {\"location\": \"Modena\"}}]";
        let (name, args) = extract_tool_call(raw).expect("mistral tool call");
        assert_eq!(name, "weather");
        assert_eq!(args.get("location").unwrap(), "Modena");
    }

    #[test]
    fn extracts_mistral_tool_call_after_prose_and_before_eos() {
        // preambolo + il ] chiuso, poi </s> (che il generatore stoppa) — legge la PRIMA call
        let raw = "Controllo.[TOOL_CALLS][{\"name\":\"datetime\",\"arguments\":{}}, {\"name\":\"x\",\"arguments\":{}}]</s>";
        let (name, _) = extract_tool_call(raw).expect("mistral tool call");
        assert_eq!(name, "datetime");
    }

    #[test]
    fn mistral_bracket_in_string_non_spezza_array() {
        let raw = "[TOOL_CALLS][{\"name\":\"note\",\"arguments\":{\"text\":\"lista] con ]\"}}]";
        let (name, args) = extract_tool_call(raw).expect("mistral tool call");
        assert_eq!(name, "note");
        assert_eq!(args.get("text").unwrap(), "lista] con ]");
    }

    #[test]
    fn strip_markers_mistral_e_cohere_eos() {
        assert_eq!(strip_markers("risposta</s>"), "risposta");
        assert_eq!(strip_markers("ciao<|END_OF_TURN_TOKEN|>"), "ciao");
        assert_eq!(strip_markers("ecco [TOOL_CALLS][{}]"), "ecco");
        assert_eq!(strip_markers("gemma nativo<end_of_turn>"), "gemma nativo");
    }

    #[test]
    fn qwen_still_wins_when_both_absent_gemma() {
        // il formato Qwen resta prioritario (tool-forcing lo usa anche per Gemma)
        let raw = "<tool_call>{\"name\":\"web_search\",\"arguments\":{\"query\":\"x\"}}</tool_call>";
        assert_eq!(extract_tool_call(raw).unwrap().0, "web_search");
    }

    #[test]
    fn strip_markers_removes_gemma_end_of_turn_and_toolcall() {
        assert_eq!(strip_markers("risposta<turn|>"), "risposta");
        assert_eq!(strip_markers("ecco <|tool_call>call:x{}"), "ecco");
    }

    #[test]
    fn prefix_tail_detects_partial_gemma_marker() {
        // soppressione streaming: una coda parziale del marker Gemma va trattenuta
        assert_eq!(toolcall_prefix_tail("testo <|tool"), 6); // "<|tool" = 6 char
        assert_eq!(toolcall_prefix_tail("x <|tool_call"), 11); // marker Gemma completo (11 char)
        assert_eq!(toolcall_prefix_tail("x <tool_call"), 10); // marker Qwen completo (10 char)
    }

    // ── Gemma 12B "thinking channel" (<|channel>thought…<channel|>) ────────────────────────
    #[test]
    fn strip_channel_thinking_rimuove_il_blocco() {
        // è ESATTAMENTE il leak visto dall'utente col 12B: resta solo la risposta finale
        let raw = "<|channel>thought\nDevo usare weather per Modena\n<channel|>A Modena oggi 26°C.";
        assert_eq!(strip_channel_thinking(raw), "A Modena oggi 26°C.");
        assert_eq!(strip_markers(raw), "A Modena oggi 26°C."); // via flusso reale (con trim)
    }

    #[test]
    fn strip_channel_canale_aperto_troncato_e_tutto_ragionamento() {
        // canale APERTO mai chiuso (risposta troncata) → è tutto reasoning → resta vuoto
        assert_eq!(strip_channel_thinking("<|channel>thought\nsto ragionando"), "");
    }

    #[test]
    fn strip_channel_senza_canale_lascia_intatto() {
        // ANTI-REGRESSIONE: risposta senza channel (Qwen / Gemma E4B) → invariata
        assert_eq!(strip_channel_thinking("Ciao, tutto bene."), "Ciao, tutto bene.");
    }

    #[test]
    fn strip_channel_rimuove_canali_multipli() {
        let raw = "<|channel>thought\na\n<channel|>Prima.<|channel>thought\nb\n<channel|> Seconda.";
        assert_eq!(strip_channel_thinking(raw), "Prima. Seconda.");
    }
}
