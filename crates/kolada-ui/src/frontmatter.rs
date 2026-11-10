//! A note's frontmatter (`---`, `key: value` lines, `---`) at rest: one
//! quiet line above the title, the same on every OS. A caught page shows
//! where it came from and when (`kitchen.example  ·  caught 8 Nov`), its
//! host a link; other frontmatter shows its values (`#recipe #lemon ·
//! Lemon tart`), or, with none worth showing, its keys. The caret on that
//! line opens the lines as written, for editing (see `note::FrontView`);
//! off it, they fold again. Display only: the text is never changed.

use std::cell::Cell;
use std::time::Instant;

use kolada_core::when::Day;
use kolada_markdown::{self as md, Block, Line};

/// The last line of the frontmatter a note opens with, if it opens with
/// one (one that never closes is text, and none).
pub fn block(lines: &[Line]) -> Option<usize> {
    if lines.first()?.block != Block::Frontmatter {
        return None;
    }
    Some(lines.iter().take_while(|l| l.block == Block::Frontmatter).count() - 1)
}

/// Whether `caret` is in the note's frontmatter: then its lines show as
/// written.
pub fn caret_in(lines: &[Line], caret: usize) -> bool {
    block(lines).is_some_and(|last| caret <= lines[last].end)
}

/// Where the caret goes as a note opens, given where it would (`at`) and
/// where writing starts under the title (`body`): never in the
/// frontmatter, which every open shows folded to its one line.
pub fn rest_caret(lines: &[Line], at: usize, body: usize) -> usize {
    if caret_in(lines, at) { body } else { at }
}

/// What the frontmatter's line needs besides the text: how open it is
/// while it opens or folds (0 folded .. 1 as written; None: as the caret
/// has it, at once), and the year it is (a date in another shows its
/// year; 0: the clock's). And what the note's layout found of it, found
/// once for the frame, not at each of its lines.
#[derive(Default)]
pub struct Front {
    pub open: Option<f32>,
    pub year: i32,
    pub(crate) kept: Kept,
}

impl Front {
    pub fn new(open: Option<f32>, year: i32) -> Front {
        Front { open, year, kept: Kept::default() }
    }
}

/// The frontmatter's last line and its one line, as the note's layout
/// (`note::NoteText`, made anew for each frame and each edit) first finds
/// them.
#[derive(Default)]
pub(crate) struct Kept {
    pub(crate) last: std::cell::OnceCell<Option<usize>>,
    pub(crate) line: std::cell::OnceCell<Option<std::rc::Rc<crate::note::FrontLine>>>,
}

/// The line, before it is fitted to the page: its text, where the
/// source's host is in it and the address it opens, and whether it is
/// only keys (drawn fainter than values).
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub text: String,
    pub link: Option<(usize, usize)>,
    pub url: Option<String>,
    pub keys: bool,
}

/// The frontmatter's `key: value` entries, in order, each value a list
/// (`[a, b]` and `- a` lines flattened). Nested maps and block text are
/// left out.
pub fn entries(yaml: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    // Whether `- item` lines belong to the last key (it had no value).
    let mut listing = false;
    for raw in yaml.lines() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t', '-']) {
            if let Some(item) = trimmed.strip_prefix('-').filter(|_| listing)
                && let Some(last) = out.last_mut()
            {
                let item = unquote(item.trim());
                if !item.is_empty() {
                    last.1.push(item);
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            listing = false;
            continue;
        };
        let (key, value) = (key.trim().to_string(), value.trim());
        if key.is_empty() {
            listing = false;
            continue;
        }
        listing = value.is_empty();
        let values = if value.starts_with('[') && value.ends_with(']') {
            value[1..value.len() - 1].split(',').map(|v| unquote(v.trim())).filter(|v| !v.is_empty()).collect()
        } else if value.is_empty() || value.starts_with(['|', '>', '{']) {
            Vec::new()
        } else {
            vec![unquote(value)]
        };
        out.push((key, values));
    }
    out
}

/// A YAML scalar as it reads: its quotes off.
fn unquote(v: &str) -> String {
    let quoted = v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')));
    if !quoted {
        return v.to_string();
    }
    let inner = &v[1..v.len() - 1];
    if v.starts_with('"') { inner.replace("\\\"", "\"").replace("\\\\", "\\") } else { inner.replace("''", "'") }
}

/// A value worth a place on the line: not empty, not a yes or no.
fn telling(v: &str) -> bool {
    let lower = v.trim().to_ascii_lowercase();
    !lower.is_empty() && !matches!(lower.as_str(), "true" | "false" | "yes" | "no" | "null" | "~")
}

/// A web address's host, as it is read (`www.` off, no port), and the
/// address it opens. None for anything that is not a web page.
pub fn host(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let (rest, open) = if let Some(r) = ["https://", "http://"].iter().find_map(|p| lower.starts_with(p).then(|| &url[p.len()..])) {
        (r, url.to_string())
    } else if lower.starts_with("www.") {
        (url, format!("https://{url}"))
    } else {
        return None;
    };
    // Where the browser's host ends: a backslash too, as browsers read one
    // (`https://evil.example\@bank.example/` opens evil.example).
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let name = authority.split(':').next().unwrap_or("").to_ascii_lowercase();
    let name = name.strip_prefix("www.").unwrap_or(&name).to_string();
    // Marks that turn or hide letters (bidi controls, zero-width and other
    // format characters) could show one host and open another: no link.
    let hidden = |c: char| matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}');
    (!name.is_empty() && !name.contains(|c: char| c.is_whitespace() || c.is_control() || hidden(c))).then_some((name, open))
}

/// A `caught` time's day: its date as written (the catcher writes the
/// local time, `2026-11-08T15:20`).
fn day_of(v: &str) -> Option<Day> {
    let v = v.trim();
    let (y, m, d) = (v.get(0..4)?.parse().ok()?, v.get(5..7)?.parse().ok()?, v.get(8..10)?.parse().ok()?);
    (v.as_bytes().get(4) == Some(&b'-') && v.as_bytes().get(7) == Some(&b'-')).then_some(())?;
    Day::new(y, m, d)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `8 Nov`, or `8 Nov 2025` in another year than `year`.
pub fn short_date(day: Day, year: i32) -> String {
    let month = MONTHS[(day.month as usize).clamp(1, 12) - 1];
    if day.year == year { format!("{} {month}", day.day) } else { format!("{} {month} {}", day.day, day.year) }
}

/// The year now, by the clock (UTC: only a few hours of a year differ).
fn this_year() -> i32 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Day::from_number((secs / 86_400) as i64).year
}

/// The line for the frontmatter `yaml` (between its `---`s), in a note
/// that reads right to left or not; None for one with nothing in it (it
/// shows as written).
pub fn shown(yaml: &str, year: i32, rtl: bool) -> Option<Shown> {
    let entries = entries(yaml);
    if entries.is_empty() {
        return None;
    }
    let year = if year == 0 { this_year() } else { year };
    let value = |name: &str| entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| v.first()).filter(|v| telling(v));
    // Right to left, each part keeps its place: a mark either side of the
    // dot ends the run of Latin before it, and one after it starts the
    // next part left to right (`9 words` stays `9 words`).
    let dot = |wide: bool| match (wide, rtl) {
        (true, false) => "  \u{00B7}  ",
        (true, true) => "  \u{200F}\u{00B7}\u{200F}  \u{200E}",
        (false, false) => " \u{00B7} ",
        (false, true) => " \u{200F}\u{00B7}\u{200F} \u{200E}",
    };
    let source = value("source");
    let caught = value("caught").and_then(|v| day_of(v));
    if source.is_some() || caught.is_some() {
        let mut out = Shown { text: String::new(), link: None, url: None, keys: false };
        match source.map(|s| (s, host(s))) {
            Some((_, Some((name, url)))) => {
                out.link = Some((0, name.len()));
                out.url = Some(url);
                out.text = name;
            }
            Some((s, None)) => out.text = s.clone(),
            None => {}
        }
        if let Some(day) = caught {
            let date = short_date(day, year);
            if out.text.is_empty() {
                out.text = format!("Caught {date}");
            } else {
                out.text = format!("{}{}caught {date}", out.text, dot(true));
            }
        }
        return Some(out);
    }
    let mut parts = Vec::new();
    for (key, values) in &entries {
        let part = if key.eq_ignore_ascii_case("tags") || key.eq_ignore_ascii_case("tag") {
            let tags: Vec<String> = values.iter().flat_map(|v| v.split([',', ' '])).filter(|t| !t.is_empty() && telling(t)).map(|t| if t.starts_with('#') { t.to_string() } else { format!("#{t}") }).collect();
            tags.join(" ")
        } else {
            values.iter().filter(|v| telling(v)).map(|v| v.trim()).collect::<Vec<_>>().join(", ")
        };
        if !part.is_empty() {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        let keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
        return Some(Shown { text: keys.join(dot(false)), link: None, url: None, keys: true });
    }
    Some(Shown { text: parts.join(dot(false)), link: None, url: None, keys: false })
}

/// The text between a note's frontmatter's `---`s.
pub fn yaml<'a>(text: &'a str, lines: &[Line]) -> Option<&'a str> {
    let last = block(lines)?;
    if last == 0 {
        return Some("");
    }
    let (a, z) = (lines[1].start, if last > 1 { lines[last - 1].end } else { lines[1].start });
    text.get(a..z.max(a))
}

/// Where a note's `source` leads, when it is a web page: what a click on
/// its host opens.
pub fn source_url(text: &str) -> Option<String> {
    let lines: Vec<Line> = md::lines(text).take_while(|l| l.block == Block::Frontmatter).collect();
    let yaml = yaml(text, &lines)?;
    let shown = shown(yaml, 1, false)?;
    shown.url.filter(|u| crate::links::is_web(u))
}

/// Whether a press on the host (the `clicks`th of a run) opens its page:
/// the first only, so a double click opens it once.
pub fn source_opens(clicks: u32) -> bool {
    clicks <= 1
}

/// The frontmatter opening or folding over time, as a shell keeps it
/// between frames: it follows the caret, on the settings' spring
/// (`chrome::folding`), and snaps with motion off. Kept in cells, so the
/// frame's view (built from `&self`) can step it.
#[derive(Default)]
pub struct Motion {
    /// The note (its name's hash) and whether its frontmatter was open,
    /// last frame.
    last: Cell<Option<(u64, bool)>>,
    /// When it began opening (true) or folding.
    since: Cell<Option<(Instant, bool)>>,
}

impl Motion {
    /// How open the frontmatter of note `note` is this frame, while it
    /// moves (for `scene::Input::front_fold`); None at rest. `open`: the
    /// caret is in it (see `caret_in`).
    pub fn frame(&self, note: u64, open: bool, animate: bool) -> Option<f32> {
        let before = self.last.replace(Some((note, open)));
        let now = Instant::now();
        match before {
            // The same note, the caret in or out of it: it opens or folds,
            // from where it is if it was still moving the other way.
            Some((n, was)) if n == note && was != open && animate => {
                let at = self.value(now).map_or(0.0, |v| time_for(if open { v } else { 1.0 - v }));
                self.since.set(Some((now.checked_sub(std::time::Duration::from_secs_f32(at)).unwrap_or(now), open)));
            }
            Some((n, _)) if n == note && animate => {}
            // Another note, or motion off: as it is, at once.
            _ => self.since.set(None),
        }
        let v = self.value(now);
        if v.is_none() {
            self.since.set(None);
        }
        v
    }

    fn value(&self, now: Instant) -> Option<f32> {
        let (at, opening) = self.since.get()?;
        crate::chrome::folding(now.duration_since(at).as_secs_f32(), opening)
    }

    /// Whether it is moving: frames are wanted until it settles.
    pub fn moving(&self) -> bool {
        self.since.get().is_some_and(|(at, _)| at.elapsed().as_secs_f32() < crate::chrome::FOLD_SECS)
    }
}

/// The time into an opening at which it is `v` of the way (folding is
/// the same curve, upside down).
fn time_for(v: f32) -> f32 {
    let f = |t: f32| crate::chrome::folding(t, true).unwrap_or(1.0);
    (0..70).map(|k| k as f32 * 0.005).min_by(|a, b| (f(*a) - v).abs().total_cmp(&(f(*b) - v).abs())).unwrap_or(0.0)
}

/// The note's page: the line as the scene, the caret and the pixels have it.
#[cfg(test)]
mod page;

#[cfg(test)]
mod tests {
    use super::*;

    const CAUGHT: &str = "source: https://kitchen.example/lemon-tart\ncaught: 2026-11-08T15:20";

    #[test]
    fn a_caught_note_shows_its_host_and_day() {
        let s = shown(CAUGHT, 2026, false).unwrap();
        assert_eq!(s.text, "kitchen.example  \u{00B7}  caught 8 Nov");
        assert_eq!(s.link, Some((0, "kitchen.example".len())));
        assert_eq!(s.url.as_deref(), Some("https://kitchen.example/lemon-tart"));
        // Another year's shows its year.
        assert_eq!(shown(CAUGHT, 2027, false).unwrap().text, "kitchen.example  \u{00B7}  caught 8 Nov 2026");
        // www. goes; a port and a login are not the host.
        assert_eq!(host("https://www.Example.org:8080/a?b").unwrap().0, "example.org");
        assert_eq!(host("http://me@site.io/x").unwrap().0, "site.io");
        assert_eq!(host("www.site.io/x").unwrap(), ("site.io".into(), "https://www.site.io/x".into()));
        assert!(host("ftp://x.io").is_none() && host("A book").is_none());
        // What a browser opens, not what the address seems to say.
        assert_eq!(host("https://evil.example\\@bank.example/").unwrap().0, "evil.example");
        let spoof = shown("source: https://evil.example\\@bank.example/", 2026, false).unwrap();
        assert!(spoof.text == "evil.example" && spoof.url.is_some(), "{spoof:?}");
        // A host with marks that turn or hide letters is no link.
        for odd in ["https://bank\u{202E}elpmaxe.evil/", "https://ba\u{200B}nk.example/", "https://bank.example\u{2066}/"] {
            assert!(host(odd).is_none(), "{odd:?}");
            let s = shown(&format!("source: {odd}"), 2026, false).unwrap();
            assert!(s.url.is_none() && s.link.is_none(), "{odd:?} shows no link");
        }
        // Only one of the two.
        assert_eq!(shown("caught: 2026-11-08T15:20", 2026, false).unwrap().text, "Caught 8 Nov");
        let only = shown("source: \"https://www.kitchen.example/a\"", 2026, false).unwrap();
        assert_eq!((only.text.as_str(), only.link), ("kitchen.example", Some((0, 15))));
        // A source that is no page reads as written, no link.
        let book = shown("source: A cookbook\ncaught: 2025-01-02", 2026, false).unwrap();
        assert_eq!((book.text.as_str(), book.url), ("A cookbook  \u{00B7}  caught 2 Jan 2025", None));
    }

    #[test]
    fn obsidian_properties_show_their_values() {
        let yaml = "tags:\n  - recipe\n  - lemon\naliases: [Lemon tart]\npublish: true";
        assert_eq!(shown(yaml, 2026, false).unwrap().text, "#recipe #lemon \u{00B7} Lemon tart");
        assert_eq!(shown("tags: recipe, lemon\ncssclasses: wide", 2026, false).unwrap().text, "#recipe #lemon \u{00B7} wide");
        assert_eq!(shown("tags: [\"#a\", b]", 2026, false).unwrap().text, "#a #b");
        // Nothing worth showing: its keys, quietly.
        let keys = shown("publish: true\ndraft: no", 2026, false).unwrap();
        assert!(keys.keys && keys.text == "publish \u{00B7} draft", "{keys:?}");
        // Nothing at all: as written.
        assert!(shown("", 2026, false).is_none());
    }

    #[test]
    fn a_note_opens_with_its_caret_past_the_frontmatter() {
        let text = format!("---\n{CAUGHT}\n---\n# Lemon tart\n\nBody\n");
        let lines: Vec<Line> = md::lines(&text).collect();
        let body = text.find("Body").unwrap();
        for at in [0, 3, text.find("caught").unwrap()] {
            assert_eq!(rest_caret(&lines, at, body), body, "from {at}");
        }
        let title = text.find("# Lemon").unwrap();
        assert_eq!(rest_caret(&lines, title + 2, body), title + 2, "past it, where it was");
        let plain: Vec<Line> = md::lines("# Hi\nthere").collect();
        assert_eq!(rest_caret(&plain, 0, 5), 0, "no frontmatter: where it was");
    }

    #[test]
    fn a_double_click_on_the_host_opens_the_page_once() {
        assert!(source_opens(1) && !source_opens(2) && !source_opens(3));
    }

    #[test]
    fn right_to_left_each_part_keeps_its_place() {
        let s = shown(CAUGHT, 2026, true).unwrap();
        assert_eq!(s.text, "kitchen.example  \u{200F}\u{00B7}\u{200F}  \u{200E}caught 8 Nov");
        assert_eq!(s.link, Some((0, 15)));
    }

    #[test]
    fn finds_the_block_and_the_caret_in_it() {
        let text = format!("---\n{CAUGHT}\n---\n# Lemon tart\n");
        let lines: Vec<Line> = md::lines(&text).collect();
        assert_eq!(block(&lines), Some(3));
        assert_eq!(yaml(&text, &lines), Some(CAUGHT));
        let title = text.find("# Lemon").unwrap();
        assert!(caret_in(&lines, 0) && caret_in(&lines, title - 1) && !caret_in(&lines, title));
        assert_eq!(source_url(&text).as_deref(), Some("https://kitchen.example/lemon-tart"));
        let plain: Vec<Line> = md::lines("# Hi\n---\n").collect();
        assert!(block(&plain).is_none() && !caret_in(&plain, 0));
    }

    #[test]
    fn opens_on_a_spring_and_snaps_with_motion_off() {
        let m = Motion::default();
        assert_eq!(m.frame(1, false, true), None, "at rest");
        let t = m.frame(1, true, true).expect("opening");
        assert!(t < 0.2 && m.moving(), "{t}");
        // Another note: at rest at once.
        assert_eq!(m.frame(2, false, true), None);
        assert!(!m.moving());
        // Motion off: never moves.
        let still = Motion::default();
        still.frame(1, false, false);
        assert_eq!(still.frame(1, true, false), None);
        assert!(!still.moving());
    }
}
