//! ORACOLO di selezione per-intento: legge richieste da stdin (una per riga) e stampa, per ognuna,
//! le categorie tool che il runtime attiverebbe (`req<TAB>cat1,cat2,…`). Il gate `gate_routing_equiv.py`
//! confronta questo output col port Python (`app_routing`) → prova anti-drift del port dell'algoritmo.
//! Run:  printf 'ciao\nleggi le email\n' | cargo run --quiet --bin select_cats
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let req = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let cats = app_lib::select_categories(&req);
        let _ = writeln!(out, "{req}\t{cats}");
    }
}
