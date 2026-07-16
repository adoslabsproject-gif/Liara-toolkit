//! RAG ingestion: chunk a document, embed each chunk and store it in vector memory.
use crate::AppState;
use tauri::State;

/// Split text into sentences, keeping each terminator. Boundaries are `.!?` and newlines.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        if matches!(ch, '.' | '!' | '?' | '\n') {
            if !cur.trim().is_empty() {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Semantic-ish chunking: pack whole SENTENCES up to ~`size` chars, never splitting mid-sentence,
/// and overlap by carrying the previous sentence into the next chunk (preserves continuity).
fn chunk_text(text: &str, size: usize, _overlap: usize) -> Vec<String> {
    let sentences = split_sentences(text);
    let mut chunks = Vec::new();
    let mut buf = String::new();
    let mut last = String::new();
    for s in sentences {
        if !buf.is_empty() && buf.len() + s.len() > size {
            chunks.push(buf.trim().to_string());
            buf = last.clone(); // overlap with the previous sentence
        }
        buf.push_str(&s);
        last = s;
    }
    if !buf.trim().is_empty() {
        chunks.push(buf.trim().to_string());
    }
    chunks.into_iter().filter(|c| c.trim().len() >= 20).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_break_on_sentence_boundaries() {
        let text = "Prima frase corta. Seconda frase un po' più lunga del normale! Terza frase qui? Quarta e ultima frase del testo.";
        let chunks = chunk_text(text, 60, 0);
        assert!(chunks.len() >= 2);
        // no chunk should end mid-sentence (must end with a terminator)
        for c in &chunks {
            let last = c.trim().chars().last().unwrap();
            assert!(matches!(last, '.' | '!' | '?'), "chunk non finisce su confine frase: {c}");
        }
    }

    #[test]
    fn tiny_text_is_one_chunk_or_dropped() {
        assert!(chunk_text("ciao", 600, 0).is_empty()); // <20 chars filtered
        assert_eq!(chunk_text("Questa è una frase abbastanza lunga da restare.", 600, 0).len(), 1);
    }
}

/// Corpo testuale di un documento: i PDF arrivano come base64/dataURL (il frontend non sa leggerne
/// il testo) → decode + estrazione qui; tutto il resto è già testo. Condiviso da `ingest_document`
/// (RAG) e `extract_doc_text` (galleria materiali peer).
fn doc_body(name: &str, text: String) -> anyhow::Result<String> {
    if crate::core::extract::is_pdf(name) {
        use base64::Engine as _;
        let payload = text.rsplit(',').next().unwrap_or(&text);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload.as_bytes())
            .map_err(|e| anyhow::anyhow!("PDF non valido: {e}"))?;
        crate::core::extract::pdf_to_text(&bytes)
            .ok_or_else(|| anyhow::anyhow!("PDF senza testo estraibile (forse scansione)"))
    } else {
        Ok(text)
    }
}

/// Testo leggibile di un documento allegato (PDF → estrazione, altro → passthrough). Usato dalla
/// galleria materiali della chat peer: il testo entra nel contesto di `liara_reply` e viaggia E2E.
#[tauri::command]
pub fn extract_doc_text(name: String, data: String) -> Result<String, String> {
    doc_body(&name, data).map_err(|e| e.to_string())
}

/// Ingest a document into the encrypted vector memory (RAG): chunk → embed → store.
#[tauri::command]
pub async fn ingest_document(name: String, text: String, state: State<'_, AppState>) -> Result<usize, String> {
    let engine_slot = state.engine.clone();
    let memory = state.memory.clone();
    let model_path = state.model_path.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<usize> {
        let engine = {
            // #8 FIX: recupera dal Mutex avvelenato (un panic di load altrove non deve uccidere l'app).
            let mut g = engine_slot.lock().unwrap_or_else(|e| e.into_inner());
            if g.is_none() {
                // #11 FIX: usa load_engine (rispetta needs_partial_gpu + n_ctx per device), non l'hardcoded
                // 8192+999 che forzava tutti i layer su GPU → sui device deboli GPU saturata → ANR/kill.
                *g = Some(std::sync::Arc::new(crate::commands::generate::load_engine(&model_path)?));
            }
            g.as_ref().unwrap().clone()
        };
        // PDFs arrive as base64 (the frontend can't read their text) → decode + extract here
        let body = doc_body(&name, text)?;
        let chunks = chunk_text(&body, 600, 80);
        let mut n = 0;
        for ch in chunks {
            if ch.len() < 20 {
                continue;
            }
            let labeled = format!("[doc:{name}] {ch}");
            if let Ok(e) = engine.embed(&labeled) {
                let _ = memory.remember("doc", &labeled, &e, 0.6);
                n += 1;
            }
        }
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
