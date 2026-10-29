//! Telling a password apart from ordinary text (design, section 6).
//!
//! A password manager's own clipboard markers win over this (the shells read
//! them). This is the fallback, for the iPhone's share sheet and anything
//! else: a single line of 8 to 128 characters, no spaces, at least three
//! kinds of character, not a link, an email address or a number, and enough
//! entropy that it isn't a word.

pub fn looks_like_password(s: &str) -> bool {
    let s = s.trim_matches(|c: char| c == '\n' || c == '\r');
    let n = s.chars().count();
    if !(8..=128).contains(&n) || s.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
        return false;
    }
    if is_email(s) {
        return false;
    }
    let digits_only = s.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | ',' | '(' | ')'));
    if digits_only {
        return false;
    }
    let classes = [
        s.chars().any(|c| c.is_lowercase()),
        s.chars().any(|c| c.is_uppercase()),
        s.chars().any(|c| c.is_ascii_digit()),
        s.chars().any(|c| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if classes < 3 {
        return false;
    }
    entropy_bits(s) >= 2.5 * 8.0
}

fn is_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else { return false };
    let tld_ok = domain.rsplit_once('.').is_some_and(|(_, t)| t.len() >= 2 && t.chars().all(|c| c.is_alphabetic()));
    !local.is_empty()
        && tld_ok
        && local.chars().all(|c| c.is_alphanumeric() || "._%+-".contains(c))
        && domain.chars().all(|c| c.is_alphanumeric() || ".-".contains(c))
}

/// Shannon entropy of the characters, times the length: a rough count of bits.
fn entropy_bits(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let n = s.chars().count() as f64;
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let h: f64 = counts.values().map(|&k| {
        let p = k as f64 / n;
        -p * p.log2()
    }).sum();
    h * n
}

#[cfg(test)]
mod tests {
    use super::looks_like_password;

    #[test]
    fn the_table() {
        let yes = [
            "Tr0ub4dor&3",
            "hunter2!Xq9",
            "aB3$kL9#mN2@",
            "x7Kp-Qm2v-Ld8w-Rz4t",
            "correct-Horse-battery-9",
            "9fG!kQ2#vB7z",
        ];
        let no = [
            "hello",
            "password",
            "Pick up dry cleaning",
            "https://kolada.app/Docs?x=1",
            "www.example.com/a1B",
            "someone@example.com",
            "+966 55 123 4567",
            "1234567890",
            "Hello, World",
            "ALLCAPSWORD",
            "aaaaaaaaA1",
            "two\nlines Ab1!",
        ];
        for s in yes {
            assert!(looks_like_password(s), "should be a password: {s}");
        }
        for s in no {
            assert!(!looks_like_password(s), "should not be a password: {s}");
        }
    }
}
