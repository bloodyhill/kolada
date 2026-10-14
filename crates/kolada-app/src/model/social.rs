//! The social preview, as on Windows (`kolada-win` `app.rs`): the side menu's
//! drawer lists the platforms; one opens the note as a post would read
//! there (where "see more" cuts it, how many characters it has), in a card
//! that scrolls, can be dragged by its header, and switches by its tabs.

use kolada_core::social::Platform;
use kolada_ui::social::{SocialView, card_bounds};

use super::*;

#[derive(Default)]
pub(super) struct Social {
    /// The preview's platform while it is open; how far it is scrolled and
    /// can be (from the last frame).
    pub platform: Option<Platform>,
    pub scroll: f32,
    pub scroll_max: f32,
    /// Where it was dragged to (DIPs from its place), and where the pointer
    /// holds it while it is dragged.
    offset: (f32, f32),
    grip: Option<(f32, f32)>,
}

fn on_card(t: Option<Target>) -> bool {
    matches!(t, Some(Target::SocialCard | Target::SocialHeader | Target::SocialTab(_) | Target::SocialClose))
}

impl Model {
    /// Opens the preview for `platform`, or closes it.
    pub fn show_social(&mut self, platform: Option<Platform>) -> bool {
        if platform != self.social.platform {
            self.social.scroll = 0.0;
        }
        self.social.platform = platform;
        if platform.is_some() {
            self.close_popup();
        }
        true
    }

    pub(super) fn social_view(&self) -> Option<SocialView> {
        self.social.platform.map(|platform| SocialView { platform, scroll: self.social.scroll, offset: self.social.offset })
    }

    /// A click on the drawer or the preview. None when it is not theirs.
    pub(super) fn social_click(&mut self, target: Option<Target>, x: f32, y: f32) -> Option<bool> {
        Some(match target? {
            Target::SocialDrawer => {
                self.session.social = if self.session.social == 2 { 1 } else { 2 };
                self.persist_session();
                true
            }
            // The drawer's platform opens its preview (again: closes it);
            // the card's tabs switch it.
            Target::Social(p) if self.social.platform == Some(p) => self.show_social(None),
            Target::Social(p) | Target::SocialTab(p) => self.show_social(Some(p)),
            Target::SocialClose => self.show_social(None),
            Target::SocialCard | Target::SocialHeader => {
                self.social.grip = Some((x - self.social.offset.0, y - self.social.offset.1));
                true
            }
            _ => return None,
        })
    }

    /// The preview held by its card and dragged to (x, y). None when it is
    /// not held.
    pub(super) fn social_dragged(&mut self, x: f32, y: f32) -> Option<bool> {
        let (gx, gy) = self.social.grip?;
        // Kept in the window, as the card is drawn.
        let (w, h) = self.viewport;
        let home = card_bounds(w, h, (0.0, 0.0));
        let at = card_bounds(w, h, (x - gx, y - gy));
        self.social.offset = (at.left - home.left, at.top - home.top);
        Some(true)
    }

    pub(super) fn social_up(&mut self) -> bool {
        self.social.grip.take().is_some()
    }

    /// The wheel over the preview scrolls the post. None elsewhere.
    pub(super) fn social_wheel(&mut self, dy: f32) -> Option<bool> {
        if self.social.platform.is_none() || !on_card(self.hover) {
            return None;
        }
        self.social.scroll = (self.social.scroll + dy).clamp(0.0, self.social.scroll_max);
        Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(test: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-social-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let long: String = (0..60).map(|i| format!("Line {i} of a long post that goes on, مرحبا.\n")).collect();
        std::fs::write(dir.join("Post.md"), format!("# Launch day\n\n{long}")).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        (m, dir)
    }

    fn click(m: &mut Model, t: Target) -> bool {
        let (r, _) = *m.hits.iter().find(|(_, h)| *h == t).unwrap_or_else(|| panic!("{t:?} on screen"));
        let done = m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
        m.pointer_up();
        m.frame(2000, 1440, 2.0);
        done
    }

    #[test]
    fn the_drawer_opens_a_platforms_preview_its_tabs_switch_and_close() {
        let (mut m, dir) = model("drawer");
        assert!(!m.hits.iter().any(|(_, t)| matches!(t, Target::Social(_))), "the drawer starts folded");
        click(&mut m, Target::SocialDrawer);
        assert_eq!(m.session.social, 2);
        click(&mut m, Target::Social(Platform::LinkedIn));
        assert_eq!(m.social.platform, Some(Platform::LinkedIn));
        assert!(m.hits.iter().any(|(_, t)| *t == Target::SocialCard));
        click(&mut m, Target::SocialTab(Platform::X));
        assert_eq!(m.social.platform, Some(Platform::X));
        // It scrolls under the wheel, within what it has.
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::SocialCard).unwrap();
        m.pointer_moved(r.left + 20.0, r.top + 200.0);
        assert!(m.wheel(400.0, true));
        assert!(m.social.scroll > 0.0 && m.social.scroll <= m.social.scroll_max.max(0.0) + 0.1);
        // Dragged by its header, it stays in the window.
        let (h, _) = *m.hits.iter().find(|(_, t)| *t == Target::SocialHeader).unwrap();
        m.pointer_down(h.left + 30.0, h.top + 10.0, 1, false);
        m.pointer_dragged(-5000.0, h.top + 10.0);
        m.pointer_up();
        m.frame(2000, 1440, 2.0);
        let (c, _) = *m.hits.iter().find(|(_, t)| *t == Target::SocialCard).unwrap();
        assert!(c.left >= 16.0, "{c:?}");
        assert!(m.escape());
        assert_eq!(m.social.platform, None, "Escape closes it");
        let _ = std::fs::remove_dir_all(dir);
    }
}
