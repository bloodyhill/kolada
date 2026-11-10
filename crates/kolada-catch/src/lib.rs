//! Catcher for Kolada's core, for the browser extension: a page caught as a note
//! by the app's own HTML reader (kolada-convert), and sent through Beam by
//! the app's own engine (kolada-sync), as a paired device of kind `other`
//! that never syncs notes. The extension's script only reads the page,
//! fetches pictures, opens sockets and makes requests; every rule is here,
//! and tested here.
//!
//! The WebAssembly interface, plain numbers only (no bindings generator),
//! as kolada-share-render's:
//!
//! - `memory`, `alloc(len) -> ptr`, `free(ptr, len)`, `out_len()`.
//! - `call(ptr, len) -> ptr`: one request, a frame (`kolada_sync::frame`:
//!   `t` names it, see `Catch::call`), answered with frames, each as a
//!   big-endian `u32` length and its bytes.
//! - Imports from module `kolada`: `random(ptr, len)`, filled from
//!   `crypto.getRandomValues`.

pub mod clip;
pub mod core;

pub use crate::core::Catch;

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

    /// The bytes at `ptr`.
    ///
    /// # Safety
    /// `ptr..ptr + len` is memory the caller wrote (or empty).
    pub unsafe fn read(ptr: usize, len: usize) -> Vec<u8> {
        if len == 0 || ptr == 0 {
            return Vec::new();
        }
        unsafe { std::slice::from_raw_parts(ptr as *const u8, len) }.to_vec()
    }

    /// `bytes` handed to the caller: its address, its length kept for
    /// `len`. Allocated as `take(len)` would, so `give_back(ptr, len)`
    /// frees it.
    pub fn give(bytes: Vec<u8>) -> usize {
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
    use std::cell::RefCell;

    thread_local! {
        static CATCH: RefCell<super::Catch> = RefCell::new(super::Catch::new());
    }

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
    /// The range is memory the caller wrote.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn call(ptr: u32, len: u32) -> u32 {
        let req = unsafe { memory::read(ptr as usize, len as usize) };
        let out = CATCH.with(|c| c.borrow_mut().call(&req));
        memory::give(out) as u32
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn out_len() -> u32 {
        memory::len() as u32
    }

    #[link(wasm_import_module = "kolada")]
    unsafe extern "C" {
        fn random(ptr: *mut u8, len: usize);
    }

    fn fill(buf: &mut [u8]) -> Result<(), getrandom02::Error> {
        unsafe { random(buf.as_mut_ptr(), buf.len()) };
        Ok(())
    }
    getrandom02::register_custom_getrandom!(fill);

    /// getrandom 0.3's custom backend (built with
    /// `--cfg getrandom_backend="custom"`).
    ///
    /// # Safety
    /// `dest..dest + len` is writable.
    #[unsafe(no_mangle)]
    unsafe extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), getrandom03::Error> {
        unsafe { random(dest, len) };
        Ok(())
    }
}
