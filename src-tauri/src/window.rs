use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

#[tauri::command]
pub fn create_capture_window(app: AppHandle) {
    let monitors = match app.available_monitors() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to get available monitors: {}", e);
            return;
        }
    };

    for monitor in monitors {
        let pos = monitor.position();
        let size = monitor.size();
        let label = format!("capture-{}-{}", pos.x, pos.y);

        // Close existing hidden windows from previous capture session
        if let Some(window) = app.get_webview_window(&label) {
            if let Err(e) = window.close() {
                log::warn!("Failed to close existing capture window {}: {}", label, e);
            }
            // Window destruction on Windows is async; give it a beat before reusing the label
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if let Err(e) = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("/#/capture".into()))
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .position(pos.x as f64, pos.y as f64)
            .inner_size(size.width as f64, size.height as f64)
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
    for (label, window) in app.webview_windows() {
        if label.starts_with("capture-") {
            if let Err(e) = window.close() {
                log::warn!("Failed to close capture window {}: {}", label, e);
            }
        }
    }
}

#[tauri::command]
pub fn create_editor_window(app: AppHandle, image_path: String) {
    // Spawn window creation off the IPC thread to avoid WebView2 deadlock on Windows
    std::thread::spawn(move || {
        // Give the IPC call time to return before we touch the window system
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

        close_capture_windows(app);
    });
}

#[tauri::command]
pub fn create_pin_window(app: AppHandle, image_path: String, x: i32, y: i32) {
    // Spawn window creation off the IPC thread to avoid WebView2 deadlock on Windows
    std::thread::spawn(move || {
        // Give the IPC call time to return before we touch the window system
        std::thread::sleep(std::time::Duration::from_millis(100));

        let label = format!(
            "pin-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        let encoded = urlencoding::encode(&image_path);
        let url = format!("/#/pin?path={}", encoded);
        if let Err(e) = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .position(x as f64, y as f64)
            .inner_size(400.0, 300.0)
            .visible(true)
            .build()
        {
            log::error!("Failed to create pin window: {}", e);
        }

        close_capture_windows(app);
    });
}

#[tauri::command]
pub fn close_pin_window(app: AppHandle, label: String) {
    if let Some(window) = app.get_webview_window(&label) {
        if let Err(e) = window.close() {
            log::warn!("Failed to close pin window {}: {}", label, e);
        }
    }
}
