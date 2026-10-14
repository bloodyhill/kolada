//! Results: console summary, JSON file, and optional frame snapshots.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use windows::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::core::PCWSTR;

use crate::analyze::{Region, Stats, stats};
use crate::capture::Output;

pub struct RunResult {
    pub index: usize,
    pub warmup: bool,
    pub window_ms: Option<f64>,
    pub first_pixel_ms: Option<f64>,
    pub content_ms: Option<f64>,
    pub complete_ms: Option<f64>,
    pub frames: usize,
    pub region: Option<Region>,
    pub region_is_window: bool,
    /// The window opened behind the stage and the bench raised it.
    pub raised: bool,
    pub error: Option<String>,
}

pub struct Environment {
    pub monitor: String,
    pub adapter: String,
    pub width: i32,
    pub height: i32,
    pub refresh_hz: u32,
    pub power: &'static str,
}

pub fn environment(output: &Output) -> Environment {
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let name: Vec<u16> = output.device.encode_utf16().chain(std::iter::once(0)).collect();
    let refresh_hz = unsafe {
        if EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode).as_bool() {
            mode.dmDisplayFrequency
        } else {
            0
        }
    };
    let mut status = SYSTEM_POWER_STATUS::default();
    let power = match unsafe { GetSystemPowerStatus(&mut status) } {
        Ok(()) if status.ACLineStatus == 1 => "ac",
        Ok(()) if status.ACLineStatus == 0 => "battery",
        _ => "unknown",
    };
    let d = output.desktop;
    Environment {
        monitor: output.device.clone(),
        adapter: output.adapter.clone(),
        width: d.right - d.left,
        height: d.bottom - d.top,
        refresh_hz,
        power,
    }
}

pub fn timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

fn fmt_ms(v: Option<f64>) -> String {
    v.map_or_else(|| "     -".into(), |v| format!("{v:6.1}"))
}

pub fn print_run(total: usize, r: &RunResult) {
    let kind = if r.warmup { "warmup" } else { "run" };
    match &r.error {
        Some(e) => println!("  {kind} {:>2}/{total}  error: {e}", r.index + 1),
        None => println!(
            "  {kind} {:>2}/{total}  window {}  first pixel {}  98% complete {}  complete {} ms{}",
            r.index + 1,
            fmt_ms(r.window_ms),
            fmt_ms(r.first_pixel_ms),
            fmt_ms(r.content_ms),
            fmt_ms(r.complete_ms),
            if r.first_pixel_ms.is_some() && r.complete_ms.is_none() {
                "   (window vanished before the run settled: excluded)"
            } else {
                match (r.region_is_window, r.raised) {
                    (false, _) => "   (work area: window not found)",
                    (true, true) => "   (raised above stage)",
                    (true, false) => "",
                }
            }
        ),
    }
}

fn measured(runs: &[RunResult], pick: fn(&RunResult) -> Option<f64>) -> Option<Stats> {
    let v: Vec<f64> = runs.iter().filter(|r| !r.warmup).filter_map(pick).collect();
    stats(&v)
}

type Metric = (&'static str, fn(&RunResult) -> Option<f64>);

const METRICS: [Metric; 4] = [
    ("window_shown", |r| r.window_ms),
    ("first_pixel", |r| r.first_pixel_ms),
    ("complete_98", |r| r.content_ms),
    ("visually_complete", |r| r.complete_ms),
];

pub fn print_summary(runs: &[RunResult]) {
    println!("\n  {:<18} {:>4} {:>8} {:>8} {:>8} {:>8}", "ms", "n", "min", "p50", "p95", "max");
    for (name, pick) in METRICS {
        if let Some(s) = measured(runs, pick) {
            println!(
                "  {name:<18} {:>4} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
                s.n, s.min, s.p50, s.p95, s.max
            );
        }
    }
    let failed = runs.iter().filter(|r| !r.warmup && r.complete_ms.is_none()).count();
    if failed > 0 {
        println!("  {failed} run(s) produced no visually-complete time");
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_opt(v: Option<f64>) -> String {
    v.map_or_else(|| "null".into(), |v| format!("{v:.3}"))
}

/// `settings` are pre-rendered `"key": value` pairs describing the run config.
pub fn write_json(
    path: &Path,
    label: &str,
    settings: &[(&str, String)],
    env: &Environment,
    runs: &[RunResult],
) -> io::Result<()> {
    let mut j = String::new();
    let _ = writeln!(j, "{{");
    let _ = writeln!(j, "  \"label\": {},", json_str(label));
    let _ = writeln!(j, "  \"timestamp\": {},", json_str(&timestamp()));
    let _ = writeln!(j, "  \"settings\": {{");
    for (i, (k, v)) in settings.iter().enumerate() {
        let comma = if i + 1 < settings.len() { "," } else { "" };
        let _ = writeln!(j, "    {}: {v}{comma}", json_str(k));
    }
    let _ = writeln!(j, "  }},");
    let _ = writeln!(
        j,
        "  \"environment\": {{ \"monitor\": {}, \"adapter\": {}, \"width\": {}, \"height\": {}, \"refresh_hz\": {}, \"power\": {} }},",
        json_str(&env.monitor),
        json_str(&env.adapter),
        env.width,
        env.height,
        env.refresh_hz,
        json_str(env.power)
    );
    let _ = writeln!(j, "  \"summary\": {{");
    let summaries: Vec<String> = METRICS
        .iter()
        .filter_map(|(name, pick)| {
            measured(runs, *pick).map(|s| {
                format!(
                    "    {}: {{ \"n\": {}, \"min\": {:.3}, \"p50\": {:.3}, \"p95\": {:.3}, \"max\": {:.3}, \"mean\": {:.3} }}",
                    json_str(name),
                    s.n,
                    s.min,
                    s.p50,
                    s.p95,
                    s.max,
                    s.mean
                )
            })
        })
        .collect();
    let _ = writeln!(j, "{}", summaries.join(",\n"));
    let _ = writeln!(j, "  }},");
    let _ = writeln!(j, "  \"runs\": [");
    for (i, r) in runs.iter().enumerate() {
        let comma = if i + 1 < runs.len() { "," } else { "" };
        let region = r.region.map_or_else(
            || "null".into(),
            |g| format!("[{}, {}, {}, {}]", g.x0, g.y0, g.x1, g.y1),
        );
        let error = r.error.as_deref().map_or_else(|| "null".into(), json_str);
        let _ = writeln!(
            j,
            "    {{ \"index\": {}, \"warmup\": {}, \"window_shown_ms\": {}, \"first_pixel_ms\": {}, \"complete_98_ms\": {}, \"visually_complete_ms\": {}, \"frames\": {}, \"region\": {region}, \"region_is_window\": {}, \"raised\": {}, \"error\": {error} }}{comma}",
            r.index,
            r.warmup,
            json_opt(r.window_ms),
            json_opt(r.first_pixel_ms),
            json_opt(r.content_ms),
            json_opt(r.complete_ms),
            r.frames,
            r.region_is_window,
            r.raised
        );
    }
    let _ = writeln!(j, "  ]");
    let _ = writeln!(j, "}}");
    fs::write(path, j)
}

/// Writes the region of a luminance thumbnail as an 8-bit grayscale BMP.
pub fn write_bmp(path: &Path, lum: &[u8], stride: usize, r: &Region) -> io::Result<()> {
    let (w, h) = (r.width(), r.height());
    let row_bytes = (w + 3) & !3;
    let pixel_offset = 14 + 40 + 256 * 4;
    let size = pixel_offset + row_bytes * h;
    let mut b = Vec::with_capacity(size);
    // BITMAPFILEHEADER
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(size as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
    // BITMAPINFOHEADER
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&8u16.to_le_bytes());
    b.extend_from_slice(&[0u8; 24]); // no compression, default resolution
    for i in 0..=255u8 {
        b.extend_from_slice(&[i, i, i, 0]);
    }
    // Rows bottom-up, padded to 4 bytes.
    for y in (r.y0..r.y1).rev() {
        b.extend_from_slice(&lum[y * stride + r.x0..y * stride + r.x1]);
        b.resize(b.len() + (row_bytes - w), 0);
    }
    fs::write(path, b)
}
