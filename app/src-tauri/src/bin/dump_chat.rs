//! Emette il blocco `[AVAILABLE_TOOLS]` Mistral REALE (dal registry, sottoinsieme routed) per la
//! richiesta passata come argomento — così il gate `verify_equiv_mistral.py` lo confronta byte-per-byte
//! con mistral-common (prova anti-drift Rust == mistral-common sui tool VERI del catalogo).
//! Run:  cargo run --quiet --bin dump_chat -- "che tempo fa a modena"
fn main() {
    let req = std::env::args().nth(1).unwrap_or_else(|| "che tempo fa a modena".to_string());
    print!("{}", app_lib::mistral_tools_block(&req));
}
