//! Built-in tools. Start small and rock-solid; the loop is proven on these.
//! Split by category; everything is re-exported so `super::builtin::{...}` keeps working.
mod basic;
mod calendar_tools;
mod email_tools;
mod files;
mod files_write;
mod notes;
mod weather;
mod web;

pub use basic::{Calculator, DateTime};
pub use calendar_tools::{CalendarAdd, CalendarDelete, CalendarList, CalendarSearch};
pub use email_tools::{EmailDraft, EmailRecent, EmailReply, EmailSearch, EmailSent};
pub use files::{FsList, FsRead, FsSearch};
pub use files_write::{FsDelete, FsMove, FsWrite};
pub use notes::{NoteAdd, NoteList, NoteSearch};
pub use weather::{SetLocation, Weather};
pub use web::{WebFetch, WebSearch};
