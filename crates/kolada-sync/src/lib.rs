//! Quick Send and device sync for Kolada: pairing, end-to-end encryption and
//! the wire protocol, with no I/O of its own
//! (docs/specs/2026-10-26-sync-quick-send.md).
//!
//! The shells own the sockets, threads, clock and secret store; this crate
//! owns everything else, so every rule here is tested without a network and
//! holds the same on every platform. Nothing here runs before the first
//! frame.

pub mod code;
pub mod crypto;
pub mod engine;
pub mod frame;
pub mod journal;
pub mod merge;
pub mod pair;
pub mod secret;
pub mod store;
pub mod syncer;
pub mod vault;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use code::{parse as parse_code, Code, CodeError};
pub use engine::{Card, Channel, Connect, Device, DeviceId, Engine, Event, Exchange, ItemId, Kind, NotePicture, Outgoing, PairState, Picture, Post, Refusal, SyncCap, Transfer, PAIR_TRIES, SYNC_ANSWER_MS, SYNC_IN_FLIGHT, SYNC_PER_MINUTE};
pub use secret::looks_like_password;

/// The production server.
pub const SERVER: &str = "https://sync.kolada.app";
