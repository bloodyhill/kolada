//! What a device is called and drawn as, the same on every platform.
//!
//! A device says what it is (its model: `windows-laptop`, `mac-desktop`,
//! `ipad`…) to the group in plain text, beside its sealed name. Its name
//! is the user's (Rename), else the computer's own, else, where that is
//! one a machine made up (`LAPTOP-B9TBEDG7`), a friendly one from its model.

/// The models a device says it is. Words outside these read by kind alone.
pub const MODELS: [&str; 9] = ["windows-laptop", "windows-desktop", "mac-laptop", "mac-desktop", "linux-laptop", "linux-desktop", "iphone", "ipad", "android"];

/// The longest name a device gives itself.
pub const NAME_MOST: usize = 40;

/// This build's model before the shell says more (its chassis, iPad).
pub fn built_model() -> &'static str {
    if cfg!(windows) {
        "windows-desktop"
    } else if cfg!(target_os = "macos") {
        "mac-desktop"
    } else if cfg!(target_os = "ios") {
        "iphone"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "linux-desktop"
    }
}

/// A desktop model for this build, a laptop if `laptop`.
pub fn desktop_model(laptop: bool) -> &'static str {
    match (built_model(), laptop) {
        ("windows-desktop", true) => "windows-laptop",
        ("mac-desktop", true) => "mac-laptop",
        ("linux-desktop", true) => "linux-laptop",
        (m, _) => m,
    }
}

/// How a device is drawn: `laptop`, `desktop`, `phone` or `tablet`; from
/// its model, else its kind (`desktop`, `ios`, `android`).
pub fn look(kind: &str, model: &str) -> &'static str {
    match model {
        "ipad" => "tablet",
        "iphone" | "android" => "phone",
        m if m.ends_with("-desktop") => "desktop",
        m if m.ends_with("-laptop") => "laptop",
        _ if kind == "ios" || kind == "android" => "phone",
        _ => "laptop",
    }
}

/// What a device is called when its name can't be read or was never
/// given: its model's, else its kind's. Never blank.
pub fn fallback(kind: &str, model: &str) -> &'static str {
    match model {
        "windows-laptop" => "Windows laptop",
        "windows-desktop" => "Windows PC",
        "mac-laptop" | "mac-desktop" => "Mac",
        "linux-laptop" => "Linux laptop",
        "linux-desktop" => "Linux PC",
        "iphone" => "iPhone",
        "ipad" => "iPad",
        "android" => "Android phone",
        _ => match kind {
            "ios" => "iPhone",
            "android" => "Android phone",
            "desktop" => "Computer",
            _ => "A device",
        },
    }
}

/// Whether a computer's name is one a machine made up rather than a
/// person: Windows' `DESKTOP-1A2B3C4` and `LAPTOP-B9TBEDG7`, `localhost`,
/// or a run of letters and digits like a serial number.
pub fn machine_like(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() || name.to_ascii_lowercase().starts_with("localhost") {
        return true;
    }
    let serial = |p: &str| {
        let digits = p.chars().filter(|c| c.is_ascii_digit()).count();
        let letters = p.chars().filter(|c| c.is_ascii_alphabetic()).count();
        p.chars().all(|c| c.is_ascii_alphanumeric()) && digits >= 1 && letters >= 1 && digits + letters >= 7 && !p.chars().any(|c| c.is_ascii_lowercase())
            || p.len() >= 8 && p.chars().all(|c| c.is_ascii_hexdigit()) && digits >= 2
    };
    let parts: Vec<&str> = name.split(['-', '_', '.']).collect();
    let made = ["DESKTOP", "LAPTOP", "WIN", "PC", "MININT", "WORKSTATION"];
    match parts.as_slice() {
        [head, tail] if made.contains(&head.to_ascii_uppercase().as_str()) && tail.chars().all(|c| c.is_ascii_alphanumeric()) && tail.chars().any(|c| c.is_ascii_digit()) => true,
        _ => parts.iter().any(|p| serial(p)),
    }
}

/// The name a device gives itself before the user names it: the
/// computer's own, unless a machine made it up; then its model's.
pub fn default_name(host: Option<&str>, model: &str) -> String {
    match host.map(str::trim) {
        Some(h) if !machine_like(h) => h.chars().take(NAME_MOST).collect(),
        _ => fallback("desktop", model).to_string(),
    }
}

/// A name the user typed, as kept: trimmed, one line, at most
/// `NAME_MOST` characters; None when nothing is left.
pub fn tidy(name: &str) -> Option<String> {
    let one: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t: String = one.trim().chars().take(NAME_MOST).collect();
    let t = t.trim_end().to_string();
    (!t.is_empty()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows' made-up names, localhost and serial-like names read as a
    /// machine's; names people give read as theirs.
    #[test]
    fn a_machines_name_is_told_from_a_persons() {
        for made in ["LAPTOP-B9TBEDG7", "DESKTOP-1A2B3C4", "WIN-8K2J3L4M5N", "localhost", "localhost.localdomain", "MININT-4F2KQ1", "a1b2c3d4e5f6", "PC-7H3K9"] {
            assert!(machine_like(made), "{made}");
        }
        for own in ["Alex's MacBook Air", "Sam-PC", "STUDIO", "Desk", "ThinkPad-X1", "ubuntu-2204", "Office iMac", "kitchen-laptop", "Mac mini"] {
            assert!(!machine_like(own), "{own}");
        }
    }

    /// The default name: the computer's unless made up, then the model's;
    /// a fallback is never blank and reads the model, else the kind.
    #[test]
    fn a_device_is_named_and_drawn_by_its_model() {
        assert_eq!(default_name(Some("LAPTOP-B9TBEDG7"), "windows-laptop"), "Windows laptop");
        assert_eq!(default_name(Some("DESKTOP-1A2B3C4"), "windows-desktop"), "Windows PC");
        assert_eq!(default_name(Some("Sam-PC"), "windows-desktop"), "Sam-PC");
        assert_eq!(default_name(None, "linux-desktop"), "Linux PC");
        assert_eq!(default_name(Some("localhost"), "linux-laptop"), "Linux laptop");
        assert_eq!(default_name(Some("Alex's MacBook Air"), "mac-laptop"), "Alex's MacBook Air");
        assert_eq!((fallback("ios", "ipad"), fallback("ios", ""), fallback("desktop", "mac-desktop"), fallback("desktop", ""), fallback("other", "")), ("iPad", "iPhone", "Mac", "Computer", "A device"));
        assert_eq!((look("ios", "ipad"), look("ios", ""), look("desktop", "windows-desktop"), look("desktop", "mac-laptop"), look("desktop", "")), ("tablet", "phone", "desktop", "laptop", "laptop"));
        assert!(MODELS.iter().all(|m| fallback("", m) != "A device"));
    }

    /// What the user types is kept as one trimmed line, never longer than
    /// the most; nothing left reverts to the default.
    #[test]
    fn a_typed_name_is_tidied() {
        assert_eq!(tidy("  Kitchen laptop \n").as_deref(), Some("Kitchen laptop"));
        assert_eq!(tidy("   "), None);
        assert_eq!(tidy(&"x".repeat(60)).map(|t| t.chars().count()), Some(NAME_MOST));
        assert_eq!(tidy("a\nb").as_deref(), Some("a b"));
    }
}
