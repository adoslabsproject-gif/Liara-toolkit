//! Tauri command handlers, grouped by domain. Each is re-exported so `lib.rs`
//! can list them in `tauri::generate_handler![...]`.
pub mod audio;
pub mod calendar;
pub mod consent;
pub mod contacts;
pub mod download;
pub mod email;
pub mod generate;
pub mod memory;
pub mod model_files;
pub mod peer;
pub mod peer_ai;
pub mod phone;
pub mod rag;
pub mod remote;
pub mod sink;
pub mod sms;
pub mod vision;

pub use audio::*;
pub use calendar::*;
pub use consent::*;
pub use contacts::*;
pub use download::*;
pub use email::*;
pub use generate::*;
pub use memory::*;
pub use model_files::*;
pub use peer::*;
pub use peer_ai::*;
pub use phone::*;
pub use rag::*;
pub use remote::*;
pub use sms::*;
pub use vision::*;
