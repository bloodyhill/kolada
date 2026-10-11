//! The words in a picture, read with the text recognition that comes with
//! macOS (Vision, `VNRecognizeTextRequest`): it runs on this machine and
//! needs nothing downloaded. Windows reads pictures the same way with its
//! own OCR (`notsy-win` `convert.rs`, `Ocr`). Called from the index's thread,
//! never the main one; Vision is reached through the Objective-C runtime so
//! no binding crate is needed.

use std::path::Path;

use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSString, NSURL};

#[link(name = "Vision", kind = "framework")]
unsafe extern "C" {}

pub use notsy_app::services::Read;

/// Vision, for the model's `Ocr` service.
pub struct MacOcr;

impl notsy_app::services::Ocr for MacOcr {
    fn read(&self, path: &Path) -> Read {
        read(path)
    }
}

/// The words in the picture at `path`.
pub fn read(path: &Path) -> Read {
    let Some(path) = path.to_str() else { return Read::Words(String::new()) };
    autoreleasepool(|_| unsafe { recognize(path) })
}

unsafe fn recognize(path: &str) -> Read {
    let (Some(handler_class), Some(request_class)) = (AnyClass::get(c"VNImageRequestHandler"), AnyClass::get(c"VNRecognizeTextRequest")) else {
        return Read::Missing;
    };
    let nothing = Read::Words(String::new());
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let options = NSDictionary::<NSString, AnyObject>::new();
    let made: Allocated<AnyObject> = msg_send![handler_class, alloc];
    let handler: Option<Retained<AnyObject>> = msg_send![made, initWithURL: &*url, options: &*options];
    let Some(handler) = handler else { return nothing };
    let made: Allocated<AnyObject> = msg_send![request_class, alloc];
    let request: Option<Retained<AnyObject>> = msg_send![made, init];
    let Some(request) = request else { return nothing };
    // Accurate (0), with the language found from the words, as Windows reads
    // in the languages the user has set.
    let _: () = msg_send![&*request, setRecognitionLevel: 0isize];
    let _: () = msg_send![&*request, setUsesLanguageCorrection: true];
    // Small print counts (a screenshot's text is a hundredth of its height,
    // under what Vision looks for unless told).
    let _: () = msg_send![&*request, setMinimumTextHeight: 0.005f32];
    let detects: bool = msg_send![&*request, respondsToSelector: sel!(setAutomaticallyDetectsLanguage:)];
    if detects {
        let _: () = msg_send![&*request, setAutomaticallyDetectsLanguage: true];
    }
    let requests = NSArray::from_slice(&[&*request]);
    let done: Result<(), Retained<NSError>> = msg_send![&*handler, performRequests: &*requests, error: _];
    if let Err(e) = &done {
        if std::env::var_os("NOTSY_VISION_DEBUG").is_some() {
            eprintln!("vision: {}", e.localizedDescription());
        }
        return nothing;
    }
    let results: Option<Retained<NSArray<AnyObject>>> = msg_send![&*request, results];
    if std::env::var_os("NOTSY_VISION_DEBUG").is_some() {
        eprintln!("vision: performed, results {:?}", results.as_ref().map(|r| r.count()));
    }
    let Some(results) = results else { return nothing };
    let mut lines = Vec::new();
    for i in 0..results.count() {
        let observation = results.objectAtIndex(i);
        let candidates: Option<Retained<NSArray<AnyObject>>> = msg_send![&*observation, topCandidates: 1usize];
        let Some(candidates) = candidates else { continue };
        if candidates.count() == 0 {
            continue;
        }
        let best = candidates.objectAtIndex(0);
        let text: Option<Retained<NSString>> = msg_send![&*best, string];
        if let Some(text) = text {
            lines.push(text.to_string());
        }
    }
    Read::Words(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vision on this Mac, with a picture of words made on the spot (the
    /// system's own tools draw it). Ignored: it needs a Mac that lets Vision
    /// run (not every headless session does), and it is bounded by the
    /// timeout the test sets itself so a stuck Vision cannot hang the suite.
    #[test]
    #[ignore = "needs Vision, which may not run over SSH"]
    fn vision_reads_the_words_in_a_picture() {
        use std::process::Command;
        let d = std::env::temp_dir().join(format!("notsy-mac-vision-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("words.txt"), "Invoice 2024\nTotal due 150 dollars\n").unwrap();
        let pdf = Command::new("cupsfilter").arg(d.join("words.txt")).output().expect("cupsfilter");
        std::fs::write(d.join("words.pdf"), &pdf.stdout).unwrap();
        let sips = Command::new("sips").args(["-s", "format", "jpeg", "-s", "dpiWidth", "216", "-s", "dpiHeight", "216"]).arg(d.join("words.pdf")).arg("--out").arg(d.join("words.jpg")).status().expect("sips");
        assert!(sips.success());
        // The PDF's words, through the shared converter.
        let text = notsy_convert::convert(notsy_convert::Kind::Pdf, &pdf.stdout).map(|n| n.markdown).unwrap_or_default();
        assert!(text.to_lowercase().contains("invoice"), "the PDF's text");
        let (tx, rx) = std::sync::mpsc::channel();
        let png = d.join("words.jpg");
        std::thread::spawn(move || {
            let _ = tx.send(read(&png));
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(60)).expect("Vision answered in a minute");
        let Read::Words(words) = got else { panic!("Vision is missing") };
        let lower = words.to_lowercase();
        assert!(words.lines().count() >= 2 && ["invoice", "2024", "total", "dollars", "150"].iter().any(|w| lower.contains(w)), "{} lines, {} chars", words.lines().count(), words.len());
        let _ = std::fs::remove_dir_all(&d);
    }
}
