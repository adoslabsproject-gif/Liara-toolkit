//! Liara core: all logic, compiled into the app on every platform.
pub mod agent;
#[cfg(target_os = "android")]
pub mod android_ctx;
pub mod audio;
pub mod calendar;
pub mod contacts;
pub mod crypto;
pub mod email;
#[cfg(test)]
mod eval;
pub mod extract;
pub mod mcp;
pub mod paths;
pub mod peer;
pub mod sms;
pub mod engine;
pub mod memory;
pub mod tools;
