//! Prints the app's REAL agent SYSTEM_PROMPT so the LoRA dataset is generated with the IDENTICAL
//! system prompt the runtime uses — no hand-maintained copy that silently drifts (anti-drift, like
//! dump_tools does for the tool catalog). Run:  cargo run --quiet --bin dump_prompt
fn main() {
    print!("{}", app_lib::core::agent::SYSTEM_PROMPT);
}
