//! Prints the app's REAL tool ROUTING (JSON): tools in registration order (each with its category)
//! + the per-category keyword table of `selected_categories`. The LoRA dataset generator reads this
//! to select tools per-intent EXACTLY as the runtime does — no hand-maintained duplicate that drifts.
//! Run:  cargo run --quiet --bin dump_routing
fn main() {
    print!("{}", app_lib::tool_routing());
}
