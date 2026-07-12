//! Prints the app's REAL tool catalog (JSON) so the LoRA dataset generator reads the same
//! tools the registry exposes — no hand-maintained duplicate that silently drifts.
//! Run:  cargo run --quiet --bin dump_tools
fn main() {
    print!("{}", app_lib::tool_catalog());
}
