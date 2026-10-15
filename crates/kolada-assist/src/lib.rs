//! The writing model's side of Kolada that every platform shares: what is
//! asked of it, the words it is shown, and the checks an answer must pass
//! before it may touch a note. The model itself runs in its own process
//! (`kolada-ai`); nothing here loads it.
//!
//! Only the note that is open is ever worked on, and only the paragraphs
//! written or changed in it. An answer that does more than fix, arrange or
//! sum up is dropped, never shown.

pub mod gist;
pub mod guard;
pub mod mask;
pub mod models;
pub mod prompt;
pub mod queue;
pub mod shape;
pub mod text;
pub mod wire;
