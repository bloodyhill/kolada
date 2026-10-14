//! Files dragged over the window, before they are let go (see `drag`): a
//! card follows the pointer saying what they are and what they become, the
//! note steps back when they go elsewhere, and pictures open a gap in it
//! where they will land. Let go, the card flies to the side menu where the
//! new notes appear, or fades where it is.

use crate::images;

use super::*;

/// How quickly the card follows the pointer, and the gap opens: springs,
/// as the caret's glide is, softer.
const FOLLOW: f32 = 20.0;
const OPEN: f32 = 24.0;
/// The gap pictures open in the note, DIPs.
const GAP: f32 = 76.0;
/// The card's corner, from the pointer: below it and to its right.
const BESIDE: (f32, f32) = (16.0, 18.0);
/// How fast the card fades in and out (per second).
const FADE: f32 = 12.0;

pub(super) struct Arrival {
    files: Vec<PathBuf>,
    title: String,
    change: String,
    /// Something here can be taken.
    readable: bool,
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
    /// The end of the line pictures would land after; the gap's height, and
    /// its speed.
    landing: Option<usize>,
    gap: f32,
    gap_velocity: f32,
}

impl Arrival {
    fn new(files: Vec<PathBuf>, read_only: bool, at: (f32, f32)) -> Arrival {
        let picture = |f: &PathBuf| images::is_image(&f.to_string_lossy());
        let note = |f: &PathBuf| {
            crate::convert::is_document(f)
                || share::is_export(f)
                || f.extension().is_some_and(|e| ["md", "markdown", "txt"].contains(&e.to_string_lossy().to_lowercase().as_str()))
        };
        let pictures = if read_only { 0 } else { files.iter().filter(|f| picture(f)).count() };
        let notes = files.iter().filter(|f| note(f)).count();
        let readable = pictures + notes > 0;
        let name = |f: &PathBuf| f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let title = match files.len() {
            1 => shorten(&name(&files[0]), 34),
            n if pictures == n => format!("{n} pictures"),
            n if notes == n && files.iter().all(|f| crate::convert::is_document(f)) => format!("{n} documents"),
            n => format!("{n} files"),
        };
        let change = if !readable {
            if read_only && files.iter().any(picture) { "Not on this page".to_string() } else { "Kolada can't read this".to_string() }
        } else {
            // Formats, as their extensions, then what they become.
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
            let exports = files.iter().any(|f| share::is_export(f) || kolada_convert::import::Source::of(f).is_some());
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
        Arrival {
            files,
            title,
            change,
            readable,
            pictures,
            notes,
            target: at,
            offset: (0.0, 0.0),
            velocity: (0.0, 0.0),
            alpha: 0.0,
            gone: false,
            landing: None,
            gap: 0.0,
            gap_velocity: 0.0,
        }
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

/// `s`, cut to `most` characters with an ellipsis.
fn shorten(s: &str, most: usize) -> String {
    if s.chars().count() <= most { s.to_string() } else { s.chars().take(most - 1).collect::<String>() + "\u{2026}" }
}

/// What a drop carries to the frame after it: the files, where (client
/// pixels), and the line end pictures land after, if the gap showed one.
pub struct Dropped {
    pub files: Vec<PathBuf>,
    pub x: i32,
    pub y: i32,
    pub at: Option<usize>,
}

impl App {
    fn dips(&self, x: i32, y: i32) -> (f32, f32) {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        (x as f32 / scale, y as f32 / scale)
    }

    /// Files dragged in at a client point in pixels: true if any can be
    /// taken.
    pub(super) fn drag_enter(&mut self, files: Vec<PathBuf>, x: i32, y: i32) -> bool {
        let p = self.dips(x, y);
        let mut a = Arrival::new(files, self.read_only, (p.0 + BESIDE.0, p.1 + BESIDE.1));
        // It rises into place as it fades in.
        a.offset = (0.0, 10.0);
        self.arrival = Some(a);
        self.drag_over(x, y)
    }

    /// The pointer moved, dragging: the card follows, and the gap goes to
    /// the line nearest it.
    pub(super) fn drag_over(&mut self, x: i32, y: i32) -> bool {
        let p = self.dips(x, y);
        let pictures = self.arrival.as_ref().is_some_and(|a| a.pictures > 0);
        let landing = if pictures { self.landing_at(p.1) } else { None };
        let Some(a) = &mut self.arrival else { return false };
        a.gone = false;
        a.aim((p.0 + BESIDE.0, p.1 + BESIDE.1));
        a.landing = landing.or(a.landing.filter(|_| pictures));
        let takes = a.readable;
        self.arrival_moving = true;
        self.request_frame();
        takes
    }

    /// The pointer left with the files: the card fades and the gap closes.
    pub(super) fn drag_leave(&mut self) {
        if let Some(a) = &mut self.arrival {
            a.gone = true;
        }
        self.arrival_moving = true;
        self.request_frame();
    }

    /// Let go at a client point in pixels: taken a moment later (the app
    /// dragged from waits for this to return), the card on its way.
    pub(super) fn drag_drop(&mut self, files: Vec<PathBuf>, x: i32, y: i32) -> bool {
        let compact = self.compact || self.session.minimal;
        let side = self.ui.show[0] > 0.5 && !compact;
        let Some(a) = &mut self.arrival else { return false };
        let takes = a.readable;
        let at = a.landing.filter(|_| a.pictures > 0);
        a.gone = true;
        // New notes go to the side menu, where they appear; pictures take
        // the gap's place at once.
        if a.notes > 0 && side {
            a.aim((24.0, 132.0));
        }
        if at.is_some() {
            a.gap = 0.0;
            a.gap_velocity = 0.0;
        }
        let files = if files.is_empty() { std::mem::take(&mut a.files) } else { files };
        self.arrival_moving = true;
        self.request_frame();
        if takes {
            let dropped = Box::into_raw(Box::new(Dropped { files, x, y, at }));
            let posted = unsafe { PostMessageW(Some(self.hwnd), window::WM_APP_DROPPED, WPARAM(0), LPARAM(dropped as isize)) };
            if posted.is_err() {
                drop(unsafe { Box::from_raw(dropped) });
            }
        }
        takes
    }

    /// The end of the line pictures dropped at height `y` (DIPs) land
    /// after: the line under it, or the one before if the pointer is in its
    /// upper half. Over the gap already open, where it is.
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
        Some(if y < line.y + line.height / 2.0 && k > 0 { lines[k - 1].para.end } else if k == lines.len() - 1 && y > last.y + last.height { last.para.end } else { line.para.end })
    }

    /// One frame of the card; false once still.
    pub(super) fn step_arrival(&mut self, dt: f32) -> bool {
        let Some(a) = &mut self.arrival else { return false };
        let moving = a.step(dt);
        if a.gone && !moving {
            self.arrival = None;
        }
        moving
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
            stack: a.files.len() > 1,
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
        let a = Arrival::new(files.iter().map(PathBuf::from).collect(), false, (0.0, 0.0));
        (a.title, a.change, a.readable)
    }

    #[test]
    fn says_what_files_are_and_what_they_become() {
        assert_eq!(titled(&["C:/in/Invoice 1042.pdf"]), ("Invoice 1042.pdf".into(), "PDF \u{2192} New note".into(), true));
        assert_eq!(titled(&["a.pdf", "b.docx", "c.pdf"]), ("3 documents".into(), "PDF \u{00B7} DOCX \u{2192} 3 new notes".into(), true));
        assert_eq!(titled(&["a.png", "b.jpeg"]), ("2 pictures".into(), "PNG \u{00B7} JPG \u{2192} This note".into(), true));
        assert_eq!(titled(&["a.png", "b.pdf"]), ("2 files".into(), "PNG \u{00B7} PDF \u{2192} New note + This note".into(), true));
        assert_eq!(titled(&["phone.nbk"]), ("phone.nbk".into(), "NBK \u{2192} New notes".into(), true));
        assert_eq!(titled(&["Work.enex"]), ("Work.enex".into(), "ENEX \u{2192} New notes".into(), true));
        assert_eq!(titled(&["Export-1a2b.zip"]), ("Export-1a2b.zip".into(), "ZIP \u{2192} New notes".into(), true));
        assert_eq!(titled(&["setup.exe"]), ("setup.exe".into(), "Kolada can't read this".into(), false));
        let long = titled(&["A very long document name that goes on and on.pdf"]).0;
        assert!(long.ends_with('\u{2026}') && long.chars().count() == 34, "{long}");
    }

    #[test]
    fn the_card_springs_after_the_pointer_and_fades_when_gone() {
        let mut a = Arrival::new(vec![PathBuf::from("a.pdf")], false, (100.0, 100.0));
        a.aim((300.0, 100.0));
        assert_eq!(a.target.0 + a.offset.0, 100.0, "it starts where it was");
        for _ in 0..120 {
            a.step(1.0 / 60.0);
        }
        assert!((a.target.0 + a.offset.0 - 300.0).abs() < 0.5 && a.alpha == 1.0);
        a.gone = true;
        let mut frames = 0;
        while a.step(1.0 / 60.0) {
            frames += 1;
        }
        assert!(a.alpha == 0.0 && frames < 60, "{frames}");
    }
}
