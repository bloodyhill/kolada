//! A note as a social media post: its caption (the note's text as the
//! platforms take it, without Markdown), how long it is by each platform's
//! count, the most it may be, and where the feed cuts it with "see more".

/// The platforms a caption is previewed for, in the order they are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    LinkedIn,
    Instagram,
    Facebook,
    X,
    /// A post on a channel's Community tab.
    YouTube,
}

pub const PLATFORMS: [Platform; 5] = [Platform::LinkedIn, Platform::Instagram, Platform::Facebook, Platform::X, Platform::YouTube];

impl Platform {
    pub fn name(self) -> &'static str {
        match self {
            Platform::LinkedIn => "LinkedIn",
            Platform::Instagram => "Instagram",
            Platform::Facebook => "Facebook",
            Platform::X => "X",
            Platform::YouTube => "YouTube",
        }
    }

    /// The longest a post may be, by the platform's own count.
    pub fn limit(self) -> usize {
        match self {
            Platform::LinkedIn => 3_000,
            Platform::Instagram => 2_200,
            Platform::Facebook => 63_206,
            Platform::X => 280,
            Platform::YouTube => 5_000,
        }
    }

    /// Where the feed stops showing a caption: after this many characters,
    /// or this many lines, whichever comes first (None: it shows it all).
    pub fn fold(self) -> Option<(usize, usize)> {
        match self {
            Platform::LinkedIn => Some((210, 3)),
            Platform::Instagram => Some((125, 2)),
            Platform::Facebook => Some((477, 5)),
            Platform::X => None,
            Platform::YouTube => Some((200, 4)),
        }
    }

    /// What the feed says where it cuts a caption.
    pub fn more(self) -> &'static str {
        match self {
            Platform::LinkedIn => "\u{2026}see more",
            Platform::Instagram => "\u{2026} more",
            Platform::Facebook => "\u{2026} See more",
            Platform::X => "Show more",
            Platform::YouTube => "\u{2026}Read more",
        }
    }

    /// The most hashtags a post may carry, where the platform says.
    pub fn hashtag_limit(self) -> Option<usize> {
        match self {
            Platform::Instagram => Some(30),
            // More, and YouTube takes none of them.
            Platform::YouTube => Some(60),
            _ => None,
        }
    }
}

/// A note's text as a caption: all of it, its title the first line, as
/// plain text, the way the platforms show it: headings, quotes and
/// emphasis marks gone, list items with bullets, links as their words,
/// pictures and comments left out.
pub fn caption(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_front = text.starts_with("---\n") || text.starts_with("---\r\n");
    let mut fence = false;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        if in_front {
            if i > 0 && line == "---" {
                in_front = false;
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            lines.push(line.to_string());
            continue;
        }
        let rest = line.trim_start();
        let is_picture = rest.starts_with("![[") || (rest.starts_with("![") && rest.contains("]("));
        if is_picture && rest.ends_with([']', ')']) {
            continue;
        }
        let (lead, body) = block_lead(rest);
        lines.push(format!("{lead}{}", inline(body)));
    }
    // The title is part of the note, so of the post: its first line.
    lines.join("\n").trim().to_string()
}

/// A line's block mark as a caption shows it, and the rest of the line.
fn block_lead(line: &str) -> (&'static str, &str) {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        return ("", line[hashes..].trim_start());
    }
    if let Some(rest) = line.strip_prefix("> ").or_else(|| line.strip_prefix('>')) {
        return ("", rest);
    }
    for (mark, lead) in [("- [ ] ", "\u{2610} "), ("- [x] ", "\u{2611} "), ("- [X] ", "\u{2611} "), ("- ", "\u{2022} "), ("* ", "\u{2022} "), ("+ ", "\u{2022} ")] {
        if let Some(rest) = line.strip_prefix(mark) {
            return (lead, rest);
        }
    }
    ("", line)
}

/// A line without Markdown's inline marks: emphasis, code, highlights and
/// strikes as their words; [[links]] as their names (or what they say);
/// [words](address) as the words and the address; %%comments%% gone.
fn inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("%%") {
            rest = after.find("%%").map_or("", |i| &after[i + 2..]);
            continue;
        }
        if let Some(after) = rest.strip_prefix("[[")
            && let Some(end) = after.find("]]")
        {
            let inner = &after[..end];
            let shown = inner.rsplit('|').next().unwrap_or(inner);
            out.push_str(shown.split('#').next().unwrap_or(shown));
            rest = &after[end + 2..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find("](")
            && let Some(end) = after[close + 2..].find(')')
        {
            let (words, address) = (&after[..close], &after[close + 2..close + 2 + end]);
            out.push_str(words);
            if !address.is_empty() && address != words {
                out.push(' ');
                out.push_str(address);
            }
            rest = &after[close + 3 + end..];
            continue;
        }
        let mark = ["**", "__", "==", "~~", "*", "_", "`"].into_iter().find(|m| rest.starts_with(m));
        match mark {
            // Underscores inside words (snake_case) are the words'.
            Some(m @ ("_" | "__")) if out.chars().last().is_some_and(char::is_alphanumeric) => {
                out.push_str(m);
                rest = &rest[m.len()..];
            }
            Some(m) => rest = &rest[m.len()..],
            None => {
                let c = rest.chars().next().unwrap_or(' ');
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}

/// A caption's length as `platform` counts it. X weighs characters: most
/// scripts' letters count one, the rest (CJK, emoji) two, and a web
/// address 23 however long; the others count characters.
pub fn length(platform: Platform, caption: &str) -> usize {
    if platform != Platform::X {
        return caption.chars().count();
    }
    let mut n = 0;
    for word in caption.split_inclusive(char::is_whitespace) {
        let trimmed = word.trim_end();
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") || trimmed.starts_with("www.") {
            n += 23 + (word.len() - trimmed.len());
            continue;
        }
        n += word.chars().map(x_weight).sum::<usize>();
    }
    n
}

/// A character's weight in X's count.
fn x_weight(c: char) -> usize {
    match c as u32 {
        // Joiners and variation selectors ride on the emoji before them.
        0x200D | 0xFE00..=0xFE0F => 0,
        0..=4351 | 8192..=8205 | 8208..=8223 | 8242..=8247 => 1,
        _ => 2,
    }
}

/// The byte where `platform`'s feed cuts `caption` for "see more", by its
/// character count alone (lines are counted where it is laid out), if it
/// is longer than that.
pub fn fold_at(platform: Platform, caption: &str) -> Option<usize> {
    let (chars, lines) = platform.fold()?;
    let by_chars = caption.char_indices().nth(chars).map(|(i, _)| i);
    let by_lines = caption.match_indices('\n').nth(lines - 1).map(|(i, _)| i);
    match (by_chars, by_lines) {
        // At a line's end.
        (Some(a), Some(b)) if b <= a => Some(caption[..b].trim_end().len()),
        (None, Some(b)) => Some(caption[..b].trim_end().len()),
        // In a line: at a word's end, as the feeds cut.
        (Some(a), _) => {
            let word = caption[..a].rfind(char::is_whitespace).filter(|&w| w + 40 > a && w > 0).unwrap_or(a);
            Some(caption[..word].trim_end().len())
        }
        (None, None) => None,
    }
}

/// Where X's count passes 280: the byte from which a caption is too long.
pub fn over_at(platform: Platform, caption: &str) -> Option<usize> {
    let limit = platform.limit();
    if length(platform, caption) <= limit {
        return None;
    }
    let mut n = 0;
    for (i, c) in caption.char_indices() {
        n += if platform == Platform::X { x_weight(c) } else { 1 };
        if n > limit {
            return Some(i);
        }
    }
    None
}

/// The hashtags in a caption.
pub fn hashtags(caption: &str) -> usize {
    caption
        .split(|c: char| c.is_whitespace())
        .filter(|w| w.len() > 1 && w.starts_with('#') && w[1..].chars().next().is_some_and(|c| c.is_alphanumeric() && !c.is_ascii_digit()))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caption_is_the_note_without_markdown() {
        let note = "# Launch day\nWe **shipped** the *public beta* today.\n\n- Faster by [[Benchmarks|10x]]\n- [ ] Tell the team\n> Speed is a feature\n![[shot.png]]\nRead [the post](https://notsy.app/beta) %%draft%%#launch";
        assert_eq!(
            caption(note),
            "Launch day\nWe shipped the public beta today.\n\n\u{2022} Faster by 10x\n\u{2610} Tell the team\nSpeed is a feature\nRead the post https://notsy.app/beta #launch"
        );
        assert_eq!(caption("Just a title"), "Just a title");
        assert_eq!(caption("---\ntags: x\n---\nTitle\nBody"), "Title\nBody");
        assert_eq!(caption("\n\n  Title  \n\nBody\n\n"), "Title\n\nBody");
        assert_eq!(caption("T\nsnake_case and __init__"), "T\nsnake_case and init__", "underscores inside words stay");
    }

    #[test]
    fn counts_as_each_platform_does() {
        assert_eq!(length(Platform::LinkedIn, "héllo 👋"), 7);
        // X: a link is 23, an emoji 2, CJK 2 each.
        assert_eq!(length(Platform::X, "see https://example.com/a/very/long/path now"), 4 + 23 + 4);
        assert_eq!(length(Platform::X, "👋"), 2);
        assert_eq!(length(Platform::X, "日本"), 4);
        assert_eq!(length(Platform::X, "👩\u{200D}💻"), 4);
    }

    #[test]
    fn cuts_where_the_feeds_do() {
        let long = "word ".repeat(60);
        let at = fold_at(Platform::Instagram, &long).unwrap();
        assert!(at <= 125 && at > 115 && long[..at].ends_with("word"), "{at}");
        assert_eq!(fold_at(Platform::Instagram, "short"), None);
        // Lines count too: Instagram shows two.
        assert_eq!(fold_at(Platform::Instagram, "one\ntwo\nthree"), Some(7));
        assert_eq!(fold_at(Platform::X, &long), None, "X shows it all, up to its limit");
        let too_long = "a".repeat(290);
        assert_eq!(over_at(Platform::X, &too_long), Some(280));
        assert_eq!(over_at(Platform::X, "fine"), None);
    }

    #[test]
    fn counts_hashtags() {
        assert_eq!(hashtags("Launch #beta #notes2026 and #1 or # alone, email@x.com"), 2);
    }
}
