//! Built-in tools. Start small and rock-solid; the loop is proven on these.
//! Split by category; everything is re-exported so `super::builtin::{...}` keeps working.
mod basic;
mod calendar_tools;
mod contacts_tools;
mod email_tools;
mod files;
mod files_write;
mod notes;
mod peer;
mod phone;
mod sms_tools;
pub(crate) mod weather; // geocode riusato da set_manual_location
mod web;

pub use basic::{Calculator, DateTime};
pub use calendar_tools::{CalendarAdd, CalendarDelete, CalendarList, CalendarSearch, CalendarUpdate};
pub use contacts_tools::ContactSearch;
pub use email_tools::{EmailDraft, EmailRecent, EmailReply, EmailSearch, EmailSend, EmailSent};
pub use files::{FsList, FsRead, FsSearch};
pub use files_write::{FsDelete, FsMove, FsWrite};
pub use notes::{NoteAdd, NoteList, NoteSearch};
pub use peer::{PeerAsk, PeerConnect, PeerProposeSlot};
pub use phone::{PhoneCall, SmsSend};
pub use sms_tools::{SmsRecent, SmsSearch};
pub use weather::{MyLocation, SetLocation, Weather};
pub use web::{WebFetch, WebSearch};
