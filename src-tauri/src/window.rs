use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri::window::Color;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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

#[tauri::command]
pub fn create_capture_window(app: AppHandle) {
    // Phase 1 — close old capture windows, then release the lock so that
    // other threads (e.g. frontend IPC) can still close windows
    {
        let _lock = capture_op_lock().lock().unwrap();
        close_capture_windows_inner(&app);
    }

    // Phase 2 — poll briefly for the async COM close to finish.
    // We do NOT hold the mutex here so the main thread event loop can
    // process the close events.  If it times out we create new windows
    // regardless; unique timestamp labels prevent label collisions.
    for _ in 0..10 {
        if !app.webview_windows().keys().any(|k| k.starts_with("capture-")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // Phase 3 — create new capture windows under the lock
    let _lock = capture_op_lock().lock().unwrap();

    let monitors = match app.available_monitors() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to get available monitors: {}", e);
            return;
        }
    };

    for monitor in monitors {
        let phys_pos = monitor.position();
        let phys_size = monitor.size();
        let scale = monitor.scale_factor();

        // Convert physical monitor coordinates to logical pixels for window
        // positioning. Under mixed-DPI multi-monitor, Monitor::position/::size
        // return physical pixels, but WebviewWindowBuilder::position/::inner_size
        // expect logical pixels.
        let logical_x = phys_pos.x as f64 / scale;
        let logical_y = phys_pos.y as f64 / scale;
        let logical_w = phys_size.width as f64 / scale;
        let logical_h = phys_size.height as f64 / scale;

        // Use a unique label every time to avoid collisions with lingering
        // windows that haven't finished asynchronous destruction.
        let label = format!(
            "capture-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        // Pass physical monitor origin + scale factor so the frontend can
        // convert CSS-pixel mouse coordinates to absolute physical pixels
        // for the screenshot backend:  phys = origin + round(css * scale).
        let url = format!(
            "/#/capture?ox={}&oy={}&scale={}",
            phys_pos.x, phys_pos.y, scale
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
                // Explicit set_focus after creation avoids Windows foreground-steal
                // restrictions when another window (pin) was recently focused.
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
    let _lock = capture_op_lock().lock().unwrap();
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
            format!(
                "data:image/bmp;base64,{}",
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
            )
        });

    let label = format!(
        "pin-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
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
