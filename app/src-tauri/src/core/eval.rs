//! Eval harness: deterministic, measurable checks on the assistant's core decisions —
//! tool ROUTING (does the right tool reach the prompt?), FACT extraction, and question
//! PUNCTUATION. No model needed, so it's fast and reproducible.
//!
//! Run it:  `cargo test --lib eval_harness -- --nocapture`  → prints the scorecard.
use crate::core::agent::parse_facts;
use crate::core::audio::punctuate_question;
use crate::core::calendar::Calendar;
use crate::core::crypto::Crypto;
use crate::core::email::EmailStore;
use crate::core::memory::Memory;
use crate::core::tools::ToolRegistry;
use std::sync::{Arc, Mutex};

fn registry() -> ToolRegistry {
    let crypto = Arc::new(Crypto::from_key(&[1u8; 32]));
    let mem = Arc::new(Memory::open(":memory:", crypto.clone()).unwrap());
    let email = Arc::new(EmailStore::open(":memory:", crypto.clone()).unwrap());
    let cal = Arc::new(Calendar::open(":memory:", crypto).unwrap());
    let pending = Arc::new(Mutex::new(None));
    ToolRegistry::build(email, pending, cal, mem)
}

/// Memory eval: recall precision + supersession correctness (synthetic embeddings).
/// NB: la recall esclude kind="fact" BY DESIGN (audit #26: già iniettati da
/// profile_block) → precision e supersession si misurano sugli EPISODI; i fact
/// si osservano via most_similar_fact (il canale che li usa davvero).
fn memory_score() -> (usize, usize) {
    let m = Memory::open(":memory:", Arc::new(Crypto::from_key(&[2u8; 32]))).unwrap();
    m.remember("episode", "ama il caffè", &[1.0, 0.0, 0.0], 0.7).unwrap();
    m.remember("episode", "lavora a Milano", &[0.0, 1.0, 0.0], 0.7).unwrap();
    m.remember("fact", "ha un cane", &[0.0, 0.0, 1.0], 0.7).unwrap();
    let mut ok = 0;
    // 1) recall precision: a query near "caffè" must rank it first
    if m.recall(&[0.9, 0.1, 0.0], 1).first().map(|(t, _)| t.contains("caffè")).unwrap_or(false) {
        ok += 1;
    }
    // 2) most_similar_fact picks the right topic (i fact restano nel canale supersession)
    if m.most_similar_fact(&[0.0, 0.0, 1.0]).map(|(_, t, _)| t.contains("cane")).unwrap_or(false) {
        ok += 1;
    }
    // 3) supersession removes a retired memory from recall
    let id = m.remember("episode", "vecchio indirizzo", &[1.0, 1.0, 1.0], 0.7).unwrap();
    m.supersede(id).unwrap();
    if m.recall(&[1.0, 1.0, 1.0], 5).iter().all(|(t, _)| !t.contains("vecchio indirizzo")) {
        ok += 1;
    }
    (ok, 3)
}

/// (user request, tool name, must-be-present)
const ROUTING: &[(&str, &str, bool)] = &[
    ("che ore sono adesso", "datetime", true),
    ("quanto fa 12 per 8", "calculator", true),
    ("scrivi una mail a Marco", "email_draft", true),
    ("leggi le ultime email ricevute", "email_recent", true),
    ("aggiungi un appuntamento dal dentista venerdì", "calendar_add", true),
    ("che impegni ho in agenda domani", "calendar_list", true),
    ("che tempo fa a Roma", "weather", true),
    ("cerca nei miei file il documento", "fs_search", true),
    ("prendi nota: comprare il latte", "note_add", true),
    ("visita example.com e dimmi cosa contiene", "web_fetch", true),
    // SELEZIONE PER INTENTO (2026-07-03): una conversazione ("ciao") NON porta con
    // sé email/calendar/file → solo i CORE. È ciò che tiene il prompt corto e la GPU
    // mobile viva. Le keyword generose garantiscono che l'intento REALE attivi la
    // famiglia giusta (test sopra: "chi mi ha scritto" → email, ecc.).
    ("ciao, come stai oggi", "email_recent", false),
    ("ciao, come stai oggi", "calendar_add", false),
    ("ciao, come stai oggi", "fs_read", false),
    ("ciao, come stai oggi", "weather", false),
    ("che ore sono", "email_draft", false),
    ("quanto fa 5 più 5", "fs_search", false),
    // copertura generosa: intenti impliciti devono comunque attivare la famiglia
    ("chi mi ha scritto oggi", "email_recent", true),
    ("che impegni ho in programma", "calendar_list", true),
];

/// (extraction output, expected number of facts)
const PARSE: &[(&str, usize)] = &[
    ("Ecco i fatti: [\"ama il caffè\", \"vive a Roma\"]", 2),
    ("nessun fatto rilevante da estrarre", 0),
    ("[\"lavora come ingegnere\"]", 1),
    ("[]", 0),
];

/// (dictated text, expected after punctuation)
const PUNCT: &[(&str, &str)] = &[
    ("che ore sono", "che ore sono?"),
    ("come stai", "come stai?"),
    ("oggi piove", "oggi piove"),
    ("ricordami di chiamare il dentista", "ricordami di chiamare il dentista"),
];

#[test]
fn eval_harness() {
    let reg = registry();
    let (mut pass, mut total) = (0usize, 0usize);

    let mut route_ok = 0;
    for (req, tool, want) in ROUTING {
        let present = reg.prompt_block_for(req).contains(&format!("\"{tool}\""));
        if present == *want {
            route_ok += 1;
            pass += 1;
        } else {
            println!("  ✗ routing: «{req}» → {tool} atteso={want} ottenuto={present}");
        }
        total += 1;
    }

    let mut parse_ok = 0;
    for (raw, n) in PARSE {
        if parse_facts(raw).len() == *n {
            parse_ok += 1;
            pass += 1;
        } else {
            println!("  ✗ parse_facts: «{raw}» atteso {n}");
        }
        total += 1;
    }

    let mut punct_ok = 0;
    for (inp, exp) in PUNCT {
        if punctuate_question(inp) == *exp {
            punct_ok += 1;
            pass += 1;
        } else {
            println!("  ✗ punct: «{inp}» → atteso «{exp}»");
        }
        total += 1;
    }

    let (mem_ok, mem_total) = memory_score();
    pass += mem_ok;
    total += mem_total;

    let pct = (pass as f32 / total as f32) * 100.0;
    println!("\n=== LIARA EVAL SCORECARD ===");
    println!("routing tool:    {route_ok}/{}", ROUTING.len());
    println!("estrazione:      {parse_ok}/{}", PARSE.len());
    println!("punteggiatura:   {punct_ok}/{}", PUNCT.len());
    println!("memoria (recall+supersede): {mem_ok}/{mem_total}");
    println!("TOTALE: {pass}/{total} ({pct:.0}%)\n");
    assert!(pct >= 90.0, "eval sotto soglia 90%: {pct:.0}%");
}
