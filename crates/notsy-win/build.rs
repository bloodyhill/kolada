//! Linker setup for the startup path:
//! - embeds the application manifest (per-monitor DPI v2, supported OS,
//!   long paths) with the MSVC linker, so no resource compiler is needed;
//! - delay-loads every DLL the first frame does not use, so the loader maps
//!   them on first call (after the first frame) instead of before `main`;
//! - compiles notsy.rc, which holds the app icon.

use std::env;
use std::path::Path;

/// DLLs not needed before the first frame. Keep in sync with the startup
/// path in src/main.rs; the startup trace shows if one slips back in.
const DELAY_LOAD: &[&str] = &["oleaut32.dll", "advapi32.dll", "shell32.dll", "ole32.dll", "imm32.dll", "winhttp.dll"];

fn main() {
    let dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let manifest = Path::new(&dir).join("notsy.exe.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed=notsy.rc");
    println!("cargo:rerun-if-changed=notsy.ico");
    for icon in ["jump-note.ico", "jump-new.ico", "jump-search.ico"] {
        println!("cargo:rerun-if-changed={icon}");
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
        for dll in DELAY_LOAD {
            println!("cargo:rustc-link-arg-bins=/DELAYLOAD:{dll}");
        }
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
        // The app icon. The manifest is embedded by the linker above, not here.
        embed_resource::compile("notsy.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("compiling notsy.rc");
    }
}
