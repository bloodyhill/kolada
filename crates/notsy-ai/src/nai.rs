//! The shim's C API (`shim/nai.c`) and a safe face on it.

use std::ffi::{CString, OsStr, c_char, c_int};
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::atomic::AtomicI32;

#[repr(C)]
struct Raw {
    _private: [u8; 0],
}

/// The last run, as the shim timed it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub n_prompt: c_int,
    /// Of those, restored from a kept opening rather than read.
    pub n_cached: c_int,
    pub n_out: c_int,
    /// Tokens the anchor held against the model's choice.
    pub n_kept: c_int,
    /// Times the answer left the anchor anyway.
    pub n_left: c_int,
    pub prefill_ms: f64,
    pub decode_ms: f64,
}

unsafe extern "C" {
    fn nai_log(path: *const c_char);
    fn nai_trace(on: c_int);
    fn nai_note(text: *const c_char, len: c_int);
    fn nai_open(path: *const c_char, threads: c_int, threads_batch: c_int, n_ctx: c_int) -> *mut Raw;
    fn nai_close(h: *mut Raw);
    fn nai_n_ctx(h: *const Raw) -> c_int;
    fn nai_template(h: *const Raw) -> *const c_char;
    fn nai_count_tokens(h: *const Raw, text: *const c_char, len: c_int) -> c_int;
    fn nai_chat(h: *const Raw, roles: *const *const c_char, texts: *const *const c_char, n: c_int, open: c_int, out: *mut c_char, cap: c_int) -> c_int;
    fn nai_last(h: *const Raw, s: *mut Stats);
    fn nai_anchor(h: *mut Raw, text: *const c_char, len: c_int, near: f32, far: f32) -> c_int;
    fn nai_prefix(h: *mut Raw, len: c_int);
    fn nai_run(
        h: *mut Raw,
        prompt: *const c_char,
        prompt_len: c_int,
        grammar: *const c_char,
        max_tokens: c_int,
        cancel: *const c_int,
        out: *mut c_char,
        cap: c_int,
        out_len: *mut c_int,
    ) -> c_int;
}

/// Sends llama.cpp's own log, and `note`, to the file at `path`; with none,
/// llama.cpp says nothing. `trace` adds each choice a fix's anchor weighs.
pub fn log(path: Option<&OsStr>, trace: bool) {
    let path = path.and_then(OsStr::to_str).and_then(|p| CString::new(p).ok());
    unsafe {
        nai_log(path.as_ref().map_or(ptr::null(), |p| p.as_ptr()));
        nai_trace(c_int::from(trace));
    }
}

/// One line in the log, if there is one.
pub fn note(line: &str) {
    let line = format!("notsy-ai: {line}\n");
    unsafe { nai_note(line.as_ptr().cast(), line.len() as c_int) };
}

pub enum Run {
    Done(String),
    Cancelled,
    /// Still going at its most tokens.
    Long,
    Failed,
}

pub struct Model(NonNull<Raw>);

impl Model {
    pub fn open(path: &Path, threads: i32, threads_batch: i32, n_ctx: i32) -> Option<Model> {
        let path = CString::new(path.to_str()?).ok()?;
        NonNull::new(unsafe { nai_open(path.as_ptr(), threads, threads_batch, n_ctx) }).map(Model)
    }

    pub fn n_ctx(&self) -> usize {
        unsafe { nai_n_ctx(self.0.as_ptr()) }.max(0) as usize
    }

    /// The chat template the model came with.
    pub fn template(&self) -> Option<String> {
        let t = unsafe { nai_template(self.0.as_ptr()) };
        (!t.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(t) }.to_string_lossy().into_owned())
    }

    /// `turns` (role, text) in the model's own chat format, open for its
    /// answer when `open`; None if llama.cpp does not know its template.
    pub fn chat(&self, turns: &[(&str, &str)], open: bool) -> Option<String> {
        let roles: Vec<CString> = turns.iter().map(|(r, _)| CString::new(*r)).collect::<Result<_, _>>().ok()?;
        let texts: Vec<CString> = turns.iter().map(|(_, t)| CString::new(*t)).collect::<Result<_, _>>().ok()?;
        let (rp, tp): (Vec<*const c_char>, Vec<*const c_char>) = roles.iter().zip(&texts).map(|(r, t)| (r.as_ptr(), t.as_ptr())).unzip();
        let cap = turns.iter().map(|(r, t)| r.len() + t.len() + 64).sum::<usize>() * 2 + 256;
        let mut out = vec![0u8; cap];
        let len = unsafe { nai_chat(self.0.as_ptr(), rp.as_ptr(), tp.as_ptr(), turns.len() as c_int, c_int::from(open), out.as_mut_ptr().cast(), cap as c_int) };
        out.truncate(usize::try_from(len).ok()?);
        String::from_utf8(out).ok()
    }

    pub fn tokens(&self, text: &str) -> usize {
        unsafe { nai_count_tokens(self.0.as_ptr(), text.as_ptr().cast(), text.len() as c_int) }.max(0) as usize
    }

    pub fn last(&self) -> Stats {
        let mut s = Stats::default();
        unsafe { nai_last(self.0.as_ptr(), &mut s) };
        s
    }

    /// The next run copies `text` and leaves it only where the model
    /// prefers another token by a log-odds margin: `near` when that token is
    /// spelled like the one written, `far` otherwise.
    pub fn anchor(&mut self, text: &str, near: f32, far: f32) {
        unsafe { nai_anchor(self.0.as_ptr(), text.as_ptr().cast(), text.len() as c_int, near, far) };
    }

    /// One answer to `prompt`, whose first `fixed` bytes many prompts share
    /// (their state is kept and restored); `cancel` turning nonzero stops it.
    pub fn run(&mut self, prompt: &str, fixed: usize, grammar: Option<&str>, max_tokens: usize, cancel: &AtomicI32) -> Run {
        unsafe { nai_prefix(self.0.as_ptr(), fixed as c_int) };
        let Ok(grammar) = grammar.map(CString::new).transpose() else { return Run::Failed };
        let cap = max_tokens * 32 + 64;
        let mut out = vec![0u8; cap];
        let mut len: c_int = 0;
        let rc = unsafe {
            nai_run(
                self.0.as_ptr(),
                prompt.as_ptr().cast(),
                prompt.len() as c_int,
                grammar.as_ref().map_or(ptr::null(), |g| g.as_ptr()),
                max_tokens as c_int,
                cancel.as_ptr(),
                out.as_mut_ptr().cast(),
                cap as c_int,
                &mut len,
            )
        };
        out.truncate(len.max(0) as usize);
        match rc {
            0 => String::from_utf8(out).map_or(Run::Failed, Run::Done),
            1 => Run::Cancelled,
            2 => Run::Long,
            _ => Run::Failed,
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { nai_close(self.0.as_ptr()) };
    }
}
