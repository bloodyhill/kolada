//! The taskbar button's menu (right-click, or pinned to Start): a new note,
//! search, and the notes written most recently. Made on a worker after the
//! first frame; each item starts Kolada, which hands it to the running one.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoUninitialize,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink};
use windows::core::{HSTRING, Interface, PWSTR, Result, w};

/// Recent notes listed at most.
const RECENT: usize = 7;

/// A string property value, as shell links take their titles.
fn text_value(s: &str) -> PROPVARIANT {
    let wide: Vec<u16> = s.encode_utf16().chain([0]).collect();
    let mut value = PROPVARIANT::default();
    unsafe {
        let buf = CoTaskMemAlloc(wide.len() * 2) as *mut u16;
        if !buf.is_null() {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), buf, wide.len());
            let inner = &mut *value.Anonymous.Anonymous;
            inner.vt = VT_LPWSTR;
            inner.Anonymous.pwszVal = PWSTR(buf);
        }
    }
    value
}

/// The exe's icons for the menu's items (resource ids, see kolada.rc).
const NOTE_ICON: i32 = 2;
const NEW_ICON: i32 = 3;
const SEARCH_ICON: i32 = 4;

/// A link that starts `exe` with `args`, titled `title`, with the exe's
/// icon resource `icon`.
fn link(exe: &Path, args: &str, title: &str, tip: &str, icon: i32) -> Result<IShellLinkW> {
    unsafe {
        let l: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let exe = HSTRING::from(exe.as_os_str());
        l.SetPath(&exe)?;
        l.SetArguments(&HSTRING::from(args))?;
        l.SetDescription(&HSTRING::from(tip))?;
        // A negative index names an icon by its resource id.
        l.SetIconLocation(&exe, -icon)?;
        let store: IPropertyStore = l.cast()?;
        let mut value = text_value(title);
        store.SetValue(&PKEY_Title, &value)?;
        store.Commit()?;
        let _ = PropVariantClear(&mut value);
        Ok(l)
    }
}

/// The vault's notes written most recently, newest first.
fn recent_notes(vault: &Path, limit: usize) -> Vec<(SystemTime, PathBuf)> {
    let mut found = Vec::new();
    let mut dirs = vec![vault.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(e.path()),
                Ok(_) if name.to_lowercase().ends_with(".md") => {
                    if let Ok(modified) = e.metadata().and_then(|m| m.modified()) {
                        found.push((modified, e.path()));
                    }
                }
                _ => {}
            }
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.truncate(limit);
    found
}

/// The arguments of the links the user took off the menu: they stay off.
fn removed(list: &IObjectArray) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        for i in 0..list.GetCount().unwrap_or(0) {
            if let Ok(l) = list.GetAt::<IShellLinkW>(i) {
                let mut buf = [0u16; 1024];
                if l.GetArguments(&mut buf).is_ok() {
                    let n = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
                    out.push(String::from_utf16_lossy(&buf[..n]));
                }
            }
        }
    }
    out
}

fn make(vault: &Path) -> Result<()> {
    let exe = std::env::current_exe().map_err(|_| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
    unsafe {
        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        let mut slots = 0u32;
        let gone: IObjectArray = list.BeginList(&mut slots)?;
        let gone = removed(&gone);
        let tasks: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        tasks.AddObject(&link(&exe, "--new", "New note", "Start writing on a fresh page", NEW_ICON)?)?;
        tasks.AddObject(&link(&exe, "--search", "Search notes", "Find a note by its name or its words", SEARCH_ICON)?)?;
        list.AddUserTasks(&tasks.cast::<IObjectArray>()?)?;
        let notes: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        let mut any = false;
        for (_, path) in recent_notes(vault, RECENT.min(slots as usize)) {
            let args = format!("\"{}\"", path.display());
            if gone.contains(&args) {
                continue;
            }
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            notes.AddObject(&link(&exe, &args, &name, &path.to_string_lossy(), NOTE_ICON)?)?;
            any = true;
        }
        if any {
            list.AppendCategory(w!("Recent notes"), &notes.cast::<IObjectArray>()?)?;
        }
        list.CommitList()
    }
}

/// Makes the menu on a worker, for the vault at `vault`.
pub fn build(vault: String) {
    if vault.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        crate::background_priority();
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let _ = make(Path::new(&vault));
        unsafe { CoUninitialize() };
    });
}
