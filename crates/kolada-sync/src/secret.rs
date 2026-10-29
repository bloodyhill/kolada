//! Telling a password apart from ordinary text (design, section 6).
//!
//! The OS's own sensitive-clipboard marker wins over everything here: a
//! password manager set it, so it is a password (the shells read it and pass
//! it as `flagged`: Windows' `ExcludeClipboardContentFromMonitorProcessing`
//! or `CanIncludeInClipboardHistory` = 0, the Mac's and iPhone's
//! `org.nspasteboard.ConcealedType` or `TransientType`, Linux's
//! `x-kde-passwordManagerHint` = `secret`).
//!
//! Without it, the text's shape: an API key or token by its known prefix, a
//! private key, a JWT; else a single line of 8 to 128 characters, no
//! spaces, that is not a link, an email address, a path, a number, a phone
//! number, a hash, a UUID, an IBAN, code, a file name or plain words, and
//! that looks generated (random, Apple's `xxxxxx-xxxxxx-xxxxxx`, a
//! passphrase) or like a person's password (`Summer2024!`). Shapes that
//! could go either way are `Unsure`: shown masked, with "Not a password".
//! Nothing about the text is kept.

/// What a clip is, as far as passwords go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secrecy {
    Plain,
    /// It could be a password: shown masked, one click makes it text.
    Unsure,
    Password,
}

impl Secrecy {
    /// Whether it is shown masked (a password, or maybe one).
    pub fn masked(self) -> bool {
        self != Secrecy::Plain
    }
}

/// Whether `s` is a password, sure or not; `flagged`: the OS marked the
/// copy sensitive.
pub fn judge(s: &str, flagged: bool) -> Secrecy {
    let t = s.trim();
    if t.is_empty() {
        return Secrecy::Plain;
    }
    if flagged || private_key(t) {
        return Secrecy::Password;
    }
    if t.chars().any(char::is_whitespace) {
        return Secrecy::Plain;
    }
    if token(t) || jwt(t) {
        return Secrecy::Password;
    }
    // `NAME=value`, as an env file has it: the value decides.
    if let Some((name, value)) = t.split_once('=')
        && !name.is_empty()
        && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && (token(value) || jwt(value))
    {
        return Secrecy::Password;
    }
    let n = t.chars().count();
    if !(8..=128).contains(&n) || not_secret(t) {
        return Secrecy::Plain;
    }
    if apple(t) {
        return Secrecy::Password;
    }
    let segments: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).collect();
    let classes = classes(t);
    if segments.iter().all(|s| plain_segment(s)) {
        if let Some(v) = passphrase(t, &segments) {
            return v;
        }
        // Words and numbers with a password's symbol, all four kinds of
        // character: what a person makes up (`Summer2024!`).
        let symbol = t.chars().any(|c| !c.is_alphanumeric() && !"-_./:,'".contains(c));
        return if classes == 4 && symbol { Secrecy::Password } else { Secrecy::Plain };
    }
    if classes >= 3 && entropy_bits(t) >= 20.0 {
        return Secrecy::Password;
    }
    if classes == 2 && n >= 10 && random_pair(t) {
        return Secrecy::Unsure;
    }
    Secrecy::Plain
}

/// The old yes or no: a password, sure or not.
pub fn looks_like_password(s: &str) -> bool {
    judge(s, false).masked()
}

/// Lower case (and letters with no case), upper case, digits, the rest.
fn classes(s: &str) -> usize {
    let letter_lower = s.chars().any(|c| c.is_alphabetic() && !c.is_uppercase());
    let upper = s.chars().any(char::is_uppercase);
    let digit = s.chars().any(|c| c.is_ascii_digit());
    let other = s.chars().any(|c| !c.is_alphanumeric());
    [letter_lower, upper, digit, other].iter().filter(|x| **x).count()
}

/// Characters a key or token is made of.
fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_.=/+".contains(c)
}

/// An API key or access token by its issuer's prefix.
fn token(t: &str) -> bool {
    const PREFIXES: &[(&str, usize)] = &[
        // OpenAI, Anthropic and others' `sk-…`.
        ("sk-", 20),
        // GitHub.
        ("ghp_", 30),
        ("gho_", 30),
        ("ghu_", 30),
        ("ghs_", 30),
        ("ghr_", 30),
        ("github_pat_", 40),
        ("glpat-", 20),
        // Slack.
        ("xoxb-", 20),
        ("xoxp-", 20),
        ("xoxa-", 20),
        ("xoxr-", 20),
        ("xoxs-", 20),
        ("xapp-", 20),
        // Google.
        ("AIza", 30),
        ("ya29.", 20),
        // Stripe.
        ("sk_live_", 16),
        ("sk_test_", 16),
        ("rk_live_", 16),
        ("rk_test_", 16),
        ("whsec_", 20),
        ("SG.", 30),
        ("npm_", 30),
        ("pypi-", 30),
        ("hf_", 30),
        ("shpat_", 24),
        ("shpss_", 24),
        ("dop_v1_", 40),
        ("AGE-SECRET-KEY-1", 40),
    ];
    if t.len() > 1024 || !t.chars().all(token_char) {
        return false;
    }
    for (prefix, least) in PREFIXES {
        if let Some(rest) = t.strip_prefix(prefix)
            && rest.len() >= *least
            && rest.chars().any(|c| c.is_ascii_digit())
            && rest.chars().any(|c| c.is_ascii_alphabetic())
        {
            return true;
        }
    }
    // AWS access keys: AKIA or ASIA and 16 more, capitals and digits.
    t.len() == 20
        && (t.starts_with("AKIA") || t.starts_with("ASIA"))
        && t[4..].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// A JSON web token: three base64url parts, the first a JSON header.
fn jwt(t: &str) -> bool {
    let parts: Vec<&str> = t.split('.').collect();
    t.len() >= 32
        && t.len() <= 8192
        && t.starts_with("eyJ")
        && parts.len() == 3
        && parts[..2].iter().all(|p| p.len() >= 8)
        && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// A PEM private key, lines and all.
fn private_key(t: &str) -> bool {
    t.len() <= 16 * 1024 && t.starts_with("-----BEGIN ") && t.lines().next().is_some_and(|l| l.ends_with("PRIVATE KEY-----"))
}

/// Shapes that are never passwords.
fn not_secret(t: &str) -> bool {
    let lower = t.to_ascii_lowercase();
    // A link, or a web address without its scheme.
    if lower.contains("://") || lower.starts_with("www.") || lower.starts_with("mailto:") {
        return true;
    }
    let host = t.split(['/', '?', '#']).next().unwrap_or("");
    if host.len() < t.len() && domain(host) || domain(t) {
        return true;
    }
    if email(t) {
        return true;
    }
    // A path.
    let b = t.as_bytes();
    if t.starts_with('/') || t.starts_with("~/") || t.starts_with("./") || t.starts_with("../") || t.starts_with("\\\\")
        || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || t.matches('\\').count() >= 2
    {
        return true;
    }
    // Numbers, phone numbers, dates, times, IP addresses.
    if t.chars().all(|c| c.is_ascii_digit() || "+-.,()/:#*".contains(c)) {
        return true;
    }
    // Hashes, UUIDs and GUIDs, MAC and IPv6 addresses, colours.
    let hex = t.trim_start_matches('#').trim_start_matches('{').trim_end_matches('}');
    if hex.chars().all(|c| c.is_ascii_hexdigit() || c == '-' || c == ':')
        && (hex.chars().all(|c| !c.is_ascii_uppercase()) || hex.chars().all(|c| !c.is_ascii_lowercase()))
    {
        return true;
    }
    iban(t) || code(t) || file_name(t)
}

/// `example.co.uk`: labels of letters, digits and hyphens, the last a TLD.
fn domain(t: &str) -> bool {
    let labels: Vec<&str> = t.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels.last().is_some_and(|tld| (2..=6).contains(&tld.len()) && tld.chars().all(|c| c.is_ascii_lowercase()))
}

fn email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else { return false };
    let tld_ok = domain.rsplit_once('.').is_some_and(|(_, t)| t.len() >= 2 && t.chars().all(|c| c.is_alphabetic()));
    !local.is_empty()
        && tld_ok
        && local.chars().all(|c| c.is_alphanumeric() || "._%+-".contains(c))
        && domain.chars().all(|c| c.is_alphanumeric() || ".-".contains(c))
}

/// A bank account number: a country, two check digits, then capitals and
/// digits.
fn iban(t: &str) -> bool {
    let b = t.as_bytes();
    (15..=34).contains(&b.len())
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..4].iter().all(u8::is_ascii_digit)
        && b[4..].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// A line of code: a call, a path through modules, an arrow, an operator,
/// a statement's end or a tag.
fn code(t: &str) -> bool {
    let call = t.find('(').is_some_and(|i| {
        let before = &t[..i];
        let name = before.rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
        name.chars().count() >= 2 && name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && t.trim_end_matches([';', '?', '.']).ends_with(')')
    });
    call || t.contains("::")
        || t.contains("->")
        || t.contains("=>")
        || t.contains("==")
        || t.contains("!=")
        || t.contains("&&")
        || t.contains("||")
        || t.contains("${")
        || t.contains("$(")
        || t.starts_with("--")
        || (t.ends_with(';') && t.contains(['=', '(']))
        || (t.starts_with('<') && t.ends_with('>'))
        || (t.contains('{') && t.ends_with('}'))
}

/// `Report_Final_v2.docx`: a name and a known extension.
fn file_name(t: &str) -> bool {
    const EXTS: &[&str] = &[
        "txt", "md", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "csv", "json", "xml", "html", "htm", "css", "js", "ts", "rs", "py", "go", "swift", "png", "jpg", "jpeg", "gif", "heic", "webp", "svg", "mp3", "mp4", "mov", "wav", "zip", "tar", "gz", "exe", "dmg", "msi", "app", "toml", "yaml", "yml",
    ];
    t.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && EXTS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Apple's strong passwords: three groups of six, one capital and one
/// digit among them (`abcdef-Ghijk2-lmnopq`).
fn apple(t: &str) -> bool {
    let groups: Vec<&str> = t.split('-').collect();
    groups.len() == 3
        && groups.iter().all(|g| g.len() == 6 && g.chars().all(|c| c.is_ascii_alphanumeric()))
        && t.chars().filter(char::is_ascii_uppercase).count() == 1
        && t.chars().filter(char::is_ascii_digit).count() == 1
}

/// Whether a run of letters and digits reads as words and numbers
/// (`Summer2024`, `iPhone15Pro`, `ORD`, `Q3`), not made up (`kQ2v`, `S3cret`).
fn plain_segment(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 3 {
        return true;
    }
    // A digit inside a word, the word going on in lower case: leet speak.
    for i in 1..chars.len().saturating_sub(1) {
        if chars[i].is_ascii_digit() && chars[i - 1].is_alphabetic() {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < chars.len() && chars[j].is_lowercase() {
                return false;
            }
        }
    }
    parts(&chars).iter().all(|p| word_like(p))
}

/// A run cut where a word would start: at digits, at a capital after lower
/// case, and before the last capital of a run that lower case follows
/// (`parseHTMLDoc`: parse, HTML, Doc).
fn parts(chars: &[char]) -> Vec<Vec<char>> {
    let mut out: Vec<Vec<char>> = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let cut = match prev {
            None => true,
            Some(p) => {
                p.is_ascii_digit() != c.is_ascii_digit()
                    || (!p.is_uppercase() && c.is_uppercase())
                    || (p.is_uppercase() && c.is_uppercase() && next.is_some_and(char::is_lowercase))
            }
        };
        if cut {
            out.push(Vec::new());
        }
        out.last_mut().unwrap().push(c);
    }
    out
}

fn word_like(p: &[char]) -> bool {
    if p.len() <= 1 || p.iter().all(|c| c.is_ascii_digit()) {
        return true;
    }
    // An acronym.
    if p.len() <= 5 && p.iter().all(|c| c.is_uppercase()) {
        return true;
    }
    // A word has a vowel (letters of other scripts are taken as words).
    p.iter().any(|c| "aeiouyAEIOUY".contains(*c) || (c.is_alphabetic() && !c.is_ascii()))
}

/// Three or more words of three letters or more, one separator between
/// them: a generated passphrase. With a digit or mixed capitals it is sure;
/// all alike (`quaint-reliable-ruse-wolf`) could be a slug, so unsure.
fn passphrase(t: &str, segments: &[&str]) -> Option<Secrecy> {
    let seps: Vec<char> = t.chars().filter(|c| !c.is_alphanumeric()).collect();
    let sep = *seps.first()?;
    if seps.iter().any(|&c| c != sep) || seps.len() + 1 != segments.len() {
        return None;
    }
    // Words, a digit or two after one of them allowed (`Flagstone7`), and
    // at most one number of their own.
    let mut words = Vec::new();
    let mut numbers = 0;
    for s in segments {
        let w = s.trim_end_matches(|c: char| c.is_ascii_digit());
        if w.is_empty() {
            numbers += 1;
        } else if w.chars().count() >= 3 && w.chars().all(char::is_alphabetic) {
            words.push(w);
        } else {
            return None;
        }
    }
    if words.len() < 3 || numbers > 1 {
        return None;
    }
    let digit = t.chars().any(|c| c.is_ascii_digit());
    let capital = |w: &&str| w.chars().next().is_some_and(char::is_uppercase);
    let mixed = words.iter().any(capital) && !words.iter().all(capital);
    if digit || mixed {
        return Some(Secrecy::Password);
    }
    (sep == '-').then_some(Secrecy::Unsure)
}

/// Two kinds of character, mixed through each other as a generator mixes
/// them: digits in letters (fewer digits than letters, else an order or
/// tracking number), or capitals in lower case that are not words.
fn random_pair(t: &str) -> bool {
    let chars: Vec<char> = t.chars().collect();
    let n = chars.len();
    let digits = chars.iter().filter(|c| c.is_ascii_digit()).count();
    let unique = chars.iter().collect::<std::collections::HashSet<_>>().len();
    if unique * 10 < n * 6 {
        return false;
    }
    if digits > 0 {
        let runs = chars.windows(2).filter(|w| !w[0].is_ascii_digit() && w[1].is_ascii_digit()).count() + usize::from(chars[0].is_ascii_digit());
        return chars.iter().all(|c| c.is_alphanumeric()) && digits * 2 <= n && runs >= 2;
    }
    chars.iter().all(|c| c.is_alphabetic()) && !parts(&chars).iter().all(|p| word_like(p))
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
    use super::{Secrecy, judge};
    use Secrecy::{Password, Plain, Unsure};

    /// Made-up characters for keys and tokens (no real key is written out
    /// here, so no secret scanner trips on this file).
    fn tail(seed: u64, len: usize, alphabet: &str) -> String {
        let a: Vec<char> = alphabet.chars().collect();
        let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                a[(x % a.len() as u64) as usize]
            })
            .collect()
    }

    const ALNUM: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    /// Every case and what it should be.
    fn table() -> Vec<(String, Secrecy)> {
        let s = |x: &str, v: Secrecy| (x.to_string(), v);
        vec![
            // People's passwords.
            s("Tr0ub4dor&3", Password),
            s("hunter2!Xq9", Password),
            s("Summer2024!", Password),
            s("P@ssw0rd123", Password),
            s("iLoveMyDog#7", Password),
            s("Kolada_2026$", Password),
            s("qwerty!234A", Password),
            s("MyS3cretPass", Password),
            // Generated: 1Password's and Bitwarden's random ones.
            s("9fG!kQ2#vB7z", Password),
            s("aB3$kL9#mN2@", Password),
            s("xK7#pQ2!mZ9vL4@wR8tY", Password),
            s("Hk4Rtz9WqPm2Lx", Password),
            s("fT8nV2rK5wQ1", Password),
            s("u6Y.bZ3*eN9-sD1^", Password),
            s("7HqX2mPzR9kLwT4v", Password),
            s("jH8&kL2@pQ9!zX4#vN6%", Password),
            s("ZbE3xwUdNkB7pFsq", Password),
            s("g3ZtRmY8qWxK", Password),
            s("x7kp2qm9vld8wz", Unsure),
            s("kQmZpLxWvRtYbN", Unsure),
            // Apple's.
            s("hivwy3-Kahgaz-voxgub", Password),
            s("dopnaw-5Wuqsi-cymqyv", Password),
            // Passphrases.
            s("correct-Horse-battery-9", Password),
            s("Unpaid-Calamity-Flagstone7", Password),
            s("quaint-reliable-ruse-wolf", Unsure),
            s("x7Kp-Qm2v-Ld8w-Rz4t", Password),
            // Keys and tokens.
            (format!("sk-proj-{}", tail(1, 48, ALNUM)), Password),
            (format!("sk-ant-api03-{}", tail(2, 80, ALNUM)), Password),
            (format!("ghp_{}", tail(3, 36, ALNUM)), Password),
            (format!("github_pat_11{}_{}", tail(4, 20, UPPER), tail(5, 59, ALNUM)), Password),
            (format!("AKIA{}", tail(6, 16, UPPER)), Password),
            (format!("xoxb-{}-{}-{}", tail(7, 12, "0123456789"), tail(8, 13, "0123456789"), tail(9, 24, ALNUM)), Password),
            (format!("xoxp-{}-{}", tail(10, 12, "0123456789"), tail(11, 32, ALNUM)), Password),
            (format!("AIza{}", tail(12, 35, ALNUM)), Password),
            (format!("sk_live_{}", tail(13, 24, ALNUM)), Password),
            (format!("glpat-{}", tail(14, 20, ALNUM)), Password),
            (format!("npm_{}", tail(15, 36, ALNUM)), Password),
            (format!("hf_{}", tail(16, 34, ALNUM)), Password),
            (format!("eyJ{}.eyJ{}.{}", tail(17, 30, ALNUM), tail(18, 40, ALNUM), tail(19, 43, ALNUM)), Password),
            (format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n{}\n-----END OPENSSH PRIVATE KEY-----", tail(20, 64, ALNUM), tail(21, 40, ALNUM)), Password),
            (format!("OPENAI_API_KEY=sk-{}", tail(22, 40, ALNUM)), Password),
            // Not passwords: words and sentences.
            s("hello", Plain),
            s("password", Plain),
            s("Pick up dry cleaning", Plain),
            s("The quick brown fox jumps over the lazy dog.", Plain),
            s("Meeting moved to 3pm, room B2!", Plain),
            s("Hello,World", Plain),
            s("ALLCAPSWORD", Plain),
            s("aaaaaaaaA1", Plain),
            s("two\nlines Ab1!", Plain),
            s("Q3-Revenue-Report", Plain),
            s("iPhone15Pro", Plain),
            s("#KoladaNotes", Plain),
            s("@kolada_app", Plain),
            s("مرحبابالعالم", Plain),
            // Links, email addresses, paths and file names.
            s("https://kolada.app/Docs?x=1", Plain),
            s("https://www.example.com/path/to/page?id=A1b2C3&ref=xY9", Plain),
            s("www.example.com/a1B", Plain),
            s("example.com/p?id=A1b2C3xY9", Plain),
            s("example.co.uk", Plain),
            s("someone@example.com", Plain),
            s("First.Last+tag@Mail-Server.org", Plain),
            s("C:\\Users\\Public\\Documents\\Report_2024.docx", Plain),
            s("/usr/local/bin/kolada", Plain),
            s("~/Notes/Inbox.md", Plain),
            s("Report_Final_v2.docx", Plain),
            s("IMG_20261029_1430.JPG", Plain),
            s("file2.txt", Plain),
            // Numbers.
            s("+966 55 123 4567", Plain),
            s("+1-555-010-9999", Plain),
            s("(555)010-9999", Plain),
            s("1234567890", Plain),
            s("1,234,567.89", Plain),
            s("2026-10-29", Plain),
            s("14:30:00", Plain),
            s("10.0.0.254", Plain),
            s("v1.2.3-beta.4", Plain),
            s("ORD-2026-004512", Plain),
            s("1Z999AA10123456784", Plain),
            s("1HGCM82633A004352", Plain),
            // IBANs.
            s("SA0380000000608010167519", Plain),
            s("GB82WEST12345698765432", Plain),
            s("DE89370400440532013000", Plain),
            // Hashes, UUIDs, addresses, colours.
            s("d41d8cd98f00b204e9800998ecf8427e", Plain),
            s("da39a3ee5e6b4b0d3255bfef95601890afd80709", Plain),
            s("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", Plain),
            s("550e8400-e29b-41d4-a716-446655440000", Plain),
            s("{6F9619FF-8B86-D011-B42D-00C04FC964FF}", Plain),
            s("2001:db8::ff00:42:8329", Plain),
            s("00:1A:2B:3C:4D:5E", Plain),
            s("#0E9594FF", Plain),
            // Code.
            s("getElementById", Plain),
            s("my_variable_name", Plain),
            s("foo.bar(baz);", Plain),
            s("user.Name=getValue(42);", Plain),
            s("std::env::var(\"HOME\")", Plain),
            s("if(x>=10){return;}", Plain),
            s("arr[i]=v[j+1];", Plain),
            s("--dry-run", Plain),
        ]
    }

    /// Cases written apart from the rules, by someone who had not seen
    /// them, and kept as they were: true for a password.
    fn written_apart() -> Vec<(String, bool)> {
        vec![
            ("Summer2024!".to_string(), true),
            ("P@ssw0rd123".to_string(), true),
            ("Tr0ub4dor&3".to_string(), true),
            ("Welcome1#Home".to_string(), true),
            ("Qwerty!2345".to_string(), true),
            ("m0nk3y_B@nana".to_string(), true),
            ("Fluffy#Cat99".to_string(), true),
            ("iL0veCoffee$".to_string(), true),
            ("xK9#mPq2$vL7nR4w".to_string(), true),
            ("7hGf!2kLp0Zq".to_string(), true),
            ("vB3$nM8@qW5zXc1P".to_string(), true),
            ("aQ4tR8yU2oP6".to_string(), true),
            ("Zx9&Kq2!Lp7#Mw4$Vb8@".to_string(), true),
            ("3hK9sLq0Pz7TxW2mNc4VbR8dFg".to_string(), true),
            ("hunter2Hunter!".to_string(), true),
            ("wxyzab-Cdefg2-hijklm".to_string(), true),
            ("nupqad-Xyr7vi-kofdyb".to_string(), true),
            ("Tidy-Mango-Orbit4".to_string(), true),
            ("correct-Horse-battery7-staple".to_string(), true),
            ("Blue.Tiger.Cloud.82".to_string(), true),
            (format!("sk-{}", tail(101, 48, ALNUM)), true),
            (format!("sk-proj-{}", tail(102, 80, ALNUM)), true),
            (format!("ghp_{}", tail(103, 36, ALNUM)), true),
            (format!("github_pat_{}_{}", tail(104, 22, ALNUM), tail(105, 59, ALNUM)), true),
            (format!("AKIA{}", tail(106, 16, ALNUM)), true),
            (format!("xoxb-123456789012-123456789012-{}", tail(107, 24, ALNUM)), true),
            (format!("sk_live_{}", tail(108, 24, ALNUM)), true),
            (format!("AIza{}", tail(109, 35, ALNUM)), true),
            (format!("glpat-{}", tail(110, 20, ALNUM)), true),
            (format!("npm_{}", tail(111, 36, ALNUM)), true),
            (format!("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.{}", tail(112, 60, ALNUM), tail(113, 43, ALNUM)), true),
            (format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n{}\n{}\n-----END OPENSSH PRIVATE KEY-----", tail(114, 64, ALNUM), tail(115, 64, ALNUM), tail(116, 40, ALNUM)), true),
            (format!("-----BEGIN RSA PRIVATE KEY-----\n{}\n{}\n-----END RSA PRIVATE KEY-----", tail(117, 64, ALNUM), tail(118, 64, ALNUM)), true),
            ("CoffeeShop2020!".to_string(), true),
            ("7Xk2mQ9pLw3Rz".to_string(), true),
            (format!("gho_{}", tail(119, 36, ALNUM)), true),
            (format!("sk-ant-api03-{}", tail(120, 90, ALNUM)), true),
            (format!("rk_live_{}", tail(121, 24, ALNUM)), true),
            (format!("xoxp-1234567890-0987654321-{}", tail(122, 24, ALNUM)), true),
            ("Passw0rd!Admin".to_string(), true),
            ("Dragon$layer88".to_string(), true),
            ("5Gz!pQ8wLm2@".to_string(), true),
            (format!("ghs_{}", tail(123, 36, ALNUM)), true),
            ("S3cur3P@ss!".to_string(), true),
            ("N3v3rG0nna#Giv3Up".to_string(), true),
            ("Lm9$wQz2#Xv".to_string(), true),
            ("hello".to_string(), false),
            ("Meeting tomorrow".to_string(), false),
            ("John Smith".to_string(), false),
            ("Maria Garcia-Lopez".to_string(), false),
            ("Can you send me the report by Friday afternoon?".to_string(), false),
            ("#SummerVibes".to_string(), false),
            ("@jane_doe".to_string(), false),
            ("https://example.com/path/to/page?id=42&ref=home".to_string(), false),
            ("www.github.com/rust-lang/rust".to_string(), false),
            ("john.doe@example.com".to_string(), false),
            ("+1 (555) 123-4567".to_string(), false),
            ("+44 20 7946 0958".to_string(), false),
            ("2024-03-15".to_string(), false),
            ("15/03/2024".to_string(), false),
            ("3:45 PM".to_string(), false),
            ("$1,299.99".to_string(), false),
            ("GB82 WEST 1234 5698 7654 32".to_string(), false),
            ("4111 1111 1111 1111".to_string(), false),
            ("1Z999AA10123456784".to_string(), false),
            ("Order #A-48392-XZ".to_string(), false),
            ("WH-1000XM4".to_string(), false),
            ("v2.3.1-beta.4".to_string(), false),
            ("report_final_v2.pdf".to_string(), false),
            ("C:\\Users\\Bob\\Documents\\report.docx".to_string(), false),
            ("/usr/local/bin/python3".to_string(), false),
            ("console.log(\"hello\");".to_string(), false),
            ("const userName = getUser();".to_string(), false),
            (".btn-primary:hover".to_string(), false),
            ("getUserById".to_string(), false),
            ("user_account_id".to_string(), false),
            ("main-content-wrapper".to_string(), false),
            ("calculateMonthlyRevenueTotals".to_string(), false),
            ("da39a3ee5e6b4b0d3255bfef95601890afd80709".to_string(), false),
            ("d41d8cd98f00b204e9800998ecf8427e".to_string(), false),
            ("550e8400-e29b-41d4-a716-446655440000".to_string(), false),
            ("10.0.0.1".to_string(), false),
            ("2001:db8::ff00:42:8329".to_string(), false),
            ("00:1A:2B:3C:4D:5E".to_string(), false),
            ("#0E9594".to_string(), false),
            ("rgb(14, 149, 148)".to_string(), false),
            ("how-to-bake-sourdough-bread".to_string(), false),
            ("# Shopping list\n- milk\n- eggs\n- **bread**".to_string(), false),
            ("🎉🎉 Congrats on the new job! 🎉".to_string(), false),
            ("مرحبا بك في المنزل".to_string(), false),
            ("今天天气很好，我们去公园吧".to_string(), false),
            ("Привет, как дела?".to_string(), false),
            ("SELECT * FROM users WHERE id = 1;".to_string(), false),
            ("npm install --save-dev typescript".to_string(), false),
            ("docker run -p 8080:80 nginx:latest".to_string(), false),
            ("const apiKey = process.env.OPENAI_API_KEY;".to_string(), false),
            ("Authorization: Bearer <your-token-here>".to_string(), false),
            ("ISBN 978-3-16-148410-0".to_string(), false),
            ("40.7128, -74.0060".to_string(), false),
            ("a3f9c12".to_string(), false),
        ]
    }

    /// Precision and recall, a masked clip counting as "a password" (that
    /// is what the user sees), and the cases it gets wrong.
    fn score(cases: &[(String, bool, Option<Secrecy>)]) -> (f64, f64, Vec<String>) {
        let (mut tp, mut fp, mut fn_) = (0, 0, 0);
        let mut wrong = Vec::new();
        for (text, want, exact) in cases {
            let got = judge(text, false);
            match (*want, got.masked()) {
                (true, true) => tp += 1,
                (false, true) => fp += 1,
                (true, false) => fn_ += 1,
                (false, false) => {}
            }
            if exact.map_or(got.masked() != *want, |e| got != e) {
                wrong.push(format!("{got:?} for {text:?}"));
            }
        }
        (tp as f64 / (tp + fp).max(1) as f64, tp as f64 / (tp + fn_).max(1) as f64, wrong)
    }

    /// The rules' own table: every case exactly as it should be.
    #[test]
    fn the_table() {
        let cases: Vec<_> = table().into_iter().map(|(t, v)| (t, v.masked(), Some(v))).collect();
        assert!(cases.len() >= 60, "{} cases", cases.len());
        let (precision, recall, wrong) = score(&cases);
        println!("the table, {} cases: precision {precision:.3}, recall {recall:.3}", cases.len());
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// The cases written apart: how the rules do on text they were not
    /// made from. Its misses are known (a kebab-case name is a slug or a
    /// passphrase alike, so it shows masked, unsure).
    #[test]
    fn the_cases_written_apart() {
        let cases: Vec<_> = written_apart().into_iter().map(|(t, v)| (t, v, None)).collect();
        let (precision, recall, wrong) = score(&cases);
        println!("written apart, {} cases: precision {precision:.3}, recall {recall:.3}", cases.len());
        for w in &wrong {
            println!("  {w}");
        }
        assert!(precision >= 0.97 && recall >= 0.97, "{wrong:#?}");
    }

    #[test]
    fn the_os_flag_wins() {
        assert_eq!(judge("plain words", true), Password);
        assert_eq!(judge("hello", true), Password);
        assert_eq!(judge("  \n", true), Plain, "nothing is nothing");
    }

    #[test]
    fn a_trailing_newline_is_no_line_break() {
        assert_eq!(judge("9fG!kQ2#vB7z\r\n", false), Password);
    }
}
