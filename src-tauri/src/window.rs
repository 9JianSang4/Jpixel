use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

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

/// Hide all `capture-*` windows and wait for the compositor to finish a frame
/// without them, so that subsequent screen capture does not include the overlay
/// UI (magnifier, toolbar, crosshair, selection box, …) in the image.
pub fn hide_capture_windows(app: &AppHandle) {
    let mut hidden = false;
    for (label, window) in app.webview_windows() {
        if label.starts_with("capture-") {
            let _ = window.hide();
            hidden = true;
        }
    }
    // Only sleep if there were actually windows to hide — gives DWM ~3 frames
    // at 60 FPS to compose the desktop without overlay windows.
    if hidden {
        std::thread::sleep(std::time::Duration::from_millis(50));
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

        if let Err(e) = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
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
            log::error!("Failed to create capture window {}: {}", label, e);
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

#[tauri::command]
pub fn create_editor_window(app: AppHandle, image_path: String) {
    // Spawn window creation off the IPC thread to avoid WebView2 deadlock.
    // WebView2's COM apartment threading model deadlocks if we create a new
    // window while still inside the IPC handler on the same thread.
    std::thread::spawn(move || {
        // Yield so the IPC handler can return to the JS caller first.
        std::thread::sleep(std::time::Duration::from_millis(100));

        let label = format!(
            "editor-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        let encoded = urlencoding::encode(&image_path);
        let url = format!("/#/editor?path={}", encoded);
        if let Err(e) = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(false)
            .title("Jpixel Editor")
            .inner_size(900.0, 700.0)
            .center()
            .visible(true)
            .build()
        {
            log::error!("Failed to create editor window: {}", e);
        }
    });
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
    std::thread::spawn(move || {
        // Give the IPC call time to return before we touch the window system
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Find the monitor containing the pin position to get its DPI scale
        // factor, so we can convert physical screenshot coordinates to logical
        // pixels for window positioning.
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

        // Convert physical pixels to logical pixels
        let logical_x = (x as f64 / scale).floor() as i32;
        let logical_y = (y as f64 / scale).floor() as i32;
        let logical_w = (width as f64 / scale).ceil().max(10.0) as f64;
        let logical_h = (height as f64 / scale).ceil().max(10.0) as f64;

        let label = format!(
            "pin-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        let encoded = urlencoding::encode(&image_path);
        let url = format!(
            "/#/pin?path={}&width={}&height={}",
            encoded, logical_w as u32, logical_h as u32
        );
        if let Err(e) = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .focused(true)
            .position(logical_x as f64, logical_y as f64)
            .inner_size(logical_w, logical_h)
            .visible(true)
            .build()
        {
            log::error!("Failed to create pin window: {}", e);
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
