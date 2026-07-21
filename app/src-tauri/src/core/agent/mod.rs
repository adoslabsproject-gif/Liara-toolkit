//! Conversation orchestration: prompt formatting + tool loop + memory extraction.
mod agent_loop;
mod format;
mod parse;
mod stream;

pub use agent_loop::{run_agent, AgentSink};
pub use format::{extraction_prompt, format_chat, Dialect, Message, SYSTEM_PROMPT};
pub(crate) use format::to_mistral_json; // wire-format JSON Mistral, usato dalla registry per [AVAILABLE_TOOLS]
pub use parse::parse_facts;
pub use stream::render_full;
