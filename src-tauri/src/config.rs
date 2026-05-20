use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use crate::error::JpixelError;
use crate::shortcut::parse_shortcut;

// ─────────────────────────────────────────────────────────────
// Pure configuration (serializable)
// ─────────────────────────────────────────────────────────────

/// Application persisted configuration.
///
/// Contains only settings that should survive across restarts.
/// Runtime transient state lives in `HotkeyState`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    #[serde(default)]
    pub double_press_enabled: bool,
    #[serde(default = "default_hotkey_f1")]
    pub screenshot_hotkey: String,
    #[serde(default = "default_hotkey_ctrl_c")]
    pub copy_hotkey: String,
    #[serde(default = "default_hotkey_ctrl_s")]
    pub save_hotkey: String,
    #[serde(default = "default_hotkey_ctrl_t")]
    pub pin_hotkey: String,
    #[serde(default = "default_hotkey_ctrl_r")]
    pub ocr_hotkey: String,
    #[serde(default)]
    pub default_action: DefaultAction,
    #[serde(default = "default_gif_fps")]
    pub gif_fps: u8,
    #[serde(default = "default_gif_quality")]
    pub gif_quality: u8,
}

fn default_hotkey_f1() -> String { "F1".to_string() }
fn default_hotkey_ctrl_c() -> String { "Ctrl+C".to_string() }
fn default_hotkey_ctrl_s() -> String { "Ctrl+S".to_string() }
fn default_hotkey_ctrl_t() -> String { "Ctrl+T".to_string() }
fn default_hotkey_ctrl_r() -> String { "Ctrl+R".to_string() }
fn default_gif_fps() -> u8 { 10 }
fn default_gif_quality() -> u8 { 128 }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DefaultAction {
    SaveAndEdit,
    Copy,
    Save,
    Pin,
}

impl Default for DefaultAction {
    fn default() -> Self {
        DefaultAction::SaveAndEdit
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            double_press_enabled: false,
            screenshot_hotkey: "F1".to_string(),
            copy_hotkey: "Ctrl+C".to_string(),
            save_hotkey: "Ctrl+S".to_string(),
            pin_hotkey: "Ctrl+T".to_string(),
            ocr_hotkey: "Ctrl+R".to_string(),
            default_action: DefaultAction::SaveAndEdit,
            gif_fps: 10,
            gif_quality: 128,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Runtime state (not serialized)
// ─────────────────────────────────────────────────────────────

/// Transient hotkey state, separate from persisted config.
#[derive(Debug, Default)]
pub struct HotkeyState {
    pub waiting_second_press: bool,
    pub last_press_time: Option<Instant>,
}

// ─────────────────────────────────────────────────────────────
// Shared state container
// ─────────────────────────────────────────────────────────────

pub type ConfigArc = Arc<Mutex<AppConfig>>;
pub type StateArc = Arc<Mutex<HotkeyState>>;

// ─────────────────────────────────────────────────────────────
// Persistence helpers
// ─────────────────────────────────────────────────────────────

pub fn config_path(app: &AppHandle) -> Result<PathBuf, JpixelError> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| JpixelError::Config(format!("Cannot resolve config dir: {}", e)))?;
    Ok(config_dir.join("config.json"))
}

/// Load config from disk, falling back to defaults on any error.
pub fn load_config(app: &AppHandle) -> AppConfig {
    let path = match config_path(app) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Using default config because path resolution failed: {}", e);
            return AppConfig::default();
        }
    };

    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<AppConfig>(&content) {
            Ok(config) => config,
            Err(e) => {
                log::warn!("Config file corrupted at {:?}, using defaults: {}", path, e);
                AppConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("No config file found at {:?}, using defaults", path);
            AppConfig::default()
        }
        Err(e) => {
            log::warn!("Failed to read config at {:?}: {}, using defaults", path, e);
            AppConfig::default()
        }
    }
}

/// Atomically save config to disk.
pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<(), JpixelError> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| JpixelError::Io(e))?;
    }

    let content = serde_json::to_string_pretty(config)
        .map_err(|e| JpixelError::Config(format!("Serialize failed: {}", e)))?;

    let temp_path = path.with_extension("json.tmp");
    std::fs::write(&temp_path, content).map_err(|e| JpixelError::Io(e))?;
    std::fs::rename(&temp_path, &path).map_err(|e| JpixelError::Io(e))?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Lock helper
// ─────────────────────────────────────────────────────────────

/// Lock a mutex, recovering from poison and logging a warning.
macro_rules! lock_or_warn {
    ($mutex:expr) => {
        $mutex.lock().unwrap_or_else(|poisoned| {
            log::warn!("Mutex was poisoned; recovering inner data");
            poisoned.into_inner()
        })
    };
}

pub(crate) use lock_or_warn;

// ─────────────────────────────────────────────────────────────
// Tauri Commands
// ─────────────────────────────────────────────────────────────

macro_rules! config_getter {
    ($name:ident, $field:ident, $ty:ty) => {
        #[tauri::command]
        pub fn $name(config: State<ConfigArc>) -> $ty {
            crate::config::lock_or_warn!(config).$field.clone()
        }
    };
}

macro_rules! config_setter {
    ($name:ident, $field:ident, $ty:ty) => {
        #[tauri::command]
        pub fn $name(
            app: AppHandle,
            config: State<ConfigArc>,
            value: $ty,
        ) -> Result<(), JpixelError> {
            let mut cfg = crate::config::lock_or_warn!(config);
            cfg.$field = value;
            let clone = cfg.clone();
            drop(cfg);
            save_config(&app, &clone)
        }
    };
}

config_getter!(get_double_press_enabled, double_press_enabled, bool);
config_setter!(set_double_press_enabled, double_press_enabled, bool);

config_getter!(get_screenshot_hotkey, screenshot_hotkey, String);
config_getter!(get_copy_hotkey, copy_hotkey, String);
config_getter!(get_save_hotkey, save_hotkey, String);
config_getter!(get_pin_hotkey, pin_hotkey, String);
config_getter!(get_ocr_hotkey, ocr_hotkey, String);

config_getter!(get_default_action, default_action, DefaultAction);
config_setter!(set_default_action, default_action, DefaultAction);

config_getter!(get_gif_fps, gif_fps, u8);
config_setter!(set_gif_fps, gif_fps, u8);

config_getter!(get_gif_quality, gif_quality, u8);
config_setter!(set_gif_quality, gif_quality, u8);

/// Update a hotkey and re-register it with the OS.
///
/// # Safety
/// The old hotkey is unregistered before the new one is registered to avoid
/// duplicate global shortcuts.
#[tauri::command]
pub fn set_screenshot_hotkey(
    app: AppHandle,
    config: State<ConfigArc>,
    hotkey: String,
) -> Result<(), JpixelError> {
    let new_shortcut = parse_shortcut(&hotkey)
        .map_err(|e| JpixelError::InvalidHotkey(format!("'{}': {}", hotkey, e)))?;

    let mut cfg = lock_or_warn!(config);
    let old_hotkey = cfg.screenshot_hotkey.clone();
    cfg.screenshot_hotkey = hotkey;
    let clone = cfg.clone();
    drop(cfg);

    if let Ok(old) = parse_shortcut(&old_hotkey) {
        let _ = app.global_shortcut().unregister(old);
    }

    app.global_shortcut()
        .register(new_shortcut)
        .map_err(|e| JpixelError::InvalidHotkey(format!("Registration failed: {}", e)))?;

    save_config(&app, &clone)
}

#[tauri::command]
pub fn set_copy_hotkey(
    app: AppHandle,
    config: State<ConfigArc>,
    hotkey: String,
) -> Result<(), JpixelError> {
    let _ = parse_shortcut(&hotkey)
        .map_err(|e| JpixelError::InvalidHotkey(format!("'{}': {}", hotkey, e)))?;
    let mut cfg = lock_or_warn!(config);
    cfg.copy_hotkey = hotkey;
    let clone = cfg.clone();
    drop(cfg);
    save_config(&app, &clone)
}

#[tauri::command]
pub fn set_save_hotkey(
    app: AppHandle,
    config: State<ConfigArc>,
    hotkey: String,
) -> Result<(), JpixelError> {
    let _ = parse_shortcut(&hotkey)
        .map_err(|e| JpixelError::InvalidHotkey(format!("'{}': {}", hotkey, e)))?;
    let mut cfg = lock_or_warn!(config);
    cfg.save_hotkey = hotkey;
    let clone = cfg.clone();
    drop(cfg);
    save_config(&app, &clone)
}

#[tauri::command]
pub fn set_pin_hotkey(
    app: AppHandle,
    config: State<ConfigArc>,
    hotkey: String,
) -> Result<(), JpixelError> {
    let _ = parse_shortcut(&hotkey)
        .map_err(|e| JpixelError::InvalidHotkey(format!("'{}': {}", hotkey, e)))?;
    let mut cfg = lock_or_warn!(config);
    cfg.pin_hotkey = hotkey;
    let clone = cfg.clone();
    drop(cfg);
    save_config(&app, &clone)
}

#[tauri::command]
pub fn set_ocr_hotkey(
    app: AppHandle,
    config: State<ConfigArc>,
    hotkey: String,
) -> Result<(), JpixelError> {
    let _ = parse_shortcut(&hotkey)
        .map_err(|e| JpixelError::InvalidHotkey(format!("'{}': {}", hotkey, e)))?;
    let mut cfg = lock_or_warn!(config);
    cfg.ocr_hotkey = hotkey;
    let clone = cfg.clone();
    drop(cfg);
    save_config(&app, &clone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_config_default_values() {
        let cfg = AppConfig::default();
        assert!(!cfg.double_press_enabled);
        assert_eq!(cfg.screenshot_hotkey, "F1");
        assert_eq!(cfg.copy_hotkey, "Ctrl+C");
        assert_eq!(cfg.save_hotkey, "Ctrl+S");
        assert_eq!(cfg.pin_hotkey, "Ctrl+T");
        assert_eq!(cfg.ocr_hotkey, "Ctrl+R");
        assert_eq!(cfg.default_action, DefaultAction::SaveAndEdit);
        assert_eq!(cfg.gif_fps, 10);
        assert_eq!(cfg.gif_quality, 128);
    }

    #[test]
    fn default_action_default_is_save_and_edit() {
        assert_eq!(DefaultAction::default(), DefaultAction::SaveAndEdit);
    }

    #[test]
    fn config_serde_roundtrip() {
        let cfg = AppConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg.screenshot_hotkey, restored.screenshot_hotkey);
        assert_eq!(cfg.default_action, restored.default_action);
        assert_eq!(cfg.gif_quality, restored.gif_quality);
    }

    #[test]
    fn config_deserialize_missing_fields_use_defaults() {
        let json = r#"{ "screenshot_hotkey": "F2" }"#;
        let cfg: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.screenshot_hotkey, "F2");
        assert_eq!(cfg.copy_hotkey, "Ctrl+C"); // default
        assert_eq!(cfg.gif_fps, 10); // default
        assert_eq!(cfg.default_action, DefaultAction::SaveAndEdit); // default
    }

    #[test]
    fn default_helper_functions() {
        assert_eq!(default_hotkey_f1(), "F1");
        assert_eq!(default_hotkey_ctrl_c(), "Ctrl+C");
        assert_eq!(default_hotkey_ctrl_s(), "Ctrl+S");
        assert_eq!(default_hotkey_ctrl_t(), "Ctrl+T");
        assert_eq!(default_hotkey_ctrl_r(), "Ctrl+R");
        assert_eq!(default_gif_fps(), 10);
        assert_eq!(default_gif_quality(), 128);
    }
}
