//! The fonts Kolada draws with on Linux, named file by file: nothing scans
//! the system's fonts (no fontconfig) at launch. Each family is looked for
//! under the few folders the common distributions put it in (Debian and
//! Ubuntu, Fedora, Arch, openSUSE): the first family of each kind that is
//! there is the one used. Fonts for other scripts and emoji are listed for
//! the engine to load on first need; the ones a machine lacks are skipped.

use std::path::{Path, PathBuf};

use kolada_text::FontSet;

/// Where each kind of font lives, by distribution.
const NOTO: &[&str] = &["/usr/share/fonts/truetype/noto", "/usr/share/fonts/opentype/noto", "/usr/share/fonts/noto", "/usr/share/fonts/google-noto", "/usr/share/fonts/truetype"];
const NOTO_CJK: &[&str] = &["/usr/share/fonts/opentype/noto", "/usr/share/fonts/noto-cjk", "/usr/share/fonts/google-noto-cjk", "/usr/share/fonts/google-noto-sans-cjk-fonts", "/usr/share/fonts/truetype/noto"];
const EMOJI: &[&str] = &["/usr/share/fonts/truetype/noto", "/usr/share/fonts/noto", "/usr/share/fonts/google-noto-color-emoji-fonts", "/usr/share/fonts/truetype"];
const DEJAVU: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu",
    "/usr/share/fonts/dejavu-sans-fonts",
    "/usr/share/fonts/dejavu-serif-fonts",
    "/usr/share/fonts/dejavu-sans-mono-fonts",
    "/usr/share/fonts/TTF",
    "/usr/share/fonts/dejavu",
    "/usr/share/fonts/truetype",
];
const LIBERATION: &[&str] = &[
    "/usr/share/fonts/truetype/liberation",
    "/usr/share/fonts/truetype/liberation2",
    "/usr/share/fonts/liberation-sans",
    "/usr/share/fonts/liberation-serif",
    "/usr/share/fonts/liberation-mono",
    "/usr/share/fonts/liberation",
    "/usr/share/fonts/TTF",
    "/usr/share/fonts/truetype",
];
const FREE: &[&str] = &["/usr/share/fonts/truetype/freefont", "/usr/share/fonts/gnu-free", "/usr/share/fonts/TTF", "/usr/share/fonts/truetype"];
/// Korean where Noto's CJK is not installed: Nanum (fonts-nanum, the
/// distributions' usual Korean font) or Un (fonts-unfonts-core).
const KOREAN: &[&str] = &["/usr/share/fonts/truetype/nanum", "/usr/share/fonts/nanum", "/usr/share/fonts/naver-nanum", "/usr/share/fonts/TTF", "/usr/share/fonts/truetype/unfonts-core", "/usr/share/fonts/un-core"];
const DROID: &[&str] =&["/usr/share/fonts/truetype/droid", "/usr/share/fonts/google-droid-sans-fonts", "/usr/share/fonts/droid", "/usr/share/fonts/TTF", "/usr/share/fonts/truetype"];

/// A family and its files (regular first: the family is there when it is).
struct Family {
    name: &'static str,
    files: &'static [&'static str],
    dirs: &'static [&'static str],
}

const fn family(name: &'static str, files: &'static [&'static str], dirs: &'static [&'static str]) -> Family {
    Family { name, files, dirs }
}

/// Each kind's families, best first. Liberation (Arial's widths) comes
/// before DejaVu, which is wide: labels laid out for Segoe UI and Helvetica
/// Neue overflow their columns in it ("Show Markdown" in the settings).
const SANS: &[Family] = &[
    family("Noto Sans", &["NotoSans-Regular.ttf", "NotoSans-Bold.ttf", "NotoSans-Italic.ttf", "NotoSans-BoldItalic.ttf", "NotoSans-SemiBold.ttf"], NOTO),
    family("Liberation Sans", &["LiberationSans-Regular.ttf", "LiberationSans-Bold.ttf", "LiberationSans-Italic.ttf", "LiberationSans-BoldItalic.ttf"], LIBERATION),
    family("DejaVu Sans", &["DejaVuSans.ttf", "DejaVuSans-Bold.ttf", "DejaVuSans-Oblique.ttf", "DejaVuSans-BoldOblique.ttf"], DEJAVU),
    family("FreeSans", &["FreeSans.ttf", "FreeSansBold.ttf", "FreeSansOblique.ttf", "FreeSansBoldOblique.ttf"], FREE),
];
const SERIF: &[Family] = &[
    family("Noto Serif", &["NotoSerif-Regular.ttf", "NotoSerif-Bold.ttf", "NotoSerif-Italic.ttf", "NotoSerif-BoldItalic.ttf"], NOTO),
    family("DejaVu Serif", &["DejaVuSerif.ttf", "DejaVuSerif-Bold.ttf", "DejaVuSerif-Italic.ttf", "DejaVuSerif-BoldItalic.ttf"], DEJAVU),
    family("Liberation Serif", &["LiberationSerif-Regular.ttf", "LiberationSerif-Bold.ttf", "LiberationSerif-Italic.ttf", "LiberationSerif-BoldItalic.ttf"], LIBERATION),
    family("FreeSerif", &["FreeSerif.ttf", "FreeSerifBold.ttf", "FreeSerifItalic.ttf", "FreeSerifBoldItalic.ttf"], FREE),
];
const MONO: &[Family] = &[
    family("DejaVu Sans Mono", &["DejaVuSansMono.ttf", "DejaVuSansMono-Bold.ttf", "DejaVuSansMono-Oblique.ttf", "DejaVuSansMono-BoldOblique.ttf"], DEJAVU),
    family("Noto Sans Mono", &["NotoSansMono-Regular.ttf", "NotoSansMono-Bold.ttf"], NOTO),
    family("Liberation Mono", &["LiberationMono-Regular.ttf", "LiberationMono-Bold.ttf", "LiberationMono-Italic.ttf", "LiberationMono-BoldItalic.ttf"], LIBERATION),
    family("FreeMono", &["FreeMono.ttf", "FreeMonoBold.ttf", "FreeMonoOblique.ttf", "FreeMonoBoldOblique.ttf"], FREE),
];
/// What the families above lack, loaded when text first needs it: colour
/// emoji, Arabic, Hebrew, the CJK scripts, symbols; then wide fonts that
/// cover much of the rest.
const MORE: &[Family] = &[
    family("Noto Color Emoji", &["NotoColorEmoji.ttf"], EMOJI),
    family("Noto Sans Arabic", &["NotoSansArabic-Regular.ttf", "NotoSansArabic-Bold.ttf", "NotoNaskhArabic-Regular.ttf", "NotoNaskhArabic-Bold.ttf"], NOTO),
    family("Noto Sans Hebrew", &["NotoSansHebrew-Regular.ttf", "NotoSansHebrew-Bold.ttf"], NOTO),
    family("Noto Sans CJK", &["NotoSansCJK-Regular.ttc", "NotoSansCJK-Bold.ttc", "NotoSansCJKsc-Regular.otf", "NotoSansCJKjp-Regular.otf", "NotoSansCJKkr-Regular.otf"], NOTO_CJK),
    family("Noto Sans Symbols", &["NotoSansSymbols-Regular.ttf", "NotoSansSymbols2-Regular.ttf", "NotoSansMath-Regular.ttf"], NOTO),
    family("DejaVu Sans", &["DejaVuSans.ttf", "DejaVuSans-Bold.ttf"], DEJAVU),
    family("Droid Sans Fallback", &["DroidSansFallbackFull.ttf", "DroidSansFallback.ttf"], DROID),
    // After Droid's Chinese: Korean fonts hold Han characters too, drawn
    // as Korean writes them, which Chinese and Japanese text should not take.
    family("NanumGothic", &["NanumGothic.ttf", "NanumGothicBold.ttf", "NanumGothic-Regular.ttf", "NanumGothic-Bold.ttf"], KOREAN),
    family("UnDotum", &["UnDotum.ttf", "UnDotumBold.ttf"], KOREAN),
    family("FreeSans", &["FreeSans.ttf", "FreeSerif.ttf"], FREE),
];

/// `name` in the first of `dirs` that has it.
fn locate(name: &str, dirs: &[&str]) -> Option<PathBuf> {
    dirs.iter().map(|d| Path::new(d).join(name)).find(|p| p.is_file())
}

/// The first family of `kinds` this machine has, with its files.
fn first(kinds: &'static [Family]) -> Option<(&'static str, Vec<PathBuf>)> {
    kinds.iter().find_map(|f| {
        let regular = locate(f.files[0], f.dirs)?;
        let dir = regular.parent()?.to_path_buf();
        // The other faces are beside the regular one, or elsewhere in its folders.
        let rest = f.files[1..].iter().filter_map(|n| Some(dir.join(n)).filter(|p| p.is_file()).or_else(|| locate(n, f.dirs)));
        Some((f.name, std::iter::once(regular).chain(rest).collect()))
    })
}

/// Every file `kinds` may have, where it may be: for the engine's fallback,
/// which skips those missing (so nothing is looked up before it is needed).
fn everywhere(kinds: &[Family]) -> Vec<PathBuf> {
    kinds.iter().flat_map(|f| f.files.iter().flat_map(move |n| f.dirs.iter().map(move |d| Path::new(d).join(n)))).collect()
}

/// The families found for sans, serif and monospace text.
struct Found {
    sans: Option<(&'static str, Vec<PathBuf>)>,
    serif: Option<(&'static str, Vec<PathBuf>)>,
    mono: Option<(&'static str, Vec<PathBuf>)>,
}

fn found() -> &'static Found {
    static FOUND: std::sync::OnceLock<Found> = std::sync::OnceLock::new();
    FOUND.get_or_init(|| {
        let found = Found { sans: first(SANS), serif: first(SERIF), mono: first(MONO) };
        if found.sans.is_none() {
            eprintln!("kolada: no fonts found (install fonts-dejavu or fonts-noto)");
        }
        found
    })
}

/// The font set for the Font option (0 sans serif, 1 serif, 2 monospace):
/// the core faces for the first screen, and fallbacks the engine loads when
/// text first needs a script, symbol or emoji they lack (or the face the
/// Font option moves to).
pub fn font_set(option: u8) -> FontSet {
    let f = found();
    let name = |k: &Option<(&'static str, Vec<PathBuf>)>| k.as_ref().map_or("", |(n, _)| *n).to_string();
    let files = |k: &Option<(&'static str, Vec<PathBuf>)>| k.as_ref().map(|(_, f)| f.clone()).unwrap_or_default();
    let (sans, serif, mono) = (name(&f.sans), name(&f.serif), name(&f.mono));
    let mut now = files(&f.sans);
    now.extend(files(&f.mono));
    let mut later = Vec::new();
    if option == 1 {
        now.extend(files(&f.serif));
    } else {
        later.extend(files(&f.serif));
    }
    later.extend(everywhere(MORE));
    let text = match option {
        1 if !serif.is_empty() => serif,
        2 if !mono.is_empty() => mono.clone(),
        _ => sans.clone(),
    };
    FontSet {
        files: now,
        fallback: later,
        // The icons ship inside the program (tools/iconfont: the same
        // private-use codepoints Windows draws from Segoe Fluent Icons).
        data: vec![include_bytes!("../../../../assets/fonts/KoladaIcons.ttf")],
        ui: sans,
        text,
        code: mono,
        icon: "Kolada Icons".into(),
    }
}

/// Linux's fonts, for the model's `Fonts` service.
pub struct LinuxFonts;

impl kolada_app::services::Fonts for LinuxFonts {
    fn font_set(&self, option: u8) -> FontSet {
        font_set(option)
    }

    /// The family notes are set in for a font option (sans 0, serif 1, mono
    /// 2), and the files that hold it when launch did not load them.
    fn note_family(&self, option: u8) -> (String, Vec<PathBuf>) {
        let f = found();
        let pick = match option {
            1 => f.serif.as_ref().or(f.sans.as_ref()),
            2 => f.mono.as_ref().or(f.sans.as_ref()),
            _ => f.sans.as_ref(),
        };
        match pick {
            Some((name, files)) if option == 1 => (name.to_string(), files.clone()),
            Some((name, _)) => (name.to_string(), Vec::new()),
            None => (String::new(), Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests;
