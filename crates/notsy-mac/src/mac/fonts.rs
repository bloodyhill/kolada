//! The fonts Notsy draws with on the Mac, named file by file: nothing scans
//! the system's fonts at launch. Missing ones are skipped by the loader.

use std::path::PathBuf;

use notsy_text::FontSet;

const SYSTEM: &str = "/System/Library/Fonts";
const SUPPLEMENTAL: &str = "/System/Library/Fonts/Supplemental";

/// The font set for the Font option (0 sans serif, 1 serif, 2 monospace):
/// a core set for the first screen, and fallbacks the engine loads when text
/// first needs a script, symbol or emoji the core lacks (or a face the
/// Font option moves to, which `Engine::set_family` loads when it is
/// not there).
pub fn font_set(option: u8) -> FontSet {
    let at = |dir: &str, names: &[&str]| -> Vec<PathBuf> { names.iter().map(|n| PathBuf::from(dir).join(n)).collect() };
    let georgia = at(SUPPLEMENTAL, &["Georgia.ttf", "Georgia Bold.ttf", "Georgia Italic.ttf", "Georgia Bold Italic.ttf"]);
    // The app's own text and sans notes, in the weights it uses; code. The
    // serif option sets the note in Georgia.
    let mut now = at(SYSTEM, &["HelveticaNeue.ttc", "Menlo.ttc"]);
    if option == 1 {
        now.extend(georgia.clone());
    }
    // What Helvetica Neue lacks: colour emoji, symbols, Arabic, Hebrew, the
    // CJK scripts; and the faces the other options use.
    let mut later = at(
        SYSTEM,
        &["Apple Color Emoji.ttc", "Apple Symbols.ttf", "GeezaPro.ttc", "AppleSDGothicNeo.ttc", "Hiragino Sans GB.ttc", "\u{30D2}\u{30E9}\u{30AE}\u{30CE}\u{89D2}\u{30B4}\u{30B7}\u{30C3}\u{30AF} W3.ttc", "CJKSymbolsFallback.ttc"],
    );
    later.extend(at(SUPPLEMENTAL, &["NewPeninimMT.ttc", "Arial Unicode.ttf", "Arial.ttf", "Arial Bold.ttf", "Arial Italic.ttf"]));
    if option != 1 {
        later.extend(georgia);
    }
    let text = match option {
        1 => "Georgia",
        2 => "Menlo",
        _ => "Helvetica Neue",
    };
    FontSet {
        files: now,
        fallback: later,
        // The icons ship inside the program: Windows draws its icons from a
        // font Windows has, so the same private-use codepoints are drawn
        // from a font built for Notsy (tools/iconfont, Lucide and Qlementine icons).
        data: vec![include_bytes!("../../../../assets/fonts/NotsyIcons.ttf")],
        ui: "Helvetica Neue".into(),
        text: text.into(),
        code: "Menlo".into(),
        icon: "Notsy Icons".into(),
    }
}

/// The Mac's fonts, for the model's `Fonts` service.
pub struct MacFonts;

impl notsy_app::services::Fonts for MacFonts {
    fn font_set(&self, option: u8) -> FontSet {
        font_set(option)
    }

    /// The family notes are set in for a font option (sans 0, serif 1, mono
    /// 2), and the files that hold it when launch did not load them.
    fn note_family(&self, option: u8) -> (String, Vec<PathBuf>) {
        let supplemental = PathBuf::from(SUPPLEMENTAL);
        match option {
            1 => ("Georgia".into(), ["Georgia.ttf", "Georgia Bold.ttf", "Georgia Italic.ttf", "Georgia Bold Italic.ttf"].iter().map(|f| supplemental.join(f)).collect()),
            2 => ("Menlo".into(), Vec::new()),
            _ => ("Helvetica Neue".into(), Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests;
