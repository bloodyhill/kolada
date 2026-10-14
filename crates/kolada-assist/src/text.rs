//! Words, scripts and small distances: enough to tell a fix from a rewrite.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Latin,
    Greek,
    Cyrillic,
    Hebrew,
    Arabic,
    /// Chinese and Japanese (Han and kana): one, since a Japanese fix may
    /// move a word between kana and kanji.
    Cjk,
    Hangul,
    Other,
}

const SCRIPTS: usize = 8;

/// The script of a letter; None for anything that is not one.
pub fn script(c: char) -> Option<Script> {
    if !c.is_alphabetic() {
        return None;
    }
    Some(match c as u32 {
        0x0041..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F => Script::Cyrillic,
        0x0590..=0x05FF => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => Script::Cjk,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Script::Hangul,
        _ => Script::Other,
    })
}

/// The script most letters of `s` are written in.
pub fn dominant(s: &str) -> Option<Script> {
    let mut counts = [0usize; SCRIPTS];
    for c in s.chars() {
        if let Some(sc) = script(c) {
            counts[sc as usize] += 1;
        }
    }
    let (best, &n) = counts.iter().enumerate().max_by_key(|&(_, n)| *n)?;
    (n > 0).then(|| [Script::Latin, Script::Greek, Script::Cyrillic, Script::Hebrew, Script::Arabic, Script::Cjk, Script::Hangul, Script::Other][best])
}

/// Combining marks (Arabic vowels, accents typed apart, joiners): part of
/// the word they sit on.
fn is_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05C7 | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670
        | 0x06D6..=0x06ED | 0x08D3..=0x08FF | 0x200C | 0x200D | 0x3099 | 0x309A | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F)
}

/// Letters written without spaces between words: each is a word of its own.
fn stands_alone(c: char) -> bool {
    script(c) == Some(Script::Cjk)
}

fn in_word(c: char) -> bool {
    (c.is_alphanumeric() && !stands_alone(c)) || is_mark(c)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Word,
    Space,
    Punct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: Kind,
    pub text: &'a str,
    /// Byte offset in the text.
    pub start: usize,
}

impl Token<'_> {
    pub fn end(&self) -> usize {
        self.start + self.text.len()
    }
}

/// `s` as words, runs of spaces and single marks of punctuation, covering
/// it exactly. An apostrophe between letters stays in its word (don't,
/// l'eau).
pub fn tokens(s: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        let mut j = i + 1;
        let kind = if in_word(c) {
            while j < chars.len() {
                let d = chars[j].1;
                let apostrophe = matches!(d, '\'' | '\u{2019}') && chars.get(j + 1).is_some_and(|&(_, e)| in_word(e));
                if in_word(d) || apostrophe {
                    j += 1;
                } else {
                    break;
                }
            }
            Kind::Word
        } else if stands_alone(c) {
            Kind::Word
        } else if c.is_whitespace() {
            while j < chars.len() && chars[j].1.is_whitespace() {
                j += 1;
            }
            Kind::Space
        } else {
            while j < chars.len() && is_mark(chars[j].1) {
                j += 1;
            }
            Kind::Punct
        };
        let end = chars.get(j).map_or(s.len(), |&(e, _)| e);
        out.push(Token { kind, text: &s[start..end], start });
        i = j;
    }
    out
}

pub fn words(s: &str) -> usize {
    tokens(s).iter().filter(|t| t.kind == Kind::Word).count()
}

/// Edits (insert, remove, replace a letter) between `a` and `b`, counted in
/// characters; gives up past `cap`.
pub fn distance(a: &str, b: &str, cap: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > cap {
        return cap + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut best = cur[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            best = best.min(cur[j]);
        }
        if best > cap {
            return cap + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<(Kind, &str)> {
        tokens(s).into_iter().map(|t| (t.kind, t.text)).collect()
    }

    #[test]
    fn tokens_cover_the_text() {
        for s in ["I don't know, really!", "مرحبا بالعالم، كيف الحال؟", "今日は雨です。", "l'eau  est\tfroide", "", "  "] {
            let joined: String = tokens(s).iter().map(|t| t.text).collect();
            assert_eq!(joined, s);
        }
        assert_eq!(kinds("don't stop."), [(Kind::Word, "don't"), (Kind::Space, " "), (Kind::Word, "stop"), (Kind::Punct, ".")]);
        assert_eq!(words("今日は雨"), 4);
        assert_eq!(words("مَرْحَبًا بِكَ"), 2);
    }

    #[test]
    fn scripts() {
        assert_eq!(dominant("Hello there"), Some(Script::Latin));
        assert_eq!(dominant("مرحبا hello"), Some(Script::Arabic));
        assert_eq!(dominant("日本語のテキスト"), Some(Script::Cjk));
        assert_eq!(dominant("안녕하세요"), Some(Script::Hangul));
        assert_eq!(dominant("123 !?"), None);
    }

    #[test]
    fn distances() {
        assert_eq!(distance("buyed", "bought", 9), 4);
        assert_eq!(distance("recieve", "receive", 9), 2);
        assert_eq!(distance("same", "same", 9), 0);
        assert!(distance("a", "abcdefgh", 3) > 3);
    }
}
