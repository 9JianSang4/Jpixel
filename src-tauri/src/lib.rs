use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

pub mod capture;
mod bubble;
pub mod clipboard;
mod config;
pub mod core;
pub mod error;
pub mod overlay;
mod pin;
mod round_rect;
mod window;

use config::{load_config, ConfigArc};

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
        // Leak the guard so the single-instance lock outlives the process.
        // Dropping it would release the lock and allow concurrent instances.
        std::mem::forget(instance);
        true
    }
}

// ─────────────────────────────────────────────────────────────
// Setup helpers
// ─────────────────────────────────────────────────────────────

fn setup_logging() {
    // In release builds, do not initialize env_logger — its stderr output
    // can cause a visible console window when launched at boot from the Run
    // registry key, even with windows_subsystem="windows". Without a logger
    // backend the `log` crate macros become no-ops.
    #[cfg(debug_assertions)]
    {
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("info"),
        )
        .init();
    }
}

fn setup_tray(app: &AppHandle) -> Result<(), tauri::Error> {
    let settings_item = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings_item, &quit_item])?;

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
        .show_menu_on_left_click(false)
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
        .on_tray_icon_event(|_tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                overlay::trigger_pin_confirm();
            }
        })
        .build(app)?;

    Ok(())
}

fn setup_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let win_hide = window.clone();
        window.on_window_event(move |event| {
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

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(config.clone())
        .invoke_handler(tauri::generate_handler![
            // Window
            window::create_capture_window,
            window::close_capture_windows,
            window::create_pin_window,
            window::close_pin_window,
            // Capture
            capture::create_pin_from_region,
            capture::read_image_base64,
            capture::save_region_dialog,
            capture::get_pixel_color,
            capture::get_magnifier_area,
            // Capture (image-based, frozen-background flow)
            capture::copy_image_to_clipboard,
            capture::save_image_dialog,
            capture::create_pin_from_image,
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
            config::get_pin_limit,
            config::set_pin_limit,
            config::get_auto_launch,
            config::set_auto_launch,
            config::get_crosshair_guides,
            config::set_crosshair_guides,
            config::get_selection_mode,
            config::set_selection_mode,
            config::get_double_press_speed,
            config::set_double_press_speed,
            config::get_default_action,
            config::set_default_action,
            // Clipboard
            clipboard::copy_region_to_clipboard,
            clipboard::copy_text_to_clipboard,
        ])
        .setup(move |app| {
            let app_handle = app.handle();

            let loaded_config = load_config(app_handle);
            {
                let mut cfg = config::lock_or_warn!(config);
                *cfg = loaded_config;
            }

            if let Err(e) = setup_tray(app_handle) {
                log::error!("Failed to setup tray: {}", e);
            }

            setup_main_window(app_handle);
            // P2.1: the native overlay owns the screenshot hotkey. Keep the
            // legacy registration code for parity work, but do not register it
            // here or it will steal Alt+X from the overlay thread.
            capture::cleanup_temp_pins_on_startup(app_handle);

            // P2.1 native path: the saved screenshot hotkey is registered by
            // the overlay thread, not the Tauri global-shortcut plugin.
            let native_hotkey = {
                let cfg = config::lock_or_warn!(config);
                cfg.screenshot_hotkey.clone()
            };
            let native_options = {
                let cfg = config::lock_or_warn!(config);
                pin::set_limit(cfg.pin_limit.count());
                config::native_overlay_options(&cfg)
            };
            if let Err(e) = overlay::start(&native_hotkey, native_options) {
                log::error!("Failed to start native overlay with '{native_hotkey}': {e}");
            }

            log::info!("Jpixel setup completed successfully");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
