//! Characterization tests: pin the observable behavior of the Memory store so the
//! modularization (and any future refactor) cannot silently change it. No LLM needed —
//! embeddings are synthetic vectors.
use super::*;
use crate::core::crypto::Crypto;
use std::sync::Arc;

fn mem() -> Memory {
    Memory::open(":memory:", Arc::new(Crypto::from_key(&[7u8; 32]))).unwrap()
}

#[test]
fn profile_roundtrip_and_empty_deletes() {
    let m = mem();
    m.set_profile("nome", "Zeli").unwrap();
    m.set_profile("citta", "Roma").unwrap();
    assert_eq!(
        m.profile_entries().unwrap(),
        vec![("nome".to_string(), "Zeli".to_string()), ("citta".to_string(), "Roma".to_string())]
    );
    // setting empty removes the key
    m.set_profile("citta", "   ").unwrap();
    assert_eq!(m.profile_entries().unwrap(), vec![("nome".to_string(), "Zeli".to_string())]);
}

#[test]
fn profile_is_encrypted_at_rest() {
    let m = mem();
    m.set_profile("nome", "Zeli").unwrap();
    let raw: String = m
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT value FROM profile WHERE key='nome'", [], |r| r.get(0))
        .unwrap();
    assert!(raw.starts_with(ENC), "il valore profilo deve essere cifrato a riposo, era: {raw}");
    assert_ne!(raw, "Zeli");
}

#[test]
fn facts_dedup_and_forget() {
    let m = mem();
    assert!(m.add_fact("ama il caffè").unwrap());
    assert!(!m.add_fact("ama il caffè").unwrap(), "fatto duplicato non va aggiunto");
    assert!(m.add_fact("vive a Roma").unwrap());
    assert_eq!(m.facts().unwrap(), vec!["ama il caffè".to_string(), "vive a Roma".to_string()]);
    m.forget_all().unwrap();
    assert!(m.facts().unwrap().is_empty());
}

#[test]
fn add_episode_does_not_error() {
    // add_episode writes the legacy `episodes` table (distinct from the vector `memories`).
    let m = mem();
    m.add_episode("user", "ciao").unwrap();
    m.add_episode("assistant", "ciao a te").unwrap();
}

#[test]
fn recent_episode_texts_reads_vector_episodes_newest_first() {
    // recent_episode_texts reads `memories` WHERE kind='episode' ORDER BY id DESC.
    let m = mem();
    m.remember("episode", "ciao", &[1.0, 0.0], 0.3).unwrap();
    m.remember("episode", "ciao a te", &[1.0, 0.0], 0.3).unwrap();
    m.remember("episode", "che ore sono", &[1.0, 0.0], 0.3).unwrap();
    let recent = m.recent_episode_texts(2);
    assert_eq!(recent.len(), 2);
    assert!(recent[0].contains("che ore sono"), "il più recente deve venire primo, era: {:?}", recent);
}

#[test]
fn conversations_crud() {
    let m = mem();
    m.save_conversation("c1", "Prima chat", "{\"x\":1}").unwrap();
    m.save_conversation("c2", "Seconda", "{\"y\":2}").unwrap();
    let list = m.list_conversations().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(m.load_conversation("c1").unwrap().as_deref(), Some("{\"x\":1}"));
    m.delete_conversation("c1").unwrap();
    assert!(m.load_conversation("c1").unwrap().is_none());
    assert_eq!(m.list_conversations().unwrap().len(), 1);
}

#[test]
fn vector_recall_ranks_most_similar_first() {
    // NB: recall() esclude kind="fact" BY DESIGN (audit #26: i fatti sono già
    // iniettati da profile_block a ogni turno → doppia iniezione). Il ranking
    // si testa su episodi/riflessioni, che la recall serve davvero.
    let m = mem();
    m.remember("episode", "il gatto dorme", &[1.0, 0.0, 0.0], 0.5).unwrap();
    m.remember("episode", "la borsa sale", &[0.0, 1.0, 0.0], 0.5).unwrap();
    let hits = m.recall(&[1.0, 0.0, 0.0], 2);
    assert_eq!(hits.len(), 2);
    assert!(hits[0].0.contains("gatto"), "il più simile deve venire primo, era: {:?}", hits);

    // ANTI-REGRESSIONE #26: un fact identico alla query NON entra nella recall…
    m.remember("fact", "fatto già nel profilo", &[1.0, 0.0, 0.0], 0.9).unwrap();
    assert!(m.recall(&[1.0, 0.0, 0.0], 5).iter().all(|(t, _)| !t.contains("profilo")),
            "i fact sono già iniettati da profile_block: la recall NON deve duplicarli");
    // …ma resta visibile alla supersession (most_similar_fact)
    assert!(m.most_similar_fact(&[1.0, 0.0, 0.0]).unwrap().1.contains("profilo"));

    // RAG namespace: docs are recalled separately, not mixed into personal recall
    m.remember("doc", "[doc:manuale] capitolo sicurezza", &[1.0, 0.0, 0.0], 0.6).unwrap();
    assert!(m.recall(&[1.0, 0.0, 0.0], 5).iter().all(|(t, _)| !t.contains("manuale")));
    assert!(m.recall_docs(&[1.0, 0.0, 0.0], 5).iter().any(|(t, _)| t.contains("manuale")));
}

#[test]
fn embedding_encrypted_at_rest() {
    let m = mem();
    let emb = vec![0.5f32, 0.5, 0.5, 0.5];
    // kind=episode: la recall lo serve (i fact sono esclusi by design, #26)
    m.remember("episode", "informazione sensibile", &emb, 0.5).unwrap();
    let raw: Vec<u8> = m
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT embedding FROM memories LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let plain: Vec<u8> = emb.iter().flat_map(|x| x.to_le_bytes()).collect();
    assert_ne!(raw, plain, "l'embedding NON deve stare in chiaro a riposo");
    assert!(raw.len() > plain.len(), "atteso nonce+tag attorno al ciphertext");
    // …and recall still decrypts it back correctly
    let hits = m.recall(&emb, 1);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].0.contains("sensibile"));
}

#[test]
fn supersede_removes_from_recall() {
    // kind=episode così il test MORDE: con "fact" l'assenza dalla recall sarebbe
    // vera comunque (esclusione #26) e la supersession non verrebbe esercitata.
    let m = mem();
    let id = m.remember("episode", "vecchio indirizzo", &[1.0, 0.0], 0.5).unwrap();
    assert_eq!(m.memory_count(), 1);
    assert!(m.recall(&[1.0, 0.0], 5).iter().any(|(t, _)| t.contains("vecchio indirizzo")),
            "pre-condizione: prima della supersession DEVE essere richiamabile");
    m.supersede(id).unwrap();
    let hits = m.recall(&[1.0, 0.0], 5);
    assert!(hits.iter().all(|(t, _)| !t.contains("vecchio indirizzo")));
}

#[test]
fn most_similar_fact_finds_match() {
    let m = mem();
    let id = m.remember("fact", "lavora come ingegnere", &[1.0, 0.0, 0.0], 0.6).unwrap();
    let found = m.most_similar_fact(&[1.0, 0.0, 0.0]);
    assert!(found.is_some());
    let (fid, text, sim) = found.unwrap();
    assert_eq!(fid, id);
    assert!(text.contains("ingegnere"));
    assert!(sim > 0.9, "cosine deve essere ~1, era {sim}");
}

#[test]
fn prune_episodes_keeps_latest() {
    let m = mem();
    for i in 0..10 {
        m.remember("episode", &format!("msg {i}"), &[1.0, 0.0], 0.3).unwrap();
    }
    m.prune_episodes(3).unwrap();
    let recent = m.recent_episode_texts(100);
    assert_eq!(recent.len(), 3);
}

#[test]
fn index_reloads_from_disk() {
    // the in-RAM vector index must rebuild (decrypt) from the encrypted store on reopen
    let path = std::env::temp_dir()
        .join(format!("liara_idx_{}.db", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let crypto = Arc::new(Crypto::from_key(&[8u8; 32]));
    {
        let m = Memory::open(&path, crypto.clone()).unwrap();
        m.remember("episode", "vive a Milano", &[1.0, 0.0], 0.7).unwrap();
        m.remember("fact", "ha due gatti", &[0.0, 1.0], 0.7).unwrap();
    }
    let m2 = Memory::open(&path, crypto).unwrap();
    let hits = m2.recall(&[1.0, 0.0], 5);
    assert!(hits.iter().any(|(t, _)| t.contains("Milano")), "l'indice deve ricaricarsi da disco");
    // anche i FACT (esclusi dalla recall #26) devono ricaricarsi: la supersession li usa
    assert!(m2.most_similar_fact(&[0.0, 1.0]).unwrap().1.contains("gatti"),
            "i fact devono ricaricarsi da disco per la supersession");
    std::fs::remove_file(&path).ok();
}

#[test]
fn bump_turn_increments() {
    let m = mem();
    assert_eq!(m.bump_turn(), 1);
    assert_eq!(m.bump_turn(), 2);
    assert_eq!(m.bump_turn(), 3);
}

#[test]
fn location_roundtrip() {
    let m = mem();
    assert!(m.location().is_none());
    m.set_location(41.9, 12.5, "Roma", "gps").unwrap();
    let (lat, lon, label) = m.location().unwrap();
    assert!((lat - 41.9).abs() < 1e-6);
    assert!((lon - 12.5).abs() < 1e-6);
    assert_eq!(label, "Roma");
}

#[test]
fn permissions_roundtrip() {
    let m = mem();
    assert!(m.get_permission("web_fetch").is_none());
    m.set_permission("web_fetch", "allow").unwrap();
    m.set_permission("fs_read", "ask").unwrap();
    assert_eq!(m.get_permission("web_fetch").as_deref(), Some("allow"));
    assert_eq!(m.list_permissions().len(), 2);
}
