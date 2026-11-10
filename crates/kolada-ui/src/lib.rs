//! Kolada's screen as plain data, and the behaviour around it that needs no
//! operating system. Each OS shell draws and reads input; this is what it
//! draws and what the input means.

pub mod agenda;
pub mod blink;
pub mod chrome;
pub mod commands;
pub mod display;
pub mod draft;
pub mod edit;
pub mod found;
pub mod frontmatter;
pub mod export;
pub mod fontfind;
pub mod geom;
pub mod grid;
pub mod images;
pub mod lightbox;
pub mod links;
pub mod math;
pub mod linkfield;
pub mod modelcard;
pub mod pictures;
pub mod preview;
pub mod note;
pub mod palette;
pub mod pixels;
pub mod qr;
pub mod quick;
pub mod raster;
pub mod scene;
pub mod scrub;
pub mod selbar;
pub mod setup;
pub mod share;
pub mod social;
pub mod spell;
/// Fonts for tests, dumps and shells that have none of their own yet.
pub mod testing;
pub mod theme;
pub mod when;
pub mod vector;
pub mod wave;
pub mod welcome;
