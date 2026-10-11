//! notsy-bench: measures how long an app takes from launch until its window
//! is visually complete, using the real frames Windows puts on the screen.
//!
//! ```text
//! notsy-bench launch --label <name> --exe <path> [options]
//! notsy-bench monitors
//! ```
//!
//! Launch options:
//!   --arg <a>            argument for the app (repeatable)
//!   --shell              launch through ShellExecute, like a Start-menu click
//!                        (resolves .lnk shortcuts and app execution aliases)
//!   --process <img.exe>  process image name that owns the window (repeatable;
//!                        default: the file name of --exe)
//!   --runs <n>           measured runs (default 30)
//!   --warmup <n>         unmeasured runs first (default 2)
//!   --between quit|hide  quit: close + terminate between runs, so every run
//!                        is a true process start (default). hide: only close
//!                        the window, for apps that stay resident in the tray.
//!   --cooldown-ms <n>    idle time before each launch (default 2000)
//!   --settle-ms <n>      quiet time that ends a run (default 2500)
//!   --timeout-ms <n>     give up after this long (default 15000)
//!   --monitor <i>        monitor index from `monitors` (default 0)
//!   --pixel-delta <n>    luminance change that counts (default 24)
//!   --min-pixels <n>     changed thumbnail pixels that count (default 16)
//!   --out <dir>          results directory (default bench-results)
//!   --save-frames        save baseline/first/complete/final frames as BMP,
//!                        plus a per-frame trace CSV
//!   --save-all-frames    save every captured frame as BMP (debugging)
//!
//! Timings are measured from the moment just before the launch call:
//!   window_shown       the app's main window first became visible (polled)
//!   first_pixel        first presented frame where the window area changed
//!   complete_98        first presented frame after which the window area
//!                      stays within 2% of its final look (the headline
//!                      number: ignores late cosmetic changes such as
//!                      auto-collapsing scrollbars, not missing content)
//!   visually_complete  first presented frame after which the window area
//!                      stays the same (a blinking caret is ignored)

mod analyze;
mod capture;
mod clock;
mod procs;
mod report;
mod stage;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use windows::Win32::Media::timeBeginPeriod;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

use analyze::{Region, Thresholds};
use capture::{Capture, Settings};
use report::RunResult;
use stage::Stage;

/// How long a closing app gets before it is terminated.
const QUIT_GRACE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq)]
enum Between {
    Quit,
    Hide,
}

struct Config {
    label: String,
    exe: String,
    args: Vec<String>,
    shell: bool,
    processes: Vec<String>,
    runs: usize,
    warmup: usize,
    between: Between,
    cooldown_ms: u64,
    settle_ms: f64,
    timeout_ms: f64,
    monitor: usize,
    pixel_delta: u8,
    min_pixels: usize,
    out: PathBuf,
    save_frames: bool,
    save_all_frames: bool,
}

const USAGE: &str = "usage: notsy-bench launch --label <name> --exe <path> [options]\n       \
                     notsy-bench monitors\n(see the top of src/main.rs for options)";

fn main() -> ExitCode {
    unsafe {
        // Physical pixels everywhere, so window rects match captured frames.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // 1 ms sleeps for window polling and cooldowns.
        timeBeginPeriod(1);
        // ShellExecuteEx needs COM.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("launch") => parse_launch(&args[1..]).and_then(|c| run_launch(&c)),
        Some("monitors") => list_monitors(),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("notsy-bench: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_launch(args: &[String]) -> Result<Config, String> {
    let mut c = Config {
        label: String::new(),
        exe: String::new(),
        args: Vec::new(),
        shell: false,
        processes: Vec::new(),
        runs: 30,
        warmup: 2,
        between: Between::Quit,
        cooldown_ms: 2000,
        settle_ms: 2500.0,
        timeout_ms: 15000.0,
        monitor: 0,
        pixel_delta: 24,
        min_pixels: 16,
        out: PathBuf::from("bench-results"),
        save_frames: false,
        save_all_frames: false,
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        fn num<T: std::str::FromStr>(flag: &str, v: String) -> Result<T, String> {
            v.parse().map_err(|_| format!("{flag}: not a number: {v}"))
        }
        match flag.as_str() {
            "--label" => c.label = value()?,
            "--exe" => c.exe = value()?,
            "--arg" => c.args.push(value()?),
            "--shell" => c.shell = true,
            "--process" => c.processes.push(value()?),
            "--runs" => c.runs = num(flag, value()?)?,
            "--warmup" => c.warmup = num(flag, value()?)?,
            "--between" => {
                c.between = match value()?.as_str() {
                    "quit" => Between::Quit,
                    "hide" => Between::Hide,
                    v => return Err(format!("--between: expected quit or hide, got {v}")),
                }
            }
            "--cooldown-ms" => c.cooldown_ms = num(flag, value()?)?,
            "--settle-ms" => c.settle_ms = num(flag, value()?)?,
            "--timeout-ms" => c.timeout_ms = num(flag, value()?)?,
            "--monitor" => c.monitor = num(flag, value()?)?,
            "--pixel-delta" => c.pixel_delta = num(flag, value()?)?,
            "--min-pixels" => c.min_pixels = num(flag, value()?)?,
            "--out" => c.out = PathBuf::from(value()?),
            "--save-frames" => c.save_frames = true,
            "--save-all-frames" => c.save_all_frames = true,
            _ => return Err(format!("unknown option {flag}\n{USAGE}")),
        }
    }
    if c.label.is_empty() || c.exe.is_empty() {
        return Err(USAGE.into());
    }
    if c.processes.is_empty() {
        let name = PathBuf::from(&c.exe)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| n.to_ascii_lowercase().ends_with(".exe"))
            .ok_or("--process is required when --exe is not an .exe")?;
        c.processes.push(name);
    }
    if c.runs == 0 {
        return Err("--runs must be at least 1".into());
    }
    Ok(c)
}

fn list_monitors() -> Result<(), String> {
    for (i, o) in capture::outputs()?.iter().enumerate() {
        let env = report::environment(o);
        println!(
            "{i}: {}  {}x{} @ {} Hz  at ({}, {})  on {}",
            o.device, env.width, env.height, env.refresh_hz, o.desktop.left, o.desktop.top, o.adapter
        );
    }
    Ok(())
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

fn run_launch(c: &Config) -> Result<(), String> {
    let outputs = capture::outputs()?;
    let output = outputs
        .get(c.monitor)
        .ok_or(format!("monitor {} not found; see `notsy-bench monitors`", c.monitor))?;
    let env = report::environment(output);
    let dir = c.out.join(format!("{}-{}", slug(&c.label), report::timestamp()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

    println!(
        "{}: {} warmup + {} runs, {} between runs, {} launch",
        c.label,
        c.warmup,
        c.runs,
        if c.between == Between::Quit { "quit" } else { "hide" },
        if c.shell { "shell" } else { "direct" }
    );
    println!(
        "  monitor {} {}x{} @ {} Hz on {}, power: {}",
        env.monitor, env.width, env.height, env.refresh_hz, env.adapter, env.power
    );

    let mut target = procs::Target::new(&c.processes);
    if c.between == Between::Quit {
        target.quit(QUIT_GRACE);
    }
    let stage = Stage::new(output.desktop)?;

    let total = c.warmup + c.runs;
    let mut runs = Vec::with_capacity(total);
    for i in 0..total {
        Stage::pump(Duration::from_millis(c.cooldown_ms));
        let mut r = one_run(c, &mut target, &stage, i, &dir);
        r.warmup = i < c.warmup;
        report::print_run(total, &r);
        runs.push(r);
        match c.between {
            Between::Quit => target.quit(QUIT_GRACE),
            Between::Hide => target.close_windows(QUIT_GRACE),
        }
    }
    drop(stage);

    report::print_summary(&runs);
    let settings = [
        ("exe", format!("{:?}", c.exe)),
        ("args", format!("{:?}", c.args)),
        ("shell", c.shell.to_string()),
        ("processes", format!("{:?}", c.processes)),
        ("between", format!("\"{}\"", if c.between == Between::Quit { "quit" } else { "hide" })),
        ("warmup", c.warmup.to_string()),
        ("runs", c.runs.to_string()),
        ("cooldown_ms", c.cooldown_ms.to_string()),
        ("settle_ms", c.settle_ms.to_string()),
        ("timeout_ms", c.timeout_ms.to_string()),
        ("pixel_delta", c.pixel_delta.to_string()),
        ("min_pixels", c.min_pixels.to_string()),
        ("thumbnail_scale", (1u32 << capture::MIP).to_string()),
    ];
    let json = dir.join("results.json");
    report::write_json(&json, &c.label, &settings, &env, &runs)
        .map_err(|e| format!("writing {}: {e}", json.display()))?;
    println!("\n  results: {}", json.display());
    Ok(())
}

fn one_run(
    c: &Config,
    target: &mut procs::Target,
    stage: &Stage,
    index: usize,
    dir: &std::path::Path,
) -> RunResult {
    let mut result = RunResult {
        index,
        warmup: false,
        window_ms: None,
        first_pixel_ms: None,
        content_ms: None,
        complete_ms: None,
        frames: 0,
        region: None,
        region_is_window: false,
        raised: false,
        error: None,
    };
    let settings = Settings {
        monitor: c.monitor,
        watch: stage.work_area,
        pixel_delta: c.pixel_delta,
        min_pixels: c.min_pixels,
        settle_ms: c.settle_ms,
        timeout_ms: c.timeout_ms,
    };
    let capture = match Capture::start(settings, || stage.repaint()) {
        Ok(cap) => cap,
        Err(e) => {
            result.error = Some(e);
            return result;
        }
    };
    let (width, height, desktop) = (capture.width, capture.height, capture.desktop);

    stage.prepare_launch();
    let t0 = clock::now();
    capture.arm(t0);
    if let Err(e) = procs::launch(&c.exe, &c.args, c.shell) {
        let _ = capture.finish();
        result.error = Some(e);
        return result;
    }

    let mut window_qpc = None;
    let mut window_rect = None;
    while !capture.is_finished() {
        let biggest = target
            .windows()
            .into_iter()
            .max_by_key(|(_, r)| (r.right - r.left) as i64 * (r.bottom - r.top) as i64);
        if let Some((hwnd, rect)) = biggest {
            window_qpc.get_or_insert_with(clock::now);
            window_rect = Some(rect);
            result.raised |= stage.raise_above(hwnd);
        }
        Stage::pump(Duration::from_millis(2));
    }
    let frames = match capture.finish() {
        Ok(f) => f,
        Err(e) => {
            result.error = Some(e);
            return result;
        }
    };

    // Only the part of the window inside the work area is the app's: the
    // taskbar draws on top of anything below it.
    let work = Region::from_window(stage.work_area, desktop, width, height)
        .unwrap_or(Region::full(width, height));
    let window_region = window_rect
        .and_then(|r| Region::from_window(r, desktop, width, height))
        .and_then(|r| r.intersect(&work));
    let region = window_region.unwrap_or(work);
    let thresholds = Thresholds { pixel_delta: c.pixel_delta, min_pixels: c.min_pixels };
    let m = analyze::moments(&frames, t0, width as usize, &region, &thresholds);

    result.window_ms = window_qpc.map(|q| clock::ms_between(t0, q));
    result.first_pixel_ms = analyze::ms_after(&frames, t0, m.first_pixel);
    result.content_ms = analyze::ms_after(&frames, t0, m.content);
    result.complete_ms = analyze::ms_after(&frames, t0, m.complete);
    result.frames = frames.len();
    result.region = Some(region);
    result.region_is_window = window_region.is_some();

    if c.save_frames {
        let moments = [
            ("0-baseline", Some(m.baseline)),
            ("1-first-pixel", m.first_pixel),
            ("2-complete-98", m.content),
            ("3-complete", m.complete),
            ("4-final", Some(frames.len() - 1)),
        ];
        for (name, idx) in moments {
            if let Some(i) = idx {
                let path = dir.join(format!("run{index:02}-{name}.bmp"));
                let _ = report::write_bmp(&path, &frames[i].lum, width as usize, &region);
            }
        }
        let csv = analyze::trace_csv(&frames, t0, width as usize, &region, &thresholds);
        let _ = std::fs::write(dir.join(format!("run{index:02}-trace.csv")), csv);
    }
    if c.save_all_frames {
        let all = dir.join(format!("run{index:02}-frames"));
        let _ = std::fs::create_dir_all(&all);
        for f in &frames[m.baseline..] {
            let ms = clock::ms_between(t0, f.qpc);
            let _ = report::write_bmp(&all.join(format!("{ms:08.1}.bmp")), &f.lum, width as usize, &region);
        }
    }
    result
}
