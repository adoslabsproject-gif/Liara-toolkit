//! Streaming UTF-8 incrementale per i token del modello (dedup review 2026-07-02 #5).
//!
//! Un token può spezzare un carattere multibyte a metà: i byte vanno accumulati
//! ed emessi SOLO come prefissi UTF-8 validi. Le due copie precedenti (in
//! `generate()` e `describe()`) avevano lo stesso difetto latente: un byte
//! DEFINITIVAMENTE invalido (error_len=Some) non veniva mai scartato → il buffer
//! si inceppava e da lì in poi non usciva più nulla. Qui: gli invalidi si
//! scartano e lo stream prosegue; un multibyte a metà (error_len=None) si
//! attende com'è giusto.

pub(crate) struct Utf8Stream {
    buf: Vec<u8>,
}

impl Utf8Stream {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Aggiunge i byte di un token e ritorna il testo emettibile ora
    /// (stringa vuota se il buffer è ancora a metà di un carattere).
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.buf.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.buf) {
                Ok(s) => {
                    out.push_str(s);
                    self.buf.clear();
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    if valid > 0 {
                        // SAFETY: from_utf8 garantisce che [..valid] è UTF-8 valido.
                        out.push_str(unsafe { std::str::from_utf8_unchecked(&self.buf[..valid]) });
                    }
                    match e.error_len() {
                        // sequenza invalida certa: scarta e continua (anti-jam)
                        Some(n) => {
                            self.buf.drain(..valid + n);
                        }
                        // multibyte incompleto in coda: tieni e aspetta i prossimi byte
                        None => {
                            self.buf.drain(..valid);
                            break;
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Utf8Stream;

    #[test]
    fn multibyte_spezzato_su_due_push() {
        let mut s = Utf8Stream::new();
        let emoji = "così😀".as_bytes(); // 'ì' e l'emoji sono multibyte
        let (a, b) = emoji.split_at(6); // taglio a metà dell'emoji
        let first = s.push(a);
        let second = s.push(b);
        assert_eq!(format!("{first}{second}"), "così😀");
        assert!(first.len() < "così😀".len(), "la prima metà non deve emettere l'emoji intera");
    }

    #[test]
    fn byte_invalido_non_incepppa_lo_stream() {
        // ANTI-REGRESSIONE: il vecchio codice di describe() (e in parte generate())
        // dopo un byte invalido non emetteva PIÙ NULLA. Qui lo scarta e prosegue.
        let mut s = Utf8Stream::new();
        let out1 = s.push(&[b'c', b'i', 0xFF, 0xFE]); // 0xFF/0xFE mai validi in UTF-8
        assert_eq!(out1, "ci");
        let out2 = s.push("ao".as_bytes());
        assert_eq!(out2, "ao", "dopo i byte invalidi lo stream deve continuare");
    }

    #[test]
    fn ascii_passa_intatto() {
        let mut s = Utf8Stream::new();
        assert_eq!(s.push(b"hello "), "hello ");
        assert_eq!(s.push(b"world"), "world");
    }
}
