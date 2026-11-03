//! Made-up exports of other apps, for tests and for trying an import by
//! hand (`kolada-win`'s welcome dump makes a set): never anyone's notes.

/// A zip holding `files` as they are (stored, not compressed).
pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            }
        }
        !c
    }
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let at = out.len() as u32;
        let head = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes());
            }
            h.extend_from_slice(&20u16.to_le_bytes());
            h.extend_from_slice(&0x0800u16.to_le_bytes());
            h.extend_from_slice(&[0; 6]);
            h.extend_from_slice(&crc(data).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if central {
                // comment length, disk, internal and external attributes
                h.extend_from_slice(&[0; 10]);
                h.extend_from_slice(&at.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(head(0x0403_4b50, false));
        out.extend_from_slice(data);
        central.extend(head(0x0201_4b50, true));
    }
    let (at, len) = (out.len() as u32, central.len() as u32);
    out.extend(central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

/// An Evernote export of `n` notes ("Note 1", ...), the first with a
/// picture.
pub fn enex(n: usize) -> String {
    let mut notes = String::new();
    for i in 1..=n {
        let picture = if i == 1 {
            // A 1x1 PNG, base64.
            "<resource><data encoding=\"base64\">iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==</data><mime>image/png</mime><resource-attributes><file-name>dot.png</file-name></resource-attributes></resource>"
        } else {
            ""
        };
        notes.push_str(&format!(
            "<note><title>Note {i}</title><content><![CDATA[<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE en-note SYSTEM \"http://xml.evernote.com/pub/enml2.dtd\"><en-note><div>Line {i} of a made-up note.</div></en-note>]]></content><created>20200101T120000Z</created><updated>20210101T120000Z</updated>{picture}</note>"
        ));
    }
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE en-export SYSTEM \"http://xml.evernote.com/pub/evernote-export4.dtd\">\n<en-export export-date=\"20261005T120000Z\" application=\"Evernote\">{notes}</en-export>")
}

/// A Notion "Markdown & CSV" export of `n` pages ("Page 1", ...), the
/// second inside the first.
pub fn notion(n: usize) -> Vec<u8> {
    let id = |i: usize| format!("{i:032x}");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 1..=n {
        let name = if i == 2 { format!("Page 1 {}/Page 2 {}.md", id(1), id(2)) } else { format!("Page {i} {}.md", id(i)) };
        files.push((name, format!("# Page {i}\n\nA made-up page.\n").into_bytes()));
    }
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    zip(&refs)
}
