use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::ShortcutState;

mod clipboard;
mod config;
mod shortcut;
mod window;

// capture module depends on window module, so declare it after
mod capture;

use config::{load_config, ConfigArc};

/// Check if another instance is running; if so, exit.
fn ensure_single_instance() -> bool {
    let instance = single_instance::SingleInstance::new("jpixel-screenshot-tool").unwrap_or_else(|e| {
        log::warn!("Failed to create single-instance lock: {}", e);
        // If we can't check, assume we're the only instance
        return single_instance::SingleInstance::new("jpixel-fallback").unwrap();
    });

    if !instance.is_single() {
        log::warn!("Another Jpixel instance is already running. Exiting.");
        return false;
    }
    true
}

fn handle_capture_hotkey(app: &AppHandle, config: &ConfigArc) {
    let mut cfg = crate::lock_config!(config);

    if !cfg.double_press_enabled {
        drop(cfg);
        let _ = app.emit("trigger-capture", ());
        window::create_capture_window(app.clone());
        return;
    }

    if cfg.waiting_second_press {
        cfg.waiting_second_press = false;
        cfg.last_press_time = None;
        drop(cfg);
        let _ = app.emit("trigger-capture", ());
        window::create_capture_window(app.clone());
    } else {
        cfg.waiting_second_press = true;
        cfg.last_press_time = Some(Instant::now());
        drop(cfg);

        let app_clone = app.clone();
        let config_clone = config.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            let mut cfg = crate::lock_config!(config_clone);
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
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    // Single instance check
    if !ensure_single_instance() {
        std::process::exit(0);
    }

    let config: ConfigArc = ConfigArc::new(std::sync::Mutex::new(config::AppConfig::default()));
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
            window::create_capture_window,
            window::close_capture_windows,
            window::create_editor_window,
            window::create_pin_window,
            window::close_pin_window,
            capture::capture_screen_region,
            capture::create_pin_from_region,
            capture::read_image_base64,
            capture::save_region_dialog,
            capture::get_pixel_color,
            capture::get_magnifier_area,
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
            clipboard::copy_region_to_clipboard,
            clipboard::copy_text_to_clipboard,
        ])
        .setup(move |app| {
            let app_handle = app.handle();

            let loaded_config = load_config(&app_handle);
            let hotkey_str = loaded_config.screenshot_hotkey.clone();
            {
                let mut cfg = crate::lock_config!(config);
                *cfg = loaded_config;
            }

            // Tray icon setup
            let settings_item =
                MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_item, &quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Jpixel")
                .menu(&menu)
                .on_menu_event({
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
                            window::create_capture_window(_tray.app_handle().clone());
                        }
                    }
                })
                .build(app)?;

            // Register global screenshot hotkey
            shortcut::register_screenshot_hotkey(&app_handle, &hotkey_str);

            // Main window close handler (minimize to tray)
            if let Some(window) = app_handle.get_webview_window("main") {
                let win_hide = window.clone();
                let _ = window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_hide.hide();
                    }
                });
                let _ = window.hide();
            }

            log::info!("Jpixel setup completed successfully");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
