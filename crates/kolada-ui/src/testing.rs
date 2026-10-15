//! Fonts for tests: the OS's own, as close as it has to what the app uses.

use std::path::PathBuf;

use kolada_text::FontSet;

pub fn fonts() -> FontSet {
    let at = |dir: &str, names: &[&str]| -> Vec<PathBuf> { names.iter().map(|n| PathBuf::from(dir).join(n)).filter(|p| p.is_file()).collect() };
    if cfg!(windows) {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()) + r"\Fonts";
        return FontSet {
            files: at(&dir, &["segoeui.ttf", "segoeuib.ttf", "segoeuii.ttf", "segoeuiz.ttf", "seguisb.ttf", "SegoeIcons.ttf", "segmdl2.ttf", "CascadiaMono.ttf", "consola.ttf", "consolab.ttf", "consolai.ttf"]),
            fallback: at(&dir, &["seguisym.ttf", "seguiemj.ttf", "YuGothR.ttc", "malgun.ttf", "msyh.ttc", "SegUIVar.ttf"]), data: Vec::new(),
            ui: "Segoe UI".into(),
            text: "Segoe UI".into(),
            code: if PathBuf::from(&dir).join("CascadiaMono.ttf").is_file() { "Cascadia Mono" } else { "Consolas" }.into(),
            icon: if PathBuf::from(&dir).join("SegoeIcons.ttf").is_file() { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }.into(),
        };
    }
    if cfg!(target_os = "macos") {
        return FontSet {
            files: at("/System/Library/Fonts", &["HelveticaNeue.ttc", "Menlo.ttc"]),
            fallback: at("/System/Library/Fonts", &["Apple Color Emoji.ttc", "Apple Symbols.ttf", "GeezaPro.ttc"]), data: vec![include_bytes!("../../../assets/fonts/KoladaIcons.ttf")],
            ui: "Helvetica Neue".into(),
            text: "Helvetica Neue".into(),
            code: "Menlo".into(),
            icon: "Kolada Icons".into(),
        };
    }
    FontSet {
        files: at("/usr/share/fonts/truetype/dejavu", &["DejaVuSans.ttf", "DejaVuSans-Bold.ttf", "DejaVuSans-Oblique.ttf", "DejaVuSansMono.ttf"]),
        // Colour emoji where the machine has them (fonts-noto-color-emoji).
        fallback: at("/usr/share/fonts/truetype/noto", &["NotoColorEmoji.ttf"]), data: Vec::new(),
        ui: "DejaVu Sans".into(),
        text: "DejaVu Sans".into(),
        code: "DejaVu Sans Mono".into(),
        icon: "DejaVu Sans".into(),
    }
}

#[cfg(test)]
mod tests {
    /// The chrome's icons are private-use codepoints only the icon font has:
    /// a stand-in set without it draws them by fallback on every frame, and
    /// the speed gate times that rather than the app (the Mac's frame went
    /// from 0.05 to 0.14 ms on stand-in fonts naming Helvetica for icons).
    #[test]
    #[cfg(any(windows, target_os = "macos"))]
    fn the_stand_in_fonts_hold_the_icon_font_the_app_draws_with() {
        let set = super::fonts();
        let icon = set.icon.clone();
        let e = kolada_text::Engine::new(set);
        assert!(e.families().iter().any(|f| *f == icon), "{icon} is among {:?}", e.families());
        assert!(icon != "Helvetica Neue", "icons are not drawn from a text face");
    }
}
