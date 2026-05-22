use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::ShortcutState;

mod capture;
mod clipboard;
mod config;
mod error;
mod ocr;
mod shortcut;
mod window;

use config::{load_config, ConfigArc, HotkeyState, StateArc};

// ─────────────────────────────────────────────────────────────
// Single instance guard
// ─────────────────────────────────────────────────────────────

fn ensure_single_instance() -> bool {
    let instance = match single_instance::SingleInstance::new("jpixel-screenshot-tool") {
        Ok(i) => i,
        Err(e) => {
            log::error!("Failed to create single-instance lock: {}. Refusing to start.", e);
            return false;
        }
    };

    if !instance.is_single() {
        log::warn!("Another Jpixel instance is already running. Exiting.");
        false
    } else {
        true
    }
}

// ─────────────────────────────────────────────────────────────
// Hotkey handling
// ─────────────────────────────────────────────────────────────

fn handle_capture_hotkey(app: &AppHandle, config: &ConfigArc, state: &StateArc) {
    let double_press = {
        let cfg = config::lock_or_warn!(config);
        cfg.double_press_enabled
    };

    if !double_press {
        let _ = app.emit("trigger-capture", ());
        window::create_capture_window(app.clone());
        return;
    }

    let should_trigger = {
        let mut st = config::lock_or_warn!(state);
        if st.waiting_second_press {
            st.waiting_second_press = false;
            st.last_press_time = None;
            true
        } else {
            st.waiting_second_press = true;
            st.last_press_time = Some(Instant::now());
            false
        }
    };

    if should_trigger {
        let _ = app.emit("trigger-capture", ());
        window::create_capture_window(app.clone());
        return;
    }

    // Start timeout thread for second press
    let app_clone = app.clone();
    let state_clone = state.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        let mut st = config::lock_or_warn!(state_clone);
        if st.waiting_second_press {
            st.waiting_second_press = false;
            st.last_press_time = None;
            let _ = app_clone.emit("hotkey-timeout", ());
        }
    });
}

// ─────────────────────────────────────────────────────────────
// Setup helpers
// ─────────────────────────────────────────────────────────────

fn setup_logging() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();
}

fn setup_tray(app: &AppHandle) -> Result<(), tauri::Error> {
    let settings_item = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings_item, &quit_item,
    ])?;

    TrayIconBuilder::new()
        .icon(
            app.default_window_icon()
                .ok_or_else(|| {
                    log::error!("Default window icon not found");
                    tauri::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "default window icon",
                    ))
                })?
                .clone(),
        )
        .tooltip("Jpixel")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| {
            match event.id.as_ref() {
                "settings" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|_tray, _event| {
            // Intentionally no-op: screenshot is triggered only by hotkey.
        })
        .build(app)?;

    Ok(())
}

fn setup_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let win_hide = window.clone();
        let _ = window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = win_hide.hide();
            }
        });
        let _ = window.hide();
    }
}

// ─────────────────────────────────────────────────────────────
// Main entry
// ─────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    setup_logging();

    if !ensure_single_instance() {
        std::process::exit(0);
    }

    let config: ConfigArc = ConfigArc::new(std::sync::Mutex::new(config::AppConfig::default()));
    let hotkey_state: StateArc = StateArc::new(std::sync::Mutex::new(HotkeyState::default()));
    let config_for_plugin = config.clone();
    let state_for_plugin = hotkey_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        handle_capture_hotkey(app, &config_for_plugin, &state_for_plugin);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(config.clone())
        .manage(hotkey_state.clone())
        .invoke_handler(tauri::generate_handler![
            // Window
            window::create_capture_window,
            window::close_capture_windows,
            window::create_editor_window,
            window::create_pin_window,
            window::close_pin_window,
            // Capture
            capture::create_pin_from_region,
            capture::read_image_base64,
            capture::save_region_dialog,
            capture::get_pixel_color,
            capture::get_magnifier_area,
            capture::ocr_region,
            // Config
            config::get_double_press_enabled,
            config::set_double_press_enabled,
            config::get_screenshot_hotkey,
            config::set_screenshot_hotkey,
            config::get_copy_hotkey,
            config::set_copy_hotkey,
            config::get_save_hotkey,
            config::set_save_hotkey,
            config::get_pin_hotkey,
            config::set_pin_hotkey,
            config::get_ocr_hotkey,
            config::set_ocr_hotkey,
            config::get_default_action,
            config::set_default_action,
            // Clipboard
            clipboard::copy_region_to_clipboard,
            clipboard::copy_text_to_clipboard,
        ])
        .setup(move |app| {
            let app_handle = app.handle();

            let loaded_config = load_config(&app_handle);
            let hotkey_str = loaded_config.screenshot_hotkey.clone();
            {
                let mut cfg = config::lock_or_warn!(config);
                *cfg = loaded_config;
            }

            if let Err(e) = setup_tray(&app_handle) {
                log::error!("Failed to setup tray: {}", e);
            }

            setup_main_window(&app_handle);
            shortcut::register_screenshot_hotkey(&app_handle, &hotkey_str);

            log::info!("Jpixel setup completed successfully");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
