//! Builds llama.cpp (CPU only, static, on the static C runtime like the rest
//! of Kolada) and the small C shim that Rust talks to.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let here = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let llama = here.parent().unwrap().parent().unwrap().join("third_party").join("llama.cpp");
    if !llama.join("CMakeLists.txt").is_file() {
        panic!("llama.cpp is missing at {}: run `git submodule update --init third_party/llama.cpp`", llama.display());
    }
    println!("cargo:rerun-if-changed=shim/nai.c");
    println!("cargo:rerun-if-changed={}", head(&llama).display());
    println!("cargo:rerun-if-env-changed=CMAKE");

    let cmake = find_cmake();
    if env::var_os("CMAKE").is_none()
        && let Some(c) = &cmake
    {
        // The cmake crate takes its executable only from the environment.
        unsafe { env::set_var("CMAKE", c) };
    }

    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let x86 = matches!(env::var("CARGO_CFG_TARGET_ARCH").as_deref(), Ok("x86_64" | "x86"));
    let mut cfg = cmake::Config::new(&llama);
    // Ninja builds several times faster than MSBuild; the VS generator is
    // the fallback when it is not beside cmake.
    if let Some(ninja) = cmake.as_deref().and_then(find_ninja) {
        cfg.generator("Ninja").define("CMAKE_MAKE_PROGRAM", &ninja);
    }
    cfg
        // A debug llama.cpp is too slow to answer at all.
        .profile("Release")
        .static_crt(true)
        .define("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreaded")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("GGML_NATIVE", "OFF")
        .define("GGML_OPENMP", "OFF")
        .define("GGML_CCACHE", "OFF")
        // CPU only: the GPU must never wake for a paragraph.
        .define("GGML_CUDA", "OFF")
        .define("GGML_VULKAN", "OFF")
        .define("GGML_OPENCL", "OFF")
        .define("GGML_BLAS", "OFF")
        .define("GGML_METAL", "OFF")
        .define("GGML_RPC", "OFF")
        .define("GGML_BUILD_TESTS", "OFF")
        .define("GGML_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_CURL", "OFF")
        .define("LLAMA_OPENSSL", "OFF")
        .define("LLAMA_BUILD_COMMON", "OFF")
        .define("LLAMA_BUILD_TESTS", "OFF")
        .define("LLAMA_BUILD_TOOLS", "OFF")
        .define("LLAMA_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_BUILD_SERVER", "OFF")
        .define("LLAMA_BUILD_APP", "OFF")
        .define("LLAMA_BUILD_MTMD", "OFF")
        .define("LLAMA_TOOLS_INSTALL", "OFF")
        .define("LLAMA_ALL_WARNINGS", "OFF");
    if x86 {
        // A fixed x86 baseline; other chips (Apple silicon, arm64) take the
        // compiler's own NEON defaults.
        for flag in ["GGML_AVX", "GGML_AVX2", "GGML_BMI2", "GGML_FMA", "GGML_F16C"] {
            cfg.define(flag, "ON");
        }
        cfg.define("GGML_AVX512", "OFF");
    }
    let dst = cfg.build();

    let lib = ["lib", "lib64"].iter().map(|d| dst.join(d)).find(|d| d.is_dir()).unwrap_or_else(|| dst.join("lib"));
    println!("cargo:rustc-link-search=native={}", lib.display());
    let mut libs: Vec<String> = fs::read_dir(&lib)
        .unwrap_or_else(|e| panic!("no libraries in {}: {e}", lib.display()))
        .filter_map(|e| {
            let p = e.ok()?.path();
            let ext = p.extension()?.to_str()?;
            if ext != "lib" && ext != "a" {
                return None;
            }
            let stem = p.file_stem()?.to_str()?;
            Some(stem.strip_prefix("lib").filter(|_| ext == "a").unwrap_or(stem).to_string())
        })
        .collect();
    // Unix linkers resolve static libraries in order: llama needs ggml, which
    // needs its cpu backend, which needs the base.
    let rank = |l: &str| ["llama", "ggml", "ggml-cpu", "ggml-base"].iter().position(|n| *n == l).unwrap_or(1);
    libs.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
    for l in &libs {
        println!("cargo:rustc-link-lib=static={l}");
    }
    match os.as_str() {
        // ggml-cpu reads the processor's name from the registry.
        "windows" => println!("cargo:rustc-link-lib=advapi32"),
        "macos" => {
            println!("cargo:rustc-link-lib=c++");
            println!("cargo:rustc-link-lib=framework=Accelerate");
        }
        "linux" => {
            println!("cargo:rustc-link-lib=stdc++");
            println!("cargo:rustc-link-lib=m");
            println!("cargo:rustc-link-lib=pthread");
        }
        _ => {}
    }

    cc::Build::new()
        .file(here.join("shim").join("nai.c"))
        .include(llama.join("include"))
        .include(llama.join("ggml").join("include"))
        .static_crt(true)
        .opt_level(2)
        .warnings(true)
        // llama.h's unions have no names, which C11 allows.
        .flag_if_supported("/wd4201")
        .flag_if_supported("/utf-8")
        .compile("nai");
    // Linux: the helper starts on glibcs older than the one it was built on.
    if os == "linux" && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        println!("cargo:rerun-if-changed=shim/glibc.c");
        cc::Build::new().file(here.join("shim").join("glibc.c")).std("c17").opt_level(2).warnings(true).compile("naiglibc");
    }
}

/// The file that changes when the submodule moves to another commit.
fn head(llama: &Path) -> PathBuf {
    let git = llama.join(".git");
    if git.is_dir() {
        return git.join("HEAD");
    }
    if let Ok(s) = fs::read_to_string(&git)
        && let Some(dir) = s.trim().strip_prefix("gitdir:")
    {
        return llama.join(dir.trim()).join("HEAD");
    }
    llama.join("CMakeLists.txt")
}

fn on_path(exe: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?).map(|d| d.join(exe)).find(|p| p.is_file())
}

/// cmake from the environment or PATH, else the one inside Visual Studio.
fn find_cmake() -> Option<PathBuf> {
    if let Some(c) = env::var_os("CMAKE") {
        return Some(c.into());
    }
    if let Some(c) = on_path("cmake.exe").or_else(|| on_path("cmake")) {
        return Some(c);
    }
    let inside = Path::new(r"Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe");
    let vswhere = Path::new(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    if let Ok(out) = Command::new(vswhere).args(["-products", "*", "-latest", "-property", "installationPath"]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let c = Path::new(line.trim()).join(inside);
            if c.is_file() {
                return Some(c);
            }
        }
    }
    let fixed = Path::new(r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools").join(inside);
    fixed.is_file().then_some(fixed)
}

/// Ninja on PATH, or the one Visual Studio ships beside its cmake.
fn find_ninja(cmake: &Path) -> Option<PathBuf> {
    on_path("ninja.exe").or_else(|| on_path("ninja")).or_else(|| {
        let n = cmake.parent()?.parent()?.parent()?.join("Ninja").join("ninja.exe");
        n.is_file().then_some(n)
    })
}
