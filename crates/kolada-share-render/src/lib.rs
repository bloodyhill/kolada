//! A shared note rendered once, when its link is made, by the same code the
//! app exports with: the notsy.co Worker loads this crate as WebAssembly
//! (`tools/build-share-wasm.sh` builds it into `services/notsy/wasm/`), so
//! the page reads exactly as the note does in Kolada.
//!
//! The WebAssembly interface, plain numbers only (no bindings generator):
//!
//! - `memory`: the module's memory.
//! - `alloc(len) -> ptr`: room for `len` bytes, for the caller to write
//!   UTF-8 into; `free(ptr, len)` gives it back.
//! - `html(md_ptr, md_len, prefix_ptr, prefix_len) -> ptr`: the note as
//!   its public page's body (`kolada_markdown::html::to_public_html`), its
//!   pictures under `prefix` (`/{id}/i/`).
//! - `plain(md_ptr, md_len) -> ptr`: the note as plain text
//!   (`kolada_markdown::plain::to_plain`).
//! - `title(md_ptr, md_len) -> ptr`: its title, as the editor names notes
//!   (`kolada_markdown::plain::title`).
//! - `out_len() -> len`: the byte length of the last output. The caller
//!   frees each output with `free(ptr, out_len())`.
//!
//! Text that is not UTF-8 is read with replacement characters. Every
//! output is UTF-8.

use kolada_markdown::{html, plain};

/// The page's body for the note `md`, its pictures under `prefix`.
pub fn render_html(md: &str, prefix: &str) -> String {
    html::to_public_html(md, prefix)
}

/// The note as plain text.
pub fn render_plain(md: &str) -> String {
    plain::to_plain(md)
}

/// The note's title.
pub fn render_title(md: &str) -> String {
    plain::title(md)
}

/// The memory the interface hands out and takes back, by address. Byte
/// buffers only, so their alignment is 1.
mod memory {
    use std::alloc::{Layout, alloc, dealloc};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static OUT_LEN: AtomicUsize = AtomicUsize::new(0);

    /// Room for `len` bytes; never null (an empty one is a dangling
    /// address that `free` leaves alone).
    pub fn take(len: usize) -> usize {
        if len == 0 {
            return 1;
        }
        let Ok(layout) = Layout::array::<u8>(len) else { return 0 };
        unsafe { alloc(layout) as usize }
    }

    /// Gives back what `take(len)` (or `give`, for an output of `len`
    /// bytes) handed out.
    ///
    /// # Safety
    /// `ptr` came from `take(len)` or `give`, with this `len`, and is not
    /// used again.
    pub unsafe fn give_back(ptr: usize, len: usize) {
        if len == 0 || ptr == 0 {
            return;
        }
        if let Ok(layout) = Layout::array::<u8>(len) {
            unsafe { dealloc(ptr as *mut u8, layout) }
        }
    }

    /// The bytes at `ptr`, as text.
    ///
    /// # Safety
    /// `ptr..ptr + len` is memory the caller wrote (or empty).
    pub unsafe fn read(ptr: usize, len: usize) -> String {
        if len == 0 || ptr == 0 {
            return String::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// `s` handed to the caller: its address, its length kept for `len`.
    /// Allocated as `take(len)` would, so `give_back(ptr, len)` frees it.
    pub fn give(s: String) -> usize {
        let bytes = s.into_bytes();
        OUT_LEN.store(bytes.len(), Ordering::Relaxed);
        let ptr = take(bytes.len());
        if !bytes.is_empty() && ptr != 0 {
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len()) };
        }
        ptr
    }

    pub fn len() -> usize {
        OUT_LEN.load(Ordering::Relaxed)
    }
}

/// The exports, on WebAssembly only: addresses there are 32 bits.
#[cfg(target_arch = "wasm32")]
mod exports {
    use super::memory;

    #[unsafe(no_mangle)]
    pub extern "C" fn alloc(len: u32) -> u32 {
        memory::take(len as usize) as u32
    }

    /// # Safety
    /// See `memory::give_back`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn free(ptr: u32, len: u32) {
        unsafe { memory::give_back(ptr as usize, len as usize) }
    }

    /// # Safety
    /// Both ranges are memory the caller wrote.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn html(md_ptr: u32, md_len: u32, prefix_ptr: u32, prefix_len: u32) -> u32 {
        let md = unsafe { memory::read(md_ptr as usize, md_len as usize) };
        let prefix = unsafe { memory::read(prefix_ptr as usize, prefix_len as usize) };
        memory::give(super::render_html(&md, &prefix)) as u32
    }

    /// # Safety
    /// The range is memory the caller wrote.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn plain(md_ptr: u32, md_len: u32) -> u32 {
        let md = unsafe { memory::read(md_ptr as usize, md_len as usize) };
        memory::give(super::render_plain(&md)) as u32
    }

    /// # Safety
    /// The range is memory the caller wrote.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn title(md_ptr: u32, md_len: u32) -> u32 {
        let md = unsafe { memory::read(md_ptr as usize, md_len as usize) };
        memory::give(super::render_title(&md)) as u32
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn out_len() -> u32 {
        memory::len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shared_note_renders_as_the_app_exports_it() {
        let md = "---\nprivate: yes\n---\n# Trip **notes**\n- [x] packed\n![](0.jpg)\n[site](https://x.io) [bad](javascript:x)";
        let html = render_html(md, "/Ab3d/i/");
        assert!(html.starts_with("<h1 dir=\"auto\">Trip <strong>notes</strong></h1>"), "{html}");
        assert!(html.contains("src=\"/Ab3d/i/0.jpg\"") && !html.contains("javascript") && !html.contains("private"), "{html}");
        assert_eq!(render_plain(md), "Trip notes\n\u{2611} packed\nsite (https://x.io) bad");
        assert_eq!(render_title(md), "Trip notes");
    }

    /// The interface's memory, as the Worker uses it: write the note in,
    /// read each output out by `out_len`, free both.
    #[test]
    fn the_interface_hands_out_and_takes_back_memory() {
        let md = "Hello **world** \u{0645}\u{0644}";
        let input = memory::take(md.len());
        unsafe { std::ptr::copy_nonoverlapping(md.as_ptr(), input as *mut u8, md.len()) };
        let out = memory::give(render_plain(&unsafe { memory::read(input, md.len()) }));
        let len = memory::len();
        let text = unsafe { std::str::from_utf8(std::slice::from_raw_parts(out as *const u8, len)).unwrap().to_string() };
        assert_eq!(text, "Hello world \u{0645}\u{0644}");
        unsafe {
            memory::give_back(out, len);
            memory::give_back(input, md.len());
        }
        // Empty in, empty out: nothing to free, no null.
        let out = memory::give(render_title(""));
        assert_eq!((memory::len(), out != 0), (0, true));
        unsafe { memory::give_back(out, 0) };
        // Bytes that are not UTF-8 read with replacement characters.
        let bad = [b'a', 0xFF, b'b'];
        assert_eq!(unsafe { memory::read(bad.as_ptr() as usize, 3) }, "a\u{FFFD}b");
    }
}
