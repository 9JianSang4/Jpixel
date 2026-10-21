//! Versioned native configuration.
//!
//! This is intentionally separate from the legacy Tauri `config` module during
//! the P2.1 transition. It contains only settings the native hot path needs.

use crate::core::hotkey;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CURRENT_VERSION: u32 = 1;
pub const DEFAULT_HOTKEY: &str = "Alt+X";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

fn default_hotkey() -> String {
    DEFAULT_HOTKEY.to_string()
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            hotkey: DEFAULT_HOTKEY.to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid hotkey: {0}")]
    InvalidHotkey(String),
}

impl NativeConfig {
    /// Parse and validate a config document. Missing fields fall back to defaults.
    pub fn from_json(json: &str) -> Result<Self, ConfigError> {
        let mut config: NativeConfig = serde_json::from_str(json)?;
        if config.version == 0 {
            config.version = CURRENT_VERSION;
        }
        config.hotkey = config.hotkey.trim().to_string();
        hotkey::parse(&config.hotkey)
            .map_err(|e| ConfigError::InvalidHotkey(format!("'{}': {e}", config.hotkey)))?;
        Ok(config)
    }

    pub fn to_json(&self) -> Result<String, ConfigError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Load a config, returning defaults for a missing/corrupt file.
    ///
    /// A corrupt file is left in place for diagnostics; the caller can decide
    /// whether to overwrite it on the next save.
    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match Self::from_json(&text) {
                Ok(config) => config,
                Err(e) => {
                    log::warn!("invalid native config at {}: {e}", path.display());
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                log::warn!("cannot read native config at {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// Write to a sibling temp file, then replace the destination.
    pub fn save_atomic(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, self.to_json()?)?;
        // std::fs::rename does not replace an existing file on Windows.
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(&temp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "jpixel-native-config-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn default_hotkey_is_alt_x() {
        let config = NativeConfig::default();
        assert_eq!(config.version, CURRENT_VERSION);
        assert_eq!(config.hotkey, "Alt+X");
    }

    #[test]
    fn missing_fields_use_defaults() {
        let config = NativeConfig::from_json(r#"{}"#).unwrap();
        assert_eq!(config.version, CURRENT_VERSION);
        assert_eq!(config.hotkey, "Alt+X");
    }

    #[test]
    fn invalid_hotkey_is_rejected() {
        let err = NativeConfig::from_json(r#"{"hotkey":"Alt+NotAKey"}"#).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidHotkey(_)));
    }

    #[test]
    fn corrupt_json_is_rejected_by_parser() {
        assert!(NativeConfig::from_json("{not json").is_err());
    }

    #[test]
    fn corrupt_file_falls_back_to_default() {
        let dir = temp_dir();
        let path = dir.join("config.json");
        std::fs::write(&path, "{not json").unwrap();
        let config = NativeConfig::load_or_default(&path);
        assert_eq!(config, NativeConfig::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn roundtrip_and_atomic_save() {
        let dir = temp_dir();
        let path = dir.join("config.json");
        let config = NativeConfig {
            version: CURRENT_VERSION,
            hotkey: "Ctrl+F1".to_string(),
        };
        config.save_atomic(&path).unwrap();
        let loaded = NativeConfig::load_or_default(&path);
        assert_eq!(loaded, config);
        let _ = std::fs::remove_dir_all(dir);
    }
}
