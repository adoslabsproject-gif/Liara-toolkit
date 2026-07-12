//! Conversation orchestration: prompt formatting + tool loop + memory extraction.
mod agent_loop;
mod format;
mod parse;
mod stream;

pub use agent_loop::{run_agent, AgentSink};
pub use format::{extraction_prompt, format_chat, Message, SYSTEM_PROMPT};
pub use parse::parse_facts;
