//! A note shared as a link on notsy.co: Windows' half of
//! `kolada_core::share`. Each request runs on a thread of its own and posts
//! what came back to the window (`WM_APP_SHARE`); nothing here runs unless
//! the user opens the share card. The install's key is made from the OS's
//! random bytes the first time a link is made, and kept with the links in
//! the app's data folder (a test instance's own).

use std::collections::HashMap;
use std::path::PathBuf;

use kolada_core::share::{self as shared, BASE, Expiry, LIMIT, Link, Problem};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::window::WM_APP_SHARE;

/// What a request came to, posted (boxed) with `WM_APP_SHARE`.
pub enum Answer {
    Created(Result<Link, Problem>),
    Listed(Result<Vec<Link>, Problem>),
    Deleted(String, Result<(), Problem>),
}

/// The open note, as it goes up: its text, where its pictures are looked
/// for (the vault, the note's folder), where it is (vault-relative, with
/// `.md`; empty for a page not saved yet), and its title.
pub struct Outgoing {
    pub text: String,
    pub vault: PathBuf,
    pub dir: PathBuf,
    pub note: String,
    pub title: String,
}

/// The most an answer may weigh (they are a few lines).
const ANSWER: usize = 256 * 1024;
/// The most a picture file may weigh to be read for sharing.
const PICTURE: u64 = 64 * 1024 * 1024;

fn random_key() -> Option<[u8; 32]> {
    let mut b = [0u8; 32];
    unsafe { BCryptGenRandom(None, &mut b, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.is_ok().then_some(b)
}

/// The install's key, made now if there is none yet.
fn key() -> Option<String> {
    shared::load_or_make_key(&kolada_core::app_data_dir()?, random_key)
}

/// The install's key if a link was ever made here (else nothing to list).
fn key_if_made() -> Option<String> {
    let path = shared::key_path(&kolada_core::app_data_dir()?);
    std::fs::read_to_string(path).ok().as_deref().and_then(shared::read_key)
}

/// The expiry chosen last and the links kept here, to show at once.
pub fn kept() -> (Expiry, Vec<Link>) {
    kolada_core::app_data_dir().and_then(|d| std::fs::read_to_string(shared::links_path(&d)).ok()).map_or_else(|| (Expiry::default(), Vec::new()), |t| shared::read_kept(&t))
}

pub fn keep(expiry: Expiry, links: &[Link]) {
    if let Some(dir) = kolada_core::app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(shared::links_path(&dir), shared::kept_text(expiry, links));
    }
}

fn post(hwnd: crate::Handoff<HWND>, answer: Answer) {
    let answer = Box::into_raw(Box::new(answer));
    if unsafe { PostMessageW(Some(hwnd.0), WM_APP_SHARE, WPARAM(0), LPARAM(answer as isize)) }.is_err() {
        drop(unsafe { Box::from_raw(answer) });
    }
}

/// The note made ready (its pictures through WIC) and sent; a link back.
pub fn create(hwnd: HWND, note: Outgoing, expiry: Expiry) {
    let hwnd = crate::Handoff(hwnd);
    std::thread::spawn(move || {
        let hwnd = hwnd;
        post(hwnd, Answer::Created(make(note, expiry)));
    });
}

fn make(note: Outgoing, expiry: Expiry) -> Result<Link, Problem> {
    let recoder = crate::images::Recoder::new().ok_or(Problem::Server)?;
    let mut known = None;
    let mut seen: HashMap<String, Option<PathBuf>> = HashMap::new();
    let mut read = |name: &str| -> Option<Vec<u8>> {
        let path = seen.entry(name.to_string()).or_insert_with(|| kolada_ui::pictures::find(&note.vault, &note.dir, name, &mut known)).clone()?;
        (std::fs::metadata(&path).ok()?.len() <= PICTURE).then(|| std::fs::read(&path).ok()).flatten()
    };
    let bundle = shared::bundle(&note.text, &mut read, &recoder, LIMIT)?;
    let key = key().ok_or(Problem::Server)?;
    let seed = random_key().map_or(kolada_core::session::now(), |b| u64::from_le_bytes(b[..8].try_into().unwrap()));
    let boundary = shared::boundary(&bundle, seed);
    let body = shared::multipart(&boundary, expiry, &bundle);
    let answer = crate::http::send("POST", &format!("{BASE}/api/links"), &[("X-Kolada-Key", &key), ("Content-Type", &shared::content_type(&boundary))], &body, ANSWER);
    let made = shared::created(answer, &bundle)?;
    Ok(Link { id: made.id, created: kolada_core::session::now(), expires: made.expires, title: note.title, note: note.note })
}

/// This install's live links, from the server (none asked for when no link
/// was ever made here).
pub fn list(hwnd: HWND) {
    let hwnd = crate::Handoff(hwnd);
    std::thread::spawn(move || {
        let hwnd = hwnd;
        let listed = match key_if_made() {
            None => Ok(Vec::new()),
            Some(key) => shared::listed(crate::http::send("GET", &format!("{BASE}/api/links"), &[("X-Kolada-Key", &key)], &[], ANSWER)),
        };
        post(hwnd, Answer::Listed(listed));
    });
}

/// A link deleted for good.
pub fn delete(hwnd: HWND, id: String) {
    let hwnd = crate::Handoff(hwnd);
    std::thread::spawn(move || {
        let hwnd = hwnd;
        let done = match key_if_made() {
            None => Ok(()),
            Some(key) => shared::deleted(crate::http::send("DELETE", &format!("{BASE}/api/links/{id}"), &[("X-Kolada-Key", &key)], &[], ANSWER)),
        };
        post(hwnd, Answer::Deleted(id, done));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_come_from_the_os() {
        let (a, b) = (random_key().unwrap(), random_key().unwrap());
        assert_ne!(a, b);
        assert_eq!(shared::key_text(&a).len(), 43);
    }

    #[test]
    #[ignore = "needs Windows' imaging component"]
    fn pictures_are_made_smaller_and_encoded_in_memory() {
        use kolada_core::share::{Kind, Raster, Recode, dimensions};
        let r = crate::images::Recoder::new().unwrap();
        // 301 x 200, opaque, a gradient (odd rows of 24-bit pixels are padded).
        let px: Vec<u8> = (0..301 * 200).flat_map(|i: u32| [(i % 251) as u8, (i % 7) as u8 * 30, 90, 255]).collect();
        let raster = Raster { width: 301, height: 200, bgra: px };
        let jpeg = r.encode(&raster, 82, false).unwrap();
        assert_eq!(Kind::of(&jpeg), Kind::Jpeg);
        assert_eq!(dimensions(&jpeg), Some((301, 200)));
        let small = r.decode(&jpeg, 100).unwrap();
        assert_eq!((small.width, small.height), (100, 66));
        assert!(!small.has_alpha());
        let png = r.encode(&Raster { width: 2, height: 2, bgra: vec![0, 0, 255, 128].repeat(4) }, 82, true).unwrap();
        assert_eq!(Kind::of(&png), Kind::Png);
        assert!(r.decode(&png, 1600).unwrap().has_alpha());
        assert!(r.decode(b"not a picture", 1600).is_none());
    }

    /// Against the real notsy.co, with a key of its own (never the install's):
    /// a generated note with a large picture is shared, read back clean,
    /// listed, deleted, and gone at once.
    #[test]
    #[ignore = "goes online to notsy.co"]
    fn a_note_is_shared_on_notsy_and_deleted() {
        use kolada_core::share::{Raster, Recode};
        let dir = std::env::temp_dir().join(format!("kolada-share-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = crate::images::Recoder::new().unwrap();
        let (w, h) = (3000u32, 2000u32);
        let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i / w % 256) as u8, 120, 255]).collect();
        std::fs::write(dir.join("big.png"), r.encode(&Raster { width: w, height: h, bgra: px }, 90, true).unwrap()).unwrap();
        let text = "# A shared test note

Some **bold** and *italic* words, a [site](https://kolada.app).

- one
- [ ] two

![[big.png]]
";
        let mut read = |name: &str| std::fs::read(dir.join(name)).ok();
        let bundle = shared::bundle(text, &mut read, &r, LIMIT).unwrap();
        eprintln!("bundle {} bytes, {} picture(s)", bundle.size(), bundle.files.len());
        let key = shared::key_text(&random_key().unwrap());
        let boundary = shared::boundary(&bundle, 7);
        let body = shared::multipart(&boundary, Expiry::Day, &bundle);
        let made = shared::created(crate::http::send("POST", &format!("{BASE}/api/links"), &[("X-Kolada-Key", &key), ("Content-Type", &shared::content_type(&boundary))], &body, ANSWER), &bundle).unwrap();
        eprintln!("made {}", made.id);
        let page = String::from_utf8(crate::http::get(&format!("{BASE}/{}", made.id), 4 << 20).unwrap()).unwrap();
        assert!(page.contains("<strong>bold</strong>") && page.contains("<img") && !page.contains("**"), "the page reads formatted");
        let txt = String::from_utf8(crate::http::get(&format!("{BASE}/{}.txt", made.id), 1 << 20).unwrap()).unwrap();
        assert!(txt.contains("bold") && !txt.contains("**") && !txt.contains("](") , "the text reads without marks");
        let picture = page.split("<img src=\"").nth(1).and_then(|s| s.split('"').next()).unwrap().to_string();
        let url = if picture.starts_with("http") { picture } else { format!("{BASE}{}", picture) };
        assert!(crate::http::get(&url, 4 << 20).is_some(), "the picture is served");
        let listed = shared::listed(crate::http::send("GET", &format!("{BASE}/api/links"), &[("X-Kolada-Key", &key)], &[], ANSWER)).unwrap();
        assert!(listed.iter().any(|l| l.id == made.id), "listed");
        shared::deleted(crate::http::send("DELETE", &format!("{BASE}/api/links/{}", made.id), &[("X-Kolada-Key", &key)], &[], ANSWER)).unwrap();
        let gone = crate::http::send("GET", &format!("{BASE}/{}", made.id), &[], &[], 1 << 20).unwrap();
        assert_eq!(gone.0, 410, "deleted is gone at once");
        assert!(crate::http::send("GET", &url, &[], &[], 1 << 20).unwrap().0 >= 400, "and so is its picture");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
