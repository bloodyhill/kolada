//! Tests for another app's exports coming in on the Mac as on Windows
//! (`kolada-win` `convert.rs` `import`): a Notion "Markdown & CSV" `.zip`
//! (named `Export-<id>` by Notion, so its folder is "Notion"), Google
//! Keep's Takeout `.zip` ("From Google Keep") and Bear's `.bear2bk` ("From
//! Bear"), taken from the drop, the settings Import and Open With alike. The importers are
//! shared (`kolada-convert`); the drop handler is `arrive.rs`.

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
        let d = std::env::temp_dir().join(format!("kolada-app-imports-{}-{name}", std::process::id()));
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

    #[test]
    fn a_keep_takeout_zip_is_told_from_notions_and_lands_in_from_google_keep() {
        let (from, v) = (dir("keep-from"), dir("keep"));
        let note = r#"{"title": "قائمة التسوق", "listContent": [{"text": "حليب", "isChecked": true}, {"text": "خبز", "isChecked": false}], "isTrashed": false, "isArchived": false, "createdTimestampUsec": 1600000000000000, "userEditedTimestampUsec": 1700000000000000, "labels": [{"name": "Home"}]}"#;
        let gone = r#"{"title": "Gone", "textContent": "x", "isTrashed": true, "userEditedTimestampUsec": 1}"#;
        let file = from.join("takeout-20261101T000000Z-001.zip");
        std::fs::write(&file, zip(&[("Takeout/Keep/a.json", note.as_bytes()), ("Takeout/Keep/b.json", gone.as_bytes())])).unwrap();
        assert!(is_document(&file));
        let done = convert_documents(&[file], &v, &v);
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(done.notes.len(), 1, "the trashed note is left out");
        let text = std::fs::read_to_string(v.join("From Google Keep").join("قائمة التسوق.md")).unwrap();
        assert_eq!(text, "# قائمة التسوق\n\n- [x] حليب\n- [ ] خبز\n\n#Home\n");
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn a_bear_backup_lands_in_from_bear_with_its_tags_and_pictures() {
        let (from, v) = (dir("bear-from"), dir("bear"));
        let file = from.join("Bear Notes 2026-11-01 at 10.00.bear2bk");
        std::fs::write(
            &file,
            zip(&[
                ("Idea #1.textbundle/text.markdown", b"# Idea #1\n\nTagged #work/2026 and #big idea#.\n\n![](assets/sketch.png)\n"),
                ("Idea #1.textbundle/info.json", br#"{"net.shinyfrog.bear": {"creationDate": "2026-01-02T03:04:05Z", "trashed": 0}}"#),
                ("Idea #1.textbundle/assets/sketch.png", b"PNG"),
            ]),
        )
        .unwrap();
        assert!(is_document(&file), "a .bear2bk is offered to the importers");
        let done = convert_documents(&[file], &v, &v);
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        let notes: Vec<_> = std::fs::read_dir(v.join("From Bear")).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(notes.len(), 1, "{notes:?}");
        let text = std::fs::read_to_string(&notes[0]).unwrap();
        assert_eq!(text, "# Idea #1\n\nTagged #work/2026 and #big-idea.\n\n![[sketch.png]]\n");
        assert_eq!(std::fs::read(v.join("sketch.png")).unwrap(), b"PNG");
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn a_converting_process_has_a_minute_and_two_seconds_a_megabyte() {
        use crate::model::arrive::convert_limit;
        let d = dir("limit");
        let (small, big) = (d.join("small.zip"), d.join("big.zip"));
        std::fs::write(&small, b"tiny").unwrap();
        std::fs::write(&big, vec![0u8; 10 * 1024 * 1024]).unwrap();
        assert_eq!(convert_limit(&[small.clone()]).as_secs(), 60);
        assert_eq!(convert_limit(&[small, big]).as_secs(), 80, "a large Takeout is not cut off at a flat minute and a half");
        let _ = std::fs::remove_dir_all(d);
    }

    /// What an export became is said on the Mac and Linux too, where it is
    /// converted in a process of its own (or on a thread), not only in the
    /// app (iOS): the notes made, and what stayed behind (an attachment the
    /// export names but does not hold), carried back on the converting
    /// process's answer.
    #[test]
    fn an_import_says_what_came_in_and_what_stayed_behind_however_it_was_converted() {
        use crate::model::arrive::{convert_child, converted_words, read_converted, Converted};
        let (from, v) = (dir("said-from"), dir("said"));
        let trip = r#"{"title": "Trip", "textContent": "Walk the coast.", "attachments": [{"filePath": "missing.jpg", "mimetype": "image/jpeg"}], "isTrashed": false, "userEditedTimestampUsec": 1700000000000000}"#;
        let pack = r#"{"title": "Packing", "listContent": [{"text": "Boots", "isChecked": true}], "isTrashed": false, "userEditedTimestampUsec": 1700000000000000}"#;
        let file = from.join("takeout-20261103T000000Z-001.zip");
        std::fs::write(&file, zip(&[("Takeout/Keep/a.json", trip.as_bytes()), ("Takeout/Keep/b.json", pack.as_bytes())])).unwrap();
        // The converting process's answer carries what stayed behind.
        let args: Vec<std::ffi::OsString> = vec![v.join("child").into(), v.join("child").into(), file.clone().into()];
        let mut out = Vec::new();
        assert_eq!(convert_child(&args, &mut out), 0);
        let done = read_converted(&String::from_utf8(out).unwrap());
        assert_eq!((done.notes.len(), done.failed.len(), done.attachments_left), (2, 0, 1));
        assert_eq!(converted_words(&done), "Made 2 notes; 1 attachment stayed behind");
        // Dropped on a model that converts on a thread, as the Mac does
        // without its program: said, not left at "Reading it...".
        let mut m = crate::model::Model::new(v.clone(), true);
        m.frame(2000, 1440, 2.0);
        m.start_arrivals(|| {});
        m.drop_files(vec![file], None);
        m.wait_for_arrivals();
        assert_eq!(m.whispered(), Some("Made 2 notes; 1 attachment stayed behind"));
        // The other words.
        let words = |notes: usize, failed: &[(&str, &str)], unwritten, left| {
            converted_words(&Converted { notes: vec![v.join("N.md"); notes], failed: failed.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), unwritten, attachments_left: left })
        };
        assert_eq!(words(3, &[], 0, 0), "Made 3 notes");
        assert_eq!(words(3, &[("x.pdf", "it is damaged")], 0, 2), "Made 3 notes; 1 could not be read, 2 attachments stayed behind");
        assert_eq!(words(3, &[], 2, 0), "Made 3 notes; 2 notes could not be written");
        assert_eq!(words(0, &[("x.pdf", "it is damaged")], 0, 0), "Could not read x.pdf: it is damaged");
        assert_eq!(words(0, &[], 4, 0), "No notes came in; 4 notes could not be written");
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }
}
