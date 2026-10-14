//! The installer's launcher, on Windows: Kolada installs itself. This puts
//! the Kolada it carries in a folder of its own under the temp folder, runs
//! it with `--setup <this program>` (it reads the files to install from
//! here, and draws the installer's screen with its own engine), waits for
//! it, and deletes the copy.

use std::ffi::OsString;
use std::fs;
use std::path::Path;

use kolada_core::pack::{self, File};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::HSTRING;

const APP: &str = "kolada.exe";

pub fn run(args: Vec<OsString>) -> i32 {
    if args.first().is_some_and(|a| a == "--pack") {
        return pack_files(&args[1..]);
    }
    let quiet = args.iter().any(|a| a == "--quiet");
    match launch(&args) {
        Ok(code) => code,
        Err(e) => {
            if !quiet {
                unsafe {
                    MessageBoxW(None, &HSTRING::from(e.as_str()), &HSTRING::from("Kolada"), MB_OK | MB_ICONERROR);
                }
            }
            1
        }
    }
}

/// `--pack <out.exe> <file>...`: this stub with the files after it.
fn pack_files(args: &[OsString]) -> i32 {
    let Some((out, files)) = args.split_first() else { return 2 };
    let Ok(me) = std::env::current_exe().and_then(fs::read) else { return 3 };
    let mut carried = Vec::new();
    for f in files {
        let path = Path::new(f);
        let (Some(name), Ok(bytes)) = (path.file_name().and_then(|n| n.to_str()), fs::read(path)) else { return 4 };
        carried.push(File { name: name.to_string(), bytes });
    }
    if !carried.iter().any(|f| f.name == APP) {
        return 5;
    }
    match fs::write(out, pack::pack(pack::stub(&me), &carried)) {
        Ok(()) => 0,
        Err(_) => 6,
    }
}

fn launch(args: &[OsString]) -> Result<i32, String> {
    let me = std::env::current_exe().map_err(|e| format!("Setup could not find itself: {e}"))?;
    let bytes = fs::read(&me).map_err(|e| format!("Setup could not read itself: {e}"))?;
    let files = pack::unpack(&bytes).ok_or("This setup has no Kolada inside it.")?;
    let app = files.iter().find(|f| f.name == APP).ok_or("This setup has no Kolada inside it.")?;
    let dir = std::env::temp_dir().join(format!("kolada-setup-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| format!("Setup could not unpack Kolada: {e}"))?;
    let exe = dir.join(APP);
    fs::write(&exe, &app.bytes).map_err(|e| format!("Setup could not unpack Kolada: {e}"))?;
    let status = std::process::Command::new(&exe).arg("--setup").arg(&me).args(args).status();
    let _ = fs::remove_dir_all(&dir);
    status.map(|s| s.code().unwrap_or(1)).map_err(|e| format!("Setup could not start Kolada: {e}"))
}
