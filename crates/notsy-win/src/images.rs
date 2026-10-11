//! Pictures in notes. A line that is only an image embed (`![[photo.png]]`,
//! or `![what](photo.png)`) shows the picture across the column. Windows'
//! imaging component (WIC) decodes and scales it on a worker, at the size
//! it is drawn, so the renderer only copies pixels.
//!
//! Embeds name files the way Obsidian does: by file name, anywhere in the
//! vault, or by a path from the vault's top or from the note's folder. A
//! picture beside the note, at the vault's top or with its attachments is
//! sized at once from its file's first bytes, so even the first frame
//! keeps its place and nothing moves when it arrives; others are searched
//! for on the worker, which starts after the first frame.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{GENERIC_READ, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, GUID_WICPixelFormat32bppPBGRA,
    IWICBitmapFrameEncode, IWICImagingFactory, WICBitmapDitherType, WICBitmapEncoderNoCache, WICBitmapInterpolationModeFant,
    WICBitmapPaletteType, WICDecodeOptions,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, Result};

use crate::window::WM_APP_IMAGE;

/// How long a picture takes to fade in once decoded.
const FADE: Duration = Duration::from_millis(180);

pub use notsy_ui::images::{Lookup, PictureSource, embed_in, is_image, with_width};
pub use notsy_ui::pixels::Pixels;

/// Work for the decoding thread.
enum Job {
    /// Find the file an embed names, and its size.
    Find { vault: PathBuf, from: PathBuf, name: String },
    /// Decode a file at a width in pixels.
    Decode { path: PathBuf, width: u32 },
}

/// What comes back from the worker.
pub enum Found {
    /// An embed's file (None: there is none) and its size in pixels.
    Place { from: PathBuf, name: String, file: Option<(PathBuf, u32, u32)> },
    Pixels { path: PathBuf, width: u32, pixels: Option<Pixels> },
}

impl PictureSource for Images {
    fn place(&mut self, vault: &Path, from: &Path, name: &str) -> Lookup {
        Images::place(self, vault, from, name)
    }

    fn pixels(&mut self, path: &Path, width: u32) -> Option<(Arc<Pixels>, f32)> {
        Images::pixels(self, path, width)
    }
}

/// Where an embed's picture is and its size in pixels (`None`: nowhere).
type Place = Option<(PathBuf, u32, u32)>;

/// Decoded pixels, and when they arrived (for their fade in).
struct Decoded {
    pixels: Arc<Pixels>,
    at: Instant,
}

/// Pictures known so far, and those asked for.
pub struct Images {
    /// The worker's queue, once it runs; what it will do until then.
    jobs: Option<Sender<Job>>,
    waiting: Vec<Job>,
    /// An embed's file and size, by the note's folder and the name.
    places: HashMap<(PathBuf, String), Place>,
    pixels: HashMap<(PathBuf, u32), Decoded>,
    asked: HashSet<(PathBuf, String)>,
    decoding: HashSet<(PathBuf, u32)>,
    /// Each vault's attachments folder, from Obsidian's settings.
    attachments: HashMap<PathBuf, Option<String>>,
    /// Pictures on their way from the web (see `app::paste`): until they
    /// come, they are being looked for.
    coming: HashSet<PathBuf>,
}

impl Images {
    /// Before the first frame: lookups work, work for the worker waits.
    pub fn new() -> Images {
        Images {
            jobs: None,
            waiting: Vec::new(),
            places: HashMap::new(),
            pixels: HashMap::new(),
            asked: HashSet::new(),
            decoding: HashSet::new(),
            attachments: HashMap::new(),
            coming: HashSet::new(),
        }
    }

    /// Pictures on their way to `paths`: drawn as being looked for until
    /// they come.
    pub fn coming(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.coming.extend(paths);
        self.forget_places();
    }

    /// A picture came (or will not): embeds are looked up again.
    pub fn came(&mut self, path: &Path) {
        self.coming.remove(path);
        self.forget_places();
    }

    /// Starts the worker (after the first frame), which tells `hwnd` what
    /// it finds.
    pub fn start(&mut self, hwnd: HWND) {
        let (jobs, rx) = channel();
        let hwnd = crate::Handoff(hwnd);
        std::thread::spawn(move || {
            let hwnd = hwnd;
            crate::background_priority();
            work(hwnd.0, rx);
        });
        for job in self.waiting.drain(..) {
            let _ = jobs.send(job);
        }
        self.jobs = Some(jobs);
    }

    fn send(&mut self, job: Job) {
        match &self.jobs {
            Some(jobs) => {
                let _ = jobs.send(job);
            }
            None => self.waiting.push(job),
        }
    }

    /// The file and size of the picture `name` in a note in `from` (a
    /// folder of `vault`), asking for it the first time.
    pub fn place(&mut self, vault: &Path, from: &Path, name: &str) -> Lookup {
        let file = name.replace("%20", " ");
        if self.coming.iter().any(|p| p.file_name().is_some_and(|f| f.to_string_lossy() == file)) {
            return Lookup::Pending;
        }
        let key = (from.to_path_buf(), name.to_string());
        match self.places.get(&key) {
            Some(Some((p, w, h))) => return Lookup::Found(p.clone(), *w, *h),
            Some(None) => return Lookup::Missing,
            None => {}
        }
        if !self.asked.insert(key.clone()) {
            return Lookup::Pending;
        }
        if let Some((p, w, h)) = self.nearby(vault, from, name) {
            self.places.insert(key, Some((p.clone(), w, h)));
            return Lookup::Found(p, w, h);
        }
        self.send(Job::Find { vault: vault.to_path_buf(), from: from.to_path_buf(), name: name.to_string() });
        Lookup::Pending
    }

    /// A picture where embeds usually are (beside the note, at the vault's
    /// top, with its attachments), sized from its first bytes.
    fn nearby(&mut self, vault: &Path, from: &Path, name: &str) -> Option<(PathBuf, u32, u32)> {
        let name = name.replace("%20", " ");
        let setting = self
            .attachments
            .entry(vault.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(vault.join(".obsidian").join("app.json")).ok().and_then(|j| attachment_setting(&j)))
            .clone();
        let attachments = setting.map(|s| attachment_dir(vault, from, &s));
        [Some(from.to_path_buf()), Some(vault.to_path_buf()), attachments].into_iter().flatten().find_map(|dir| {
            let p = dir.join(&name);
            let (w, h) = header_size(&p)?;
            Some((p, w, h))
        })
    }

    /// `path` decoded `width` pixels wide, asking for it the first time;
    /// and how far it has faded in. While another size is on its way, the
    /// one there is stands in.
    pub fn pixels(&mut self, path: &Path, width: u32) -> Option<(Arc<Pixels>, f32)> {
        let key = (path.to_path_buf(), width);
        if let Some(d) = self.pixels.get(&key) {
            return Some((d.pixels.clone(), (d.at.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0)));
        }
        if self.decoding.insert(key.clone()) {
            self.send(Job::Decode { path: key.0, width });
        }
        self.pixels.iter().find(|((p, _), _)| p == path).map(|(_, d)| (d.pixels.clone(), 1.0))
    }

    /// Whether a picture is still fading in (a little past its end, so its
    /// last frame is drawn whole).
    pub fn fading(&self) -> bool {
        self.pixels.values().any(|d| d.at.elapsed() < FADE + Duration::from_millis(20))
    }

    /// Something the worker found; true if the view should be drawn again.
    pub fn arrived(&mut self, found: Found) -> bool {
        match found {
            Found::Place { from, name, file } => {
                self.places.insert((from, name), file);
            }
            Found::Pixels { path, width, pixels } => {
                self.decoding.remove(&(path.clone(), width));
                if let Some(p) = pixels {
                    // A new size of a picture shown already does not fade.
                    let shown = self.pixels.keys().any(|(p, _)| *p == path);
                    let at = if shown { Instant::now() - FADE } else { Instant::now() };
                    // Other sizes of it are no longer needed.
                    self.pixels.retain(|(p, _), _| *p != path);
                    self.pixels.insert((path, width), Decoded { pixels: Arc::new(p), at });
                }
            }
        }
        true
    }

    /// Files changed (a paste, a drop): embeds are looked up again.
    pub fn forget_places(&mut self) {
        self.places.clear();
        self.asked.clear();
        self.attachments.clear();
    }
}

pub use notsy_ui::pictures::{attachment_dir, attachment_setting, find, header_size};


fn factory() -> Result<IWICImagingFactory> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
}

/// A picture's size in pixels.
fn size(wic: &IWICImagingFactory, path: &Path) -> Result<(u32, u32)> {
    unsafe {
        let decoder = wic.CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeOptions(0))?;
        let frame = decoder.GetFrame(0)?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h)?;
        Ok((w, h))
    }
}

/// A picture decoded and scaled to `width` pixels (keeping its shape).
fn decode(wic: &IWICImagingFactory, path: &Path, width: u32) -> Result<Pixels> {
    unsafe {
        let decoder = wic.CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeOptions(0))?;
        let frame = decoder.GetFrame(0)?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h)?;
        let width = width.clamp(1, 8192);
        let height = ((h as f64 * width as f64 / w.max(1) as f64).round() as u32).clamp(1, 16384);
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&frame, width, height, WICBitmapInterpolationModeFant)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherType(0), None, 0.0, WICBitmapPaletteType(0))?;
        let mut bytes = vec![0u8; (width * height * 4) as usize];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut bytes)?;
        let data = bytes.as_chunks::<4>().0.iter().map(|p| u32::from_le_bytes(*p)).collect();
        Ok(Pixels { width, height, data })
    }
}

/// Writes 32-bit BGRA pixels (bottom-up rows if `bottom_up`) as a PNG file.
pub fn write_png(path: &Path, width: u32, height: u32, bgra: &[u8], bottom_up: bool) -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        let wic = factory()?;
        let stride = width * 4;
        let rows: Vec<u8> = if bottom_up {
            bgra.chunks_exact(stride as usize).rev().flatten().copied().collect()
        } else {
            bgra.to_vec()
        };
        let stream = wic.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), windows::Win32::Foundation::GENERIC_WRITE.0)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut options: Option<IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
        frame.Initialize(options.as_ref())?;
        frame.SetSize(width, height)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WritePixels(height, stride, &rows)?;
        frame.Commit()?;
        encoder.Commit()
    }
}

fn post(hwnd: HWND, found: Found) {
    let found = Box::into_raw(Box::new(found));
    if unsafe { PostMessageW(Some(hwnd), WM_APP_IMAGE, WPARAM(0), LPARAM(found as isize)) }.is_err() {
        drop(unsafe { Box::from_raw(found) });
    }
}

fn work(hwnd: HWND, jobs: Receiver<Job>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let Ok(wic) = factory() else { return };
    let mut known = None;
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Find { vault, from, name } => {
                let file = find(&vault, &from, &name, &mut known).and_then(|p| size(&wic, &p).ok().map(|(w, h)| (p, w, h)));
                post(hwnd, Found::Place { from, name, file });
            }
            Job::Decode { path, width } => {
                let pixels = decode(&wic, &path, width).ok();
                post(hwnd, Found::Pixels { path, width, pixels });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs Windows' imaging component"]
    fn writes_and_reads_a_png() {
        let dir = std::env::temp_dir().join(format!("notsy-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("red.png");
        // 4 x 2, opaque red (BGRA).
        let px: Vec<u8> = std::iter::repeat_n([0u8, 0, 255, 255], 8).flatten().collect();
        write_png(&path, 4, 2, &px, false).unwrap();
        let wic = factory().unwrap();
        assert_eq!(size(&wic, &path).unwrap(), (4, 2));
        assert_eq!(header_size(&path), Some((4, 2)));
        let p = decode(&wic, &path, 8).unwrap();
        assert_eq!((p.width, p.height), (8, 4));
        assert_eq!(p.data[0], 0xFFFF_0000, "premultiplied opaque red");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
