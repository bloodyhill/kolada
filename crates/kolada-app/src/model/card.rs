//! Files dragged over the window, before they are let go, as on Windows
//! (`kolada-win` `app/arrive.rs`): a card follows the pointer saying what
//! they are and what they become, the note steps back when they go
//! elsewhere, and pictures open a gap in it where they will land. Let go,
//! the card flies to the side menu where new notes appear, or fades.

use kolada_ui::images;

use super::arrive::{is_document, is_note_file};
use super::motion::settle;
use super::*;

/// How quickly the card follows the pointer, and the gap opens.
const FOLLOW: f32 = 20.0;
const OPEN: f32 = 24.0;
/// The gap pictures open in the note, DIPs.
const GAP: f32 = 76.0;
/// The card's corner, from the pointer: below it and to its right.
const BESIDE: (f32, f32) = (16.0, 18.0);
/// How fast the card fades in and out (per second).
const FADE: f32 = 12.0;

pub(super) struct Arrival {
    title: String,
    change: String,
    /// Something here can be taken.
    readable: bool,
    count: usize,
    /// How many are pictures (into this note) and how many become notes.
    pictures: usize,
    notes: usize,
    /// Where the card is going (DIPs), and how far from there it still is.
    target: (f32, f32),
    offset: (f32, f32),
    velocity: (f32, f32),
    alpha: f32,
    /// Let go or left: fading out.
    gone: bool,
    /// The end of the line pictures would land after; the gap's height and speed.
    landing: Option<usize>,
    gap: f32,
    gap_velocity: f32,
}

/// `s`, cut to `most` characters with an ellipsis.
fn shorten(s: &str, most: usize) -> String {
    if s.chars().count() <= most { s.to_string() } else { s.chars().take(most - 1).collect::<String>() + "\u{2026}" }
}

impl Arrival {
    fn new(files: &[PathBuf], read_only: bool, at: (f32, f32)) -> Arrival {
        let picture = |f: &PathBuf| images::is_image(&f.to_string_lossy());
        let note = |f: &PathBuf| is_document(f) || is_note_file(f);
        let pictures = if read_only { 0 } else { files.iter().filter(|f| picture(f)).count() };
        let notes = files.iter().filter(|f| note(f)).count();
        let readable = pictures + notes > 0;
        let name = |f: &PathBuf| f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let exported = |f: &PathBuf| super::arrive::is_nbk(f) || kolada_convert::import::Source::of(f).is_some();
        let title = match files.len() {
            1 => shorten(&name(&files[0]), 34),
            n if pictures == n => format!("{n} pictures"),
            n if notes == n && files.iter().all(|f| kolada_convert::Kind::of(f).is_some()) => format!("{n} documents"),
            n => format!("{n} files"),
        };
        let change = if !readable {
            if read_only && files.iter().any(picture) { "Not on this page".to_string() } else { "Kolada can't read this".to_string() }
        } else {
            let mut formats: Vec<String> = Vec::new();
            for f in files.iter().filter(|f| note(f) || (!read_only && picture(f))) {
                let ext = f.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
                let ext = if ext == "HTM" { "HTML".to_string() } else if ext == "JPEG" { "JPG".to_string() } else { ext };
                if !ext.is_empty() && !formats.contains(&ext) {
                    formats.push(ext);
                }
            }
            if formats.len() > 3 {
                let more = formats.len() - 2;
                formats.truncate(2);
                formats.push(format!("+{more}"));
            }
            let exports = files.iter().any(exported);
            let mut to = Vec::new();
            match notes {
                0 => {}
                1 if !exports => to.push("New note".to_string()),
                _ if exports => to.push("New notes".to_string()),
                n => to.push(format!("{n} new notes")),
            }
            if pictures > 0 {
                to.push("This note".to_string());
            }
            format!("{} \u{2192} {}", formats.join(" \u{00B7} "), to.join(" + "))
        };
        Arrival { title, change, readable, count: files.len(), pictures, notes, target: at, offset: (0.0, 10.0), velocity: (0.0, 0.0), alpha: 0.0, gone: false, landing: None, gap: 0.0, gap_velocity: 0.0 }
    }

    /// Sends the card toward `to`, from where it is now.
    fn aim(&mut self, to: (f32, f32)) {
        self.offset.0 += self.target.0 - to.0;
        self.offset.1 += self.target.1 - to.1;
        self.target = to;
    }

    /// One frame of its springs and fades; false once all is still.
    fn step(&mut self, dt: f32) -> bool {
        let moving_x = settle(&mut self.offset.0, &mut self.velocity.0, dt, FOLLOW);
        let moving_y = settle(&mut self.offset.1, &mut self.velocity.1, dt, FOLLOW);
        let want = if self.gone { 0.0 } else { 1.0 };
        self.alpha += (want - self.alpha) * (1.0 - (-FADE * dt).exp());
        let fading = (want - self.alpha).abs() > 0.01;
        if !fading {
            self.alpha = want;
        }
        let open = if self.gone || self.landing.is_none() || self.pictures == 0 { 0.0 } else { GAP };
        let mut d = self.gap - open;
        let opening = settle(&mut d, &mut self.gap_velocity, dt, OPEN);
        self.gap = (open + d).max(0.0);
        moving_x || moving_y || fading || opening
    }
}

impl Model {
    /// Files dragged in at (x, y) DIPs: true if any can be taken.
    pub fn drag_enter(&mut self, files: &[PathBuf], x: f32, y: f32) -> bool {
        self.arrival = Some(Arrival::new(files, self.settings.read_only, (x + BESIDE.0, y + BESIDE.1)));
        self.drag_over(x, y)
    }

    /// The pointer moved, dragging: the card follows, and the gap goes to
    /// the line nearest it.
    pub fn drag_over(&mut self, x: f32, y: f32) -> bool {
        let pictures = self.arrival.as_ref().is_some_and(|a| a.pictures > 0);
        let landing = if pictures { self.landing_at(y) } else { None };
        let Some(a) = &mut self.arrival else { return false };
        a.gone = false;
        a.aim((x + BESIDE.0, y + BESIDE.1));
        a.landing = landing.or(a.landing.filter(|_| pictures));
        if !self.ui.animate {
            a.offset = (0.0, 0.0);
            a.alpha = 1.0;
            a.gap = if a.landing.is_some() && a.pictures > 0 { GAP } else { 0.0 };
        }
        a.readable
    }

    /// The pointer left with the files: the card fades and the gap closes.
    pub fn drag_leave(&mut self) -> bool {
        if let Some(a) = &mut self.arrival {
            a.gone = true;
        }
        if !self.ui.animate {
            self.arrival = None;
        }
        true
    }

    /// Let go: the card is on its way (new notes to the side menu, where
    /// they appear); the line end pictures land after, if the gap showed one.
    pub fn drag_drop(&mut self) -> Option<usize> {
        let side = self.panels.show[0] > 0.5 && !self.compact && !self.session.minimal;
        let a = self.arrival.as_mut()?;
        let at = a.landing.filter(|_| a.pictures > 0);
        a.gone = true;
        if a.notes > 0 && side {
            a.aim((24.0, 132.0));
        }
        if at.is_some() {
            a.gap = 0.0;
            a.gap_velocity = 0.0;
        }
        if !self.ui.animate {
            self.arrival = None;
        }
        at
    }

    /// The end of the line pictures dropped at height `y` land after: the
    /// line under it, or the one before if the pointer is in its upper half.
    /// Over the gap already open, where it is.
    fn landing_at(&self, y: f32) -> Option<usize> {
        let lines = &self.lines;
        let last = lines.last()?;
        if let Some((a, i)) = self.arrival.as_ref().and_then(|a| Some((a, lines.iter().position(|l| Some(l.para.end) == a.landing)?))) {
            let top = lines[i].y + lines[i].height;
            let bottom = lines.get(i + 1).map_or(top + a.gap, |n| n.y);
            if y >= top - 4.0 && y <= bottom + 4.0 {
                return a.landing;
            }
        }
        let k = lines.iter().position(|l| y < l.y + l.height).unwrap_or(lines.len() - 1);
        let line = &lines[k];
        Some(if y < line.y + line.height / 2.0 && k > 0 {
            lines[k - 1].para.end
        } else if k == lines.len() - 1 && y > last.y + last.height {
            last.para.end
        } else {
            line.para.end
        })
    }

    /// What the card says while files are dragged over: their name or how
    /// many, and what they become ("DOCX · PDF → 2 new notes").
    pub fn drag_card(&self) -> Option<(&str, &str)> {
        self.arrival.as_ref().filter(|a| !a.gone).map(|a| (a.title.as_str(), a.change.as_str()))
    }

    pub(super) fn arrival_moving(&self) -> bool {
        self.arrival.is_some()
    }

    /// One frame of the card; whether it changed.
    pub(super) fn step_arrival(&mut self, dt: f32) -> bool {
        let Some(a) = &mut self.arrival else { return false };
        let moving = a.step(dt);
        if a.gone && !moving {
            self.arrival = None;
        }
        true
    }

    /// The card and the gap, as the scene draws them.
    pub(super) fn arrival_view(&self) -> Option<scene::Arriving<'_>> {
        let a = self.arrival.as_ref()?;
        Some(scene::Arriving {
            title: &a.title,
            change: &a.change,
            x: a.target.0 + a.offset.0,
            y: a.target.1 + a.offset.1,
            alpha: a.alpha,
            readable: a.readable,
            stack: a.count > 1,
            // The note steps back when nothing lands in it.
            dim: if a.readable && a.pictures == 0 { a.alpha } else { 0.0 },
            landing: a.landing.filter(|_| a.gap > 0.5).map(|at| (at, a.gap, a.pictures)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titled(files: &[&str]) -> (String, String, bool) {
        let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        let a = Arrival::new(&files, false, (0.0, 0.0));
        (a.title, a.change, a.readable)
    }

    #[test]
    fn says_what_files_are_and_what_they_become() {
        assert_eq!(titled(&["/in/Invoice 1042.pdf"]), ("Invoice 1042.pdf".into(), "PDF \u{2192} New note".into(), true));
        assert_eq!(titled(&["a.pdf", "b.docx", "c.pdf"]), ("3 documents".into(), "PDF \u{00B7} DOCX \u{2192} 3 new notes".into(), true));
        assert_eq!(titled(&["a.png", "b.jpeg"]), ("2 pictures".into(), "PNG \u{00B7} JPG \u{2192} This note".into(), true));
        assert_eq!(titled(&["a.png", "b.pdf"]), ("2 files".into(), "PNG \u{00B7} PDF \u{2192} New note + This note".into(), true));
        assert_eq!(titled(&["phone.nbk"]), ("phone.nbk".into(), "NBK \u{2192} New notes".into(), true));
        assert_eq!(titled(&["Work.enex"]), ("Work.enex".into(), "ENEX \u{2192} New notes".into(), true));
        assert_eq!(titled(&["setup.exe"]), ("setup.exe".into(), "Kolada can't read this".into(), false));
        assert_eq!(titled(&["ملاحظة.md"]).1, "MD \u{2192} New note");
    }

    #[test]
    fn the_card_follows_the_pointer_and_pictures_open_a_gap_where_they_land() {
        let dir = std::env::temp_dir().join(format!("kolada-app-card-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Note.md"), "# Note\n\nFirst line.\n\nSecond line.\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        let second = m.lines.iter().find(|l| m.text()[l.para.start..].starts_with("Second")).unwrap();
        let (y, end) = (second.y + second.height * 0.75, second.para.end);
        assert!(m.drag_enter(&[PathBuf::from("/x/cat.png")], 700.0, y));
        let view = m.arrival_view().unwrap();
        assert_eq!((view.x, view.y), (716.0, y + 18.0), "beside the pointer");
        assert_eq!(view.landing.map(|l| l.0), Some(end), "the gap after the line under the pointer");
        m.frame(2000, 1440, 2.0);
        assert_eq!(m.drag_drop(), Some(end));
        // A file nothing reads: no copy offered.
        assert!(!m.drag_enter(&[PathBuf::from("/x/setup.exe")], 700.0, y));
        m.drag_leave();
        assert!(m.arrival_view().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every frame of a drop's card flying off and fading, drawn as the
    /// window draws them (only where they change), ends as a whole frame
    /// would draw it: nothing of the card is left behind.
    #[test]
    fn a_dropped_cards_flight_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("kolada-app-card-repaint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Note.md"), "# Note\n\nFirst line.\n").unwrap();
        let mut m = Model::new(dir.clone(), false);
        m.ui.animate = true;
        let (w, h) = (1600, 1000);
        m.frame(w, h, 1.0);
        let files = [dir.join("cat.png")];
        // In at the side menu's edge, across the page, let go low down.
        m.drag_enter(&files, 42.0, 133.0);
        m.frame(w, h, 1.0);
        for i in 0..30 {
            m.drag_over(42.0 + i as f32 * 20.0, 133.0 + i as f32 * 14.0);
            m.tick();
            m.frame(w, h, 1.0);
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        m.drag_drop();
        for _ in 0..400 {
            if !m.wants_frames() {
                break;
            }
            m.tick();
            m.frame(w, h, 1.0);
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        assert!(!m.wants_frames(), "the card comes to rest");
        let shown = m.surface().pixels().to_vec();
        m.surface = Surface::new((1, 1, 96));
        m.frame(w, h, 1.0);
        let whole = m.surface().pixels();
        let differ = shown.iter().zip(whole).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "pixels left behind by the card");
        let _ = std::fs::remove_dir_all(dir);
    }
}
