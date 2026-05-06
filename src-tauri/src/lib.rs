use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[derive(Default)]
struct AppConfig {
    double_press_enabled: bool,
    waiting_second_press: bool,
    last_press_time: Option<Instant>,
}

type ConfigArc = Arc<Mutex<AppConfig>>;

#[tauri::command]
fn get_double_press_enabled(config: tauri::State<ConfigArc>) -> bool {
    config.lock().unwrap().double_press_enabled
}

#[tauri::command]
fn set_double_press_enabled(config: tauri::State<ConfigArc>, enabled: bool) {
    config.lock().unwrap().double_press_enabled = enabled;
}

#[tauri::command]
fn create_capture_window(app: AppHandle) {
    let monitors = match app.available_monitors() {
        Ok(m) => m,
        Err(_) => return,
    };

    for monitor in monitors {
        let pos = monitor.position();
        let size = monitor.size();
        let label = format!("capture-{}-{}", pos.x, pos.y);

        if app.get_webview_window(&label).is_some() {
            continue;
        }

        let _ = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("/#/capture".into()))
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .position(pos.x as f64, pos.y as f64)
            .inner_size(size.width as f64, size.height as f64)
            .focused(true)
            .visible(true)
            .build();
    }
}

#[tauri::command]
fn close_capture_windows(app: AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("capture-") {
            let _ = window.close();
        }
    }
}

#[tauri::command]
fn create_editor_window(app: AppHandle, image_path: String) {
    let label = format!("editor-{}", Instant::now().elapsed().as_millis());
    let encoded = image_path.replace('\\', "/").replace(' ', "%20");
    let url = format!("/#/editor?path={}", encoded);
    let _ = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(false)
        .title("Jpixel Editor")
        .inner_size(900.0, 700.0)
        .center()
        .visible(true)
        .build();
}

#[tauri::command]
fn create_pin_window(app: AppHandle, image_path: String) {
    let label = format!("pin-{}", Instant::now().elapsed().as_millis());
    let _ = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("/#/pin".into()))
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .inner_size(400.0, 300.0)
        .visible(true)
        .build();

    let _ = image_path;
}

#[tauri::command]
fn capture_screen_region(app: AppHandle, x: i32, y: i32, width: u32, height: u32) -> Result<String, String> {
    use screenshots::Screen;
    use arboard::Clipboard;

    let screen = Screen::from_point(x, y).map_err(|e| e.to_string())?;
    let image = screen
        .capture_area(x, y, width, height)
        .map_err(|e| e.to_string())?;

    let picture_dir = app.path().picture_dir().map_err(|e| e.to_string())?;
    let save_dir = picture_dir.join("Jpixel");
    std::fs::create_dir_all(&save_dir).map_err(|e| e.to_string())?;

    let filename = format!(
        "jpixel-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let path = save_dir.join(&filename);
    image.save(&path).map_err(|e| e.to_string())?;

    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    let img_data = arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: std::borrow::Cow::Borrowed(image.as_raw()),
    };
    clipboard.set_image(img_data).map_err(|e| e.to_string())?;

    Ok(path.to_string_lossy().to_string())
}

fn handle_capture_hotkey(app: &AppHandle, config: &ConfigArc) {
    let mut cfg = config.lock().unwrap();

    if !cfg.double_press_enabled {
        drop(cfg);
        let _ = app.emit("trigger-capture", ());
        create_capture_window(app.clone());
        return;
    }

    if cfg.waiting_second_press {
        cfg.waiting_second_press = false;
        cfg.last_press_time = None;
        drop(cfg);
        let _ = app.emit("trigger-capture", ());
        create_capture_window(app.clone());
    } else {
        cfg.waiting_second_press = true;
        cfg.last_press_time = Some(Instant::now());
        drop(cfg);

        let app_clone = app.clone();
        let config_clone = config.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            let mut cfg = config_clone.lock().unwrap();
            if cfg.waiting_second_press {
                cfg.waiting_second_press = false;
                cfg.last_press_time = None;
                let _ = app_clone.emit("hotkey-timeout", ());
            }
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config: ConfigArc = Arc::new(Mutex::new(AppConfig::default()));
    let config_for_plugin = config.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        handle_capture_hotkey(app, &config_for_plugin);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(config.clone())
        .invoke_handler(tauri::generate_handler![
            create_capture_window,
            close_capture_windows,
            create_editor_window,
            create_pin_window,
            capture_screen_region,
            get_double_press_enabled,
            set_double_press_enabled
        ])
        .setup(move |app| {
            let app_handle = app.handle();

            // System tray
            let settings_item = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_item, &quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Jpixel")
                .menu(&menu)
                .on_menu_event({
                    let _app_handle = app_handle.clone();
                    move |app: &AppHandle, event| match event.id.as_ref() {
                        "settings" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|_tray: &tauri::tray::TrayIcon, event| {
                    if let TrayIconEvent::Click { button, .. } = event {
                        if button == tauri::tray::MouseButton::Left {
                            let _ = _tray.app_handle().emit("trigger-capture", ());
                            create_capture_window(_tray.app_handle().clone());
                        }
                    }
                })
                .build(app)?;

            // Register global shortcut F1
            let shortcut = Shortcut::new(
                None,
                tauri_plugin_global_shortcut::Code::F1,
            );
            let _ = app.global_shortcut().unregister(shortcut);
            if let Err(e) = app.global_shortcut().register(shortcut) {
                eprintln!("Warning: failed to register global shortcut F1: {}", e);
            }

            // Hide main window on startup
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.hide();
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
