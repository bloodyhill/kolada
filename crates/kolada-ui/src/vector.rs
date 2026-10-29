//! Marks drawn from vector outlines, not typed: the social platforms'
//! logos. SVG path data is flattened to polygons and filled, with 16
//! scanlines a pixel and exact coverage across each, into a mask at the
//! size the mark is drawn; the mask, in an ink's color, is drawn as a
//! picture. Masks and colored marks are kept, so a frame reuses them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::pixels::Pixels;

/// The logos, as their makers publish them (24-unit squares, X's 1024).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Logo {
    LinkedIn,
    Instagram,
    Facebook,
    X,
    YouTube,
    /// Kolada's bird (`assets/logo.svg`), and its eye, drawn over it in white.
    Bird,
    BirdEye,
    /// Lucide's star and folder (allsvgicons.com, ISC), their strokes
    /// outlined into fills by `tools/iconfont --outline`.
    Star,
    Folder,
    /// Lucide's open folder and note (`file-text`), outlined the same way.
    FolderOpen,
    Note,
    /// Carbon's recently-viewed clock (allsvgicons.com, Apache 2.0): Recent.
    Recent,
    /// Basil's add-outline (allsvgicons.com, CC BY 4.0, Craftwork): the
    /// side menu's New note, on every platform. Its plus sits in the ring's
    /// hole, so one even-odd fill draws both.
    NewNote,
    /// Beam's mark: a dot and two quarter rings about it, filled.
    Beam,
    /// A device in Settings > Devices: a computer (its lid over its base)
    /// and a phone, solid, each screen cut out as negative space; drawn
    /// here, fills only, even-odd.
    Laptop,
    Phone,
    /// A sent beam's check, white on its teal disc: a filled tick.
    Check,
}

impl Logo {
    /// Path data, the view box's size, and whether it fills even-odd.
    fn outline(self) -> (&'static str, f32, bool) {
        match self {
            Logo::LinkedIn => (
                "M19 3a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zm-.5 15.5v-5.3a3.26 3.26 0 0 0-3.26-3.26c-.85 0-1.84.52-2.32 1.3v-1.11h-2.79v8.37h2.79v-4.93c0-.77.62-1.4 1.39-1.4a1.4 1.4 0 0 1 1.4 1.4v4.93zM6.88 8.56a1.68 1.68 0 0 0 1.68-1.68c0-.93-.75-1.69-1.68-1.69a1.69 1.69 0 0 0-1.69 1.69c0 .93.76 1.68 1.69 1.68m1.39 9.94v-8.37H5.5v8.37z",
                24.0,
                false,
            ),
            Logo::Instagram => (
                "M16.125 3c2.62 0 4.75 2.13 4.75 4.75v8.5c0 2.62-2.13 4.75-4.75 4.75h-8.25c-2.62 0-4.75-2.13-4.75-4.75v-8.5c0-2.62 2.13-4.75 4.75-4.75zM12 7.25c-2.62 0-4.75 2.13-4.75 4.75s2.13 4.75 4.75 4.75s4.75-2.13 4.75-4.75S14.62 7.25 12 7.25m0 1.5a3.25 3.25 0 1 1 0 6.5a3.25 3.25 0 0 1 0-6.5m5.5-3.25c-.55 0-1 .45-1 1s.45 1 1 1s1-.45 1-1s-.45-1-1-1",
                24.0,
                false,
            ),
            Logo::Facebook => (
                "M19.75 3H4.25C3.56 3 3 3.56 3 4.25v15.5c0 .69.56 1.25 1.25 1.25h8.25v-7.5H10V12h2.5v-1.75a2.755 2.755 0 0 1 2.75-2.75h2.25V9h-2.25c-.69 0-1.25.56-1.25 1.25V12h2.5v1.5H14V21h5.75c.69 0 1.25-.56 1.25-1.25V4.25C21 3.56 20.44 3 19.75 3",
                24.0,
                false,
            ),
            Logo::X => (
                "M823.111 912H200.89C151.8 912 112 872.2 112 823.111V200.89c0-49.09 39.8-88.89 88.889-88.89H823.11c49.09 0 88.89 39.8 88.89 88.889V823.11c0 49.09-39.8 88.89-88.889 88.89M286 291l171.773 245.278L286 735h45.297l146.569-170.031L596.942 735H740L555.891 472.108L715 289h-44.67L535.793 443.41L429.058 291zm69.053 37.65l258.935 368.699h56.96L412.013 328.65z",
                1024.0,
                true,
            ),
            // Simple Icons' YouTube (CC0): the rounded screen, its play
            // button cut out.
            Logo::Star => ("M10.75 1.66L10.90 1.48L12.45 2.74L12.30 2.93ZM11.24 1.21L11.45 1.11L12.31 2.92L12.10 3.02ZM11.88 1.01L12.12 1.01L12.12 3.01L11.88 3.01ZM12.55 1.11L12.76 1.21L11.90 3.02L11.69 2.92ZM13.10 1.48L13.25 1.66L11.70 2.93L11.55 2.74ZM13.37 1.85L15.68 6.53L13.89 7.42L11.58 2.74ZM15.55 6.33L16.20 7.11L14.67 8.40L14.02 7.62ZM15.82 6.82L16.76 7.21L16.00 9.06L15.06 8.68ZM16.52 7.14L21.69 7.90L21.40 9.88L16.24 9.12ZM21.90 7.96L22.12 8.04L21.41 9.91L21.19 9.82ZM22.49 8.29L22.65 8.45L21.20 9.83L21.04 9.66ZM22.88 8.83L22.95 9.06L21.05 9.68L20.97 9.45ZM22.99 9.50L22.96 9.73L20.98 9.47L21.01 9.24ZM22.81 10.14L22.68 10.34L21.00 9.25L21.13 9.05ZM22.54 10.51L18.80 14.15L17.41 12.72L21.14 9.08ZM18.95 13.96L18.42 14.82L16.72 13.77L17.26 12.90ZM18.56 14.37L18.49 15.38L16.50 15.24L16.57 14.22ZM18.48 15.14L19.36 20.28L17.39 20.62L16.51 15.48ZM19.37 20.50L19.36 20.73L17.36 20.64L17.38 20.40ZM19.24 21.16L19.13 21.37L17.37 20.42L17.48 20.21ZM18.84 21.70L18.65 21.84L17.47 20.22L17.66 20.08ZM18.24 22.01L18.01 22.06L17.65 20.09L17.88 20.05ZM17.57 22.04L17.34 21.97L17.87 20.05L18.09 20.11ZM17.14 21.90L12.52 19.47L13.45 17.70L18.07 20.12ZM12.75 19.55L11.76 19.31L12.24 17.37L13.23 17.61ZM12.24 19.31L11.25 19.55L10.77 17.61L11.76 17.37ZM11.48 19.47L6.86 21.90L5.93 20.12L10.55 17.70ZM6.66 21.98L6.43 22.04L5.91 20.11L6.14 20.04ZM5.99 22.05L5.76 22.01L6.12 20.05L6.35 20.09ZM5.35 21.84L5.16 21.70L6.34 20.08L6.53 20.22ZM4.87 21.37L4.76 21.16L6.52 20.21L6.63 20.41ZM4.64 20.73L4.63 20.50L6.62 20.40L6.64 20.63ZM4.64 20.28L5.52 15.14L7.49 15.48L6.61 20.62ZM5.51 15.38L5.44 14.37L7.43 14.22L7.50 15.24ZM5.58 14.82L5.05 13.96L6.74 12.90L7.28 13.77ZM5.20 14.15L1.46 10.51L2.86 9.08L6.59 12.72ZM1.32 10.34L1.19 10.15L2.87 9.05L3.00 9.25ZM1.04 9.73L1.01 9.50L2.99 9.23L3.02 9.47ZM1.05 9.06L1.12 8.83L3.02 9.45L2.95 9.67ZM1.35 8.45L1.51 8.28L2.96 9.66L2.80 9.83ZM1.88 8.04L2.10 7.95L2.81 9.82L2.59 9.91ZM2.31 7.90L7.47 7.14L7.76 9.12L2.60 9.88ZM7.24 7.21L8.18 6.82L8.94 8.68L8.00 9.06ZM7.80 7.11L8.45 6.33L9.98 7.62L9.33 8.40ZM8.32 6.53L10.63 1.85L12.42 2.74L10.11 7.42ZM11.52 2.29L12.42 2.74L12.30 2.93ZM11.52 2.29L10.63 1.85L10.75 1.66ZM11.67 2.11L12.45 2.74L12.10 3.02ZM11.67 2.11L10.90 1.48L11.24 1.21ZM11.88 2.01L12.31 2.92L11.88 3.01ZM11.88 2.01L11.45 1.11L11.88 1.01ZM12.12 2.01L12.12 3.01L11.69 2.92ZM12.12 2.01L12.12 1.01L12.55 1.11ZM12.33 2.11L11.90 3.02L11.55 2.74ZM12.33 2.11L12.76 1.21L13.10 1.48ZM12.48 2.29L11.70 2.93L11.58 2.74ZM12.48 2.29L13.25 1.66L13.37 1.85ZM14.02 7.62L13.89 7.42L14.78 6.97ZM15.55 6.33L15.68 6.53L14.78 6.97ZM15.06 8.68L14.67 8.40L15.44 7.75ZM15.82 6.82L16.20 7.11L15.44 7.75ZM16.24 9.12L16.00 9.06L16.38 8.13ZM16.52 7.14L16.76 7.21L16.38 8.13ZM21.55 8.89L21.40 9.88L21.19 9.82ZM21.55 8.89L21.69 7.90L21.90 7.96ZM21.76 8.97L21.41 9.91L21.04 9.66ZM21.76 8.97L22.12 8.04L22.49 8.29ZM21.93 9.14L21.20 9.83L20.97 9.45ZM21.93 9.14L22.65 8.45L22.88 8.83ZM22.00 9.37L21.05 9.68L21.01 9.24ZM22.00 9.37L22.95 9.06L22.99 9.50ZM21.97 9.60L20.98 9.47L21.13 9.05ZM21.97 9.60L22.96 9.73L22.81 10.14ZM21.84 9.79L21.00 9.25L21.14 9.08ZM21.84 9.79L22.68 10.34L22.54 10.51ZM17.26 12.90L17.41 12.72L18.10 13.43ZM18.95 13.96L18.80 14.15L18.10 13.43ZM16.57 14.22L16.72 13.77L17.57 14.30ZM18.56 14.37L18.42 14.82L17.57 14.30ZM16.51 15.48L16.50 15.24L17.49 15.31ZM18.48 15.14L18.49 15.38L17.49 15.31ZM18.38 20.45L17.39 20.62L17.38 20.40ZM18.38 20.45L19.36 20.28L19.37 20.50ZM18.36 20.68L17.36 20.64L17.48 20.21ZM18.36 20.68L19.36 20.73L19.24 21.16ZM18.25 20.89L17.37 20.42L17.66 20.08ZM18.25 20.89L19.13 21.37L18.84 21.70ZM18.06 21.03L17.47 20.22L17.88 20.05ZM18.06 21.03L18.65 21.84L18.24 22.01ZM17.83 21.07L17.65 20.09L18.09 20.11ZM17.83 21.07L18.01 22.06L17.57 22.04ZM17.60 21.01L17.87 20.05L18.07 20.12ZM17.60 21.01L17.34 21.97L17.14 21.90ZM13.23 17.61L13.45 17.70L12.99 18.58ZM12.75 19.55L12.52 19.47L12.99 18.58ZM11.76 17.37L12.24 17.37L12.00 18.34ZM12.24 19.31L11.76 19.31L12.00 18.34ZM10.55 17.70L10.77 17.61L11.01 18.58ZM11.48 19.47L11.25 19.55L11.01 18.58ZM6.40 21.01L5.93 20.12L6.14 20.04ZM6.40 21.01L6.86 21.90L6.66 21.98ZM6.17 21.07L5.91 20.11L6.35 20.09ZM6.17 21.07L6.43 22.04L5.99 22.05ZM5.94 21.03L6.12 20.05L6.53 20.22ZM5.94 21.03L5.76 22.01L5.35 21.84ZM5.75 20.89L6.34 20.08L6.63 20.41ZM5.75 20.89L5.16 21.70L4.87 21.37ZM5.64 20.68L6.52 20.21L6.64 20.63ZM5.64 20.68L4.76 21.16L4.64 20.73ZM5.63 20.45L6.62 20.40L6.61 20.62ZM5.63 20.45L4.63 20.50L4.64 20.28ZM7.50 15.24L7.49 15.48L6.51 15.31ZM5.51 15.38L5.52 15.14L6.51 15.31ZM7.28 13.77L7.43 14.22L6.43 14.30ZM5.58 14.82L5.44 14.37L6.43 14.30ZM6.59 12.72L6.74 12.90L5.90 13.43ZM5.20 14.15L5.05 13.96L5.90 13.43ZM2.16 9.80L2.86 9.08L3.00 9.25ZM2.16 9.80L1.46 10.51L1.32 10.34ZM2.03 9.60L2.87 9.05L3.02 9.47ZM2.03 9.60L1.19 10.15L1.04 9.73ZM2.00 9.37L2.99 9.23L2.95 9.67ZM2.00 9.37L1.01 9.50L1.05 9.06ZM2.07 9.14L3.02 9.45L2.80 9.83ZM2.07 9.14L1.12 8.83L1.35 8.45ZM2.23 8.97L2.96 9.66L2.59 9.91ZM2.23 8.97L1.51 8.28L1.88 8.04ZM2.45 8.89L2.81 9.82L2.60 9.88ZM2.45 8.89L2.10 7.95L2.31 7.90ZM8.00 9.06L7.76 9.12L7.62 8.13ZM7.24 7.21L7.47 7.14L7.62 8.13ZM9.33 8.40L8.94 8.68L8.56 7.75ZM7.80 7.11L8.18 6.82L8.56 7.75ZM10.11 7.42L9.98 7.62L9.22 6.97ZM8.32 6.53L8.45 6.33L9.22 6.97Z", 24.0, false),
            Logo::Folder => ("M19.74 19.03L20.74 18.77L21.26 20.70L20.26 20.97ZM20.29 19.02L21.02 18.29L22.44 19.71L21.71 20.44ZM20.77 18.74L21.03 17.74L22.97 18.26L22.70 19.26ZM21.00 18.00L21.00 8.00L23.00 8.00L23.00 18.00ZM21.03 8.26L20.77 7.26L22.70 6.74L22.97 7.74ZM21.02 7.71L20.29 6.98L21.71 5.56L22.44 6.29ZM20.74 7.23L19.74 6.97L20.26 5.03L21.26 5.30ZM20.00 7.00L12.10 7.00L12.10 5.00L20.00 5.00ZM11.86 6.97L10.90 6.74L11.38 4.79L12.34 5.03ZM10.47 6.50L9.74 5.84L11.08 4.36L11.81 5.03ZM9.58 5.66L8.77 4.46L10.43 3.34L11.24 4.54ZM8.92 4.64L8.20 3.98L9.55 2.50L10.28 3.16ZM8.63 4.21L7.69 3.97L8.17 2.03L9.12 2.27ZM7.93 4.00L4.00 4.00L4.00 2.00L7.93 2.00ZM4.26 3.97L3.26 4.23L2.74 2.30L3.74 2.03ZM3.71 3.98L2.98 4.71L1.56 3.29L2.29 2.56ZM3.23 4.26L2.97 5.26L1.03 4.74L1.30 3.74ZM3.00 5.00L3.00 18.00L1.00 18.00L1.00 5.00ZM2.97 17.74L3.23 18.74L1.30 19.26L1.03 18.26ZM2.98 18.29L3.71 19.02L2.29 20.44L1.56 19.71ZM3.26 18.77L4.26 19.03L3.74 20.97L2.74 20.70ZM4.00 19.00L20.00 19.00L20.00 21.00L4.00 21.00ZM20.26 20.97L20.00 21.00L20.00 20.00ZM19.74 19.03L20.00 19.00L20.00 20.00ZM21.71 20.44L21.26 20.70L21.00 19.73ZM20.29 19.02L20.74 18.77L21.00 19.73ZM22.70 19.26L22.44 19.71L21.73 19.00ZM20.77 18.74L21.02 18.29L21.73 19.00ZM23.00 18.00L22.97 18.26L22.00 18.00ZM21.00 18.00L21.03 17.74L22.00 18.00ZM22.97 7.74L23.00 8.00L22.00 8.00ZM21.03 8.26L21.00 8.00L22.00 8.00ZM22.44 6.29L22.70 6.74L21.73 7.00ZM21.02 7.71L20.77 7.26L21.73 7.00ZM21.26 5.30L21.71 5.56L21.00 6.27ZM20.74 7.23L20.29 6.98L21.00 6.27ZM20.00 5.00L20.26 5.03L20.00 6.00ZM20.00 7.00L19.74 6.97L20.00 6.00ZM12.10 6.00L12.10 5.00L12.34 5.03ZM12.10 6.00L12.10 7.00L11.86 6.97ZM11.14 5.77L11.38 4.79L11.81 5.03ZM11.14 5.77L10.90 6.74L10.47 6.50ZM10.41 5.10L11.08 4.36L11.24 4.54ZM10.41 5.10L9.74 5.84L9.58 5.66ZM10.28 3.16L10.43 3.34L9.60 3.90ZM8.92 4.64L8.77 4.46L9.60 3.90ZM9.12 2.27L9.55 2.50L8.88 3.24ZM8.63 4.21L8.20 3.98L8.88 3.24ZM7.93 2.00L8.17 2.03L7.93 3.00ZM7.93 4.00L7.69 3.97L7.93 3.00ZM3.74 2.03L4.00 2.00L4.00 3.00ZM4.26 3.97L4.00 4.00L4.00 3.00ZM2.29 2.56L2.74 2.30L3.00 3.27ZM3.71 3.98L3.26 4.23L3.00 3.27ZM1.30 3.74L1.56 3.29L2.27 4.00ZM3.23 4.26L2.98 4.71L2.27 4.00ZM1.00 5.00L1.03 4.74L2.00 5.00ZM3.00 5.00L2.97 5.26L2.00 5.00ZM1.03 18.26L1.00 18.00L2.00 18.00ZM2.97 17.74L3.00 18.00L2.00 18.00ZM1.56 19.71L1.30 19.26L2.27 19.00ZM2.98 18.29L3.23 18.74L2.27 19.00ZM2.74 20.70L2.29 20.44L3.00 19.73ZM3.26 18.77L3.71 19.02L3.00 19.73ZM4.00 21.00L3.74 20.97L4.00 20.00ZM4.00 19.00L4.26 19.03L4.00 20.00Z", 24.0, false),
            Logo::FolderOpen => ("M5.11 13.54L6.61 10.64L8.39 11.56L6.89 14.46ZM6.70 10.50L7.13 9.93L8.73 11.13L8.30 11.70ZM7.40 9.68L8.00 9.30L9.07 10.99L8.47 11.37ZM8.34 9.17L9.04 9.02L9.44 10.98L8.74 11.12ZM9.24 9.00L20.00 9.00L20.00 11.00L9.24 11.00ZM20.22 9.03L21.11 9.23L20.66 11.18L19.78 10.97ZM21.51 9.43L22.21 10.00L20.95 11.55L20.25 10.98ZM22.49 10.36L22.87 11.18L21.05 12.01L20.68 11.19ZM22.96 11.62L22.94 12.52L20.94 12.48L20.96 11.57ZM22.91 12.75L21.37 18.75L19.43 18.25L20.97 12.25ZM21.29 18.96L20.89 19.74L19.11 18.82L19.51 18.04ZM20.61 20.07L19.92 20.61L18.70 19.02L19.39 18.49ZM19.52 20.79L18.66 20.98L18.24 19.02L19.09 18.84ZM18.45 21.00L4.00 21.00L4.00 19.00L18.45 19.00ZM3.74 20.97L2.74 20.70L3.26 18.77L4.26 19.03ZM2.29 20.44L1.56 19.71L2.98 18.29L3.71 19.02ZM1.30 19.26L1.03 18.26L2.97 17.74L3.23 18.74ZM1.00 18.00L1.00 5.00L3.00 5.00L3.00 18.00ZM1.03 4.74L1.30 3.74L3.23 4.26L2.97 5.26ZM1.56 3.29L2.29 2.56L3.71 3.98L2.98 4.71ZM2.74 2.30L3.74 2.03L4.26 3.97L3.26 4.23ZM4.00 2.00L7.90 2.00L7.90 4.00L4.00 4.00ZM8.14 2.03L9.10 2.26L8.62 4.21L7.66 3.97ZM9.53 2.50L10.26 3.16L8.92 4.64L8.19 3.97ZM10.42 3.34L11.23 4.54L9.57 5.66L8.76 4.46ZM11.08 4.36L11.80 5.02L10.45 6.50L9.72 5.84ZM11.37 4.79L12.31 5.03L11.83 6.97L10.88 6.73ZM12.07 5.00L18.00 5.00L18.00 7.00L12.07 7.00ZM18.26 5.03L19.26 5.30L18.74 7.23L17.74 6.97ZM19.71 5.56L20.44 6.29L19.02 7.71L18.29 6.98ZM20.70 6.74L20.97 7.74L19.03 8.26L18.77 7.26ZM21.00 8.00L21.00 10.00L19.00 10.00L19.00 8.00ZM7.50 11.10L8.39 11.56L8.30 11.70ZM7.50 11.10L6.61 10.64L6.70 10.50ZM7.93 10.53L8.73 11.13L8.47 11.37ZM7.93 10.53L7.13 9.93L7.40 9.68ZM8.54 10.14L9.07 10.99L8.74 11.12ZM8.54 10.14L8.00 9.30L8.34 9.17ZM9.24 10.00L9.44 10.98L9.24 11.00ZM9.24 10.00L9.04 9.02L9.24 9.00ZM20.00 10.00L20.00 11.00L19.78 10.97ZM20.00 10.00L20.00 9.00L20.22 9.03ZM20.88 10.20L20.66 11.18L20.25 10.98ZM20.88 10.20L21.11 9.23L21.51 9.43ZM21.58 10.77L20.95 11.55L20.68 11.19ZM21.58 10.77L22.21 10.00L22.49 10.36ZM21.96 11.60L21.05 12.01L20.96 11.57ZM21.96 11.60L22.87 11.18L22.96 11.62ZM21.94 12.50L20.94 12.48L20.97 12.25ZM21.94 12.50L22.94 12.52L22.91 12.75ZM20.40 18.50L19.43 18.25L19.51 18.04ZM20.40 18.50L21.37 18.75L21.29 18.96ZM20.00 19.28L19.11 18.82L19.39 18.49ZM20.00 19.28L20.89 19.74L20.61 20.07ZM19.31 19.81L18.70 19.02L19.09 18.84ZM19.31 19.81L19.92 20.61L19.52 20.79ZM18.45 20.00L18.24 19.02L18.45 19.00ZM18.45 20.00L18.66 20.98L18.45 21.00ZM4.00 20.00L4.00 19.00L4.26 19.03ZM4.00 20.00L4.00 21.00L3.74 20.97ZM3.00 19.73L3.26 18.77L3.71 19.02ZM3.00 19.73L2.74 20.70L2.29 20.44ZM2.27 19.00L2.98 18.29L3.23 18.74ZM2.27 19.00L1.56 19.71L1.30 19.26ZM2.00 18.00L2.97 17.74L3.00 18.00ZM2.00 18.00L1.03 18.26L1.00 18.00ZM2.00 5.00L3.00 5.00L2.97 5.26ZM2.00 5.00L1.00 5.00L1.03 4.74ZM2.27 4.00L3.23 4.26L2.98 4.71ZM2.27 4.00L1.30 3.74L1.56 3.29ZM3.00 3.27L3.71 3.98L3.26 4.23ZM3.00 3.27L2.29 2.56L2.74 2.30ZM4.00 3.00L4.26 3.97L4.00 4.00ZM4.00 3.00L3.74 2.03L4.00 2.00ZM7.90 3.00L7.90 4.00L7.66 3.97ZM7.90 3.00L7.90 2.00L8.14 2.03ZM8.86 3.23L8.62 4.21L8.19 3.97ZM8.86 3.23L9.10 2.26L9.53 2.50ZM9.59 3.90L8.92 4.64L8.76 4.46ZM9.59 3.90L10.26 3.16L10.42 3.34ZM9.72 5.84L9.57 5.66L10.40 5.10ZM11.08 4.36L11.23 4.54L10.40 5.10ZM10.88 6.73L10.45 6.50L11.12 5.76ZM11.37 4.79L11.80 5.02L11.12 5.76ZM12.07 7.00L11.83 6.97L12.07 6.00ZM12.07 5.00L12.31 5.03L12.07 6.00ZM18.00 6.00L18.00 7.00L17.74 6.97ZM18.00 6.00L18.00 5.00L18.26 5.03ZM19.00 6.27L18.74 7.23L18.29 6.98ZM19.00 6.27L19.26 5.30L19.71 5.56ZM19.73 7.00L19.02 7.71L18.77 7.26ZM19.73 7.00L20.44 6.29L20.70 6.74ZM20.00 8.00L19.03 8.26L19.00 8.00ZM20.00 8.00L20.97 7.74L21.00 8.00ZM7.00 14.00L6.71 14.71L6.00 15.00L5.29 14.71L5.00 14.00L5.29 13.29L6.00 13.00L6.71 13.29ZM21.00 10.00L20.71 10.71L20.00 11.00L19.29 10.71L19.00 10.00L19.29 9.29L20.00 9.00L20.71 9.29Z", 24.0, false),
            Logo::Note => ("M5.74 22.97L4.74 22.70L5.26 20.77L6.26 21.03ZM4.29 22.44L3.56 21.71L4.98 20.29L5.71 21.02ZM3.30 21.26L3.03 20.26L4.97 19.74L5.23 20.74ZM3.00 20.00L3.00 4.00L5.00 4.00L5.00 20.00ZM3.03 3.74L3.30 2.74L5.23 3.26L4.97 4.26ZM3.56 2.29L4.29 1.56L5.71 2.98L4.98 3.71ZM4.74 1.30L5.74 1.03L6.26 2.97L5.26 3.23ZM6.00 1.00L14.00 1.00L14.00 3.00L6.00 3.00ZM14.19 1.02L15.12 1.20L14.73 3.16L13.81 2.98ZM15.48 1.35L16.26 1.88L15.15 3.54L14.37 3.01ZM16.41 2.00L20.00 5.59L18.58 7.00L15.00 3.41ZM20.12 5.74L20.65 6.52L18.99 7.63L18.46 6.85ZM20.80 6.88L20.98 7.81L19.02 8.19L18.84 7.27ZM21.00 8.00L21.00 20.00L19.00 20.00L19.00 8.00ZM20.97 20.26L20.70 21.26L18.77 20.74L19.03 19.74ZM20.44 21.71L19.71 22.44L18.29 21.02L19.02 20.29ZM19.26 22.70L18.26 22.97L17.74 21.03L18.74 20.77ZM18.00 23.00L6.00 23.00L6.00 21.00L18.00 21.00ZM6.00 22.00L6.00 21.00L6.26 21.03ZM6.00 22.00L6.00 23.00L5.74 22.97ZM5.00 21.73L5.26 20.77L5.71 21.02ZM5.00 21.73L4.74 22.70L4.29 22.44ZM4.27 21.00L4.98 20.29L5.23 20.74ZM4.27 21.00L3.56 21.71L3.30 21.26ZM4.00 20.00L4.97 19.74L5.00 20.00ZM4.00 20.00L3.03 20.26L3.00 20.00ZM4.00 4.00L5.00 4.00L4.97 4.26ZM4.00 4.00L3.00 4.00L3.03 3.74ZM4.27 3.00L5.23 3.26L4.98 3.71ZM4.27 3.00L3.30 2.74L3.56 2.29ZM5.00 2.27L5.71 2.98L5.26 3.23ZM5.00 2.27L4.29 1.56L4.74 1.30ZM6.00 2.00L6.26 2.97L6.00 3.00ZM6.00 2.00L5.74 1.03L6.00 1.00ZM14.00 2.00L14.00 3.00L13.81 2.98ZM14.00 2.00L14.00 1.00L14.19 1.02ZM14.92 2.18L14.73 3.16L14.37 3.01ZM14.92 2.18L15.12 1.20L15.48 1.35ZM15.70 2.71L15.15 3.54L15.00 3.41ZM15.70 2.71L16.26 1.88L16.41 2.00ZM19.29 6.29L18.58 7.00L18.46 6.85ZM19.29 6.29L20.00 5.59L20.12 5.74ZM19.82 7.08L18.99 7.63L18.84 7.27ZM19.82 7.08L20.65 6.52L20.80 6.88ZM20.00 8.00L19.02 8.19L19.00 8.00ZM20.00 8.00L20.98 7.81L21.00 8.00ZM20.00 20.00L19.00 20.00L19.03 19.74ZM20.00 20.00L21.00 20.00L20.97 20.26ZM19.73 21.00L18.77 20.74L19.02 20.29ZM19.73 21.00L20.70 21.26L20.44 21.71ZM19.00 21.73L18.29 21.02L18.74 20.77ZM19.00 21.73L19.71 22.44L19.26 22.70ZM18.00 22.00L17.74 21.03L18.00 21.00ZM18.00 22.00L18.26 22.97L18.00 23.00ZM15.00 2.00L15.00 7.00L13.00 7.00L13.00 2.00ZM14.97 6.74L15.10 7.24L13.17 7.76L13.03 7.26ZM14.84 6.79L15.21 7.16L13.79 8.57L13.43 8.21ZM14.76 6.90L15.26 7.03L14.74 8.97L14.24 8.83ZM15.00 7.00L20.00 7.00L20.00 9.00L15.00 9.00ZM13.03 7.26L13.00 7.00L14.00 7.00ZM14.97 6.74L15.00 7.00L14.00 7.00ZM13.43 8.21L13.17 7.76L14.13 7.50ZM14.84 6.79L15.10 7.24L14.13 7.50ZM14.24 8.83L13.79 8.57L14.50 7.87ZM14.76 6.90L15.21 7.16L14.50 7.87ZM15.00 9.00L14.74 8.97L15.00 8.00ZM15.00 7.00L15.26 7.03L15.00 8.00ZM15.00 2.00L14.71 2.71L14.00 3.00L13.29 2.71L13.00 2.00L13.29 1.29L14.00 1.00L14.71 1.29ZM21.00 8.00L20.71 8.71L20.00 9.00L19.29 8.71L19.00 8.00L19.29 7.29L20.00 7.00L20.71 7.29ZM10.00 10.00L8.00 10.00L8.00 8.00L10.00 8.00ZM11.00 9.00L10.71 9.71L10.00 10.00L9.29 9.71L9.00 9.00L9.29 8.29L10.00 8.00L10.71 8.29ZM9.00 9.00L8.71 9.71L8.00 10.00L7.29 9.71L7.00 9.00L7.29 8.29L8.00 8.00L8.71 8.29ZM16.00 14.00L8.00 14.00L8.00 12.00L16.00 12.00ZM17.00 13.00L16.71 13.71L16.00 14.00L15.29 13.71L15.00 13.00L15.29 12.29L16.00 12.00L16.71 12.29ZM9.00 13.00L8.71 13.71L8.00 14.00L7.29 13.71L7.00 13.00L7.29 12.29L8.00 12.00L8.71 12.29ZM16.00 18.00L8.00 18.00L8.00 16.00L16.00 16.00ZM17.00 17.00L16.71 17.71L16.00 18.00L15.29 17.71L15.00 17.00L15.29 16.29L16.00 16.00L16.71 16.29ZM9.00 17.00L8.71 17.71L8.00 18.00L7.29 17.71L7.00 17.00L7.29 16.29L8.00 16.00L8.71 16.29Z", 24.0, false),
            Logo::Beam => ("M6.5 14.5a3 3 0 1 0 0 6a3 3 0 1 0 0-6ZM6.5 10.1a7.4 7.4 0 0 1 7.4 7.4h-2.5A4.9 4.9 0 0 0 6.5 12.6ZM6.5 3.5a14 14 0 0 1 14 14h-2.5A11.5 11.5 0 0 0 6.5 6Z", 24.0, false),
            Logo::Check => ("M9.6 18.2 3.9 12.5l2.1-2.1 3.6 3.6 8.4-8.4 2.1 2.1Z", 24.0, false),
            Logo::Laptop => ("M5.5 5h13A1.5 1.5 0 0 1 20 6.5v8a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 14.5v-8A1.5 1.5 0 0 1 5.5 5ZM6.5 7.5v6h11v-6ZM2.5 17h19a1 1 0 0 1 0 2h-19a1 1 0 0 1 0-2Z", 24.0, true),
            Logo::Phone => ("M9.5 2h5A2.5 2.5 0 0 1 17 4.5v15a2.5 2.5 0 0 1-2.5 2.5h-5A2.5 2.5 0 0 1 7 19.5v-15A2.5 2.5 0 0 1 9.5 2ZM9.25 4.75v11.5h5.5v-11.5ZM10.75 19a.75.75 0 0 0 0 1.5h2.5a.75.75 0 0 0 0-1.5Z", 24.0, true),
            Logo::NewNote => ("M7.007 12a.75.75 0 0 1 .75-.75h3.493V7.757a.75.75 0 0 1 1.5 0v3.493h3.493a.75.75 0 1 1 0 1.5H12.75v3.493a.75.75 0 0 1-1.5 0V12.75H7.757a.75.75 0 0 1-.75-.75M7.317 3.769a42.5 42.5 0 0 1 9.366 0c1.827.204 3.302 1.643 3.516 3.48c.37 3.157.37 6.346 0 9.503c-.215 1.837-1.69 3.275-3.516 3.48a42.5 42.5 0 0 1-9.366 0c-1.827-.205-3.302-1.643-3.516-3.48a41 41 0 0 1 0-9.503c.214-1.837 1.69-3.276 3.516-3.48m9.2 1.49a41 41 0 0 0-9.034 0A2.486 2.486 0 0 0 5.29 7.424a39.4 39.4 0 0 0 0 9.154a2.486 2.486 0 0 0 2.193 2.164c2.977.332 6.057.332 9.034 0a2.486 2.486 0 0 0 2.192-2.164a39.4 39.4 0 0 0 0-9.154a2.486 2.486 0 0 0-2.192-2.163", 24.0, true),
            Logo::Recent => ("M20.59 22L15 16.41V7h2v8.58l5 5.01zM16 2A13.94 13.94 0 0 0 6 6.23V2H4v8h8V8H7.08A12 12 0 1 1 4 16H2A14 14 0 1 0 16 2", 32.0, false),
            Logo::Bird => ("M484.3 31.0C514.7 28.4 545.9 30.2 576.4 30.2C633.7 30.3 691.1 30.3 748.4 30.2C779.9 30.2 813.6 26.9 844.6 33.7C871.0 39.5 895.3 53.1 914.2 72.0C953.5 111.3 953.2 155.4 953.1 206.6C953.1 233.8 953.1 261.0 953.2 288.1C953.2 318.8 958.0 353.4 941.1 380.9C926.5 404.7 901.5 416.2 880.9 433.9C863.0 449.2 850.7 470.2 844.6 492.4C838.2 515.5 840.9 540.5 840.8 564.2C840.7 608.2 840.9 652.1 840.8 696.1C840.7 744.8 841.7 787.8 816.0 831.8C787.3 881.2 736.6 918.5 678.4 927.5C657.1 930.9 635.7 930.4 614.2 930.4C593.2 930.3 572.2 930.4 551.2 930.4C458.4 930.4 365.6 930.4 272.9 930.4C236.0 930.4 199.2 930.5 162.4 930.3C131.8 930.2 102.5 931.0 74.0 918.7C61.4 913.3 49.3 905.8 39.4 896.5C31.7 889.3 24.9 881.4 19.4 872.5C2.6 845.0 0.0 811.8 8.6 781.3C13.0 765.7 20.6 751.4 27.2 736.7C39.4 709.7 52.4 682.9 64.3 655.7C123.3 521.3 188.1 389.2 247.8 255.0C257.0 234.2 266.5 213.4 276.2 192.8C284.7 174.5 292.6 156.4 303.8 139.4C343.2 79.2 410.9 37.4 484.3 31.0Z", 957.99, false),
            Logo::BirdEye => ("M652.1 215.9C643.5 217.2 635.4 220.4 627.8 224.6C573.3 255.3 585.4 334.5 645.0 351.2C653.4 353.6 662.5 354.6 671.2 353.5C732.9 345.9 757.7 268.4 708.1 230.2C700.7 224.5 692.1 220.2 683.1 217.7C673.2 214.9 662.3 214.2 652.1 215.9Z", 957.99, false),
            Logo::YouTube => (
                "M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814zM9.545 15.568V8.432L15.818 12l-6.273 3.568z",
                24.0,
                true,
            ),
        }
    }
}

type Point = (f32, f32);

/// Reads SVG path data into closed polygons, curves and arcs flattened
/// finely enough for `detail` segments a quarter turn. Pieces of fewer than
/// three points are not shapes and are left out.
pub fn polygons(d: &str, detail: usize) -> Vec<Vec<Point>> {
    subpaths(d, detail).into_iter().map(|(points, _)| points).filter(|p| p.len() > 2).collect()
}

/// Reads SVG path data into its subpaths, curves and arcs flattened finely
/// enough for `detail` segments a quarter turn: each with its points, and
/// whether it was closed (`z`) or left open (a stroke ends there).
pub fn subpaths(d: &str, detail: usize) -> Vec<(Vec<Point>, bool)> {
    let mut p = Reader { s: d.as_bytes(), at: 0 };
    let mut out: Vec<(Vec<Point>, bool)> = Vec::new();
    let mut shape: Vec<Point> = Vec::new();
    let (mut cur, mut start) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    // The last curve's second control point, for S and T.
    let mut last_control: Option<Point> = None;
    let mut cmd = b'M';
    let close = |shape: &mut Vec<Point>, out: &mut Vec<(Vec<Point>, bool)>, closed: bool| {
        if shape.len() > 1 {
            out.push((std::mem::take(shape), closed));
        } else {
            shape.clear();
        }
    };
    while let Some(next) = p.command_or_number() {
        if let Some(c) = next {
            cmd = c;
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { cur } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let Some(pt) = p.point() else { break };
                close(&mut shape, &mut out, false);
                cur = (base.0 + pt.0, base.1 + pt.1);
                start = cur;
                shape.push(cur);
                // Pairs after a move are lines.
                cmd = if rel { b'l' } else { b'L' };
                last_control = None;
            }
            b'L' => {
                let Some(pt) = p.point() else { break };
                cur = (base.0 + pt.0, base.1 + pt.1);
                shape.push(cur);
                last_control = None;
            }
            b'H' => {
                let Some(x) = p.number() else { break };
                cur = (if rel { cur.0 + x } else { x }, cur.1);
                shape.push(cur);
                last_control = None;
            }
            b'V' => {
                let Some(y) = p.number() else { break };
                cur = (cur.0, if rel { cur.1 + y } else { y });
                shape.push(cur);
                last_control = None;
            }
            b'C' | b'S' => {
                let smooth = cmd.eq_ignore_ascii_case(&b's');
                let c1 = if smooth {
                    last_control.map_or(cur, |c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1))
                } else {
                    let Some(a) = p.point() else { break };
                    (base.0 + a.0, base.1 + a.1)
                };
                let (Some(b), Some(e)) = (p.point(), p.point()) else { break };
                let (c2, end) = ((base.0 + b.0, base.1 + b.1), (base.0 + e.0, base.1 + e.1));
                cubic(&mut shape, cur, c1, c2, end, detail);
                last_control = Some(c2);
                cur = end;
            }
            b'Q' | b'T' => {
                let smooth = cmd.eq_ignore_ascii_case(&b't');
                let c = if smooth {
                    last_control.map_or(cur, |c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1))
                } else {
                    let Some(a) = p.point() else { break };
                    (base.0 + a.0, base.1 + a.1)
                };
                let Some(e) = p.point() else { break };
                let end = (base.0 + e.0, base.1 + e.1);
                // As a cubic: control points two thirds toward the quadratic's.
                let c1 = (cur.0 + 2.0 / 3.0 * (c.0 - cur.0), cur.1 + 2.0 / 3.0 * (c.1 - cur.1));
                let c2 = (end.0 + 2.0 / 3.0 * (c.0 - end.0), end.1 + 2.0 / 3.0 * (c.1 - end.1));
                cubic(&mut shape, cur, c1, c2, end, detail);
                last_control = Some(c);
                cur = end;
            }
            b'A' => {
                let (Some(rx), Some(ry), Some(rot)) = (p.number(), p.number(), p.number()) else { break };
                let (Some(large), Some(sweep)) = (p.flag(), p.flag()) else { break };
                let Some(e) = p.point() else { break };
                let end = (base.0 + e.0, base.1 + e.1);
                arc(&mut shape, cur, (rx, ry), rot, large, sweep, end, detail);
                cur = end;
                last_control = None;
            }
            b'Z' => {
                close(&mut shape, &mut out, true);
                cur = start;
                shape.push(cur);
                last_control = None;
                // A number after Z would be malformed: stop there.
                cmd = b'M';
                if p.peek_number() {
                    continue;
                }
                continue;
            }
            _ => break,
        }
    }
    close(&mut shape, &mut out, false);
    out
}

struct Reader<'a> {
    s: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn skip(&mut self) {
        while self.at < self.s.len() && matches!(self.s[self.at], b' ' | b',' | b'\n' | b'\r' | b'\t') {
            self.at += 1;
        }
    }

    fn peek_number(&mut self) -> bool {
        self.skip();
        self.s.get(self.at).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }

    /// The next command letter (Some), or None when a number follows (the
    /// last command repeats); nothing at the end.
    fn command_or_number(&mut self) -> Option<Option<u8>> {
        self.skip();
        let c = *self.s.get(self.at)?;
        if c.is_ascii_alphabetic() && !matches!(c, b'e' | b'E') {
            self.at += 1;
            return Some(Some(c));
        }
        Some(None)
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let begin = self.at;
        let s = self.s;
        let mut i = self.at;
        if i < s.len() && matches!(s[i], b'-' | b'+') {
            i += 1;
        }
        let mut dot = false;
        while i < s.len() && (s[i].is_ascii_digit() || (s[i] == b'.' && !dot)) {
            dot |= s[i] == b'.';
            i += 1;
        }
        if i < s.len() && matches!(s[i], b'e' | b'E') {
            let mut j = i + 1;
            if j < s.len() && matches!(s[j], b'-' | b'+') {
                j += 1;
            }
            if j < s.len() && s[j].is_ascii_digit() {
                i = j;
                while i < s.len() && s[i].is_ascii_digit() {
                    i += 1;
                }
            }
        }
        if i == begin {
            return None;
        }
        self.at = i;
        std::str::from_utf8(&s[begin..i]).ok()?.parse().ok()
    }

    fn point(&mut self) -> Option<Point> {
        Some((self.number()?, self.number()?))
    }

    /// An arc's flag: one digit, which may touch what follows.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.s.get(self.at)?;
        self.at += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
}

fn cubic(out: &mut Vec<Point>, p0: Point, c1: Point, c2: Point, p3: Point, detail: usize) {
    let n = detail.max(4) * 2;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push((a * p0.0 + b * c1.0 + c * c2.0 + d * p3.0, a * p0.1 + b * c1.1 + c * c2.1 + d * p3.1));
    }
}

/// An elliptical arc from `p0` to `p1`, as SVG gives it (its endpoints,
/// radii, rotation and flags), through its center.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<Point>, p0: Point, (rx, ry): Point, rotation: f32, large: bool, sweep: bool, p1: Point, detail: usize) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || p0 == p1 {
        out.push(p1);
        return;
    }
    let phi = rotation.to_radians();
    let (sin, cos) = libm::sincosf(phi);
    let (dx, dy) = ((p0.0 - p1.0) / 2.0, (p0.1 - p1.1) / 2.0);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    // Radii too small to reach are scaled up until they do.
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let k = lambda.sqrt();
        rx *= k;
        ry *= k;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = (num / den).max(0.0).sqrt();
    if large == sweep {
        coef = -coef;
    }
    let (cx1, cy1) = (coef * rx * y1 / ry, -coef * ry * x1 / rx);
    let (cx, cy) = (cos * cx1 - sin * cy1 + (p0.0 + p1.0) / 2.0, sin * cx1 + cos * cy1 + (p0.1 + p1.1) / 2.0);
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let a = libm::atan2f(ux * vy - uy * vx, ux * vx + uy * vy);
        if a.is_nan() { 0.0 } else { a }
    };
    let (ux, uy) = ((x1 - cx1) / rx, (y1 - cy1) / ry);
    let (vx, vy) = ((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let start = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let n = ((delta.abs() / std::f32::consts::FRAC_PI_2) * detail as f32).ceil().max(2.0) as usize;
    for i in 1..=n {
        let t = start + delta * i as f32 / n as f32;
        let (s, c) = libm::sincosf(t);
        out.push((cx + rx * c * cos - ry * s * sin, cy + rx * c * sin + ry * s * cos));
    }
    // Land exactly where the path says.
    if let Some(last) = out.last_mut() {
        *last = p1;
    }
}

/// Fills `polygons` (in a `view`-unit square) into a `size` x `size`
/// coverage mask, 0 to 255 a pixel.
pub fn rasterize(polygons: &[Vec<Point>], view: f32, evenodd: bool, size: usize) -> Vec<u8> {
    const SUB: usize = 16;
    let k = size as f32 / view;
    let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
    for poly in polygons {
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (a, b) = ((a.0 * k, a.1 * k), (b.0 * k, b.1 * k));
            if a.1 == b.1 {
                continue;
            }
            // Top to bottom, with which way it went.
            if a.1 < b.1 { edges.push((a.0, a.1, b.0, b.1, 1)) } else { edges.push((b.0, b.1, a.0, a.1, -1)) }
        }
    }
    let mut cover = vec![0.0f32; size * size];
    let mut hits: Vec<(f32, i32)> = Vec::new();
    for row in 0..size {
        let line = &mut cover[row * size..(row + 1) * size];
        for s in 0..SUB {
            let y = row as f32 + (s as f32 + 0.5) / SUB as f32;
            hits.clear();
            for &(x0, y0, x1, y1, dir) in &edges {
                if y >= y0 && y < y1 {
                    hits.push((x0 + (y - y0) / (y1 - y0) * (x1 - x0), dir));
                }
            }
            hits.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for w in hits.windows(2) {
                winding += w[0].1;
                let inside = if evenodd { winding % 2 != 0 } else { winding != 0 };
                if !inside {
                    continue;
                }
                // Exact coverage across the span, a pixel at a time.
                let (a, b) = (w[0].0.clamp(0.0, size as f32), w[1].0.clamp(0.0, size as f32));
                let mut x = a;
                while x < b {
                    let px = x.floor();
                    let end = (px + 1.0).min(b);
                    if let Some(c) = line.get_mut(px as usize) {
                        *c += (end - x) / SUB as f32;
                    }
                    x = end;
                }
            }
        }
    }
    cover.iter().map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

type Key = (Logo, usize, u32);

static MARKS: Mutex<Option<HashMap<Key, Arc<Pixels>>>> = Mutex::new(None);

/// `logo` `size` pixels square in the 0xRRGGBB `color`, premultiplied, as
/// a picture draws it.
pub fn mark(logo: Logo, size: usize, color: u32) -> Arc<Pixels> {
    let size = size.clamp(1, 512);
    let mut marks = MARKS.lock().unwrap_or_else(|e| e.into_inner());
    let marks = marks.get_or_insert_with(HashMap::new);
    if let Some(p) = marks.get(&(logo, size, color)) {
        return p.clone();
    }
    let (d, view, evenodd) = logo.outline();
    let mask = rasterize(&polygons(d, 8), view, evenodd, size);
    let (r, g, b) = ((color >> 16) & 0xFF, (color >> 8) & 0xFF, color & 0xFF);
    let data = mask.iter().map(|&a| {
        let a = a as u32;
        (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255)
    });
    let pixels = Arc::new(Pixels { width: size as u32, height: size as u32, data: data.collect() });
    // A theme switch or a new size makes new ones: a few dozen at most.
    if marks.len() > 256 {
        marks.clear();
    }
    marks.insert((logo, size, color), pixels.clone());
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subpaths_keep_open_pieces_and_say_which_are_closed() {
        let s = subpaths("M0 0H4M1 1h.01M2 2h4v4zM9 9l1 0", 4);
        assert_eq!(s.len(), 4);
        assert_eq!((s[0].0.len(), s[0].1), (2, false), "a line is an open piece");
        assert_eq!((s[1].0.len(), s[1].1), (2, false), "so is a dot");
        assert!(s[2].1 && s[2].0.len() >= 3, "a triangle is closed");
        assert_eq!(polygons("M0 0H4M1 1h.01M2 2h4v4zM9 9l1 0", 4).len(), 1, "only the triangle is a polygon");
    }

    fn covered(mask: &[u8], size: usize, x: usize, y: usize) -> u8 {
        mask[y * size + x]
    }

    #[test]
    fn the_new_note_mark_is_a_plus_inside_a_hollow_ring() {
        let (d, view, evenodd) = Logo::NewNote.outline();
        let size = 48;
        let mask = rasterize(&polygons(d, 8), view, evenodd, size);
        // 24 units to 48 pixels: a unit is two pixels.
        assert!(covered(&mask, size, 24, 24) > 200, "the plus's middle is filled");
        assert!(covered(&mask, size, 24, 17) > 200 && covered(&mask, size, 31, 24) > 200, "and its arms");
        assert!(covered(&mask, size, 14, 14) < 30, "between the plus and the ring is open");
        assert!(covered(&mask, size, 24, 8) > 200, "the ring's top is filled");
        assert!(covered(&mask, size, 2, 2) < 30, "outside the ring is open");
    }

    #[test]
    fn reads_every_kind_of_path_data() {
        // Relative moves and lines, H and V, a close.
        let square = polygons("m2 2h4v4H2z", 4);
        assert_eq!(square.len(), 1);
        assert_eq!(&square[0][..4], &[(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)]);
        // Numbers that run together: "-.5.25" is -0.5 and 0.25.
        let p = polygons("M0 0L-.5.25L1e1 0z", 4);
        assert_eq!(p[0][1], (-0.5, 0.25));
        assert_eq!(p[0][2], (10.0, 0.0));
        // Arc flags written together, as some exporters do.
        let arcs = polygons("M0 5a5 5 0 1010 0a5 5 0 1 0-10 0z", 8);
        let far = arcs[0].iter().map(|p| p.1).fold(f32::MIN, f32::max);
        assert!((far - 10.0).abs() < 0.2, "a full circle reaches y = 10: {far}");
    }

    #[test]
    fn fills_with_holes_and_soft_edges() {
        // A square with a square hole, wound the other way (nonzero).
        let mask = rasterize(&polygons("M0 0H8V8H0zM2 2V6H6V2z", 4), 8.0, false, 8);
        assert_eq!(covered(&mask, 8, 1, 1), 255, "the ring is solid");
        assert_eq!(covered(&mask, 8, 4, 4), 0, "the hole is open");
        // The same hole wound the same way, even-odd.
        let mask = rasterize(&polygons("M0 0H8V8H0zM2 2H6V6H2z", 4), 8.0, true, 8);
        assert_eq!(covered(&mask, 8, 4, 4), 0);
        // An edge through the middle of a pixel covers half of it.
        let mask = rasterize(&polygons("M0 0H2.5V4H0z", 4), 4.0, false, 4);
        assert!((covered(&mask, 4, 2, 1) as i32 - 128).abs() <= 2);
    }

    #[test]
    fn draws_the_logos() {
        for logo in [Logo::LinkedIn, Logo::Instagram, Logo::Facebook, Logo::X, Logo::YouTube] {
            let (d, view, evenodd) = logo.outline();
            let mask = rasterize(&polygons(d, 8), view, evenodd, 48);
            let ink = mask.iter().filter(|&&a| a > 128).count();
            // A filled mark with cut-outs: most of it inked, not all.
            assert!(ink > 48 * 48 / 3 && ink < 48 * 48 * 9 / 10, "{logo:?}: {ink} of {} pixels", 48 * 48);
            // The corners are rounded or outside: never solid.
            assert!(covered(&mask, 48, 0, 0) < 128, "{logo:?}");
        }
        // Their cut-outs: LinkedIn's "i" dot, Instagram's lens, X's cross.
        let at = |logo: Logo, x: f32, y: f32| {
            let (d, view, evenodd) = logo.outline();
            let mask = rasterize(&polygons(d, 8), view, evenodd, 96);
            covered(&mask, 96, (x / view * 96.0) as usize, (y / view * 96.0) as usize)
        };
        assert_eq!(at(Logo::LinkedIn, 6.88, 6.9), 0, "the dot is open");
        assert_eq!(at(Logo::LinkedIn, 12.0, 4.0), 255, "the square around it is inked");
        assert_eq!(at(Logo::Instagram, 12.0, 12.0), 255, "the lens's middle is inked");
        assert_eq!(at(Logo::Instagram, 12.0, 8.0), 0, "the ring around it is open");
        assert_eq!(at(Logo::X, 150.0, 512.0), 255, "the square's edge is inked");
        assert_eq!(at(Logo::X, 300.0, 300.0), 0, "the cross is cut out");
        assert_eq!(at(Logo::YouTube, 11.5, 12.0), 0, "the play button is cut out");
        assert_eq!(at(Logo::YouTube, 4.0, 12.0), 255, "the screen around it is inked");
    }
}
