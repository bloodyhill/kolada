//! The social preview: the open note as a post on LinkedIn, Instagram,
//! Facebook or X, live as it is written. It shows what the feed shows,
//! where the feed cuts the caption for "see more" (and, below a fold, what
//! it hides), how long the caption is by the platform's count and the most
//! it may be. The platforms' logos are drawn from their outlines (see
//! `vector`), in one ink.

use notsy_core::social::{self, PLATFORMS, Platform};
use notsy_text::{Align, Format, Layout, Role, Style as Look};
use std::rc::Rc;
use windows_core::Result;

use crate::note::fit_picture;
use crate::scene::{self, Eng, Fill, Fonts, Ink, Input, Picture, Rect, Scene, Target, Text, VectorMark};
use crate::vector::Logo;

/// The preview's width, as a phone shows a post (narrower in a narrow
/// window), its tab strip's height, its padding, its footer's height.
pub const CARD_WIDTH: f32 = 400.0;
const TABS: f32 = 48.0;
/// Each platform's tab across the top, and its mark in it, DIPs.
const TAB: f32 = 38.0;
const TAB_MARK: f32 = 18.0;
const PAD: f32 = 18.0;
const FOOTER: f32 = 76.0;
/// The least of the card that stays in the window when it is dragged:
/// its tabs, some of the post, its numbers.
const KEEP: f32 = 260.0;

/// What the preview shows: the platform, how far it is scrolled, and how
/// far it was dragged from its place (DIPs).
#[derive(Clone, Copy, Debug)]
pub struct SocialView {
    pub platform: Platform,
    pub scroll: f32,
    pub offset: (f32, f32),
}

/// The room the card has in a window `width` x `height`: at the right,
/// below the window buttons, moved by `offset`, and kept inside the window.
/// It is as tall as its post, up to this.
pub fn card_bounds(width: f32, height: f32, offset: (f32, f32)) -> Rect {
    let w = CARD_WIDTH.min(width - 32.0);
    let home = (width - w - 16.0).max(16.0);
    let left = (home + offset.0).clamp(16.0, home);
    let top = (48.0 + offset.1).clamp(48.0, (height - 16.0 - KEEP).max(48.0));
    Rect::new(left, top, left + w, height - 16.0)
}

fn logo(platform: Platform) -> Logo {
    match platform {
        Platform::LinkedIn => Logo::LinkedIn,
        Platform::Instagram => Logo::Instagram,
        Platform::Facebook => Logo::Facebook,
        Platform::X => Logo::X,
        Platform::YouTube => Logo::YouTube,
    }
}

/// A platform's logo filling the square `r`, in `ink`.
pub fn mark(scene: &mut Scene, platform: Platform, r: Rect, ink: Ink) {
    let s = r.width().min(r.height());
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let rect = Rect::new(cx - s / 2.0, cy - s / 2.0, cx + s / 2.0, cy + s / 2.0);
    scene.vectors.push(VectorMark { layer: scene.layer_now(), rect, logo: logo(platform), ink, alpha: 1.0 });
}

fn centered(eng: &Eng, text: &str, fonts: &Fonts, size: f32, bold: bool, r: Rect) -> Result<Rc<Layout>> {
    let format = Format { align: Align::Center, size, weight: if bold { 700 } else { fonts.label.weight }, ..fonts.label };
    Ok(scene::layout(eng, text, &format, r.width(), r.height()))
}

fn text_height(l: &Layout) -> Result<f32> {
    Ok(l.measure().height)
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A caption as the feed shows it, `width` wide: what shows (its first
/// `lead` bytes bold, as Instagram puts who posted), with the feed's "see
/// more" ending its last line; and what it hides, if anything: a label for
/// the fold, the text, and its ink.
struct Folded {
    shown: Rc<Layout>,
    hidden: Option<(String, Rc<Layout>, Ink)>,
    /// How many of the caption's characters show before the fold.
    cut: Option<usize>,
}

fn fold(eng: &Eng, fonts: &Fonts, platform: Platform, caption: &str, lead: usize, width: f32) -> Result<Folded> {
    let layout = |text: &str, looks: &[Look]| {
        Rc::new(scene::layout_styled(eng, text, &Format { rtl: Some(scene::is_rtl(text)), ..fonts.peek }, width, 100_000.0, looks))
    };
    // Its first `lead` bytes semibold, as Instagram puts who posted.
    let bold_lead = |text: &str| -> Vec<Look> {
        if lead > 0 { vec![Look { range: 0..lead.min(text.len()), weight: Some(600), ..Look::default() }] } else { Vec::new() }
    };
    // X shows it all, but not past its limit.
    if platform.fold().is_none() {
        let over = social::over_at(platform, &caption[lead..]).map(|at| lead + at);
        let shown_text = &caption[..over.unwrap_or(caption.len())];
        let shown = layout(shown_text.trim_end(), &bold_lead(shown_text));
        let hidden = match over {
            Some(at) => Some((format!("Past the {} characters {} allows", platform.limit(), platform.name()), layout(caption[at..].trim(), &[]), Ink::Danger)),
            None => None,
        };
        return Ok(Folded { shown, hidden, cut: None });
    }
    let (_, most_lines) = platform.fold().unwrap_or((usize::MAX, usize::MAX));
    let whole = layout(caption, &bold_lead(caption));
    let by_chars = social::fold_at(platform, &caption[lead..]).map(|at| lead + at);
    let lines = whole.lines();
    if by_chars.is_none() && lines.len() <= most_lines {
        return Ok(Folded { shown: whole, hidden: None, cut: None });
    }
    // Where the feed stops: its character count or its lines, whichever
    // comes first; then back a word at a time until the text and "see
    // more" end within those lines, as the feeds cut.
    let by_lines = lines.iter().take(most_lines).last().map_or(caption.len(), |l| l.end);
    let mut end = by_chars.unwrap_or(caption.len()).min(by_lines);
    let more = platform.more();
    let shown = loop {
        let text = format!("{}{more}", caption[..end].trim_end());
        let at = text.len() - more.len();
        let mut looks = bold_lead(&text);
        looks.push(Look { range: at..text.len(), weight: Some(600), ink: Ink::Link.id(), ..Look::default() });
        let l = layout(&text, &looks);
        if l.lines().len() <= most_lines || end <= lead {
            break l;
        }
        end = caption[..end].trim_end().rfind(char::is_whitespace).unwrap_or(0).max(lead);
    };
    let rest = caption[end..].trim();
    let label = format!("Hidden until \u{201C}{}\u{201D}", more.trim_start_matches('\u{2026}').trim());
    let hidden = (!rest.is_empty()).then(|| (label, layout(rest, &[]), Ink::Faint));
    Ok(Folded { shown, hidden, cut: Some(caption[lead..end.max(lead)].trim_end().chars().count()) })
}

/// The preview card, over the page, when it is open.
pub fn card(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let Some(view) = input.social else { return Ok(()) };
    let platform = view.platform;
    let caption = social::caption(input.text);
    // The post first, to know how tall the card is: as tall as it, up to
    // the window's bottom (and then it scrolls).
    let bounds = card_bounds(width, height, view.offset);
    let mut post_scene = Scene::default();
    let (content, cut) = post(eng, fonts, input, platform, &caption, bounds.left, bounds.right, &mut post_scene)?;
    let body_h = content.min(bounds.height() - TABS - 1.0 - FOOTER).max(0.0);
    let card = Rect::new(bounds.left, bounds.top, bounds.right, bounds.top + TABS + 1.0 + body_h + FOOTER);
    scene.overlay(None);
    scene.fills.push(Fill::shadow(card.offset(0.0, 8.0), 14.0, 26.0, 1.6));
    scene.fills.push(Fill::solid(card, 14.0, Ink::Border));
    scene.fills.push(Fill::solid(card.inset(1.0), 13.0, Ink::Panel));
    // It moves by its tab strip, or anywhere on it that is not a button.
    scene.hits.push((card, Target::SocialCard));
    scene.hits.push((Rect::new(card.left, card.top, card.right, card.top + TABS), Target::SocialHeader));

    // The platforms, one click apart, and a way out.
    let mut x = card.left + 12.0;
    for p in PLATFORMS {
        let tab = Rect::new(x, card.top + 6.0, x + TAB, card.top + TABS - 4.0);
        let on = p == platform;
        let hover = input.hover == Some(Target::SocialTab(p));
        if on {
            scene.fills.push(Fill::solid(Rect::new(tab.left + 11.0, tab.bottom - 2.0, tab.right - 11.0, tab.bottom), 1.0, Ink::Accent));
        }
        if hover && !on {
            scene.fills.push(Fill::solid(tab.inset(2.0), 8.0, Ink::Hover));
        }
        let ink = if on { Ink::Text } else if hover { Ink::Muted } else { Ink::Faint };
        let (mid, half) = (((tab.left + tab.right) / 2.0, tab.top + 17.0), TAB_MARK / 2.0);
        mark(scene, p, Rect::new(mid.0 - half, mid.1 - half, mid.0 + half, mid.1 + half), ink);
        scene.hits.push((tab, Target::SocialTab(p)));
        x += TAB;
    }
    let close = Rect::new(card.right - 44.0, card.top + 8.0, card.right - 12.0, card.top + 40.0);
    if input.hover == Some(Target::SocialClose) {
        scene.fills.push(Fill::solid(close, 8.0, Ink::Hover));
    }
    let x_icon = scene::layout(eng, "\u{E8BB}", &fonts.icon_small, close.width(), close.height());
    scene.texts.push(Text { x: close.left, y: close.top, layout: x_icon, ink: Ink::Muted, alpha: 1.0 });
    scene.hits.push((close, Target::SocialClose));
    scene.fills.push(Fill::solid(Rect::new(card.left + 1.0, card.top + TABS, card.right - 1.0, card.top + TABS + 1.0), 0.0, Ink::Border));

    // The numbers, at the bottom: length against the limit, where the
    // feed folds it, hashtags.
    let length = social::length(platform, &caption);
    let limit = platform.limit();
    let foot = Rect::new(card.left + PAD, card.bottom - FOOTER, card.right - PAD, card.bottom - 10.0);
    scene.fills.push(Fill::solid(Rect::new(card.left + 1.0, foot.top - 1.0, card.right - 1.0, foot.top), 0.0, Ink::Border));
    let over = length > limit;
    let bar = Rect::new(foot.left, foot.top + 12.0, foot.right, foot.top + 15.0);
    scene.fills.push(Fill::solid(bar, 1.5, Ink::Field));
    let filled = (length as f32 / limit as f32).min(1.0);
    scene.fills.push(Fill::solid(Rect::new(bar.left, bar.top, bar.left + bar.width() * filled, bar.bottom), 1.5, if over { Ink::Danger } else { Ink::Accent }));
    let count = format!("{} / {} characters{}", thousands(length), thousands(limit), if over { format!(" \u{00B7} {} over", thousands(length - limit)) } else { String::new() });
    // Where it folds as drawn: by characters or by lines, at a word.
    let folds = match (platform.fold(), cut) {
        (Some((chars, lines)), Some(shown)) => {
            format!("\u{201C}{}\u{201D} after {shown} characters (it shows {chars}, or {lines} lines)", platform.more().trim_start_matches('\u{2026}').trim())
        }
        (Some((chars, lines)), None) => format!("Shows in full (the feed folds after {chars} characters or {lines} lines)"),
        (None, _) => "Shows in full, up to the limit".to_string(),
    };
    let tags = social::hashtags(&caption);
    let tags = match platform.hashtag_limit() {
        Some(most) => format!("{tags} / {most} hashtags"),
        None => format!("{tags} hashtag{}", if tags == 1 { "" } else { "s" }),
    };
    let mut y = bar.bottom + 6.0;
    for (text, ink) in [(count, if over { Ink::Danger } else { Ink::Muted }), (folds, Ink::Faint), (tags, Ink::Faint)] {
        let l = scene::single_line(eng, &text, &fonts.small, foot.width(), 18.0);
        scene.texts.push(Text { x: foot.left, y, layout: l, ink, alpha: 1.0 });
        y += 17.0;
    }

    // The post, scrolled between the tabs and the numbers.
    let body = Rect::new(card.left + 1.0, card.top + TABS + 1.0, card.right - 1.0, card.top + TABS + 1.0 + body_h);
    let max = (content - body_h).max(0.0);
    scene.social_scroll_max = max;
    let dy = body.top - view.scroll.clamp(0.0, max);
    scene.overlay(Some(body));
    let layer = scene.layer_now();
    scene.fills.extend(post_scene.fills.into_iter().map(|f| Fill { rect: f.rect.offset(0.0, dy), ..f }));
    scene.texts.extend(post_scene.texts.into_iter().map(|t| Text { y: t.y + dy, ..t }));
    scene.pictures.extend(post_scene.pictures.into_iter().map(|p| Picture { layer, rect: p.rect.offset(0.0, dy), ..p }));
    scene.vectors.extend(post_scene.vectors.into_iter().map(|v| VectorMark { layer, rect: v.rect.offset(0.0, dy), ..v }));
    Ok(())
}

/// Who posts: the Windows user's name, as the preview's author.
fn author() -> String {
    std::env::var("USERNAME").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "You".to_string())
}

/// The post as the platform shows it, from its top: who, the caption
/// folded as the feed folds it, the note's first picture, what a reader
/// can do. Its height, and how many characters show before the fold.
#[allow(clippy::too_many_arguments)]
fn post(eng: &Eng, fonts: &Fonts, input: &Input, platform: Platform, caption: &str, left0: f32, right: f32, scene: &mut Scene) -> Result<(f32, Option<usize>)> {
    let mut cut = None;
    let inner = (right - left0) - 2.0 * PAD;
    let mut y = PAD;
    let left = left0 + PAD;
    let name = author();
    // Its author: a round picture, a name, when.
    let avatar = if platform == Platform::LinkedIn { 44.0 } else { 38.0 };
    let face = Rect::new(left, y, left + avatar, y + avatar);
    scene.fills.push(Fill::solid(face, avatar / 2.0, Ink::Field));
    let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let l = centered(eng, &initial, fonts, avatar * 0.42, true, face)?;
    scene.texts.push(Text { x: face.left, y: face.top, layout: l, ink: Ink::Muted, alpha: 1.0 });
    let handle = name.to_lowercase().replace(' ', "");
    let (who, meta) = match platform {
        Platform::LinkedIn => (name.clone(), "Your headline\nnow \u{00B7} \u{1F310}".to_string()),
        Platform::Instagram => (handle.clone(), "Original audio".to_string()),
        Platform::Facebook => (name.clone(), "Just now \u{00B7} \u{1F310}".to_string()),
        Platform::X => (format!("{name}  @{handle} \u{00B7} now"), String::new()),
        Platform::YouTube => (format!("@{handle} \u{00B7} now"), String::new()),
    };
    let tx = face.right + 10.0;
    let who_l = scene::single_line(eng, &who, &fonts.peek_title, right - PAD - tx, 20.0);
    scene.texts.push(Text { x: tx, y: y + 1.0, layout: who_l, ink: Ink::Text, alpha: 1.0 });
    if !meta.is_empty() {
        let meta_l = scene::layout(eng, &meta, &fonts.small, right - PAD - tx, 40.0);
        scene.texts.push(Text { x: tx, y: y + 21.0, layout: meta_l, ink: Ink::Faint, alpha: 1.0 });
    }
    y += avatar + 14.0;

    // The note's first picture, as the post's: edge to edge, or (X) inset
    // and rounded.
    let picture = first_picture(input);
    let draw_picture = |scene: &mut Scene, y: &mut f32, full: bool| {
        if let Some((path, w, h)) = &picture {
            let x0 = if full { left0 + 1.0 } else { left };
            let pw = if full { (right - left0) - 2.0 } else { inner };
            let (pw, ph) = fit_picture(*w, *h, pw, Some(pw));
            let ph = ph.min(pw * 1.25);
            let r = Rect::new(x0, *y, x0 + pw, *y + ph);
            let round = if full { 0.0 } else { 14.0 };
            let ready = input.images.and_then(|images| images.borrow_mut().pixels(path, (pw * input.scale.max(1.0)).round() as u32));
            match ready {
                Some((pixels, alpha)) => scene.pictures.push(Picture { layer: scene.layer_now(), rect: r, pixels, alpha, round }),
                None => scene.fills.push(Fill::solid(r, round, Ink::Field)),
            }
            *y += ph + 12.0;
        }
    };
    // Instagram leads with the picture, then its row of actions.
    if platform == Platform::Instagram {
        draw_picture(scene, &mut y, true);
        if picture.is_none() {
            let r = Rect::new(left0 + 1.0, y, right - 1.0, y + inner);
            scene.fills.push(Fill::solid(r, 0.0, Ink::Field));
            let l = centered(eng, "A post needs a picture", fonts, 12.0, false, r)?;
            scene.texts.push(Text { x: r.left, y: r.top, layout: l, ink: Ink::Faint, alpha: 1.0 });
            y += inner + 12.0;
        }
        y = actions(eng, fonts, scene, platform, left, y, inner)?;
    }
    if caption.is_empty() {
        let l = scene::layout(eng, "Write the note to see it here.", &fonts.peek, inner, 40.0);
        scene.texts.push(Text { x: left, y, layout: l, ink: Ink::Faint, alpha: 1.0 });
        y += 24.0;
    } else {
        // Instagram's caption starts with who posted it.
        let lead = if platform == Platform::Instagram { format!("{handle} ") } else { String::new() };
        let folded = fold(eng, fonts, platform, &format!("{lead}{caption}"), lead.len(), inner)?;
        cut = folded.cut;
        let h = text_height(&folded.shown)?;
        scene.texts.push(Text { x: left, y, layout: folded.shown, ink: Ink::Text, alpha: 1.0 });
        y += h + 12.0;
        // Below a fold: what the feed keeps until it is opened.
        if let Some((label, hidden, ink)) = folded.hidden {
            let tag = scene::layout(eng, &label, &fonts.small, inner, 18.0);
            let (tw, mid) = (tag.measure().width, y + 9.0);
            let gap = 8.0;
            let lx = left + (inner - tw) / 2.0;
            let dashes = |scene: &mut Scene, from: f32, to: f32| {
                let mut x = from;
                while x + 4.0 <= to {
                    scene.fills.push(Fill::faded(Rect::new(x, mid - 0.5, x + 4.0, mid + 0.5), 0.5, Ink::Muted, 0.7));
                    x += 7.0;
                }
            };
            dashes(scene, left, lx - gap);
            dashes(scene, lx + tw + gap, left + inner);
            scene.texts.push(Text { x: lx, y: mid - 9.0, layout: tag, ink: Ink::Faint, alpha: 1.0 });
            y += 26.0;
            let h = text_height(&hidden)?;
            scene.texts.push(Text { x: left, y, layout: hidden, ink, alpha: if ink == Ink::Danger { 0.85 } else { 1.0 } });
            y += h + 12.0;
        }
    }
    if platform != Platform::Instagram {
        draw_picture(scene, &mut y, !matches!(platform, Platform::X | Platform::YouTube));
        y = actions(eng, fonts, scene, platform, left, y, inner)?;
    }
    Ok((y + PAD - 12.0, cut))
}

/// The row of what a reader can do with the post, as the platform has it.
fn actions(eng: &Eng, fonts: &Fonts, scene: &mut Scene, platform: Platform, left: f32, y: f32, width: f32) -> Result<f32> {
    let items: &[(&str, &str)] = match platform {
        Platform::LinkedIn => &[("\u{E8E1}", "Like"), ("\u{E90A}", "Comment"), ("\u{E8EE}", "Repost"), ("\u{E724}", "Send")],
        Platform::Instagram => &[("\u{EB51}", ""), ("\u{E90A}", ""), ("\u{E724}", "")],
        Platform::Facebook => &[("\u{E8E1}", "Like"), ("\u{E90A}", "Comment"), ("\u{E72D}", "Share")],
        Platform::X => &[("\u{E90A}", ""), ("\u{E8EE}", ""), ("\u{EB51}", ""), ("\u{E9D9}", "")],
        Platform::YouTube => &[("\u{E8E1}", ""), ("\u{E90A}", ""), ("\u{E72D}", "Share")],
    };
    scene.fills.push(Fill::solid(Rect::new(left, y, left + width, y + 1.0), 0.0, Ink::Border));
    let slot = if platform == Platform::Instagram { 40.0 } else { width / items.len() as f32 };
    for (i, (icon, label)) in items.iter().enumerate() {
        let x = left + slot * i as f32;
        let text = if label.is_empty() { icon.to_string() } else { format!("{icon}  {label}") };
        // The icon leads the label, in the icon font (a private-use glyph is
        // three bytes of UTF-8).
        let mut mark = Look::new(0..icon.len());
        mark.role = Some(Role::Icon);
        mark.size = Some(14.0);
        let align = if platform == Platform::Instagram { Align::Start } else { Align::Center };
        let format = Format { wrap: false, ellipsis: true, center_y: true, align, rtl: Some(false), ..fonts.small };
        let l = Rc::new(scene::layout_styled(eng, &text, &format, slot, 36.0, &[mark]));
        scene.texts.push(Text { x, y: y + 4.0, layout: l, ink: Ink::Muted, alpha: 1.0 });
    }
    Ok(y + 44.0)
}

/// The note's first picture, found: its file and size in pixels.
fn first_picture(input: &Input) -> Option<(std::path::PathBuf, u32, u32)> {
    let images = input.images?;
    let name = input.text.lines().find_map(crate::images::embed_in)?;
    let vault = std::path::Path::new(input.vault_path);
    let dir = vault.join(input.active_note).parent().map_or_else(|| vault.to_path_buf(), std::path::Path::to_path_buf);
    match images.borrow_mut().place(vault, &dir, name) {
        crate::images::Lookup::Found(path, w, h) => Some((path, w, h)),
        _ => None,
    }
}
