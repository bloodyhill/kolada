//! The writing model's own process, `kolada-ai.exe`, beside this one. It
//! starts a moment after the window shows, below normal priority, in a job
//! that ends it with Kolada (or when writing help is turned off). Requests go
//! down its standard input as frames (see `kolada_assist::wire`) from a
//! writer thread; a reader thread takes its replies off its standard output
//! into a channel and nudges the window. Without its files it stays off,
//! without a word.

use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use kolada_assist::wire::{self, Reply, Request};
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
use crate::window::WM_APP_ASSIST;

/// The one model, beside the exe or in the app's data (shared by every
/// Kolada, test instances too): LFM2.5-1.2B-Instruct trained for Kolada's
/// fixes (tools/train), which its name tells kolada-ai. A build may name
/// another (KOLADA_MODEL_FILE, as tools/stage.ps1 -Model sets it).
const MODEL: &str = match option_env!("KOLADA_MODEL_FILE") {
    Some(m) => m,
    None => kolada_core::before::MODEL,
};

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
    /// sent. What it answers arrives in `events`, and `hwnd` is nudged.
    pub fn start(hwnd: HWND) -> Option<Helper> {
        let job = Arc::new(Job::new()?);
        let (frames, pending) = channel::<Vec<u8>>();
        let (events_out, events) = channel();
        let (window, their_job) = (Handoff(hwnd), job.clone());
        std::thread::spawn(move || {
            let hwnd = window;
            let job = their_job;
            let started = files().and_then(|(helper, model)| spawn(&helper, &model, &job));
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

/// The helper and its model, if both are there.
fn files() -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    // An install updated from Notsy may still have the helper by its old name.
    let helper = [HELPER, kolada_core::before::HELPER].into_iter().map(|n| exe.with_file_name(n)).find(|h| h.is_file())?;
    let beside = exe.parent()?.join("models").join(MODEL);
    if beside.is_file() {
        return Some((helper, beside));
    }
    // The real app data, not the test instance's: the model is shared.
    let shared = kolada_core::real_data_dir()?.join("models").join(MODEL);
    shared.is_file().then_some((helper, shared))
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
