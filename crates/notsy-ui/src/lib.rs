//! Notsy's screen as plain data, and the behaviour around it that needs no
//! operating system. Each OS shell draws and reads input; this is what it
//! draws and what the input means.

pub mod agenda;
pub mod chrome;
pub mod display;
pub mod edit;
pub mod geom;
pub mod images;
pub mod links;
pub mod pictures;
pub mod note;
pub mod pixels;
pub mod raster;
pub mod scene;
pub mod scrub;
pub mod setup;
pub mod social;
pub mod spell;
/// Fonts for tests, dumps and shells that have none of their own yet.
pub mod testing;
pub mod theme;
pub mod when;
pub mod vector;
pub mod wave;
