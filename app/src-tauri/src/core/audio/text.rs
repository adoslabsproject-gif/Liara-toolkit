//! Post-processing for dictated text. Whisper-base punctuation is weak in Italian, so we
//! add a trailing "?" when the utterance clearly reads as a question.

/// Italian interrogatives that, when they START the phrase, signal a question.
const Q_STARTS: &[&str] = &[
    "chi", "che", "cosa", "come", "com'", "cos'", "dove", "dov'", "quando", "perché", "perche",
    "perchè", "quale", "quali", "qual", "quanto", "quanta", "quanti", "quante", "quant'", "puoi",
    "può", "puo", "sai", "saresti", "potresti", "hai", "vuoi", "mi dici", "mi sai", "ci sono",
];

/// Append "?" if the text looks like a question and has no ending punctuation.
pub fn punctuate_question(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() || t.ends_with('?') || t.ends_with('!') || t.ends_with('.') {
        return t.to_string();
    }
    let lower = t.to_lowercase();
    let first = lower.split_whitespace().next().unwrap_or("");
    let looks_q = Q_STARTS.iter().any(|w| {
        // word-boundary match on the first token, or a two-word starter like "mi dici"
        first == *w || lower.starts_with(&format!("{w} "))
    });
    if looks_q {
        format!("{t}?")
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_q_for_interrogatives() {
        assert_eq!(punctuate_question("che ore sono"), "che ore sono?");
        assert_eq!(punctuate_question("Come stai"), "Come stai?");
        assert_eq!(punctuate_question("dove sei"), "dove sei?");
        assert_eq!(punctuate_question("puoi aiutarmi"), "puoi aiutarmi?");
        assert_eq!(punctuate_question("perché piove"), "perché piove?");
    }

    #[test]
    fn leaves_statements_alone() {
        assert_eq!(punctuate_question("oggi piove"), "oggi piove");
        assert_eq!(punctuate_question("ricordami di chiamare"), "ricordami di chiamare");
    }

    #[test]
    fn keeps_existing_punctuation() {
        assert_eq!(punctuate_question("che ore sono?"), "che ore sono?");
        assert_eq!(punctuate_question("vai!"), "vai!");
        assert_eq!(punctuate_question("ok."), "ok.");
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(punctuate_question("   "), "");
    }

    #[test]
    fn does_not_match_substring_in_other_words() {
        // "chissà" starts with "chi" but is not the interrogative "chi"
        assert_eq!(punctuate_question("chissà se viene"), "chissà se viene");
    }
}
