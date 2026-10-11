//! Colors, as 0xRRGGBB. Notsy is graphite or paper with text in three
//! strengths; the only color is the logo's red (#DC4848): the caret and
//! the app's markers, links, and a faint tint behind ==marked== text.
//! Selected text is grey, not red, which would read as an error.

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

/// Graphite, with the logo's red.
pub const DARK: Theme = Theme {
    background: 0x111113,
    panel: 0x19191C,
    border: 0x26262B,
    text: 0xECECEF,
    muted: 0x9A9AA3,
    faint: 0x5B5B64,
    accent: 0xDC4848,
    hover: 0x232327,
    selection: 0x26262C,
    highlight: 0x33333A,
    link: 0xDC4848,
    mark: 0x522324,
    raised: 0x303036,
    field: 0x1E1E22,
    danger: 0xC42B1C,
    on_danger: 0xFFFFFF,
    shadow: 0.5,
};

/// Paper and ink, with the logo's red; links a shade deeper, to read as
/// text on paper (4.8:1 against 3.9:1).
pub const LIGHT: Theme = Theme {
    background: 0xF7F7F8,
    panel: 0xFFFFFF,
    border: 0xE5E5E9,
    text: 0x17171A,
    muted: 0x5A5A63,
    faint: 0xA3A3AB,
    accent: 0xDC4848,
    hover: 0xF0F0F2,
    selection: 0xEEEEF1,
    highlight: 0xDEDEE4,
    link: 0xC53B3B,
    mark: 0xF1D0D1,
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
        }
    }
}

pub fn for_mode(dark: bool) -> &'static Theme {
    if dark { &DARK } else { &LIGHT }
}
