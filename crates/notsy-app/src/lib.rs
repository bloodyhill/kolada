//! Notsy's app model, shared by every OS shell.
//!
//! [`model::Model`] is the whole state of the app (the session, the open
//! note, search, the trash, the time scrubber, settings, find, pictures, the
//! agenda, favorites, spelling...) and what it draws, driven by plain calls:
//! frames, edits, UTF-16 text input, pointer, keys. It has no window and no
//! OS calls. What it needs from the OS comes in through [`services::Services`],
//! which the shell builds and gives to the model; what it wants the OS to do
//! (open a URL, copy to the pasteboard, show a window) it hands back as
//! [`model::share::Effect`]s for the shell to perform.

pub mod gists;
pub mod helper;
pub mod launch;
pub mod model;
pub mod services;
mod when;

pub use model::Model;
pub use services::Services;
