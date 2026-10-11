//! The writing model at work on the open note, and on nothing else. After a
//! pause, the paragraphs written or changed near the caret go to it (never
//! code, tables or frontmatter, nor anything in the note open over the
//! page); a fix it is sure of goes in on its own, one Ctrl+Z takes
//! it back whole, and a paragraph put back so is left alone. Marks typed by
//! hand become a list's once the caret leaves the line, and a list written
//! in a run becomes one too. A long note gets a line saying what it is
//! about, beside its dates. Every request carries the open note's
//! generation, so nothing asked about one note lands in another.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, channel};

use notsy_assist::mask::{self, Masked};
use notsy_assist::text::{self as words, Script};
use notsy_assist::wire::{Kind, Reply, Request, Status};
use notsy_assist::{gist, guard, shape};

use super::*;
use crate::assist::{Event, Helper, NO_MODEL};
use crate::gists::{self, Gist, Gists};
use crate::spell::line_key;

/// A pause this long in writing sends what was written near the caret.
pub(super) const ASSIST_TIMER: usize = 45;
const PAUSE_MS: u32 = 900;
/// A note written in gets its gist this long after the last edit.
pub(super) const GIST_TIMER: usize = 46;
const GIST_MS: u32 = 20_000;
/// The helper starts this long after the first frame; after a crash, again
/// after the first wait, then the second, and then no more.
pub(super) const ASSIST_START: usize = 47;
const START_MS: u32 = 1_500;
const RESTART_MS: [u32; 2] = [5_000, 30_000];
/// Paragraphs sent at most after a pause, and how many lines from the
/// caret's they are looked for.
const PER_PAUSE: usize = 8;
const NEAR: usize = 40;
/// Unanswered this long, a request is forgotten (its answer may have been
/// lost while the app was busy) and asked again; after a pause that left
/// some unanswered, the timer comes back this late to do so.
const FORGOTTEN: Duration = Duration::from_secs(15);
const RETRY_MS: u32 = 15_500;
/// The title and the paragraph before, as sent with a paragraph, bytes.
const CONTEXT: usize = 300;
/// Line keys kept at most before those of lines no longer in the note go.
const KEEP: usize = 4000;
/// Lines looked through, each way, for a paragraph asked about that moved.
const SEEK: usize = 200;
/// What the model changed washes in the accent, fading over this long.
const WASH: Duration = Duration::from_millis(650);
const WASH_ALPHA: f32 = 0.28;
/// The gist fades out and back in over this long.
const GIST_FADE: Duration = Duration::from_millis(180);

/// A paragraph asked about, until its answer comes: its line's key and
/// start then, its text after the block mark, and what was sent with it.
#[derive(Clone)]
struct Asked {
    id: u64,
    kind: Kind,
    key: u64,
    slot: usize,
    content: String,
    masked: Option<Masked>,
    context: String,
    at: Instant,
}

/// Whether `edits` put in words of the title sent as context (`context`'s
/// part before the dash) that the paragraph does not have: the model
/// taking it for the paragraph's start, not a fix.
fn echoes_title(content: &str, context: &str, edits: &[guard::Edit]) -> bool {
    let title = context.split(" \u{2014} ").next().unwrap_or("").trim().to_lowercase();
    !title.is_empty() && !content.to_lowercase().contains(&title) && edits.iter().any(|e| e.text.to_lowercase().contains(&title))
}

/// Text just changed, since when, and the text version it is for.
struct Wash {
    ranges: Vec<(usize, usize)>,
    since: Instant,
    version: u64,
}

pub(super) struct Writing {
    helper: Option<Helper>,
    /// Restarts after a crash so far; off for the rest of the session (its
    /// files are missing, its model would not load, or it kept failing).
    restarts: usize,
    dead: bool,
    /// The open note's generation.
    note: u64,
    next_id: u64,
    /// Line keys: in the note as it opened, answered (fixed, or right as
    /// they were), and fixes undone (left alone until the line changes).
    untouched: HashSet<u64>,
    done: HashSet<u64>,
    declined: HashSet<u64>,
    asked: Vec<Asked>,
    /// The last pause left paragraphs unsent; the timer is waiting to ask
    /// again what went unanswered.
    more: bool,
    retrying: bool,
    /// Lists found in the paragraph the caret is in, by its key: they go in
    /// when the caret leaves it.
    held: Vec<(u64, String)>,
    /// Where the caret's line started in the last frame.
    caret_line: Option<usize>,
    wash: Option<Wash>,
    whispered: bool,
    gists: Option<Gists>,
    gists_in: Option<Receiver<Gists>>,
    /// The gist being made: its request, and the words the note had.
    gist_asked: Option<(u64, usize)>,
    /// Whether the gist shows (not while typing), and since when it fades.
    gist_shown: (bool, Option<Instant>),
}

impl Writing {
    pub(super) fn new() -> Writing {
        Writing {
            helper: None,
            restarts: 0,
            dead: false,
            note: 0,
            next_id: 0,
            untouched: HashSet::new(),
            done: HashSet::new(),
            declined: HashSet::new(),
            asked: Vec::new(),
            more: false,
            retrying: false,
            held: Vec::new(),
            caret_line: None,
            wash: None,
            whispered: false,
            gists: None,
            gists_in: None,
            gist_asked: None,
            gist_shown: (true, None),
        }
    }
}

/// Prose, as the spell checker has it, and not a table's row.
fn is_prose(text: &str, l: &md::Line) -> bool {
    let content = &text[l.content().min(l.end)..l.end];
    !matches!(l.block, Block::Fence | Block::Code | Block::Frontmatter | Block::Rule)
        && !content.trim_start().starts_with('|')
        && content.chars().any(char::is_alphabetic)
}

/// Enough to fix: three words, or four characters of Chinese or Japanese.
fn enough_words(content: &str) -> bool {
    let cjk = content.chars().filter(|&c| words::script(c) == Some(Script::Cjk)).count();
    words::words(content) - cjk >= 3 || cjk >= 4
}

/// `s` cut to at most `n` bytes, on a character's edge.
fn cap(s: &str, n: usize) -> &str {
    let mut end = n.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Where the sentence at byte `caret` of `content` starts: after the last
/// full stop, question or exclamation mark before it (in any script).
fn sentence_start(content: &str, caret: usize) -> usize {
    let caret = cap(content, caret).len();
    content[..caret].char_indices().rev().find(|&(_, c)| matches!(c, '.' | '!' | '?' | '\u{3002}' | '\u{FF01}' | '\u{FF1F}' | '\u{061F}')).map_or(0, |(i, c)| i + c.len_utf8())
}

impl App {
    fn assist_on(&self) -> bool {
        self.session.assist == 0
    }

    /// A moment after the first frame: the gists are read, and the main
    /// window starts the helper (a note's own window, once it has
    /// something to ask).
    pub(super) fn assist_soon(&mut self) {
        self.start_timer(ASSIST_START, START_MS);
    }

    /// Handles the assistant's timers; false for others.
    pub(super) fn assist_timer(&mut self, id: usize) -> bool {
        match id {
            ASSIST_START => {
                self.stop_timer(ASSIST_START);
                self.read_gists();
                if !self.compact || self.writing.restarts > 0 {
                    self.start_helper();
                }
                self.want_gist(false);
            }
            ASSIST_TIMER => self.pause_round(),
            GIST_TIMER => {
                self.stop_timer(GIST_TIMER);
                self.want_gist(true);
            }
            _ => return false,
        }
        true
    }

    fn read_gists(&mut self) {
        if self.assist_on() && self.writing.gists.is_none() && self.writing.gists_in.is_none() {
            let (to, from) = channel();
            Gists::load(to, self.hwnd);
            self.writing.gists_in = Some(from);
        }
    }

    fn start_helper(&mut self) {
        if self.writing.helper.is_some() || self.writing.dead || !self.assist_on() {
            return;
        }
        self.writing.helper = Helper::start(self.hwnd);
        if self.writing.helper.is_none() {
            self.writing.dead = true;
            return;
        }
        // What was written while it was not there.
        self.start_timer(ASSIST_TIMER, PAUSE_MS);
    }

    /// Ends the helper (closing its input) and forgets what it was asked.
    pub(super) fn stop_helper(&mut self) {
        self.writing.helper = None;
        self.writing.asked.clear();
        self.writing.held.clear();
        self.writing.gist_asked = None;
        for id in [ASSIST_TIMER, GIST_TIMER, ASSIST_START] {
            self.stop_timer(id);
        }
    }

    /// Writing help turned on or off.
    pub(super) fn assist_setting(&mut self) {
        if self.assist_on() {
            // What is in the note now stays as it is.
            self.remember_untouched();
            self.read_gists();
            if !self.compact {
                self.start_helper();
            }
            self.want_gist(false);
        } else {
            self.stop_helper();
            self.writing.wash = None;
            self.writing.untouched.clear();
        }
    }

    /// What the helper and the gists' reader sent since last time.
    pub(super) fn assist_events(&mut self) {
        if let Some(Ok(gists)) = self.writing.gists_in.as_ref().map(Receiver::try_recv) {
            self.writing.gists = Some(gists);
            self.writing.gists_in = None;
            self.want_gist(false);
            self.request_frame();
        }
        while let Some(event) = self.writing.helper.as_ref().and_then(|h| h.events.try_recv().ok()) {
            match event {
                Event::Reply(reply) => self.arrived(reply),
                Event::Ended(code) => self.helper_ended(code),
            }
        }
    }

    fn helper_ended(&mut self, code: Option<u32>) {
        self.stop_helper();
        let again = match code {
            None | Some(NO_MODEL) => None,
            Some(_) => RESTART_MS.get(self.writing.restarts).copied(),
        };
        match again {
            Some(ms) => {
                self.writing.restarts += 1;
                self.start_timer(ASSIST_START, ms);
            }
            None => self.writing.dead = true,
        }
    }

    /// Every line of the note as it is now: left alone.
    pub(super) fn remember_untouched(&mut self) {
        self.writing.untouched.clear();
        if self.assist_on() && !self.read_only {
            let text = self.editor.text();
            self.writing.untouched.extend(self.editor.outline().iter().map(|l| line_key(&text[l.start..l.end])));
        }
    }

    /// Another note is open (or this one again, from its file): what was
    /// asked about the last one is dropped, here and in the helper.
    pub(super) fn note_opened(&mut self) {
        let w = &mut self.writing;
        w.note += 1;
        w.asked.clear();
        w.held.clear();
        w.done.clear();
        w.declined.clear();
        w.wash = None;
        w.gist_asked = None;
        w.caret_line = None;
        w.more = false;
        w.retrying = false;
        if let Some(h) = &w.helper {
            h.send(&Request { kind: Kind::Cancel, id: 0, note: w.note, slot: 0, context: String::new(), text: String::new() });
        }
        self.stop_timer(ASSIST_TIMER);
        self.stop_timer(GIST_TIMER);
        self.remember_untouched();
        self.want_gist(false);
    }

    /// The text changed: after a pause, what changed is looked at.
    pub(super) fn assist_edited(&mut self) {
        if !self.assist_on() || self.read_only {
            return;
        }
        self.writing.retrying = false;
        self.start_timer(ASSIST_TIMER, PAUSE_MS);
        self.start_timer(GIST_TIMER, GIST_MS);
    }

    /// A pause in writing: the paragraphs written near the caret go to the
    /// model.
    fn pause_round(&mut self) {
        self.stop_timer(ASSIST_TIMER);
        self.writing.retrying = false;
        if !self.assist_on() || self.read_only || self.writing.dead {
            return;
        }
        let now = Instant::now();
        self.writing.asked.retain(|a| now.duration_since(a.at) < FORGOTTEN);
        self.prune();
        let (found, more) = self.candidates(now);
        if found.is_empty() {
            return;
        }
        // A note's own window starts its helper once it has something to ask.
        if self.writing.helper.is_none() && self.compact && self.writing.restarts == 0 {
            self.start_helper();
        }
        let w = &mut self.writing;
        let Some(helper) = &w.helper else { return };
        for (mut request, mut asked) in found {
            w.next_id += 1;
            (request.id, asked.id) = (w.next_id, w.next_id);
            if !helper.send(&request) {
                break;
            }
            // A newer request for the same place replaces the older.
            w.asked.retain(|a| !(a.kind == asked.kind && a.slot == asked.slot));
            w.asked.push(asked);
        }
        w.more = more;
        if !w.asked.is_empty() {
            w.retrying = true;
            self.start_timer(ASSIST_TIMER, RETRY_MS);
        }
    }

    /// What to ask about, the caret's line first, then those above it, then
    /// those below; and whether more were left for later.
    fn candidates(&self, now: Instant) -> (Vec<(Request, Asked)>, bool) {
        let (text, lines) = (self.editor.text(), self.editor.outline());
        let mut out = Vec::new();
        if lines.is_empty() {
            return (out, false);
        }
        let head = self.editor.selection().head;
        let here = lines.partition_point(|l| l.start <= head).max(1) - 1;
        let title = crate::note::title_line(text, lines);
        let heading = self.editor.title();
        let heading = cap(&heading, CONTEXT);
        let w = &self.writing;
        let order = std::iter::once(here).chain((here.saturating_sub(NEAR)..here).rev()).chain(here + 1..lines.len().min(here + 1 + NEAR));
        let mut paragraphs = 0;
        for i in order {
            let l = lines[i];
            if l.end > text.len() || !is_prose(text, &l) {
                continue;
            }
            // A title written here is fixed too (the file's name follows, as
            // it follows any edit to it), never made a list.
            let is_title = Some(i) == title;
            let key = line_key(&text[l.start..l.end]);
            if w.untouched.contains(&key) || w.done.contains(&key) || w.declined.contains(&key) || w.asked.iter().any(|a| a.key == key) {
                continue;
            }
            let content = &text[l.content().min(l.end)..l.end];
            if !enough_words(content) {
                continue;
            }
            if paragraphs == PER_PAUSE {
                return (out, true);
            }
            paragraphs += 1;
            // Read, never rewritten: the title and the paragraph before.
            let before = lines[..i]
                .iter()
                .rposition(|p| !text[p.start..p.end].trim().is_empty())
                .filter(|&j| Some(j) != title && is_prose(text, &lines[j]))
                .map(|j| &text[lines[j].content().min(lines[j].end)..lines[j].end]);
            // The title alone is left out: the model takes it for the start
            // of the paragraph.
            let context = before.map_or_else(String::new, |p| format!("{heading} \u{2014} {}", cap(p, CONTEXT)));
            let masked = mask::mask(content);
            let asked = Asked { id: 0, kind: Kind::Fix, key, slot: l.start, content: content.to_string(), masked: Some(masked), context, at: now };
            let request = Request {
                kind: Kind::Fix,
                id: 0,
                note: w.note,
                slot: l.start as u64,
                context: asked.context.clone(),
                text: asked.masked.as_ref().map_or_else(String::new, |m| m.text.clone()),
            };
            // Whether it is a list is asked once the caret is elsewhere.
            let shape = (l.block == Block::Body && !is_title && !(l.start..=l.end).contains(&head) && shape::worth_asking(content)).then(|| {
                let request = Request { kind: Kind::Shape, id: 0, note: w.note, slot: l.start as u64, context: asked.context.clone(), text: content.to_string() };
                (request, Asked { kind: Kind::Shape, masked: None, ..asked.clone() })
            });
            out.push((request, asked));
            out.extend(shape);
        }
        (out, false)
    }

    /// Keeps the sets of keys small: past `KEEP`, those of lines no longer
    /// in the note go.
    fn prune(&mut self) {
        let w = &self.writing;
        if w.untouched.len().max(w.done.len()).max(w.declined.len()) <= KEEP {
            return;
        }
        let text = self.editor.text();
        let here: HashSet<u64> = self.editor.outline().iter().map(|l| line_key(&text[l.start..l.end])).collect();
        let w = &mut self.writing;
        for set in [&mut w.untouched, &mut w.done, &mut w.declined] {
            set.retain(|k| here.contains(k));
        }
    }

    /// The line whose text has `key`, looked for from where `slot` is.
    fn find_line(&self, key: u64, slot: usize) -> Option<md::Line> {
        let (text, lines) = (self.editor.text(), self.editor.outline());
        let at = lines.partition_point(|l| l.start <= slot).max(1) - 1;
        let is = |i: usize| lines.get(i).filter(|l| l.end <= text.len() && line_key(&text[l.start..l.end]) == key).copied();
        (0..SEEK).find_map(|d| is(at + d).or_else(|| at.checked_sub(d).and_then(is)))
    }

    fn key_of_line_at(&self, at: usize) -> u64 {
        let l = self.editor.line_at(at);
        line_key(&self.editor.text()[l.start..l.end])
    }

    fn arrived(&mut self, reply: Reply) {
        if reply.note != self.writing.note {
            return;
        }
        if self.writing.gist_asked.is_some_and(|(id, _)| id == reply.id) {
            return self.gist_arrived(reply);
        }
        let Some(i) = self.writing.asked.iter().position(|a| a.id == reply.id) else { return };
        let asked = self.writing.asked.remove(i);
        match reply.status {
            Status::Cancelled => {}
            Status::Failed => {
                self.writing.done.insert(asked.key);
            }
            Status::Done if asked.kind == Kind::Fix => self.fix_arrived(asked, &reply.text),
            Status::Done => self.shape_arrived(asked, &reply.text),
        }
        if self.writing.asked.is_empty() {
            if std::mem::take(&mut self.writing.more) {
                self.writing.retrying = false;
                self.start_timer(ASSIST_TIMER, PAUSE_MS);
            } else if std::mem::take(&mut self.writing.retrying) {
                self.stop_timer(ASSIST_TIMER);
            }
        }
    }

    fn fix_arrived(&mut self, asked: Asked, answer: &str) {
        let Some(l) = self.find_line(asked.key, asked.slot) else { return };
        let from = l.content().min(l.end);
        let content = &self.editor.text()[from..l.end];
        let Some(masked) = asked.masked.as_ref().filter(|_| content == asked.content) else { return };
        let fits = |e: &guard::Edit| e.start <= e.end && e.end <= content.len() && content.is_char_boundary(e.start) && content.is_char_boundary(e.end);
        // The words the spell checker marked in it, in its text's offsets.
        let marks = self.spell.as_ref().and_then(|s| s.marks.get(&asked.key));
        let misspelled: Vec<(usize, usize)> =
            marks.into_iter().flatten().filter(|&&(a, _)| a as usize >= l.mark).map(|&(a, b)| (a as usize - l.mark, b as usize - l.mark)).collect();
        let Some(edits) = guard::fix_spelled(content, masked, answer, &misspelled).filter(|edits| edits.iter().all(fits) && !echoes_title(content, &asked.context, edits)) else {
            // Not a fix it is sure of: this text is not asked about again.
            self.writing.done.insert(asked.key);
            return;
        };
        // In the paragraph being written, only what is before the sentence
        // at the caret goes in; the rest waits for it to be finished.
        let head = self.editor.selection().head;
        let cut = if (l.start..=l.end).contains(&head) { sentence_start(content, head.saturating_sub(from)) } else { content.len() };
        let all = edits.len();
        let edits: Vec<(usize, usize, String)> = edits.into_iter().filter(|e| e.end <= cut).map(|e| (from + e.start, from + e.end, e.text)).collect();
        let whole = edits.len() == all;
        if edits.is_empty() {
            if whole {
                self.writing.done.insert(asked.key);
            }
            return;
        }
        if self.amend(edits).is_none() {
            return;
        }
        let key = self.key_of_line_at(l.start);
        if whole {
            self.writing.done.insert(key);
        }
        // A list asked for before the fix is for the fixed text now.
        let line = self.editor.line_at(l.start);
        let now = self.editor.text()[line.content().min(line.end)..line.end].to_string();
        for a in self.writing.asked.iter_mut().filter(|a| a.kind == Kind::Shape && a.key == asked.key) {
            (a.key, a.content) = (key, now.clone());
        }
    }

    fn shape_arrived(&mut self, asked: Asked, answer: &str) {
        let Some(l) = self.find_line(asked.key, asked.slot).filter(|l| l.block == Block::Body) else { return };
        let content = &self.editor.text()[l.content().min(l.end)..l.end];
        if content != asked.content {
            return;
        }
        let Some(list) = shape::list(content, answer) else { return };
        let head = self.editor.selection().head;
        if (l.start..=l.end).contains(&head) {
            let held = &mut self.writing.held;
            held.retain(|(k, _)| *k != asked.key);
            if held.len() == 16 {
                held.remove(0);
            }
            held.push((asked.key, list));
            return;
        }
        self.shape_line(l, list);
    }

    /// Puts `list` in place of the paragraph at `l`; its items are done.
    fn shape_line(&mut self, l: md::Line, list: String) {
        let from = l.content().min(l.end);
        let end = from + list.len();
        if self.amend(vec![(from, l.end, list)]).is_none() {
            return;
        }
        let text = self.editor.text();
        let lines = self.editor.outline();
        let first = lines.partition_point(|p| p.end < l.start);
        let keys: Vec<u64> = lines[first..].iter().take_while(|p| p.start <= end).map(|p| line_key(&text[p.start..p.end])).collect();
        self.writing.done.extend(keys);
    }

    /// Changes `edits` (ranges of the current text) as one step Ctrl+Z
    /// takes back, washed in the accent for a moment. The view stays where
    /// it is, whatever changed above it. None if nothing changed.
    fn amend(&mut self, edits: Vec<(usize, usize, String)>) -> Option<Vec<(usize, usize)>> {
        let top = self.session.scroll_line;
        let shift: i64 = edits.iter().filter(|(_, e, _)| *e as u64 <= top).map(|(s, e, t)| t.len() as i64 - (e - s) as i64).sum();
        self.session.scroll_line = (top as i64 + shift).max(0) as u64;
        let reveal = self.reveal_caret;
        let mut ranges = Vec::new();
        self.edit(|e| ranges = e.amend(&edits));
        self.reveal_caret = reveal;
        if ranges.is_empty() {
            self.session.scroll_line = top;
            return None;
        }
        self.writing.wash = Some(Wash { ranges: ranges.clone(), since: Instant::now(), version: self.editor.version() });
        if !std::mem::replace(&mut self.writing.whispered, true) {
            self.whisper("Fixed \u{00B7} Ctrl+Z undoes".to_string());
        }
        Some(ranges)
    }

    /// After Ctrl+Z: a fix taken back leaves its paragraph alone from now on.
    pub(super) fn assist_undone(&mut self) {
        let Some((a, b)) = self.editor.undone_amend() else { return };
        let text = self.editor.text();
        let lines = self.editor.outline();
        let first = lines.partition_point(|l| l.end < a);
        let keys: Vec<u64> = lines[first..].iter().take_while(|l| l.start <= b).map(|l| line_key(&text[l.start..l.end])).collect();
        self.writing.declined.extend(keys);
        self.writing.wash = None;
    }

    /// Before a frame: if the caret left a line since the last one, a list
    /// found in it goes in, or the list marks typed by hand are tidied; and
    /// fixes that waited for its sentence to end are asked for again.
    pub(super) fn assist_caret(&mut self) {
        if !self.assist_on() || self.read_only {
            return;
        }
        let start = self.editor.line_at(self.editor.selection().head).start;
        let Some(was) = self.writing.caret_line.replace(start).filter(|&was| was != start) else { return };
        let lines = self.editor.outline();
        let i = lines.partition_point(|l| l.start < was);
        let Some(&l) = lines.get(i).filter(|l| l.start == was && l.end <= self.editor.text().len()) else { return };
        let text = self.editor.text();
        let is_title = crate::note::title_line(text, lines) == Some(i);
        let key = line_key(&text[l.start..l.end]);
        let w = &self.writing;
        if w.untouched.contains(&key) || w.declined.contains(&key) {
            return;
        }
        let answered = w.done.contains(&key);
        if l.block == Block::Body && !is_title {
            if let Some(k) = w.held.iter().position(|(k, _)| *k == key) {
                let (_, list) = self.writing.held.remove(k);
                return self.shape_line(l, list);
            }
            if let Some(tidy) = shape::rules(&text[l.start..l.end]) {
                if self.amend(vec![(l.start, l.end, tidy)]).is_some() && answered {
                    let key = self.key_of_line_at(l.start);
                    self.writing.done.insert(key);
                }
                return;
            }
        }
        if !answered {
            self.writing.retrying = false;
            self.start_timer(ASSIST_TIMER, PAUSE_MS);
        }
    }

    /// While a wash or the gist fades, frames keep coming.
    pub(super) fn assist_moving(&self) -> bool {
        self.writing.wash.is_some() || self.writing.gist_shown.1.is_some()
    }

    /// Steps the wash and the gist's fade; the wash goes (with a last frame
    /// to clear it) once faded, or once the text changes under it.
    pub(super) fn step_assist(&mut self) {
        if let Some(w) = &self.writing.wash {
            if w.version != self.editor.version() || w.since.elapsed() >= WASH {
                self.writing.wash = None;
            }
            self.dirty = true;
        }
        if let Some(since) = self.writing.gist_shown.1 {
            if since.elapsed() >= GIST_FADE + Duration::from_millis(20) {
                self.writing.gist_shown.1 = None;
            }
            self.dirty = true;
        }
    }

    pub(super) fn wash_view(&self) -> Option<(&[(usize, usize)], f32)> {
        let w = self.writing.wash.as_ref()?;
        let fade = 1.0 - chrome::ease(w.since.elapsed().as_secs_f32() / WASH.as_secs_f32());
        (w.version == self.editor.version()).then_some((w.ranges.as_slice(), WASH_ALPHA * fade))
    }

    /// The open note's gist and how visible it is, if it has one.
    pub(super) fn gist_view(&self) -> Option<(&str, f32)> {
        if !self.assist_on() || self.read_only || self.session.note.is_empty() {
            return None;
        }
        let (shown, since) = self.writing.gist_shown;
        let t = since.map_or(1.0, |s| s.elapsed().as_secs_f32() / GIST_FADE.as_secs_f32());
        let alpha = if shown { chrome::ease(t) } else { 1.0 - chrome::ease(t) };
        if alpha <= 0.01 {
            return None;
        }
        let gist = self.writing.gists.as_ref()?.get(gists::key(&self.note_path()))?;
        Some((gist.text.as_str(), alpha))
    }

    /// A character typed: the gist steps aside.
    pub(super) fn gist_typing(&mut self) {
        if self.writing.gist_shown.0 {
            self.writing.gist_shown = (false, Some(Instant::now()));
        }
    }

    /// Typing paused: the gist comes back.
    pub(super) fn gist_pause(&mut self) {
        if !self.writing.gist_shown.0 {
            self.writing.gist_shown = (true, Some(Instant::now()));
            self.request_frame();
        }
    }

    fn note_words(&self) -> usize {
        match self.words.get() {
            (v, n) if v == self.editor.version() => n,
            _ => {
                let n = count_words(self.editor.text());
                self.words.set((self.editor.version(), n));
                n
            }
        }
    }

    /// Asks for the open note's gist if it is long and has none, or one
    /// from when it read quite differently. `may_start`: a note's own
    /// window may start its helper for it (it was written in).
    fn want_gist(&mut self, may_start: bool) {
        if !self.assist_on() || self.read_only || self.session.note.is_empty() || self.writing.gist_asked.is_some() {
            return;
        }
        let Some(gists) = &self.writing.gists else { return };
        let words = self.note_words();
        if words < gist::MIN_WORDS || gists.get(gists::key(&self.note_path())).is_some_and(|g| !gist::stale(g.words as usize, words)) {
            return;
        }
        if self.writing.helper.is_none() && may_start && self.compact && self.writing.restarts == 0 {
            self.start_helper();
        }
        let Some(helper) = &self.writing.helper else { return };
        let id = self.writing.next_id + 1;
        let request = Request { kind: Kind::Gist, id, note: self.writing.note, slot: 0, context: String::new(), text: gist::source(self.editor.text()) };
        if helper.send(&request) {
            self.writing.next_id = id;
            self.writing.gist_asked = Some((id, words));
        }
    }

    fn gist_arrived(&mut self, reply: Reply) {
        let Some((_, words)) = self.writing.gist_asked.take() else { return };
        if reply.status != Status::Done || self.session.note.is_empty() {
            return;
        }
        let Some(line) = gist::accept(&reply.text, &self.editor.title(), self.editor.text()) else { return };
        let key = gists::key(&self.note_path());
        if let Some(gists) = &mut self.writing.gists {
            gists.set(key, Gist { words: words as u32, text: line });
            self.request_frame();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_counts_as_a_paragraph_to_fix() {
        assert!(enough_words("i has went"));
        assert!(!enough_words("two words"));
        assert!(enough_words("今日は雨"));
        assert!(!enough_words("雨です"));
        let text = "| a | b c d |\n```\ncode here now\n```\nplain words here";
        let lines: Vec<md::Line> = md::lines(text).collect();
        let prose: Vec<bool> = lines.iter().map(|l| is_prose(text, l)).collect();
        assert_eq!(prose, [false, false, false, false, true]);
    }

    #[test]
    fn only_finished_sentences_are_fixed_under_the_caret() {
        let s = "I has went home. then i sees";
        assert_eq!(sentence_start(s, s.len()), "I has went home.".len());
        assert_eq!(sentence_start(s, 5), 0);
        assert_eq!(sentence_start("هل ذهبت؟ نعم", "هل ذهبت؟ نعم".len()), "هل ذهبت؟".len());
        assert_eq!(sentence_start("今日は雨。明日", "今日は雨。明日".len()), "今日は雨。".len());
        assert_eq!(cap("éé", 3), "é");
    }

    #[test]
    fn a_title_taken_for_the_paragraph_is_no_fix() {
        let e = |start, end, text: &str| guard::Edit { start, end, text: text.into() };
        let context = "Plan \u{2014} We moved the launch.";
        assert!(echoes_title("I recieve mail", context, &[e(0, 0, "Plan "), e(2, 9, "receive")]));
        assert!(!echoes_title("I recieve mail", context, &[e(2, 9, "receive")]));
        assert!(!echoes_title("The plan is set", context, &[e(0, 0, "Plan: ")]));
        assert!(!echoes_title("I recieve mail", "", &[e(0, 0, "Plan ")]));
    }
}
