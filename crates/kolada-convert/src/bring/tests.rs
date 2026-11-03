use super::*;
use crate::fixtures;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kolada-bring-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A documents converter that writes a note per file, as a shell's would.
fn convert(file: &Path, dir: &Path) -> Result<Vec<PathBuf>, String> {
    if file.extension().is_some_and(|e| e == "bad") {
        return Err("it is damaged".to_string());
    }
    let to = dir.join(format!("{}.md", file.file_stem().unwrap().to_string_lossy()));
    std::fs::write(&to, "# A document\n").map_err(|e| e.to_string())?;
    Ok(vec![to])
}

fn none(_: &Path, _: &Path) -> Result<Vec<PathBuf>, String> {
    unreachable!("no documents here")
}

/// Every file under `dir`, as paths from it with `/`, sorted.
fn tree(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                todo.push(e.path());
            } else {
                out.push(e.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn an_obsidian_vault_comes_with_its_pictures_and_not_its_settings() {
    let (from, vault) = (temp("obsidian-from"), temp("obsidian-vault"));
    write(&from.join("Ideas.md"), "# Ideas\n![[dot.png]]\n");
    write(&from.join("Work/Plans.md"), "# Plans\n");
    write(&from.join("dot.png"), "PNG");
    write(&from.join(".obsidian/app.json"), "{}");
    write(&from.join(".trash/Gone.md"), "gone");
    let p = Progress::default();
    let b = bring(Kind::Vault, &[from.clone()], &vault, "From Obsidian", &vault.join("attachments"), &p, &none).unwrap();
    assert_eq!(b.folder, vault.join("From Obsidian"));
    assert_eq!(tree(&vault), ["From Obsidian/Ideas.md", "From Obsidian/Work/Plans.md", "From Obsidian/dot.png"]);
    assert_eq!((b.notes.len(), p.get()), (2, (2, 2)), "notes counted, the picture not");
    // Brought in again: a folder of its own beside the first.
    let again = bring(Kind::Vault, &[from.clone()], &vault, "From Obsidian", &vault.join("attachments"), &Progress::default(), &none).unwrap();
    assert_eq!(again.folder, vault.join("From Obsidian 2"));
    let _ = (std::fs::remove_dir_all(from), std::fs::remove_dir_all(vault));
}

#[test]
fn a_folder_brings_its_notes_only_and_keeps_their_dates() {
    let (from, vault) = (temp("folder-from"), temp("folder-vault"));
    write(&from.join("a.md"), "a");
    write(&from.join("b.txt"), "b");
    write(&from.join("c.docx"), "not a note");
    write(&from.join("sub/d.markdown"), "d");
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::File::options().write(true).open(from.join("a.md")).unwrap().set_modified(old).unwrap();
    let b = bring(Kind::Folder, &[from.clone()], &vault, "Research", &vault, &Progress::default(), &none).unwrap();
    assert_eq!(tree(&vault), ["Research/a.md", "Research/b.txt", "Research/sub/d.markdown"]);
    assert_eq!(b.notes.len(), 3);
    assert_eq!(std::fs::metadata(vault.join("Research/a.md")).unwrap().modified().unwrap(), old, "dated as it was");
    let _ = (std::fs::remove_dir_all(from), std::fs::remove_dir_all(vault));
}

#[test]
fn a_folder_inside_the_vault_or_holding_it_is_not_brought_in() {
    let vault = temp("inside");
    write(&vault.join("Old/a.md"), "a");
    let p = Progress::default();
    assert!(bring(Kind::Folder, &[vault.join("Old")], &vault, "Old", &vault, &p, &none).is_err());
    assert!(bring(Kind::Vault, &[vault.parent().unwrap().to_path_buf()], &vault, "x", &vault, &p, &none).is_err());
    let empty = temp("inside-empty");
    assert_eq!(bring(Kind::Folder, &[empty.clone()], &vault, "Empty", &vault, &p, &none), Err("it has no notes".to_string()));
    assert!(!vault.join("Empty").exists(), "no folder left behind");
    let _ = (std::fs::remove_dir_all(vault), std::fs::remove_dir_all(empty));
}

#[test]
fn an_evernote_export_counts_its_notes_as_they_are_written() {
    let (dir, vault) = (temp("enex-from"), temp("enex-vault"));
    let file = dir.join("My notes.enex");
    std::fs::write(&file, fixtures::enex(5)).unwrap();
    let p = Progress::default();
    let b = bring(Kind::Export(Source::Evernote), &[file], &vault, "From Evernote", &vault.join("Attachments"), &p, &none).unwrap();
    assert_eq!((b.notes.len(), p.get()), (5, (5, 5)));
    assert!(vault.join("From Evernote/Note 3.md").is_file());
    assert!(!b.stopped && b.failed.is_empty());
    // Two notebooks picked: both, into one folder.
    let second = dir.join("Work.enex");
    std::fs::write(&second, fixtures::enex(3)).unwrap();
    let both = bring(Kind::Export(Source::Evernote), &[dir.join("My notes.enex"), second], &vault, "From Evernote", &vault.join("Attachments"), &Progress::default(), &none).unwrap();
    assert_eq!((both.notes.len(), both.folder.file_name().unwrap().to_str().unwrap()), (8, "From Evernote 2"));
    let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(vault));
}

#[test]
fn a_notion_export_keeps_its_pages_inside_one_another() {
    let (dir, vault) = (temp("notion-from"), temp("notion-vault"));
    let file = dir.join("Export-1234.zip");
    std::fs::write(&file, fixtures::notion(3)).unwrap();
    let b = bring(Kind::Export(Source::Notion), &[file], &vault, "From Notion", &vault, &Progress::default(), &none).unwrap();
    assert_eq!(tree(&vault), ["From Notion/Page 1.md", "From Notion/Page 1/Page 2.md", "From Notion/Page 3.md"]);
    assert_eq!(b.notes.len(), 3);
    let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(vault));
}

#[test]
fn a_notes_app_export_comes_in_its_folders() {
    let (dir, vault) = (temp("nbk-from"), temp("nbk-vault"));
    let file = dir.join("Notes.nbk");
    std::fs::write(&file, include_bytes!("../../../kolada-core/testdata/notes.nbk")).unwrap();
    let p = Progress::default();
    let b = bring(Kind::Nbk, &[file], &vault, "From Notes", &vault, &p, &none).unwrap();
    assert!(!b.notes.is_empty() && b.notes.iter().all(|n| n.starts_with(vault.join("From Notes"))));
    assert_eq!(p.get().0, b.notes.len());
    let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(vault));
}

#[test]
fn documents_become_a_note_each_and_say_which_could_not_be_read() {
    let (dir, vault) = (temp("docs-from"), temp("docs-vault"));
    let files = [dir.join("Report.docx"), dir.join("Broken.bad"), dir.join("Page.html")];
    let p = Progress::default();
    let b = bring(Kind::Documents, &files, &vault, "From documents", &vault, &p, &convert).unwrap();
    assert_eq!(tree(&vault), ["From documents/Page.md", "From documents/Report.md"]);
    assert_eq!(b.failed, [("Broken.bad".to_string(), "it is damaged".to_string())]);
    assert_eq!(p.get(), (3, 3));
    // None readable: said why, and no folder left.
    let only = bring(Kind::Documents, &[dir.join("Broken.bad")], &vault, "Again", &vault, &Progress::default(), &convert);
    assert_eq!(only, Err("it is damaged".to_string()));
    assert!(!vault.join("Again").exists());
    let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(vault));
}

#[test]
fn stopped_part_way_it_takes_back_everything_it_wrote() {
    let (dir, vault) = (temp("stop-from"), temp("stop-vault"));
    write(&vault.join("Mine.md"), "kept");
    // Documents: stopped while the second is read.
    let p = Progress::default();
    let files: Vec<PathBuf> = (1..=4).map(|i| dir.join(format!("Doc {i}.docx"))).collect();
    let stop_on_second = |f: &Path, d: &Path| {
        if f.ends_with("Doc 2.docx") {
            p.stop();
        }
        convert(f, d)
    };
    let b = bring(Kind::Documents, &files, &vault, "From documents", &vault, &p, &stop_on_second).unwrap();
    assert!(b.stopped && b.notes.is_empty());
    assert_eq!(tree(&vault), ["Mine.md"], "only what was there before");
    // An export: stopped after its first note, its pictures taken back too.
    let file = dir.join("Big.enex");
    std::fs::write(&file, fixtures::enex(50)).unwrap();
    let p = Progress::default();
    p.stop();
    let b = bring(Kind::Export(Source::Evernote), &[file], &vault, "From Evernote", &vault.join("Attachments"), &p, &none).unwrap();
    assert!(b.stopped);
    assert_eq!(tree(&vault), ["Mine.md"]);
    assert!(!vault.join("Attachments").exists() && !vault.join("From Evernote").exists(), "nor the folders it made");
    // A vault: stopped before it began, nothing copied.
    write(&dir.join("v/a.md"), "a");
    let b = bring(Kind::Vault, &[dir.join("v")], &vault, "From Obsidian", &vault, &p, &none).unwrap();
    assert!(b.stopped);
    assert_eq!(tree(&vault), ["Mine.md"]);
    let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(vault));
}

#[test]
fn a_file_that_is_not_what_it_says_is_said_so() {
    let (dir, vault) = (temp("bad-from"), temp("bad-vault"));
    let file = dir.join("Not really.enex");
    std::fs::write(&file, "hello").unwrap();
    let p = Progress::default();
    assert_eq!(bring(Kind::Export(Source::Evernote), &[file], &vault, "From Evernote", &vault, &p, &none), Err(NOT_AN_EXPORT.to_string()));
    assert!(bring(Kind::Nbk, &[dir.join("missing.nbk")], &vault, "From Notes", &vault, &p, &none) == Err("it is not there any more".to_string()));
    assert!(tree(&vault).is_empty());
    let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(vault));
}

/// A Takeout picked on Notion's tile, or dropped, is Keep's: it comes in
/// as Keep's, named so. A zip that is only a zip, on Keep's or Bear's
/// tile, is not their export, and is said so.
#[test]
fn an_export_on_another_apps_tile_comes_in_as_what_it_is() {
    let (dir, vault) = (temp("found-from"), temp("found-vault"));
    let keep = r#"{"color": "DEFAULT", "isTrashed": false, "isPinned": false, "isArchived": false, "title": "A made-up note", "textContent": "Some words", "userEditedTimestampUsec": 1700000000000000}"#;
    let takeout = dir.join("takeout-001.zip");
    std::fs::write(&takeout, fixtures::zip(&[("Takeout/Keep/a.json", keep.as_bytes())])).unwrap();
    let p = Progress::default();
    let b = bring(Kind::Export(Source::Notion), &[takeout.clone()], &vault, "From Notion", &vault.join("Attachments"), &p, &none).unwrap();
    assert_eq!(b.found, Some(Source::GoogleKeep));
    assert_eq!(b.folder, vault.join("From Google Keep"));
    assert_eq!(b.notes.len(), 1);
    // On its own tile, nothing is found to say.
    let b = bring(Kind::Export(Source::GoogleKeep), &[takeout], &vault, "From Google Keep", &vault.join("Attachments"), &p, &none).unwrap();
    assert_eq!((b.found, b.folder), (None, vault.join("From Google Keep 2")));
    // A zip of a Notion export on Keep's tile or Bear's.
    let notion = dir.join("Export-1.zip");
    std::fs::write(&notion, fixtures::notion(2)).unwrap();
    for tile in [Source::GoogleKeep, Source::Bear] {
        assert_eq!(bring(Kind::Export(tile), &[notion.clone()], &vault, "From it", &vault.join("Attachments"), &p, &none), Err(NOT_AN_EXPORT.to_string()), "{tile:?}");
    }
    assert!(!vault.join("From it").exists());
    // Plain Markdown zipped: no app's on Notion's tile, named for its file
    // (as `import::folder_name` has it); on Bear's, taken for Bear's export.
    let plain = dir.join("My notes.zip");
    std::fs::write(&plain, fixtures::zip(&[("A made-up note.md", b"# A made-up note\n")])).unwrap();
    let b = bring(Kind::Export(Source::Notion), &[plain.clone()], &vault, "From Notion", &vault.join("Attachments"), &p, &none).unwrap();
    assert_eq!((b.found, b.plain, b.folder), (None, true, vault.join("My notes")));
    let b = bring(Kind::Export(Source::Bear), &[plain], &vault, "From Bear", &vault.join("Attachments"), &p, &none).unwrap();
    assert_eq!((b.found, b.plain, b.folder), (None, false, vault.join("From Bear")));
    let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(vault));
}