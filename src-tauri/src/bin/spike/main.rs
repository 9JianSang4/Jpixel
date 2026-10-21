//! Jpixel native-overlay spike.
//!
//! Stage 1 is intentionally standalone: no Tauri, no settings, no pin windows.
//! It only validates: hotkey -> focused monitor -> DXGI capture -> native
//! overlay -> selection -> clipboard, on Windows with mixed DPI.

mod gdi;
mod monitor;
mod overlay;
mod report;
mod test_pattern;

use anyhow::{anyhow, Result};
use report::ReportRow;
use std::time::Duration;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

fn enable_per_monitor_v2() {
    // A previous manifest may already have set awareness; failure is fine.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

#[derive(Debug, Clone)]
struct Options {
    bench: bool,
    list_monitors: bool,
    test_overlay: bool,
    monitor: usize,
    iterations: usize,
    hotkey: u32,
    hotkey_mods: u32,
    hotkey_name: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bench: false,
            list_monitors: false,
            test_overlay: false,
            monitor: 1,
            iterations: 10,
            hotkey: 0x58, // VK_X
            hotkey_mods: 0x0001, // Alt
            hotkey_name: "Alt+X".to_string(),
        }
    }
}

fn parse_key(token: &str) -> Result<u32> {
    let vk = match token {
        "F1" => 0x70,
        "F2" => 0x71,
        "F3" => 0x72,
        "F4" => 0x73,
        "F5" => 0x74,
        "F6" => 0x75,
        "F7" => 0x76,
        "F8" => 0x77,
        "F9" => 0x78,
        "F10" => 0x79,
        "F11" => 0x7A,
        "F12" => 0x7B,
        other if other.len() == 1 => {
            let byte = other.as_bytes()[0];
            if !byte.is_ascii_uppercase() && !byte.is_ascii_digit() {
                return Err(anyhow!("unsupported key: {other}"));
            }
            byte as u32
        }
        other => return Err(anyhow!("unsupported key: {other}")),
    };
    Ok(vk)
}

fn parse_hotkey(value: &str) -> Result<(u32, u32, String)> {
    let mut mods = 0u32;
    let mut key: Option<u32> = None;
    let mut names: Vec<String> = Vec::new();
    for part in value.split('+') {
        let token = part.trim().to_ascii_uppercase();
        if token.is_empty() {
            return Err(anyhow!("invalid --hotkey value: {value}"));
        }
        match token.as_str() {
            "CTRL" | "CONTROL" => {
                mods |= 0x0002;
                names.push("Ctrl".to_string());
            }
            "ALT" => {
                mods |= 0x0001;
                names.push("Alt".to_string());
            }
            "SHIFT" => {
                mods |= 0x0004;
                names.push("Shift".to_string());
            }
            "WIN" | "SUPER" => {
                mods |= 0x0008;
                names.push("Win".to_string());
            }
            other => {
                if key.is_some() {
                    return Err(anyhow!(
                        "--hotkey must contain exactly one non-modifier key: {value}"
                    ));
                }
                key = Some(parse_key(other)?);
                names.push(other.to_string());
            }
        }
    }
    let vk = key.ok_or_else(|| anyhow!("--hotkey is missing a key: {value}"))?;
    Ok((vk, mods, names.join("+")))
}

fn parse_options() -> Result<Options> {
    let mut opts = Options::default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bench" => opts.bench = true,
            "--list-monitors" => opts.list_monitors = true,
            "--test-overlay" => opts.test_overlay = true,
            "--monitor" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| anyhow!("--monitor requires a number"))?;
                opts.monitor = value
                    .parse::<usize>()
                    .map_err(|_| anyhow!("invalid --monitor value: {}", value))?;
            }
            "--iterations" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| anyhow!("--iterations requires a number"))?;
                opts.iterations = value
                    .parse::<usize>()
                    .map_err(|_| anyhow!("invalid --iterations value: {}", value))?;
            }
            "--hotkey" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| anyhow!("--hotkey requires a value like F1 or F8"))?;
                let (vk, mods, name) = parse_hotkey(value)?;
                opts.hotkey = vk;
                opts.hotkey_mods = mods;
                opts.hotkey_name = name;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown argument: {}", other)),
        }
        i += 1;
    }
    if opts.iterations == 0 {
        return Err(anyhow!("--iterations must be at least 1"));
    }
    Ok(opts)
}

fn print_help() {
    println!(
        "jpixel native-overlay spike\n\
         \n\
         Usage:\n\
           spike.exe                          manual mode (Alt+X capture, Enter copy, Esc cancel)\n\
           spike.exe --hotkey Ctrl+F1         modifier+F-key fallback\n\
           spike.exe --hotkey Ctrl+Alt+J      another non-F-key fallback\n\
           spike.exe --hotkey F8              bare F-key (may be intercepted)\n\
           spike.exe --test-overlay [--monitor N]\n\
                                              show the overlay immediately, no hotkey needed\n\
           spike.exe --list-monitors          print monitor indices and exit\n\
           spike.exe --bench [--monitor N] [--iterations N]\n\
         \n\
         Manual mode:\n\
           1. start spike.exe\n\
           2. click the target app on the monitor you want to capture\n\
           3. press Alt+X (default), drag a rectangle,\n\
              press Enter to copy or Esc to cancel\n\
         \n\
         Bare F-keys may be intercepted by Windows Help or keyboard software.\n\
         Use Alt+X, Ctrl+F1 or Ctrl+Alt+J instead.\n\
         \n\
         Logs are written to spike-log.csv in the current directory.\n"
    );
}

fn main() -> Result<()> {
    enable_per_monitor_v2();
    test_pattern::ensure_patterns()?;

    let opts = parse_options()?;
    let monitors = monitor::enumerate()?;
    println!("monitors:");
    for (index, info) in monitors.iter().enumerate() {
        println!("  {}", info.label(index));
    }

    if opts.list_monitors {
        return Ok(());
    }

    overlay::init()?;
    overlay::warm_up(&monitors);
    println!("idle working set: {} MB", report::working_set_mb());
    report::log(&ReportRow {
        mode: "idle".to_string(),
        monitor: "-".to_string(),
        rect: "-".to_string(),
        scale: 0.0,
        capture_ms: 0.0,
        first_frame_ms: 0.0,
        clipboard_ms: None,
        pixel_match: None,
        max_diff: None,
        note: "startup".to_string(),
    })?;

    if opts.test_overlay {
        let target = monitor::by_index(&monitors, opts.monitor)?;
        let capture = overlay::capture_monitor(&target, "test")?;
        println!(
            "test overlay shown on {}: capture={:.1}ms first_frame={:.1}ms",
            capture.monitor_label, capture.capture_ms, capture.first_frame_ms
        );
        println!("drag a rectangle, Enter = copy, Esc = cancel; close this console to exit");
        overlay::run_message_loop();
        return Ok(());
    }

    if opts.bench {
        let target = monitor::by_index(&monitors, opts.monitor)?;
        println!(
            "bench target: {} ({} iterations)",
            target.label(opts.monitor - 1),
            opts.iterations
        );
        for iteration in 0..opts.iterations {
            let capture = overlay::capture_monitor(&target, "bench")?;
            overlay::set_bench_selection()?;
            let copy = overlay::copy_current_selection()?;
            report::log(&ReportRow {
                mode: "bench".to_string(),
                monitor: capture.monitor_label,
                rect: capture.monitor_rect,
                scale: capture.scale,
                capture_ms: capture.capture_ms,
                first_frame_ms: capture.first_frame_ms,
                clipboard_ms: Some(copy.clipboard_ms),
                pixel_match: Some(copy.pixel_match),
                max_diff: Some(copy.max_diff),
                note: format!("iteration={}", iteration + 1),
            })?;
            overlay::hide_overlay();
            std::thread::sleep(Duration::from_millis(100));
        }
        return Ok(());
    }

    if let Err(e) = overlay::register_hotkey(opts.hotkey, opts.hotkey_mods, &opts.hotkey_name) {
        let _ = report::log(&ReportRow {
            mode: "error".to_string(),
            monitor: "-".to_string(),
            rect: "-".to_string(),
            scale: 0.0,
            capture_ms: 0.0,
            first_frame_ms: 0.0,
            clipboard_ms: None,
            pixel_match: None,
            max_diff: None,
            note: format!("register-hotkey-{}: {}", opts.hotkey_name, e),
        });
        return Err(e);
    }
    let _ = report::log(&ReportRow {
        mode: "idle".to_string(),
        monitor: "-".to_string(),
        rect: "-".to_string(),
        scale: 0.0,
        capture_ms: 0.0,
        first_frame_ms: 0.0,
        clipboard_ms: None,
        pixel_match: None,
        max_diff: None,
        note: format!("manual-ready {}", opts.hotkey_name),
    });
    println!(
        "manual mode ready: {} = capture, Enter = copy, Esc = cancel",
        opts.hotkey_name
    );
    println!("keep this console minimized; log file: {}", report::LOG_FILE);
    overlay::run_message_loop();
    Ok(())
}
