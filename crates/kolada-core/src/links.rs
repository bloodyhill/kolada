//! Keeping `[[links]]` pointing at a note when it is renamed.

/// Where `text` links to the note named `old`, and what each link's target
/// becomes for the name `new`: `(start, end, replacement)` byte ranges of
/// the targets, in order. Covers `[[old]]`, `[[old|shown]]`,
/// `[[old#heading]]`, `![[old]]` and `[[folder/old]]`; names compare
/// ignoring case, as Obsidian resolves them.
pub fn renamed(text: &str, old: &str, new: &str) -> Vec<(usize, usize, String)> {
    let old = old.trim().to_lowercase();
    let mut out = Vec::new();
    if old.is_empty() {
        return out;
    }
    let mut from = 0;
    while let Some(i) = text[from..].find("[[") {
        let start = from + i + 2;
        let line_end = text[start..].find('\n').map_or(text.len(), |n| start + n);
        let Some(close) = text[start..line_end].find("]]") else {
            from = start;
            continue;
        };
        let inner = &text[start..start + close];
        let target_len = inner.find(['|', '#', '^']).unwrap_or(inner.len());
        let target = &inner[..target_len];
        // The name is the target's last segment; a folder before it stays.
        let name_at = target.rfind('/').map_or(0, |s| s + 1);
        let name = target[name_at..].trim();
        let name = name.strip_suffix(".md").unwrap_or(name);
        if name.to_lowercase() == old {
            let lead = target[name_at..].len() - target[name_at..].trim_start().len();
            let s = start + name_at + lead;
            let e = start + target.trim_end().len();
            let md = if target.trim_end().ends_with(".md") { ".md" } else { "" };
            out.push((s, e, format!("{new}{md}")));
        }
        from = start + close + 2;
    }
    out
}

/// `text` with `renamed`'s replacements made.
pub fn apply(text: &str, replacements: &[(usize, usize, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (s, e, r) in replacements {
        out.push_str(&text[at..*s]);
        out.push_str(r);
        at = *e;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(text: &str) -> String {
        apply(text, &renamed(text, "Pricing ideas", "Pricing"))
    }

    #[test]
    fn retargets_every_form_of_link() {
        assert_eq!(rename("See [[Pricing ideas]]."), "See [[Pricing]].");
        assert_eq!(rename("[[pricing IDEAS|the tiers]]"), "[[Pricing|the tiers]]");
        assert_eq!(rename("[[Pricing ideas#Tiers]] and ![[Pricing ideas]]"), "[[Pricing#Tiers]] and ![[Pricing]]");
        assert_eq!(rename("[[Work/Pricing ideas]]"), "[[Work/Pricing]]");
        assert_eq!(rename("[[Pricing ideas.md]]"), "[[Pricing.md]]");
    }

    #[test]
    fn leaves_other_links_and_text_alone() {
        let text = "[[Pricing ideas 2]] [[Ideas]] Pricing ideas [[broken\n]]";
        assert!(renamed(text, "Pricing ideas", "Pricing").is_empty());
        assert_eq!(rename("ملاحظة [[Pricing ideas]] ✓"), "ملاحظة [[Pricing]] ✓");
    }
}
