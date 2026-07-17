use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri::window::Color;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use crate::capture::{default_capture, Rect, temp_jpixel_dir, timestamp_filename, cleanup_temp_captures};
use crate::capture::screen::ScreenCapture;

#[cfg(target_os = "windows")]
extern "system" {
    fn DwmFlush() -> i32;
}

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
};

/// Serialise concurrent capture-window operations across IPC threads.
///
/// On Windows, WebView2 window close / create must be marshalled onto the
/// main-thread COM apartment.  Calling `window.close()` from multiple
/// threads concurrently does not deadlock by itself, but it can cause window
/// operations to accumulate faster than the event loop drains them, leading
/// to progressive lag and eventual process hang.  This mutex ensures only
/// one thread at a time performs capture-window operations.
fn capture_op_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Hide all `capture-*` windows so the subsequent screen capture doesn't
/// include the overlay UI.  Uses direct Win32 `ShowWindow(SW_HIDE)` which
/// is sub-microsecond — avoids the ~40 ms Tauri/Wry/WebView2 abstraction
/// overhead of `window.hide()`.
pub fn hide_capture_windows(app: &AppHandle) {
    let t0 = std::time::Instant::now();
    let windows: Vec<_> = app.webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with("capture-"))
        .collect();
    let t_collect = t0.elapsed();

    if windows.is_empty() {
        return;
    }

    for (_label, window) in &windows {
        #[cfg(target_os = "windows")]
        {
            if let Ok(wh) = window.window_handle() {
                if let RawWindowHandle::Win32(handle) = wh.as_raw() {
                    unsafe {
                        windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                            windows::Win32::Foundation::HWND(handle.hwnd.get() as isize),
                            windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
                        );
                    }
                    continue;
                }
            }
        }
        let _ = window.hide();
    }

    let t_hide = t0.elapsed();
    log::info!(
        "[perf] hide_capture_windows: collect={:?} hide={:?} total={:?} windows={}",
        t_collect,
        t_hide - t_collect,
        t_hide,
        windows.len()
    );

    std::thread::yield_now();

    // Wait for DWM compositor to actually render the hidden window state so
    // that the subsequent screen capture does not include the overlay UI.
    #[cfg(target_os = "windows")]
    unsafe {
        DwmFlush();
    }
}

/// Iterate all `capture-*` windows and call `.close()` on each.
///
/// Does **not** acquire [`capture_op_lock`] — callers are responsible for
/// holding it when needed.
fn close_capture_windows_inner(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("capture-") {
            if let Err(e) = window.close() {
                log::warn!("Failed to close capture window {}: {}", label, e);
            }
        }
    }
}

/// Per-monitor captured background data, passed from the hotkey handler
/// to window creation after capture is complete.
struct MonitorBg {
    phys_x: i32,
    phys_y: i32,
    phys_w: u32,
    phys_h: u32,
    scale: f64,
    bg_path: String,
}

#[tauri::command]
pub fn create_capture_window(app: AppHandle) {
    // ── Phase 0 — Capture ALL monitors FIRST ──────────────────
    // This must happen before any window close/create operation so the
    // frozen background reflects the exact moment the hotkey fired.
    // Using `capture_region_no_flush` skips DwmFlush — at this point no
    // overlay windows exist yet, so there is nothing to flush. Skipping
    // the flush avoids GPU pipeline stalls that cause video players to
    // stutter or buffer.

    let monitors = match app.available_monitors() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to get available monitors: {}", e);
            return;
        }
    };

    let temp_dir = match temp_jpixel_dir(&app) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to get temp dir: {}", e);
            return;
        }
    };
    cleanup_temp_captures(&temp_dir);

    let mut bg_list: Vec<MonitorBg> = Vec::with_capacity(monitors.len());

    for monitor in &monitors {
        let phys_pos = monitor.position();
        let phys_size = monitor.size();
        let scale = monitor.scale_factor();

        let cap_rect = Rect {
            x: phys_pos.x,
            y: phys_pos.y,
            width: phys_size.width,
            height: phys_size.height,
        };

        // BMP encoding is much faster than PNG (~10×) — no compression,
        // just a header + raw pixel data.  The file will be loaded by the
        // WebView via Tauri's asset:// protocol, which serves raw bytes
        // in-process without any base64 IPC roundtrip.
        let bg_filename = timestamp_filename("jpixel-cap", "bmp");
        let bg_path = temp_dir.join(&bg_filename);

        match default_capture().capture_region_no_flush(cap_rect) {
            Ok(image) => {
                let image: image::RgbaImage = image;
                if let Err(e) = image.save(&bg_path) {
                    log::error!("Failed to save capture bg {}: {}", bg_filename, e);
                    continue;
                }
            }
            Err(e) => {
                log::error!("Failed to capture monitor ({},{} {}x{}): {}",
                    phys_pos.x, phys_pos.y, phys_size.width, phys_size.height, e);
                continue;
            }
        }

        bg_list.push(MonitorBg {
            phys_x: phys_pos.x,
            phys_y: phys_pos.y,
            phys_w: phys_size.width,
            phys_h: phys_size.height,
            scale,
            bg_path: bg_path.to_string_lossy().to_string(),
        });
    }

    if bg_list.is_empty() {
        log::error!("No monitors captured — aborting window creation");
        return;
    }

    // ── Phase 1 — Close old capture windows ──────────────────
    {
        let _lock = crate::config::lock_or_warn!(capture_op_lock());
        close_capture_windows_inner(&app);
    }

    // ── Phase 2 — Poll for async COM close to finish ─────────
    let mut waited = false;
    for _ in 0..4 {
        if !app.webview_windows().keys().any(|k| k.starts_with("capture-")) {
            break;
        }
        waited = true;
        std::thread::sleep(Duration::from_millis(50));
    }
    if waited && app.webview_windows().keys().any(|k| k.starts_with("capture-")) {
        log::warn!("Old capture windows did not close within 200 ms — proceeding anyway");
    }

    // ── Phase 3 — Create windows with pre-captured backgrounds ──
    let _lock = crate::config::lock_or_warn!(capture_op_lock());

    for bg in &bg_list {
        let bg_encoded = urlencoding::encode(&bg.bg_path);

        let logical_x = bg.phys_x as f64 / bg.scale;
        let logical_y = bg.phys_y as f64 / bg.scale;
        let logical_w = bg.phys_w as f64 / bg.scale;
        let logical_h = bg.phys_h as f64 / bg.scale;

        let label = format!(
            "capture-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        let url = format!(
            "/#/capture?ox={}&oy={}&scale={}&bg={}&bw={}&bh={}",
            bg.phys_x, bg.phys_y, bg.scale, bg_encoded, bg.phys_w, bg.phys_h
        );

        match WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .position(logical_x, logical_y)
            .inner_size(logical_w, logical_h)
            .focused(true)
            .visible(true)
            .build()
        {
            Ok(w) => {
                let _ = w.set_focus();
            }
            Err(e) => {
                log::error!("Failed to create capture window {}: {}", label, e);
            }
        }
    }
}

#[tauri::command]
pub fn close_capture_windows(app: AppHandle) {
    // Acquire the op-lock so we never race with create_capture_window or
    // other concurrent close_capture_windows calls.
    let _lock = crate::config::lock_or_warn!(capture_op_lock());
    close_capture_windows_inner(&app);
}

/// Spawn a pin window off the IPC thread to avoid WebView2 deadlock on Windows.
pub fn spawn_pin_window(
    app: AppHandle,
    image_path: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) {
    // Read and encode the image before spawning the thread — the data URL
    // is ready before the frontend even starts loading.
    let b64 = std::fs::read(&image_path)
        .ok()
        .map(|bytes| {
            let mime = match std::path::Path::new(&image_path)
                .extension()
                .and_then(|e| e.to_str())
            {
                Some("bmp") => "image/bmp",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                _ => "image/png",
            };
            format!(
                "data:{};base64,{}",
                mime,
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
            )
        });

    let label = format!(
        "pin-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    // Unique event name per pin window so old windows never pick up a
    // new window's image data.
    let event_name = format!("pin-image-data-{}", label);

    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));

        let scale = app
            .available_monitors()
            .ok()
            .and_then(|monitors| {
                monitors.iter().find(|m| {
                    let p = m.position();
                    let s = m.size();
                    x >= p.x
                        && x < p.x + s.width as i32
                        && y >= p.y
                        && y < p.y + s.height as i32
                }).map(|m| m.scale_factor())
            })
            .unwrap_or(1.0);

        let logical_x = (x as f64 / scale).floor() as i32;
        let logical_y = (y as f64 / scale).floor() as i32;
        let logical_w = (width as f64 / scale).ceil().max(10.0) as f64;
        let logical_h = (height as f64 / scale).ceil().max(10.0) as f64;

        let encoded = urlencoding::encode(&image_path);
        // Pass the unique event name to the frontend so it knows what to listen for
        let url = format!(
            "/#/pin?path={}&width={}&height={}&evt={}",
            encoded, logical_w as u32, logical_h as u32, &event_name
        );
        let window = match WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
            .decorations(false)
            .shadow(false)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0))
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .focused(false)
            .position(logical_x as f64, logical_y as f64)
            .inner_size(logical_w, logical_h)
            .visible(true)
            .build()
        {
            Ok(w) => w,
            Err(e) => {
                log::error!("Failed to create pin window: {}", e);
                return;
            }
        };

        // Pin windows must never steal focus — clicks pass through to the
        // window but don't activate it.  Drag (via data-tauri-drag-region)
        // and scroll-wheel zoom still work without activation.
        #[cfg(target_os = "windows")]
        if let Ok(wh) = window.window_handle() {
            if let RawWindowHandle::Win32(handle) = wh.as_raw() {
                unsafe {
                    let hwnd = windows::Win32::Foundation::HWND(handle.hwnd.get() as isize);
                    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
                    SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_NOACTIVATE.0 as i32);
                }
            }
        }

        // Push image data via event for instant render.
        if let Some(data_url) = b64 {
            let _ = app.emit_to(&label, &event_name, data_url);
        }
    });
}

#[tauri::command]
pub fn create_pin_window(
    app: AppHandle,
    image_path: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) {
    spawn_pin_window(app.clone(), image_path, x, y, width, height);
    close_capture_windows(app);
}

#[tauri::command]
pub fn close_pin_window(app: AppHandle, label: String) {
    if let Some(window) = app.get_webview_window(&label) {
        if let Err(e) = window.close() {
            log::warn!("Failed to close pin window {}: {}", label, e);
        }
    }
}
