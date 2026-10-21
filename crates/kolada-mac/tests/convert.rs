//! The app's converting process, run as the app runs it: its own program
//! with `--convert-into` (`kolada_app::model::arrive`).

#![cfg(target_os = "macos")]

use std::path::PathBuf;

#[test]
fn documents_convert_in_a_process_of_their_own() {
    let dir = std::env::temp_dir().join(format!("kolada-mac-convert-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("page.html");
    std::fs::write(&page, "<html><body><h1>Trip</h1><ul><li>Bags</li><li>Hat</li></ul></body></html>").unwrap();
    let broken = dir.join("broken.docx");
    std::fs::write(&broken, b"PK not a document").unwrap();
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_kolada-mac"));
    let notes = dir.join("notes");
    let done = kolada_app::model::arrive::convert_in_process(&exe, &[page, broken], &notes, &dir.join("pics"), &kolada_app::services::NoServices).expect("the process starts");
    assert_eq!(done.notes.len(), 1, "{:?}", done.failed);
    let text = std::fs::read_to_string(&done.notes[0]).unwrap();
    assert!(text.starts_with("# Trip") && text.contains("- Bags"), "{} bytes", text.len());
    assert_eq!(done.failed.len(), 1);
    assert_eq!(done.failed[0].0, "broken.docx");
    let _ = std::fs::remove_dir_all(&dir);
}
