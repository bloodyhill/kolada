//! The screen's plain parts: theme roles, rectangles, and the filled shapes
//! every frame is drawn from. No OS types, so every shell shares them.

/// Theme roles; `theme::Theme` maps them to colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    Text,
    Muted,
    Faint,
    Accent,
    Panel,
    Border,
    Hover,
    Selection,
    Highlight,
    Link,
    Mark,
    Raised,
    /// The page itself (the window background).
    Page,
    Field,
    Shadow,
    Danger,
    OnDanger,
    /// Text on the accent: white.
    OnAccent,
}

const INKS: [Ink; 18] = [
    Ink::Text,
    Ink::Muted,
    Ink::Faint,
    Ink::Accent,
    Ink::Panel,
    Ink::Border,
    Ink::Hover,
    Ink::Selection,
    Ink::Highlight,
    Ink::Link,
    Ink::Mark,
    Ink::Raised,
    Ink::Page,
    Ink::Field,
    Ink::Shadow,
    Ink::Danger,
    Ink::OnDanger,
    Ink::OnAccent,
];

impl Ink {
    /// A number for this ink, never 0: what a range of text is tagged with
    /// to be drawn in it.
    pub fn id(self) -> u32 {
        self as u32 + 1
    }

    /// The ink a tag stands for (0 and unknown tags: none, the text's own).
    pub fn from_id(id: u32) -> Option<Ink> {
        INKS.get(id.wrapping_sub(1) as usize).copied()
    }

    /// Every ink.
    pub fn all() -> &'static [Ink] {
        &INKS
    }
}

/// A rectangle in DIPs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect { left, top, right, bottom }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.left + d, self.top + d, self.right - d, self.bottom - d)
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.left + dx, self.top + dy, self.right + dx, self.bottom + dy)
    }
}

/// A circle melted into a shape with a smooth union, so the two read as one
/// liquid form (the magnetic bulge of a collapsed pill).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    /// Width of the melted neck between the circle and the shape, DIPs.
    pub smooth: f32,
}

/// A rounded rectangle, optionally melted together with a `blob`. With
/// `blur` > 0 its edge fades out over that many DIPs (drop shadows). A
/// `ring` shape skips its fully covered interior, for shadows that
/// something opaque is drawn over anyway.
#[derive(Clone, Copy, Debug)]
pub struct Fill {
    pub rect: Rect,
    pub radius: f32,
    pub blur: f32,
    pub ink: Ink,
    pub alpha: f32,
    pub ring: bool,
    pub blob: Option<Blob>,
    /// A string's wave (see `wave`): the rectangle is the string at rest.
    pub wave: Option<WaveShape>,
}

/// An elastic string drawn as a pill whose middle line is displaced by `d`
/// (DIPs, toward the note), at points spread evenly along it.
#[derive(Clone, Copy, Debug)]
pub struct WaveShape {
    pub vertical: bool,
    pub d: [f32; crate::wave::SAMPLES],
}

impl Fill {
    pub fn solid(rect: Rect, radius: f32, ink: Ink) -> Fill {
        Fill { rect, radius, blur: 0.0, ink, alpha: 1.0, ring: false, blob: None, wave: None }
    }

    pub fn faded(rect: Rect, radius: f32, ink: Ink, alpha: f32) -> Fill {
        Fill { alpha, ..Fill::solid(rect, radius, ink) }
    }

    pub fn shadow(rect: Rect, radius: f32, blur: f32, alpha: f32) -> Fill {
        Fill { rect, radius, blur, ink: Ink::Shadow, alpha, ring: true, blob: None, wave: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ink_has_its_own_nonzero_tag_that_leads_back_to_it() {
        let mut seen = std::collections::HashSet::new();
        for &ink in Ink::all() {
            assert_ne!(ink.id(), 0);
            assert!(seen.insert(ink.id()), "{ink:?}");
            assert_eq!(Ink::from_id(ink.id()), Some(ink));
        }
        assert_eq!(Ink::from_id(0), None);
        assert_eq!(Ink::from_id(99), None);
        assert_eq!(Ink::all().len(), 18);
    }
}
