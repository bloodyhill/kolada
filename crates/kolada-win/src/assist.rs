//! The writing model's own process, `kolada-ai.exe`, beside this one. It
//! starts a moment after the window shows, below normal priority, in a job
//! that ends it with Kolada (or when writing help is turned off). Requests go
//! down its standard input as frames (see `kolada_assist::wire`) from a
//! writer thread; a reader thread takes its replies off its standard output
//! into a channel and nudges the window. Without its files it stays off,
//! without a word. Its model is the one the person chose, downloaded into
//! the app's data (`Download`).

use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use kolada_assist::models;
use kolada_assist::wire::{self, Reply, Request};
use kolada_core::fetch;
use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, HWND, LPARAM, SetHandleInformation, WPARAM};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, GetExitCodeProcess, PROCESS_INFORMATION, ResumeThread,
    STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{PCWSTR, PWSTR, w};

use crate::Handoff;
use crate::http::Http;
use crate::window::WM_APP_ASSIST;

/// The helper, beside the exe.
pub const HELPER: &str = "kolada-ai.exe";

/// The helper's exit code when its model would not load: starting it again
/// would not help.
pub const NO_MODEL: u32 = 2;

pub enum Event {
    Reply(Reply),
    /// The helper ended, with its exit code; None if it never started (its
    /// files are missing, or Windows would not start it).
    Ended(Option<u32>),
}

/// Tells the window something waits in a channel.
pub fn nudge(hwnd: HWND) {
    let _ = unsafe { PostMessageW(Some(hwnd), WM_APP_ASSIST, WPARAM(0), LPARAM(0)) };
}

/// The job the helper runs in: closing it (or this process ending) ends it.
struct Job {
    handle: HANDLE,
    /// Set before the job is ended, so a helper still being started is
    /// ended too.
    stopped: AtomicBool,
}

unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    fn new() -> Option<Job> {
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.ok()?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        let set = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const std::ffi::c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        let job = Job { handle, stopped: AtomicBool::new(false) };
        set.is_ok().then_some(job)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

/// A handle closed when it goes.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub struct Helper {
    frames: Sender<Vec<u8>>,
    pub events: Receiver<Event>,
    job: Arc<Job>,
}

impl Helper {
    /// Starts the helper on a thread of its own, which then writes what is
    /// sent, on the model chosen (its id, see `files`). What it answers
    /// arrives in `events`, and `hwnd` is nudged.
    pub fn start(hwnd: HWND, model: Option<String>) -> Option<Helper> {
        let job = Arc::new(Job::new()?);
        let (frames, pending) = channel::<Vec<u8>>();
        let (events_out, events) = channel();
        let (window, their_job) = (Handoff(hwnd), job.clone());
        std::thread::spawn(move || {
            let hwnd = window;
            let job = their_job;
            let started = files(model.as_deref()).and_then(|(helper, model)| spawn(&helper, &model, &job));
            let Some((mut stdin, stdout, process)) = started else {
                let _ = events_out.send(Event::Ended(None));
                return nudge(hwnd.0);
            };
            let (reader_window, process) = (Handoff(hwnd.0), Handoff(process));
            std::thread::spawn(move || {
                let hwnd = reader_window;
                let process = process;
                crate::background_priority();
                let mut from = BufReader::new(stdout);
                while let Ok(Some(body)) = wire::read_frame(&mut from) {
                    let Some(reply) = Reply::parse(&body) else { continue };
                    if events_out.send(Event::Reply(reply)).is_err() {
                        break;
                    }
                    nudge(hwnd.0);
                }
                // Its output closed: it ended, or is ending.
                let mut code = 1;
                unsafe {
                    WaitForSingleObject(process.0, 10_000);
                    let _ = GetExitCodeProcess(process.0, &mut code);
                    let _ = CloseHandle(process.0);
                }
                let _ = events_out.send(Event::Ended(Some(code)));
                nudge(hwnd.0);
            });
            crate::background_priority();
            // Closing its input when `frames` goes ends the helper.
            while let Ok(frame) = pending.recv() {
                if wire::write_frame(&mut stdin, &frame).is_err() {
                    break;
                }
            }
        });
        Some(Helper { frames, events, job })
    }

    /// False once the helper can no longer be written to.
    pub fn send(&self, request: &Request) -> bool {
        self.frames.send(request.frame()).is_ok()
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        self.job.stopped.store(true, Ordering::SeqCst);
        let _ = unsafe { TerminateJobObject(self.job.handle, 0) };
    }
}

/// The helper and its model, if both are there; `model` is the id of the
/// model the person chose (`kolada_assist::models`).
fn files(model: Option<&str>) -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    // A test instance's own download first, then the real app data's: a
    // model there is shared by every Kolada.
    let own = kolada_core::testing().then(kolada_core::app_data_dir).flatten();
    [own, kolada_core::real_data_dir()].into_iter().flatten().find_map(|data| files_in(dir, Some(&data), model)).or_else(|| files_in(dir, None, model))
}

/// The writing helper is beside the program (only then is a model worth
/// offering).
pub fn helper_here() -> bool {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).is_some_and(|dir| [HELPER, kolada_core::before::HELPER].iter().any(|n| dir.join(n).is_file()))
}

/// Where a chosen model downloads: the real app data's `models`, shared by
/// every Kolada; a test instance's own, so that a test never changes the
/// person's models.
pub fn models_dir() -> Option<PathBuf> {
    let data = if kolada_core::testing() { kolada_core::app_data_dir() } else { kolada_core::real_data_dir() };
    Some(data?.join("models"))
}

/// What a model's download says as it goes.
pub enum Fetched {
    /// Bytes in so far, of all of them: at most a thousand times in all.
    Progress(u64, u64),
    Ended(fetch::Outcome),
}

/// The chosen model downloading into `models_dir`, on a thread of its own
/// at background priority; what it says arrives in `events`, and the window
/// is nudged. Dropping it stops the download (what came is kept, and the
/// next one resumes from there).
pub struct Download {
    pub id: &'static str,
    pub events: Receiver<Fetched>,
    cancel: Arc<AtomicBool>,
}

impl Download {
    /// None when `id` is no model in the catalogue, or it has nowhere to
    /// come from yet (`models::HUB`, `fetch::address`).
    pub fn start(hwnd: HWND, id: &str) -> Option<Download> {
        let m = models::by_id(id)?;
        let url = fetch::address(models::url(m), m.file)?;
        let dir = models_dir()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let (out, events) = channel();
        let (window, stop) = (Handoff(hwnd), cancel.clone());
        std::thread::spawn(move || {
            let hwnd = window;
            crate::background_priority();
            let want = fetch::Want { url: &url, file: m.file, bytes: m.bytes, sha256: m.sha256 };
            fetch_into(&dir, &want, &Http, &stop, &out, &|| nudge(hwnd.0));
        });
        Some(Download { id: m.id, events, cancel })
    }
}

/// The download's thread: `want` from `source` into `dir`, telling `out`
/// how it goes (a thousandth at a time at most) and how it ended, with a
/// `nudge` after each.
fn fetch_into(dir: &Path, want: &fetch::Want, source: &dyn fetch::Source, cancel: &AtomicBool, out: &Sender<Fetched>, nudge: &dyn Fn()) {
    let mut shown = None;
    let mut progress = |done: u64, total: u64| {
        let step = done.saturating_mul(1000) / total.max(1);
        if shown.replace(step) != Some(step) && out.send(Fetched::Progress(done, total)).is_ok() {
            nudge();
        }
    };
    let outcome = match std::fs::create_dir_all(dir) {
        Ok(()) => fetch::download(dir, want, source, &mut progress, cancel),
        Err(_) => fetch::Outcome::Failed(fetch::Failure::Disk),
    };
    let _ = out.send(Fetched::Ended(outcome));
    nudge();
}

impl Drop for Download {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// Once `keep` is in place: the other models of the catalogue, and what
/// came of their downloads, go from `models_dir`, so switching sizes frees
/// the space.
pub fn remove_other_models(keep: &str) {
    let Some(dir) = models_dir() else { return };
    let files: Vec<&str> = models::MODELS.iter().map(|m| m.file).collect();
    fetch::remove_others(&dir, Some(keep), &files);
}

/// `files` for the exe in `dir` and the app's data in `data`. A model
/// trained for Kolada in `<dir>/models` comes first (a test build staged
/// with one: tools/stage.ps1 -Model), the chosen one if it is there; else
/// the chosen model in `<data>/models`, once all of it has downloaded. No
/// model, no helper.
fn files_in(dir: &Path, data: Option<&Path>, model: Option<&str>) -> Option<(PathBuf, PathBuf)> {
    // An install updated from Notsy may still have the helper by its old name.
    let helper = [HELPER, kolada_core::before::HELPER].into_iter().map(|n| dir.join(n)).find(|h| h.is_file())?;
    let chosen = model.and_then(kolada_assist::models::by_id);
    if let Some(beside) = beside(&dir.join("models"), chosen.map(|m| m.file)) {
        return Some((helper, beside));
    }
    let m = chosen?;
    let shared = data?.join("models").join(m.file);
    let whole = std::fs::metadata(&shared).is_ok_and(|f| f.is_file() && f.len() == m.bytes);
    whole.then_some((helper, shared))
}

/// A model trained for Kolada in `models`: `chosen` if it is one of them,
/// else the first by name.
fn beside(models: &Path, chosen: Option<&str>) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(models)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            kolada_assist::models::trained(name) && name.to_ascii_lowercase().ends_with(".gguf") && p.is_file()
        })
        .collect();
    found.sort();
    let pick = found.iter().position(|p| chosen.is_some_and(|c| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(c)))).unwrap_or(0);
    (!found.is_empty()).then(|| found.swap_remove(pick))
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Starts `helper` on `model` with pipes for its input and output, in
/// `job`, and returns our ends and the process.
fn spawn(helper: &Path, model: &Path, job: &Job) -> Option<(File, File, HANDLE)> {
    if job.stopped.load(Ordering::SeqCst) {
        return None;
    }
    let inherit = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: true.into() };
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    unsafe { CreatePipe(&mut read, &mut write, Some(&inherit), 0) }.ok()?;
    let (their_in, stdin) = (Owned(read), unsafe { File::from_raw_handle(write.0 as RawHandle) });
    unsafe { SetHandleInformation(write, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }.ok()?;
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    unsafe { CreatePipe(&mut read, &mut write, Some(&inherit), 1 << 16) }.ok()?;
    let (their_out, stdout) = (Owned(write), unsafe { File::from_raw_handle(read.0 as RawHandle) });
    unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }.ok()?;
    let nul = unsafe {
        CreateFileW(w!("NUL"), GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, Some(&inherit), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None)
    }
    .ok()
    .map(Owned)?;
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: their_in.0,
        hStdOutput: their_out.0,
        hStdError: nul.0,
        ..Default::default()
    };
    let mut line: Vec<u16> = Vec::new();
    for part in [OsStr::new("\""), helper.as_os_str(), OsStr::new("\" --model \""), model.as_os_str(), OsStr::new("\"")] {
        line.extend(part.encode_wide());
    }
    line.push(0);
    let app = wide(helper.as_os_str());
    let dir = wide(helper.parent()?.as_os_str());
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(app.as_ptr()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS,
            None,
            PCWSTR(dir.as_ptr()),
            &startup,
            &mut info,
        )
    }
    .ok()?;
    // Only the helper holds its ends now: when it ends, its output closes.
    drop((their_in, their_out, nul));
    let thread = Owned(info.hThread);
    // In the job before it runs a single instruction; and not at all if
    // the job ended meanwhile.
    let joined = unsafe { AssignProcessToJobObject(job.handle, info.hProcess) }.is_ok();
    if !joined || job.stopped.load(Ordering::SeqCst) {
        unsafe {
            let _ = TerminateProcess(info.hProcess, 1);
            let _ = CloseHandle(info.hProcess);
        }
        return None;
    }
    unsafe { ResumeThread(thread.0) };
    Some((stdin, stdout, info.hProcess))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_assist::models;
    use std::fs;

    /// An exe's folder with the helper in it, and an app data folder, both
    /// empty of models.
    fn folders(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("kolada-models-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (dir, data) = (root.join("app"), root.join("data"));
        fs::create_dir_all(dir.join("models")).unwrap();
        fs::create_dir_all(data.join("models")).unwrap();
        fs::write(dir.join(HELPER), b"").unwrap();
        (dir, data)
    }

    #[test]
    fn no_model_means_no_helper() {
        let (dir, data) = folders("none");
        assert_eq!(files_in(&dir, Some(&data), None), None);
        assert_eq!(files_in(&dir, Some(&data), Some("full")), None, "chosen but not downloaded");
        // A model not trained for Kolada, a download cut short, and the
        // model's license are no model.
        fs::write(dir.join("models").join("LFM2.5-1.2B-Instruct-Q4_0.gguf"), b"x").unwrap();
        fs::write(dir.join("models").join("kolada-writer-mini-v1.gguf.part"), b"x").unwrap();
        fs::write(dir.join("models").join("LICENSE-LFM.txt"), b"x").unwrap();
        assert_eq!(files_in(&dir, Some(&data), None), None);
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_model_beside_the_exe_is_used_for_test_builds() {
        let (dir, data) = folders("beside");
        let staged = dir.join("models").join("Kolada-writer-mini-v1.gguf");
        fs::write(&staged, b"x").unwrap();
        let helper = dir.join(HELPER);
        assert_eq!(files_in(&dir, Some(&data), None), Some((helper.clone(), staged.clone())), "with no choice made");
        assert_eq!(files_in(&dir, None, Some("full")), Some((helper.clone(), staged.clone())), "over a choice not beside it");
        // Two staged: the chosen one.
        let full = models::by_id("full").unwrap();
        fs::write(dir.join("models").join(full.file), b"x").unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("full")), Some((helper.clone(), dir.join("models").join(full.file))));
        // No helper, no help.
        fs::remove_file(&helper).unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("full")), None);
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    /// A server in memory, answering every range in small pieces.
    struct Memory(Vec<u8>);

    impl fetch::Source for Memory {
        fn get_from(&self, _url: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
            for piece in self.0[from as usize..].chunks(100) {
                if !body(piece) {
                    break;
                }
            }
            Some(if from > 0 { 206 } else { 200 })
        }
    }

    #[test]
    fn a_model_downloads_telling_the_window_how_it_goes() {
        let (_, data) = folders("fetch");
        let dir = data.join("models");
        let file: Vec<u8> = (0..250_000u32).map(|i| (i % 253) as u8).collect();
        let sha = kolada_core::update::hex(&kolada_core::update::sha256(&file));
        let want = fetch::Want { url: "https://huggingface.co/x", file: "Kolada-writer-mini-v1.gguf", bytes: file.len() as u64, sha256: &sha };
        let (out, events) = channel();
        let nudges = std::cell::Cell::new(0);
        fetch_into(&dir, &want, &Memory(file.clone()), &AtomicBool::new(false), &out, &|| nudges.set(nudges.get() + 1));
        let said: Vec<Fetched> = events.try_iter().collect();
        assert_eq!(nudges.get(), said.len(), "a nudge for each");
        let steps: Vec<(u64, u64)> = said.iter().filter_map(|s| if let Fetched::Progress(d, t) = s { Some((*d, *t)) } else { None }).collect();
        // 2,500 pieces came, the window heard of at most a thousandth each.
        assert!(steps.len() > 10 && steps.len() <= 1001, "{} steps", steps.len());
        assert!(steps.windows(2).all(|w| w[0].0 < w[1].0) && steps.iter().all(|&(_, t)| t == file.len() as u64));
        assert!(matches!(said.last(), Some(Fetched::Ended(fetch::Outcome::Done(p))) if *p == dir.join(want.file)));
        assert!(fs::read(dir.join(want.file)).unwrap() == file);
        // Stopped before it starts: it says so, and nothing is in place.
        let other = fetch::Want { file: "Kolada-writer-med-v1.gguf", ..want };
        fetch_into(&dir, &other, &Memory(file), &AtomicBool::new(true), &out, &|| {});
        assert!(matches!(events.try_iter().last(), Some(Fetched::Ended(fetch::Outcome::Cancelled))));
        assert!(!dir.join(other.file).exists());
        let _ = fs::remove_dir_all(data.parent().unwrap());
    }

    #[test]
    fn the_chosen_model_comes_from_the_data_folder_by_id() {
        let (dir, data) = folders("chosen");
        let light = models::by_id("light").unwrap();
        let file = data.join("models").join(light.file);
        // Part of it: still downloading.
        fs::File::create(&file).unwrap().set_len(light.bytes - 1).unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("light")), None);
        fs::File::options().write(true).open(&file).unwrap().set_len(light.bytes).unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("light")), Some((dir.join(HELPER), file.clone())));
        // Another choice, or none, is not this file.
        assert_eq!(files_in(&dir, Some(&data), Some("full")), None);
        assert_eq!(files_in(&dir, Some(&data), None), None);
        assert_eq!(files_in(&dir, Some(&data), Some("nonsense")), None);
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }
}
