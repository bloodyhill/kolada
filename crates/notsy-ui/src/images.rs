//! Pictures in notes, as the screen sees them: which lines embed one and at
//! what width, and what is known of a picture's file. Each shell decodes
//! and scales the files its own way (Windows' WIC, the Mac's ImageIO) behind
//! `PictureSource`; the screen only asks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::pixels::Pixels;

/// File types shown as pictures.
const KINDS: [&str; 9] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico"];

/// Whether `name` is a picture by its extension.
pub fn is_image(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| KINDS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The picture a line embeds, if the line is only that: the file it names.
pub fn embed_in(line: &str) -> Option<&str> {
    let s = line.trim();
    let target = if let Some(inner) = s.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")) {
        // ![[photo.png|300]]: a size after the bar.
        inner.split('|').next()?.trim()
    } else {
        let rest = s.strip_prefix("![")?;
        let (_, url) = rest.split_once("](")?;
        let url = url.strip_suffix(')')?;
        url.split_whitespace().next()?
    };
    (is_image(target) && !target.contains("://")).then_some(target)
}

/// The width a picture line asks for, as Obsidian writes it: after a bar,
/// `![[photo.png|300]]`, `![[photo.png|300x200]]`, `![alt|300](photo.png)`.
pub fn embed_width(line: &str) -> Option<u32> {
    let s = line.trim();
    let size = if let Some(inner) = s.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")) {
        inner.rsplit('|').next().filter(|_| inner.contains('|'))?
    } else {
        let (alt, _) = s.strip_prefix("![")?.split_once("](")?;
        alt.rsplit('|').next().filter(|_| alt.contains('|'))?
    };
    let width = size.trim().split('x').next()?.parse::<u32>().ok()?;
    (width > 0).then_some(width)
}

/// The picture line `line` asking for `width` (or its own size, without).
pub fn with_width(line: &str, width: Option<u32>) -> String {
    let s = line.trim();
    let bar = |w: Option<u32>| w.map(|w| format!("|{w}")).unwrap_or_default();
    if let Some(inner) = s.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")) {
        let mut parts: Vec<&str> = inner.split('|').collect();
        // A size already there goes; a caption stays.
        if parts.len() > 1 && embed_width(line).is_some() {
            parts.pop();
        }
        return format!("![[{}{}]]", parts.join("|"), bar(width));
    }
    if let Some((alt, rest)) = s.strip_prefix("![").and_then(|r| r.split_once("](")) {
        let alt = if embed_width(line).is_some() { alt.rsplit_once('|').map_or(alt, |(a, _)| a) } else { alt };
        return format!("![{alt}{}]({rest}", bar(width));
    }
    line.to_string()
}

/// What is known of an embed's file.
pub enum Lookup {
    /// Being looked for.
    Pending,
    /// There is none.
    Missing,
    /// It, and its size in pixels.
    Found(PathBuf, u32, u32),
}

/// What a shell tells the screen about pictures: where an embed's file is,
/// and its pixels at the size they are drawn.
pub trait PictureSource {
    /// Where the picture `name` (embedded in a note in `from`, in `vault`) is.
    fn place(&mut self, vault: &Path, from: &Path, name: &str) -> Lookup;

    /// The file's pixels at `width` pixels across, and how far into their
    /// fade in they are (0..1); None while they are being decoded.
    fn pixels(&mut self, path: &Path, width: u32) -> Option<(Arc<Pixels>, f32)>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_picture_line() {
        assert_eq!(embed_in("![[Pasted image 20260930.png]]"), Some("Pasted image 20260930.png"));
        assert_eq!(embed_in("  ![[photo.JPG|300]] "), Some("photo.JPG"));
        assert_eq!(embed_in("![a cat](cats/tabby.webp)"), Some("cats/tabby.webp"));
        assert_eq!(embed_in("![[Some note]]"), None, "a note, not a picture");
        assert_eq!(embed_in("see ![[photo.png]] here"), None, "not the whole line");
        assert_eq!(embed_in("![x](https://example.com/a.png)"), None, "not a local file");
        assert!(is_image("a.PNG") && !is_image("notes.md"));
    }

    #[test]
    fn reads_and_writes_the_size_a_line_asks_for() {
        assert_eq!(embed_width("![[shot.png|300]]"), Some(300));
        assert_eq!(embed_width("![[shot.png|300x200]]"), Some(300));
        assert_eq!(embed_width("![a cat|240](cat.png)"), Some(240));
        assert_eq!(embed_width("![[shot.png]]"), None);
        assert_eq!(embed_width("![[shot.png|a caption]]"), None);
        assert_eq!(with_width("![[shot.png]]", Some(480)), "![[shot.png|480]]");
        assert_eq!(with_width("![[shot.png|300]]", Some(480)), "![[shot.png|480]]");
        assert_eq!(with_width("![[shot.png|300]]", None), "![[shot.png]]");
        assert_eq!(with_width("![a cat](cat.png)", Some(200)), "![a cat|200](cat.png)");
        assert_eq!(with_width("![a cat|240](cat.png)", Some(200)), "![a cat|200](cat.png)");
        assert_eq!(embed_in(&with_width("![[Pasted image 1.png]]", Some(320))), Some("Pasted image 1.png"));
    }
}
