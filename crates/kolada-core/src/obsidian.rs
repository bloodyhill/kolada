//! What an Obsidian vault's settings say, where Kolada follows them: where
//! attachments go, and daily notes (their folder, how they are named, and
//! the template they start from).

use std::path::Path;

/// A string setting from a settings file (`"key": "value"`), unescaped;
/// enough JSON for Obsidian's flat settings files.
pub fn string(json: &str, key: &str) -> Option<String> {
    let quoted = format!("\"{key}\"");
    let at = json.find(&quoted)?;
    let rest = json[at + quoted.len()..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// A settings file of the vault's (`.obsidian/<name>`), if it has one.
fn settings(vault: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(vault.join(".obsidian").join(name)).ok()
}

/// How the vault keeps daily notes.
#[derive(Debug, PartialEq)]
pub struct Daily {
    /// From the vault's top ("" for the top itself).
    pub folder: String,
    /// Moment.js style, as Obsidian takes it: `YYYY-MM-DD`.
    pub format: String,
    /// A note to start each from, from the vault's top, without `.md`.
    pub template: String,
}

pub fn daily(vault: &Path) -> Daily {
    let json = settings(vault, "daily-notes.json").unwrap_or_default();
    let get = |key| string(&json, key).map(|s| s.trim().trim_matches('/').to_string()).unwrap_or_default();
    let format = get("format");
    Daily { folder: get("folder"), format: if format.is_empty() { "YYYY-MM-DD".to_string() } else { format }, template: get("template") }
}

/// A day and time, as a date format needs them.
#[derive(Clone, Copy, Debug)]
pub struct When {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// 0 for Sunday.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// `when` written the way a moment.js format says: `YYYY-MM-DD`,
/// `dddd, MMMM Do`; text in [brackets] as it is.
pub fn format(format: &str, when: When) -> String {
    let month = MONTHS[(when.month.clamp(1, 12) - 1) as usize];
    let weekday = DAYS[(when.weekday % 7) as usize];
    let ordinal = |d: u8| {
        let suffix = match (d % 10, d % 100) {
            (_, 11..=13) => "th",
            (1, _) => "st",
            (2, _) => "nd",
            (3, _) => "rd",
            _ => "th",
        };
        format!("{d}{suffix}")
    };
    // Longest first, so MMMM is not read as MM MM.
    let tokens: [(&str, String); 16] = [
        ("YYYY", format!("{:04}", when.year)),
        ("MMMM", month.to_string()),
        ("dddd", weekday.to_string()),
        ("MMM", month[..3].to_string()),
        ("ddd", weekday[..3].to_string()),
        ("YY", format!("{:02}", when.year % 100)),
        ("MM", format!("{:02}", when.month)),
        ("DD", format!("{:02}", when.day)),
        ("Do", ordinal(when.day)),
        ("dd", weekday[..2].to_string()),
        ("HH", format!("{:02}", when.hour)),
        ("mm", format!("{:02}", when.minute)),
        ("M", when.month.to_string()),
        ("D", when.day.to_string()),
        ("H", when.hour.to_string()),
        ("m", when.minute.to_string()),
    ];
    let mut out = String::new();
    let mut rest = format;
    while let Some(c) = rest.chars().next() {
        if c == '[' {
            let end = rest.find(']').unwrap_or(rest.len());
            out.push_str(&rest[1..end]);
            rest = &rest[(end + 1).min(rest.len())..];
        } else if let Some((token, value)) = tokens.iter().find(|(t, _)| rest.starts_with(t)) {
            out.push_str(value);
            rest = &rest[token.len()..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// A template's text for a note named `title`: `{{title}}`, `{{date}}`
/// (or `{{date:FORMAT}}`) and `{{time}}` filled in, as Obsidian does.
pub fn fill(template: &str, title: &str, when: When) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(at) = rest.find("{{") {
        out.push_str(&rest[..at]);
        let Some(end) = rest[at..].find("}}") else { break };
        let inner = rest[at + 2..at + end].trim();
        let (name, custom) = inner.split_once(':').map_or((inner, None), |(n, f)| (n.trim(), Some(f.trim())));
        match name {
            "title" => out.push_str(title),
            "date" => out.push_str(&self::format(custom.unwrap_or("YYYY-MM-DD"), when)),
            "time" => out.push_str(&self::format(custom.unwrap_or("HH:mm"), when)),
            _ => out.push_str(&rest[at..at + end + 2]),
        }
        rest = &rest[at + end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHEN: When = When { year: 2026, month: 9, day: 30, weekday: 3, hour: 9, minute: 5 };

    #[test]
    fn reads_settings() {
        let json = r#"{"folder": "Journal/Daily", "format": "YYYY-MM-DD", "template": "Templates\/Day \"one\""}"#;
        assert_eq!(string(json, "folder").as_deref(), Some("Journal/Daily"));
        assert_eq!(string(json, "template").as_deref(), Some("Templates/Day \"one\""));
        assert_eq!(string(json, "missing"), None);
        assert_eq!(string(r#"{"a": "café"}"#, "a").as_deref(), Some("café"));
    }

    #[test]
    fn names_days_as_moment_does() {
        assert_eq!(format("YYYY-MM-DD", WHEN), "2026-09-30");
        assert_eq!(format("dddd, MMMM Do YYYY", WHEN), "Wednesday, September 30th 2026");
        assert_eq!(format("D MMM YY", WHEN), "30 Sep 26");
        assert_eq!(format("[Week of] YYYY-MM-DD", WHEN), "Week of 2026-09-30");
        assert_eq!(format("HH:mm", WHEN), "09:05");
        let first = When { day: 1, ..WHEN };
        assert_eq!(format("Do", first), "1st");
        assert_eq!(format("Do", When { day: 12, ..WHEN }), "12th");
    }

    #[test]
    fn fills_templates() {
        assert_eq!(fill("# {{title}}\nMade {{date:D MMMM}} at {{time}}.\n{{other}}", "2026-09-30", WHEN), "# 2026-09-30\nMade 30 September at 09:05.\n{{other}}");
    }
}
