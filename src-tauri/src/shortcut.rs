use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use tauri::AppHandle;

/// Parse a single key string into a Tauri `Code`.
///
/// Supported keys: A-Z, 0-9, F1-F12, Enter, Escape/Esc, Space, Tab,
/// Backspace, Delete/Del, Arrow directions, Home, End, PageUp, PageDown,
/// Insert/Ins, PrintScreen/PrtSc/Print, Pause.
pub fn parse_code(key: &str) -> Result<tauri_plugin_global_shortcut::Code, String> {
    use tauri_plugin_global_shortcut::Code;

    // Alphanumeric keys
    if key.len() == 1 {
        let c = key.chars().next().unwrap();
        return match c {
            'A'..='Z' => parse_alpha(c),
            '0'..='9' => parse_digit(c),
            _ => Err(format!("Unsupported single key: {}", key)),
        };
    }

    // Named keys
    match key {
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

fn parse_alpha(c: char) -> Result<tauri_plugin_global_shortcut::Code, String> {
    use tauri_plugin_global_shortcut::Code;
    match c {
        'A' => Ok(Code::KeyA),
        'B' => Ok(Code::KeyB),
        'C' => Ok(Code::KeyC),
        'D' => Ok(Code::KeyD),
        'E' => Ok(Code::KeyE),
        'F' => Ok(Code::KeyF),
        'G' => Ok(Code::KeyG),
        'H' => Ok(Code::KeyH),
        'I' => Ok(Code::KeyI),
        'J' => Ok(Code::KeyJ),
        'K' => Ok(Code::KeyK),
        'L' => Ok(Code::KeyL),
        'M' => Ok(Code::KeyM),
        'N' => Ok(Code::KeyN),
        'O' => Ok(Code::KeyO),
        'P' => Ok(Code::KeyP),
        'Q' => Ok(Code::KeyQ),
        'R' => Ok(Code::KeyR),
        'S' => Ok(Code::KeyS),
        'T' => Ok(Code::KeyT),
        'U' => Ok(Code::KeyU),
        'V' => Ok(Code::KeyV),
        'W' => Ok(Code::KeyW),
        'X' => Ok(Code::KeyX),
        'Y' => Ok(Code::KeyY),
        'Z' => Ok(Code::KeyZ),
        _ => unreachable!(),
    }
}

fn parse_digit(c: char) -> Result<tauri_plugin_global_shortcut::Code, String> {
    use tauri_plugin_global_shortcut::Code;
    match c {
        '0' => Ok(Code::Digit0),
        '1' => Ok(Code::Digit1),
        '2' => Ok(Code::Digit2),
        '3' => Ok(Code::Digit3),
        '4' => Ok(Code::Digit4),
        '5' => Ok(Code::Digit5),
        '6' => Ok(Code::Digit6),
        '7' => Ok(Code::Digit7),
        '8' => Ok(Code::Digit8),
        '9' => Ok(Code::Digit9),
        _ => unreachable!(),
    }
}

/// Parse a human-readable hotkey string into a Tauri `Shortcut`.
///
/// Format: "Ctrl+Shift+A", "F1", "Alt+Enter", etc.
/// Modifiers: Ctrl/Control, Alt, Shift, Super/Cmd/Command/Meta/Win.
pub fn parse_shortcut(s: &str) -> Result<Shortcut, String> {
    let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
    let mut modifiers = tauri_plugin_global_shortcut::Modifiers::empty();
    let mut key_part = "";

    for part in &parts {
        match *part {
            "Ctrl" | "Control" => modifiers |= tauri_plugin_global_shortcut::Modifiers::CONTROL,
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

/// Register the screenshot hotkey, replacing any existing registration.
pub fn register_screenshot_hotkey(app: &AppHandle, hotkey_str: &str) {
    match parse_shortcut(hotkey_str) {
        Ok(shortcut) => {
            let _ = app.global_shortcut().unregister(shortcut.clone());
            match app.global_shortcut().register(shortcut) {
                Ok(()) => log::info!("Registered global shortcut: {}", hotkey_str),
                Err(e) => log::warn!("Failed to register global shortcut {}: {}", hotkey_str, e),
            }
        }
        Err(e) => log::warn!("Failed to parse global shortcut '{}': {}", hotkey_str, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_code ────────────────────────────────────────────

    #[test]
    fn parse_code_alpha_uppercase() {
        assert!(parse_code("A").is_ok());
        assert!(parse_code("Z").is_ok());
    }

    #[test]
    fn parse_code_digits() {
        for d in '0'..='9' {
            assert!(parse_code(&d.to_string()).is_ok(), "digit {} failed", d);
        }
    }

    #[test]
    fn parse_code_function_keys() {
        for i in 1..=12 {
            assert!(parse_code(&format!("F{}", i)).is_ok());
        }
    }

    #[test]
    fn parse_code_named_keys_and_aliases() {
        let cases = [
            "Enter", "Escape", "Esc", "Space", "Tab", "Backspace",
            "Delete", "Del", "ArrowUp", "Up", "ArrowDown", "Down",
            "ArrowLeft", "Left", "ArrowRight", "Right", "Home", "End",
            "PageUp", "PageDown", "Insert", "Ins",
            "PrintScreen", "PrtSc", "Print", "Pause",
        ];
        for key in &cases {
            assert!(parse_code(key).is_ok(), "named key {} failed", key);
        }
    }

    #[test]
    fn parse_code_invalid_lowercase() {
        assert!(parse_code("a").is_err());
    }

    #[test]
    fn parse_code_invalid_special_char() {
        assert!(parse_code("@").is_err());
        assert!(parse_code(" ").is_err());
    }

    #[test]
    fn parse_code_invalid_unknown() {
        assert!(parse_code("UnknownKey").is_err());
    }

    // ── parse_shortcut ────────────────────────────────────────

    #[test]
    fn parse_shortcut_single_key() {
        let s = parse_shortcut("F1").unwrap();
        assert_eq!(s.key, parse_code("F1").unwrap());
    }

    #[test]
    fn parse_shortcut_combination() {
        let s = parse_shortcut("Ctrl+Shift+A").unwrap();
        assert_eq!(s.key, parse_code("A").unwrap());
        assert!(!s.mods.is_empty());
    }

    #[test]
    fn parse_shortcut_modifier_aliases() {
        for alias in &["Cmd", "Meta", "Win", "Super"] {
            let s = parse_shortcut(&format!("{}+A", alias)).unwrap();
            assert_eq!(s.key, parse_code("A").unwrap());
        }
    }

    #[test]
    fn parse_shortcut_empty_fails() {
        assert!(parse_shortcut("").is_err());
    }

    #[test]
    fn parse_shortcut_modifiers_only_fails() {
        assert!(parse_shortcut("Ctrl+").is_err());
        assert!(parse_shortcut("Ctrl+Shift").is_err());
    }
}
