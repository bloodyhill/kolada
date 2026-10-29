//! The pairing code, `F4FS-2RSA`: 8 characters of Crockford's base32.
//!
//! The first 3 are the server's slot, which only routes the two devices to
//! each other. The last 5 are the secret: made on the device that shows the
//! code, used only as SPAKE2's password, and never sent anywhere. Input
//! forgives the usual slips: lower case, `O` for `0`, `I` and `L` for `1`,
//! and any dashes or spaces.

use crate::crypto::random;

pub const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
pub const SLOT_LEN: usize = 3;
pub const SECRET_LEN: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    pub slot: String,
    pub secret: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeError {
    /// Not 8 characters once dashes and spaces are gone.
    Length,
    /// A character that isn't in the alphabet, even forgivingly.
    Character,
}

impl Code {
    /// As shown: `F4FS-2RSA`.
    pub fn show(&self) -> String {
        let all = format!("{}{}", self.slot, self.secret);
        format!("{}-{}", &all[..4], &all[4..])
    }

    /// SPAKE2's password: the secret, bound to its slot, so a transcript
    /// from one slot is no use against another.
    pub fn password(&self) -> Vec<u8> {
        format!("{}|{}", self.secret, self.slot).into_bytes()
    }
}

/// A fresh secret: 5 characters drawn evenly (32 divides 256).
pub fn new_secret() -> String {
    let b: [u8; SECRET_LEN] = random();
    b.iter().map(|x| ALPHABET[(x & 31) as usize] as char).collect()
}

/// One character, read forgivingly, or None.
pub fn normal(c: char) -> Option<char> {
    let c = c.to_ascii_uppercase();
    let c = match c {
        'O' => '0',
        'I' | 'L' => '1',
        _ => c,
    };
    ALPHABET.contains(&(c as u8)).then_some(c)
}

pub fn parse(s: &str) -> Result<Code, CodeError> {
    let mut out = String::new();
    for c in s.chars() {
        if c == '-' || c.is_whitespace() {
            continue;
        }
        match normal(c) {
            Some(c) => out.push(c),
            None => return Err(CodeError::Character),
        }
    }
    if out.len() != SLOT_LEN + SECRET_LEN {
        return Err(CodeError::Length);
    }
    Ok(Code { slot: out[..SLOT_LEN].to_string(), secret: out[SLOT_LEN..].to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_as_two_groups_of_four() {
        let c = Code { slot: "F4F".into(), secret: "S2RSA".into() };
        assert_eq!(c.show(), "F4FS-2RSA");
    }

    #[test]
    fn reads_forgivingly() {
        let want = Code { slot: "F4F".into(), secret: "S2RSA".into() };
        for s in ["F4FS-2RSA", "f4fs2rsa", "F4FS 2RSA", " f4f-s2r-sa "] {
            assert_eq!(parse(s), Ok(want.clone()), "{s}");
        }
        assert_eq!(parse("0OIL1-ABC").unwrap().slot, "001");
        assert_eq!(parse("0OIL1-ABC").unwrap().secret, "11ABC");
    }

    #[test]
    fn refuses_what_isnt_a_code() {
        assert_eq!(parse("F4FS-2RS"), Err(CodeError::Length));
        assert_eq!(parse("F4FS-2RSAA"), Err(CodeError::Length));
        assert_eq!(parse("F4FS-2RSU"), Err(CodeError::Character));
        assert_eq!(parse("F4FS-2R!A"), Err(CodeError::Character));
    }

    #[test]
    fn secrets_use_the_whole_alphabet_and_nothing_else() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..400 {
            let s = new_secret();
            assert_eq!(s.len(), SECRET_LEN);
            for c in s.chars() {
                assert!(ALPHABET.contains(&(c as u8)));
                seen.insert(c);
            }
        }
        assert_eq!(seen.len(), 32);
    }

    #[test]
    fn the_password_binds_the_slot() {
        let a = Code { slot: "AAA".into(), secret: "S2RSA".into() };
        let b = Code { slot: "AAB".into(), secret: "S2RSA".into() };
        assert_ne!(a.password(), b.password());
    }
}
