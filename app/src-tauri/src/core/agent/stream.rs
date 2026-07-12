//! Streaming router: decide, token per token, QUALE parte dell'output del modello mostrare
//! all'utente e quale trattenere/nascondere. Estratto da `run_agent` (review round-3 #4): prima
//! era una closure di ~60 righe con 4 flag intrecciati, corretta ma NON testabile in isolamento —
//! l'unico punto ad alta complessità e zero copertura dell'orchestrazione. Ora è una struct con
//! una macchina a stati esplicita e una batteria di test.
//!
//! Tre responsabilità, nell'ordine:
//!   1. **thinking channel** (Gemma 12B, `<|channel>…​<channel|>`): ragionamento interno → nascosto;
//!   2. **tool-call block** (`<tool_call…` Qwen o `<|tool_call…` Gemma): da qui in poi si sopprime tutto
//!      (il JSON del tool non va mostrato); la prosa PRIMA del blocco viene emessa;
//!   3. **coda-marker parziale**: se la coda potrebbe essere l'inizio di un marker tool-call, la si
//!      trattiene finché non si sa se diventa davvero un tool-call.
use super::parse::toolcall_prefix_tail;

pub(super) struct StreamRouter {
    raw: String,      // tutto l'output grezzo accumulato finora
    emitted: usize,   // quanti byte di `raw` sono già stati mandati alla UI
    suppress: bool,   // true una volta iniziato un blocco tool-call → non emettere più nulla
    channel_done: bool, // true una volta superato il thinking-channel di Gemma
}

impl StreamRouter {
    pub(super) fn new() -> Self {
        Self { raw: String::new(), emitted: 0, suppress: false, channel_done: false }
    }

    /// Accumula un pezzo di token e ritorna il testo da mostrare ORA (stringa vuota se non c'è
    /// nulla di emettibile: siamo dentro un tool-call, dentro un channel aperto, o su una coda
    /// parziale di marker).
    pub(super) fn push(&mut self, piece: &str) -> String {
        self.raw.push_str(piece);
        if self.suppress {
            return String::new();
        }
        // 1) thinking channel (Gemma 12B): finché è APERTO trattieni la coda; a canale CHIUSO
        //    avanza oltre il blocco e riprendi lo streaming normale.
        if !self.channel_done {
            if let Some(ch) = self.raw.find("<|channel>") {
                match self.raw[ch..].find("<channel|>") {
                    None => {
                        // canale aperto: emetti solo la prosa PRIMA di <|channel>, poi ferma
                        return self.take(ch);
                    }
                    Some(rel) => {
                        self.emitted = self.emitted.max(ch + rel + "<channel|>".len());
                        self.channel_done = true;
                    }
                }
            }
        }
        // 2) primo marker di tool-call dopo l'emesso: Qwen `<tool_call` o Gemma `<|tool_call`
        if let Some(tc) = self.next_toolcall_marker() {
            let out = self.take(tc);
            self.emitted = self.raw.len(); // da qui in poi tutto soppresso
            self.suppress = true;
            return out;
        }
        // 3) emetti fino a dove NON c'è una coda-parziale di marker
        let upto = self.raw.len() - toolcall_prefix_tail(&self.raw);
        self.take(upto)
    }

    /// Fine generazione: emetti l'eventuale coda rimasta, a meno che si sia dentro un tool-call o
    /// un thinking-channel ancora APERTO (in quel caso la coda è tutta roba da nascondere).
    pub(super) fn finish(&mut self) -> String {
        if self.suppress {
            return String::new();
        }
        let channel_open =
            !self.channel_done && self.raw[self.emitted.min(self.raw.len())..].contains("<|channel>");
        if channel_open {
            return String::new();
        }
        let upto = self.raw.len() - toolcall_prefix_tail(&self.raw);
        self.take(upto)
    }

    /// L'output grezzo completo (per l'estrazione del tool-call e lo strip finale dei marker).
    pub(super) fn into_raw(self) -> String {
        self.raw
    }

    /// Emette `raw[emitted..upto]` e avanza `emitted`. Vuoto se `upto <= emitted`.
    fn take(&mut self, upto: usize) -> String {
        if upto > self.emitted {
            let out = self.raw[self.emitted..upto].to_string();
            self.emitted = upto;
            out
        } else {
            String::new()
        }
    }

    /// Posizione del primo marker di tool-call (Qwen o Gemma) DOPO `emitted`, se presente.
    fn next_toolcall_marker(&self) -> Option<usize> {
        let tail = &self.raw[self.emitted..];
        [tail.find("<|tool_call"), tail.find("<tool_call")]
            .into_iter()
            .flatten()
            .min()
            .map(|p| p + self.emitted)
    }
}

#[cfg(test)]
mod tests {
    use super::StreamRouter;

    /// Fa passare l'intero output pezzo per pezzo e concatena ciò che viene emesso alla UI.
    fn drive(pieces: &[&str]) -> String {
        let mut r = StreamRouter::new();
        let mut out = String::new();
        for p in pieces {
            out.push_str(&r.push(p));
        }
        out.push_str(&r.finish());
        out
    }

    #[test]
    fn testo_semplice_passa_intatto() {
        assert_eq!(drive(&["Ciao, ", "come ", "stai?"]), "Ciao, come stai?");
    }

    #[test]
    fn prosa_prima_del_toolcall_emessa_json_soppresso() {
        // la frase prima del <tool_call> si vede; il JSON del tool NO.
        let out = drive(&["Controllo subito.\n", "<tool_call>\n{\"name\":\"datetime\"}", "\n</tool_call>"]);
        assert_eq!(out, "Controllo subito.\n");
    }

    #[test]
    fn toolcall_gemma_soppresso() {
        let out = drive(&["Ecco ", "<|tool_call>call:weather{}", "<tool_call|>"]);
        assert_eq!(out, "Ecco ");
    }

    #[test]
    fn coda_parziale_di_marker_trattenuta_poi_emessa() {
        // "<too" potrebbe diventare "<tool_call" → trattenuto finché si scioglie in testo normale.
        let mut r = StreamRouter::new();
        let a = r.push("testo <too");
        assert_eq!(a, "testo "); // "<too" trattenuto
        let b = r.push("laggia va bene"); // non era un tool-call
        assert_eq!(format!("{a}{b}{}", r.finish()), "testo <toolaggia va bene");
    }

    #[test]
    fn thinking_channel_gemma_nascosto() {
        // <|channel>…<channel|> è ragionamento interno → non mostrato; resta la risposta.
        let out = drive(&["<|channel>thought\nuso weather\n<channel|>", "A Modena 26°C."]);
        assert_eq!(out, "A Modena 26°C.");
    }

    #[test]
    fn channel_aperto_troncato_non_emette_ragionamento() {
        // canale mai chiuso (risposta troncata a metà ragionamento) → niente da mostrare.
        assert_eq!(drive(&["<|channel>thought\nsto ragionando"]), "");
    }

    #[test]
    fn prosa_prima_del_channel_emessa() {
        let out = drive(&["Ok. ", "<|channel>thought\nx\n<channel|>", "Fatto."]);
        assert_eq!(out, "Ok. Fatto.");
    }
}
