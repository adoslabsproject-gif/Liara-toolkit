//! Document text extraction. PDFs are the bulk of a person's real files, so fs_read and the
//! RAG ingest both route binary documents through here instead of failing as "non leggibile".

/// Extract UTF-8 text from PDF bytes. Returns None if it's not a parseable/textual PDF.
///
/// ROBUSTEZZA (review round-4 #2): `pdf_extract` NON ritorna sempre `Err` sui PDF malformati — su
/// molti va in **panic**, e `.ok()` cattura solo l'`Err`, non il panic. Senza guardia, un allegato
/// PDF rotto (fs_read o ingest_document) faceva morire il turno con un errore criptico e rischiava di
/// avvelenare un Mutex. `catch_unwind` isola il panic e lo tratta come "PDF non estraibile" → None.
/// (Il profilo è `panic=unwind`, quindi catch_unwind funziona.)
pub fn pdf_to_text(bytes: &[u8]) -> Option<String> {
    let extracted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(bytes).ok()
    }));
    match extracted {
        Ok(Some(text)) => {
            let text = text.trim().to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        // `Ok(None)` = estrazione fallita in modo pulito; `Err(_)` = pdf-extract è andato in panic.
        _ => None,
    }
}

/// True if the filename looks like a PDF.
pub fn is_pdf(name: &str) -> bool {
    name.to_lowercase().ends_with(".pdf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pdf_by_extension() {
        assert!(is_pdf("relazione.pdf"));
        assert!(is_pdf("RELAZIONE.PDF"));
        assert!(!is_pdf("note.txt"));
        assert!(!is_pdf("immagine.png"));
    }

    #[test]
    fn non_pdf_bytes_yield_none() {
        assert!(pdf_to_text(b"questo non e un pdf").is_none());
    }
}
