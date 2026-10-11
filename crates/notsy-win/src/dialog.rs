//! The system's file dialogs, for importing and exporting. COM and the shell
//! load on the first call, never at startup (ole32 and shell32 are
//! delay-loaded).

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileDialog,
    IFileOpenDialog, IFileSaveDialog, IShellItem, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, Interface, PCWSTR, Result};

fn init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
}

fn path_of(item: &IShellItem) -> Result<PathBuf> {
    unsafe {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string().map_err(|_| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(PathBuf::from(path?))
    }
}

/// Sets a dialog's title and its file types: (name, pattern) pairs such as
/// ("Notes", "*.md;*.txt"). The strings must outlive the call to Show.
fn setup(d: &IFileDialog, title: &str, filters: &[(&str, &str)], keep: &mut Vec<HSTRING>) -> Result<()> {
    unsafe {
        d.SetTitle(&HSTRING::from(title))?;
        if !filters.is_empty() {
            for (name, spec) in filters {
                keep.push(HSTRING::from(*name));
                keep.push(HSTRING::from(*spec));
            }
            let specs: Vec<COMDLG_FILTERSPEC> = keep
                .chunks(2)
                .map(|p| COMDLG_FILTERSPEC { pszName: PCWSTR(p[0].as_ptr()), pszSpec: PCWSTR(p[1].as_ptr()) })
                .collect();
            d.SetFileTypes(&specs)?;
        }
    }
    Ok(())
}

/// Where to save a file of one of `types` (name, pattern, extension; the
/// first offered first), named `stem`, and which type was chosen; none if
/// the user cancels.
pub fn save_file_as(hwnd: HWND, title: &str, stem: &str, types: &[(&str, &str, &str)]) -> Option<(PathBuf, usize)> {
    init();
    let pick = || -> Result<(PathBuf, usize)> {
        unsafe {
            let d: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
            d.SetOptions(d.GetOptions()? | FOS_FORCEFILESYSTEM)?;
            let mut keep = Vec::new();
            let filters: Vec<(&str, &str)> = types.iter().map(|(name, spec, _)| (*name, *spec)).collect();
            setup(&d.cast()?, title, &filters, &mut keep)?;
            d.SetFileName(&HSTRING::from(stem))?;
            if let Some((_, _, ext)) = types.first() {
                d.SetDefaultExtension(&HSTRING::from(*ext))?;
            }
            d.Show(Some(hwnd))?;
            // One-based, as the dialog counts its types.
            let chosen = d.GetFileTypeIndex()?.saturating_sub(1) as usize;
            Ok((path_of(&d.GetResult()?)?, chosen))
        }
    };
    pick().ok()
}

/// Files the user picks, or none if they cancel.
pub fn open_files(hwnd: HWND, title: &str, filters: &[(&str, &str)]) -> Vec<PathBuf> {
    init();
    let pick = || -> Result<Vec<PathBuf>> {
        unsafe {
            let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            d.SetOptions(d.GetOptions()? | FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)?;
            let mut keep = Vec::new();
            setup(&d.cast()?, title, filters, &mut keep)?;
            d.Show(Some(hwnd))?;
            let items = d.GetResults()?;
            (0..items.GetCount()?).map(|i| path_of(&items.GetItemAt(i)?)).collect()
        }
    };
    pick().unwrap_or_default()
}

/// A folder the user picks.
pub fn pick_folder(hwnd: HWND, title: &str) -> Option<PathBuf> {
    init();
    let pick = || -> Result<PathBuf> {
        unsafe {
            let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            d.SetOptions(d.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
            d.SetTitle(&HSTRING::from(title))?;
            d.Show(Some(hwnd))?;
            path_of(&d.GetResult()?)
        }
    };
    pick().ok()
}

/// Where the user saves a file, named `name` to start with, of the type
/// `filter` with the extension `ext`.
pub fn save_file(hwnd: HWND, title: &str, name: &str, filter: (&str, &str), ext: &str) -> Option<PathBuf> {
    init();
    let pick = || -> Result<PathBuf> {
        unsafe {
            let d: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
            d.SetOptions(d.GetOptions()? | FOS_FORCEFILESYSTEM)?;
            let mut keep = Vec::new();
            setup(&d.cast()?, title, &[filter], &mut keep)?;
            d.SetFileName(&HSTRING::from(name))?;
            d.SetDefaultExtension(&HSTRING::from(ext))?;
            d.Show(Some(hwnd))?;
            path_of(&d.GetResult()?)
        }
    };
    pick().ok()
}
