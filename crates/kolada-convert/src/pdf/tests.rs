use super::*;

#[test]
fn refuses_what_is_not_a_pdf() {
    assert!(convert(b"not a pdf").is_err());
    assert!(convert(b"%PDF-1.4\n%junk only").is_err());
}

/// What every sample PDF says, as the note should have it.
const LAUNCH: &str = "# Launch plan\n\nShip the **public beta** on *October 12*, see the post at https://notsy.app/beta. This paragraph is long enough to wrap onto a second line on the page, so the converter has to join the lines it was broken into back into one paragraph.\n\n## What ships\n\n- Live Markdown\n- Pictures\n1. Write the notes\n2. Record the video\n\n## Second page follows\n\nThe second page has its own paragraph, and the converter keeps the pages in order.\n";

#[test]
fn reads_fonts_embedded_as_most_pdfs_have_them() {
    let n = convert(include_bytes!("../../testdata/sample-embedded.pdf")).unwrap();
    assert_eq!(n.markdown, LAUNCH);
    assert_eq!(n.title.as_deref(), Some("Launch plan"));
}

#[test]
fn reads_objects_packed_in_object_streams() {
    let n = convert(include_bytes!("../../testdata/sample-objstm.pdf")).unwrap();
    assert_eq!(n.markdown, LAUNCH);
}

#[test]
fn reads_the_standard_fonts() {
    let n = convert(include_bytes!("../../testdata/sample-standard.pdf")).unwrap();
    assert_eq!(n.markdown, "# Plain fonts\n\nCaf\u{e9} menus cost 5 euros, a test of WinAnsi encoding with \"quotes\" and an \u{fc}mlaut.\n");
}

#[test]
fn reads_pdfs_locked_against_copying_and_pages_turned_upright() {
    let plain = "# Plain fonts\n\nCaf\u{e9} menus cost 5 euros, a test of WinAnsi encoding with \"quotes\" and an \u{fc}mlaut.\n";
    for bytes in [&include_bytes!("../../testdata/sample-rc4.pdf")[..], include_bytes!("../../testdata/sample-aes128.pdf"), include_bytes!("../../testdata/sample-rotated.pdf")] {
        assert_eq!(convert(bytes).unwrap().markdown, plain);
    }
    let refused = convert(include_bytes!("../../testdata/sample-aes256.pdf")).err().unwrap();
    assert!(refused.0.contains("AES-256"), "{}", refused.0);
}

#[test]
fn reads_an_invoices_amounts_under_their_headings() {
    let m = convert(include_bytes!("../../testdata/sample-invoice.pdf")).unwrap().markdown;
    assert_eq!(
        m,
        "# Invoice 1042\n\n| Item | Qty | Rate | Amount |\n| --- | --- | --- | --- |\n| Design review | 2 | 150.00 | 300.00 |\n| Launch video | 1 | 1,200.00 | 1,200.00 |\n\nPaid within 30 days of the date above.\n"
    );
}

#[test]
fn reads_a_browsers_pdf_with_its_lists_table_links_and_arabic() {
    let m = convert(include_bytes!("../../testdata/sample-chromium.pdf")).unwrap().markdown;
    for want in [
        "# Launch plan\n\nShip the **public beta** on *October 12*, see [the post](https://notsy.app/beta).",
        "## What ships\n\n- Live Markdown\n- Pictures\n    - pasted\n    - dropped\n1. Write the notes\n2. Record the video",
        "| Item | Owner | Due |\n| --- | --- | --- |\n| Press kit | Sam | Oct 5 |\n| Pricing page | Lena | Oct 8 |",
        "> Speed is a feature.",
        "```\ncargo build --release\nnotsy --trace\n```",
        "\u{0645}\u{0644}\u{0627}\u{062d}\u{0638}\u{0629} \u{0642}\u{0635}\u{064a}\u{0631}\u{0629} \u{0628}\u{0627}\u{0644}\u{0639}\u{0631}\u{0628}\u{064a}\u{0629}",
    ] {
        assert!(m.contains(want), "missing {want:?} in:\n{m}");
    }
    assert!(!m.contains("file:"), "links to files on the machine that made it:\n{m}");
}

#[test]
fn damage_is_survived() {
    let good = include_bytes!("../../testdata/sample-objstm.pdf");
    // Cut short, its cross-references lost: found by scanning.
    let _ = convert(&good[..good.len() * 2 / 3]);
    // Scrambled here and there: an error or text, never a panic.
    let mut bytes = good.to_vec();
    for (i, b) in bytes.iter_mut().enumerate().skip(200) {
        if i % 211 == 0 {
            *b ^= 0x5A;
        }
    }
    let _ = convert(&bytes);
}

/// Prints a page's glyphs by line, for looking at a layout by eye
/// (KOLADA_PDF=file, absolute).
#[test]
#[ignore = "reads the file named by KOLADA_PDF"]
fn print_glyphs() {
    let bytes = std::fs::read(std::env::var("KOLADA_PDF").unwrap()).unwrap();
    let file = File::open(&bytes).unwrap();
    let catalog = file.entry(&file.trailer, "Root").unwrap();
    let pages_root = file.entry(catalog.as_dict().unwrap(), "Pages").unwrap();
    let mut leaves = Vec::new();
    collect(&file, &pages_root, &Inherited::default(), &mut leaves, &mut Vec::new(), 0);
    let mut reader = content::Reader::new(&file);
    let page_number: usize = std::env::var("KOLADA_PAGE").ok().and_then(|p| p.parse().ok()).unwrap_or(1);
    let (page, inherited) = &leaves[page_number - 1];
    let (ctm, ..) = upright(inherited.box_.unwrap_or([0.0, 0.0, 612.0, 792.0]), inherited.rotate.unwrap_or(0));
    reader.run(&contents(&file, page), inherited.resources.as_ref().unwrap(), ctm);
    let mask = std::env::var("KOLADA_MASK").is_ok();
    let mut last_y = f32::MAX;
    let mut line = String::new();
    for g in &reader.glyphs {
        if (g.y - last_y).abs() > 1.0 {
            if !line.is_empty() {
                println!("{line}");
            }
            line = format!("y {:7.2} size {:5.2}: ", g.y, g.size);
            last_y = g.y;
        }
        line.push_str(&format!("[{:.1}+{:.1} {}{}]", g.x, g.w, if mask { masked(&g.text) } else { g.text.clone() }, if g.bold { "*" } else { "" }));
    }
    println!("{line}");
}

/// Someone's own document, its shape kept and its words not: letters as x
/// (Arabic as ب), digits as 9.
#[cfg(test)]
pub fn masked(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{0600}'..='\u{06FF}' | '\u{FB50}'..='\u{FEFF}' if c.is_alphabetic() => 'ب',
            c if c.is_alphabetic() => 'x',
            c if c.is_numeric() => '9',
            c => c,
        })
        .collect()
}
