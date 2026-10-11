//! Tests for another app's exports coming in on the Mac as on Windows
//! (`notsy-win` `convert.rs` `import`): a Notion "Markdown & CSV" `.zip`
//! (named `Export-<id>` by Notion, so its folder is "Notion"), taken from
//! the drop, the settings Import and Open With alike. The importers are
//! shared (`notsy-convert`); the drop handler is `arrive.rs`.

#[cfg(test)]
mod tests {
    use crate::model::arrive::{convert_documents, is_document};
    use std::path::PathBuf;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    /// A zip of stored (uncompressed) files.
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
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
            let head = |sig: u32, is_central: bool| {
                let mut h = Vec::new();
                h.extend_from_slice(&sig.to_le_bytes());
                if is_central {
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
                if is_central {
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

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("notsy-app-imports-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_notion_export_becomes_notes_in_a_folder_named_notion_with_links_and_tables() {
        let (from, v) = (dir("notion-from"), dir("notion"));
        let trip = format!("# Trip\n\nSee [Packing](Trip%20{ID}/Packing%20{ID}.md).\n");
        let budget = "Item,Cost\nFlight,300\n";
        let bytes = zip(&[
            (&format!("Trip {ID}.md"), trip.as_bytes()),
            (&format!("Trip {ID}/Packing {ID}.md"), b"# Packing\n\nSocks\n"),
            (&format!("Trip {ID}/Budget {ID}.csv"), budget.as_bytes()),
        ]);
        let file = from.join(format!("Export-{ID}.zip"));
        std::fs::write(&file, bytes).unwrap();
        assert!(is_document(&file), "a .zip is offered to the importers");
        let done = convert_documents(&[file], &v, &v);
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(done.notes.len(), 3, "{:?}", done.notes);
        let trip = std::fs::read_to_string(v.join("Notion").join("Trip.md")).unwrap();
        assert!(trip.contains("[[Packing]]") && !trip.contains(ID), "links are wiki links, ids are off: {} chars", trip.len());
        assert!(v.join("Notion").join("Trip").join("Packing.md").is_file(), "a subpage goes in a folder beside its page");
        let table = std::fs::read_to_string(v.join("Notion").join("Trip").join("Budget.md")).unwrap();
        assert!(table.contains("| Item | Cost |") && table.contains("| Flight | 300 |"), "a database is a table: {} chars", table.len());
        // Something that is not an export is said so, and nothing else is lost.
        std::fs::write(from.join("plain.zip"), b"not a zip").unwrap();
        let done = convert_documents(&[from.join("plain.zip")], &v, &v);
        assert_eq!((done.notes.len(), done.failed.len()), (0, 1));
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }
}
