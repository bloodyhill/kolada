//! Colors, as 0xRRGGBB. Kolada is graphite or paper with text in three
//! strengths; the only color is the logo's teal (#0E9594): the caret and
//! the app's markers, links (a light teal on graphite, a deeper one on
//! paper), and a tint behind ==marked== text. Selected text is grey.
//! Graphite is a neutral grey, lifted off black; each of its inks keeps
//! the contrast it had on the old near-black (text 16:1, muted 6.8:1,
//! faint 2.8:1), so nothing reads brighter or dimmer than it should; paper's
//! the same against the page.
//!
//! Surfaces, lowest to highest, each a step apart that reads at a glance
//! (as the systems' own light and dark surfaces are), lifted by fill and a
//! soft shadow, never a rim: the well (under a list's cards), the page, the
//! panel (a sheet, a popup's bars, a menu), raised (cards, chips, a chosen
//! segment); fields and soft buttons are inset into what they sit on.
//! Graphite climbs lighter as it rises; paper climbs whiter, its insets
//! greyer.

use crate::geom::Ink;

pub struct Theme {
    pub background: u32,
    panel: u32,
    border: u32,
    text: u32,
    muted: u32,
    faint: u32,
    accent: u32,
    hover: u32,
    selection: u32,
    /// Selected note text.
    highlight: u32,
    /// Links and tags in notes.
    link: u32,
    /// Behind `==marked==` text.
    mark: u32,
    /// Cards, chips and a chosen segment.
    raised: u32,
    /// Fields, tracks and soft buttons: inset.
    field: u32,
    well: u32,
    pressed: u32,
    /// A button: its words in the full text ink on it; the one asked for
    /// is the text ink itself, its words the page's.
    button: u32,
    danger: u32,
    on_danger: u32,
    /// Opacity of drop shadows: dark themes need far more to read at all.
    pub shadow: f32,
}

/// Graphite, with the logo's teal (4.8:1); links in a light teal.
pub const DARK: Theme = Theme {
    background: 0x1A1A1A,
    panel: 0x242424,
    border: 0x2D2D2D,
    text: 0xF6F6F6,
    muted: 0xA8A8A8,
    faint: 0x666666,
    accent: 0x0E9594,
    hover: 0x363636,
    selection: 0x2D2D2D,
    highlight: 0x393939,
    link: 0xAED9E0,
    mark: 0x173E3D,
    raised: 0x2E2E2E,
    field: 0x303030,
    well: 0x1A1A1A,
    pressed: 0x474747,
    button: 0x3A3A3A,
    danger: 0xC42B1C,
    on_danger: 0xFFFFFF,
    shadow: 0.5,
};

/// Paper and ink, with the logo's teal; links a shade deeper, to read as
/// text on paper (4.8:1 against 3.4:1), and ==marks== on a pale cyan.
pub const LIGHT: Theme = Theme {
    background: 0xF7F7F8,
    panel: 0xFFFFFF,
    border: 0xE5E5E9,
    text: 0x141417,
    muted: 0x55555E,
    faint: 0x94949D,
    accent: 0x0E9594,
    hover: 0xE2E2E7,
    selection: 0xEEEEF1,
    highlight: 0xDEDEE4,
    link: 0x0B7A79,
    mark: 0xC4FFF9,
    raised: 0xFFFFFF,
    field: 0xEAEAEE,
    well: 0xEEEEF1,
    pressed: 0xD6D6DC,
    button: 0xE3E3E8,
    danger: 0xC42B1C,
    on_danger: 0xFFFFFF,
    shadow: 0.09,
};

impl Theme {
    pub fn color(&self, ink: Ink) -> u32 {
        match ink {
            Ink::Text => self.text,
            Ink::Muted => self.muted,
            Ink::Faint => self.faint,
            Ink::Accent => self.accent,
            Ink::Panel => self.panel,
            Ink::Border => self.border,
            Ink::Hover => self.hover,
            Ink::Selection => self.selection,
            Ink::Highlight => self.highlight,
            Ink::Link => self.link,
            Ink::Mark => self.mark,
            Ink::Raised => self.raised,
            Ink::Page => self.background,
            Ink::Field => self.field,
            Ink::Well => self.well,
            Ink::Pressed => self.pressed,
            Ink::Button => self.button,
            // The side menu reads as its own surface: on paper the well's
            // grey (its fields, rows and cards raised white on it); on
            // graphite the panel's, as the well there is the page.
            Ink::Side => if self.well == self.background { self.panel } else { self.well },
            Ink::Shadow | Ink::Scrim => 0x000000,
            Ink::Danger => self.danger,
            Ink::OnDanger => self.on_danger,
            Ink::OnAccent => 0xFFFFFF,
            // Paper's, on graphite too: a QR reads dark on light.
            Ink::QrGround => LIGHT.panel,
            Ink::QrModule => LIGHT.text,
        }
    }
}

pub fn for_mode(dark: bool) -> &'static Theme {
    if dark { &DARK } else { &LIGHT }
}
