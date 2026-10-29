//! Colors, as 0xRRGGBB. Kolada is graphite or paper with text in three
//! strengths; the only color is the logo's teal (#0E9594): the caret and
//! the app's markers, links (a light teal on graphite, a deeper one on
//! paper), and a tint behind ==marked== text. Selected text is grey.
//! Graphite is a neutral grey, lifted off black; each of its inks keeps
//! the contrast it had on the old near-black (text 16:1, muted 6.8:1,
//! faint 2.8:1), so nothing reads brighter or dimmer than it should.

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
    /// A chosen segment of a segmented control.
    raised: u32,
    field: u32,
    danger: u32,
    on_danger: u32,
    /// Opacity of drop shadows: dark themes need far more to read at all.
    pub shadow: f32,
}

/// Graphite, with the logo's teal (4.8:1); links in a light teal.
pub const DARK: Theme = Theme {
    background: 0x1A1A1A,
    panel: 0x212121,
    border: 0x2D2D2D,
    text: 0xF6F6F6,
    muted: 0xA2A2A2,
    faint: 0x616161,
    accent: 0x0E9594,
    hover: 0x2A2A2A,
    selection: 0x2D2D2D,
    highlight: 0x393939,
    link: 0xAED9E0,
    mark: 0x173E3D,
    raised: 0x363636,
    field: 0x252525,
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
    text: 0x17171A,
    muted: 0x5A5A63,
    faint: 0xA3A3AB,
    accent: 0x0E9594,
    hover: 0xF0F0F2,
    selection: 0xEEEEF1,
    highlight: 0xDEDEE4,
    link: 0x0B7A79,
    mark: 0xC4FFF9,
    raised: 0xFFFFFF,
    field: 0xF2F2F4,
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
            Ink::Shadow => 0x000000,
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
