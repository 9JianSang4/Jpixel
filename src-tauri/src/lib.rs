use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[derive(Serialize, Deserialize)]
struct AppConfig {
    double_press_enabled: bool,
    screenshot_hotkey: String,
    #[serde(skip)]
    waiting_second_press: bool,
    #[serde(skip)]
    last_press_time: Option<Instant>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            double_press_enabled: false,
            screenshot_hotkey: "F1".to_string(),
            waiting_second_press: false,
            last_press_time: None,
        }
    }
}

type ConfigArc = Arc<Mutex<AppConfig>>;

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(config_dir.join("config.json"))
}

fn load_config(app: &AppHandle) -> AppConfig {
    let path = match config_path(app) {
        Ok(p) => p,
        Err(_) => return AppConfig::default(),
    };
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(config) = serde_json::from_str::<AppConfig>(&content) {
            return config;
        }
    }
    AppConfig::default()
}

fn save_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(())
}

fn parse_code(key: &str) -> Result<tauri_plugin_global_shortcut::Code, String> {
    use tauri_plugin_global_shortcut::Code;
    match key {
        "A" => Ok(Code::KeyA),
        "B" => Ok(Code::KeyB),
        "C" => Ok(Code::KeyC),
        "D" => Ok(Code::KeyD),
        "E" => Ok(Code::KeyE),
        "F" => Ok(Code::KeyF),
        "G" => Ok(Code::KeyG),
        "H" => Ok(Code::KeyH),
        "I" => Ok(Code::KeyI),
        "J" => Ok(Code::KeyJ),
        "K" => Ok(Code::KeyK),
        "L" => Ok(Code::KeyL),
        "M" => Ok(Code::KeyM),
        "N" => Ok(Code::KeyN),
        "O" => Ok(Code::KeyO),
        "P" => Ok(Code::KeyP),
        "Q" => Ok(Code::KeyQ),
        "R" => Ok(Code::KeyR),
        "S" => Ok(Code::KeyS),
        "T" => Ok(Code::KeyT),
        "U" => Ok(Code::KeyU),
        "V" => Ok(Code::KeyV),
        "W" => Ok(Code::KeyW),
        "X" => Ok(Code::KeyX),
        "Y" => Ok(Code::KeyY),
        "Z" => Ok(Code::KeyZ),
        "0" => Ok(Code::Digit0),
        "1" => Ok(Code::Digit1),
        "2" => Ok(Code::Digit2),
        "3" => Ok(Code::Digit3),
        "4" => Ok(Code::Digit4),
        "5" => Ok(Code::Digit5),
        "6" => Ok(Code::Digit6),
        "7" => Ok(Code::Digit7),
        "8" => Ok(Code::Digit8),
        "9" => Ok(Code::Digit9),
        "F1" => Ok(Code::F1),
        "F2" => Ok(Code::F2),
        "F3" => Ok(Code::F3),
        "F4" => Ok(Code::F4),
        "F5" => Ok(Code::F5),
        "F6" => Ok(Code::F6),
        "F7" => Ok(Code::F7),
        "F8" => Ok(Code::F8),
        "F9" => Ok(Code::F9),
        "F10" => Ok(Code::F10),
        "F11" => Ok(Code::F11),
        "F12" => Ok(Code::F12),
        "Enter" => Ok(Code::Enter),
        "Escape" | "Esc" => Ok(Code::Escape),
        "Space" => Ok(Code::Space),
        "Tab" => Ok(Code::Tab),
        "Backspace" => Ok(Code::Backspace),
        "Delete" | "Del" => Ok(Code::Delete),
        "ArrowUp" | "Up" => Ok(Code::ArrowUp),
        "ArrowDown" | "Down" => Ok(Code::ArrowDown),
        "ArrowLeft" | "Left" => Ok(Code::ArrowLeft),
        "ArrowRight" | "Right" => Ok(Code::ArrowRight),
        "Home" => Ok(Code::Home),
        "End" => Ok(Code::End),
        "PageUp" => Ok(Code::PageUp),
        "PageDown" => Ok(Code::PageDown),
        "Insert" | "Ins" => Ok(Code::Insert),
        "PrintScreen" | "PrtSc" | "Print" => Ok(Code::PrintScreen),
        "Pause" => Ok(Code::Pause),
        _ => Err(format!("Unsupported key: {}", key)),
    }
}

fn parse_shortcut(s: &str) -> Result<Shortcut, String> {
    let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
    let mut modifiers = tauri_plugin_global_shortcut::Modifiers::empty();
    let mut key_part = "";

    for part in &parts {
        match *part {
            "Ctrl" | "Control" => {
                modifiers |= tauri_plugin_global_shortcut::Modifiers::CONTROL
            }
            "Alt" => modifiers |= tauri_plugin_global_shortcut::Modifiers::ALT,
            "Shift" => modifiers |= tauri_plugin_global_shortcut::Modifiers::SHIFT,
            "Super" | "Cmd" | "Command" | "Meta" | "Win" => {
                modifiers |= tauri_plugin_global_shortcut::Modifiers::SUPER
            }
            _ => key_part = part,
        }
    }

    if key_part.is_empty() {
        return Err("No key specified".to_string());
    }

    let code = parse_code(key_part)?;
    let mods_opt = if modifiers.is_empty() {
        None
    } else {
        Some(modifiers)
    };
    Ok(Shortcut::new(mods_opt, code))
}

#[tauri::command]
fn get_double_press_enabled(config: tauri::State<ConfigArc>) -> bool {
    config.lock().unwrap().double_press_enabled
}

#[tauri::command]
fn set_double_press_enabled(
    app: AppHandle,
    config: tauri::State<ConfigArc>,
    enabled: bool,
) -> Result<(), String> {
    {
        let mut cfg = config.lock().unwrap();
        cfg.double_press_enabled = enabled;
    }
    save_config(&app, &config.lock().unwrap())
}

#[tauri::command]
fn get_screenshot_hotkey(config: tauri::State<ConfigArc>) -> String {
    config.lock().unwrap().screenshot_hotkey.clone()
}

#[tauri::command]
fn set_screenshot_hotkey(
    app: AppHandle,
    config: tauri::State<ConfigArc>,
    hotkey: String,
) -> Result<(), String> {
    let old_hotkey = config.lock().unwrap().screenshot_hotkey.clone();

    let new_shortcut = parse_shortcut(&hotkey)?;

    if let Ok(old) = parse_shortcut(&old_hotkey) {
        let _ = app.global_shortcut().unregister(old);
    }

    app.global_shortcut()
        .register(new_shortcut)
        .map_err(|e| e.to_string())?;

    {
        let mut cfg = config.lock().unwrap();
        cfg.screenshot_hotkey = hotkey;
    }

    save_config(&app, &config.lock().unwrap())
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
fn capture_screen_region(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<String, String> {
    use arboard::Clipboard;
    use screenshots::Screen;

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
            set_double_press_enabled,
            get_screenshot_hotkey,
            set_screenshot_hotkey
        ])
        .setup(move |app| {
            let app_handle = app.handle();

            let loaded_config = load_config(&app_handle);
            let hotkey_str = loaded_config.screenshot_hotkey.clone();
            {
                let mut cfg = config.lock().unwrap();
                *cfg = loaded_config;
            }

            let settings_item =
                MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
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

            if let Ok(shortcut) = parse_shortcut(&hotkey_str) {
                let _ = app.global_shortcut().unregister(shortcut.clone());
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    eprintln!(
                        "Warning: failed to register global shortcut {}: {}",
                        hotkey_str, e
                    );
                }
            }

            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.hide();
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
