//! Hotkey parsing shared by config validation and the native overlay.
//!
//! The parser is deliberately small and strict: a hotkey is one optional set of
//! modifiers plus exactly one non-modifier key. Registration happens in the
//! overlay thread with RegisterHotKey.

use std::fmt;

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub vk: u32,
    pub mods: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyParseError(pub String);

impl fmt::Display for HotkeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HotkeyParseError {}

pub fn parse(value: &str) -> Result<Hotkey, HotkeyParseError> {
    let mut mods = 0u32;
    let mut key: Option<u32> = None;
    let mut names: Vec<String> = Vec::new();

    for raw in value.split('+') {
        let token = raw.trim().to_ascii_uppercase();
        if token.is_empty() {
            return Err(HotkeyParseError(format!("empty segment in '{value}'")));
        }
        match token.as_str() {
            "CTRL" | "CONTROL" => {
                if mods & MOD_CONTROL != 0 {
                    return Err(HotkeyParseError("duplicate Ctrl".to_string()));
                }
                mods |= MOD_CONTROL;
                names.push("Ctrl".to_string());
            }
            "ALT" => {
                if mods & MOD_ALT != 0 {
                    return Err(HotkeyParseError("duplicate Alt".to_string()));
                }
                mods |= MOD_ALT;
                names.push("Alt".to_string());
            }
            "SHIFT" => {
                if mods & MOD_SHIFT != 0 {
                    return Err(HotkeyParseError("duplicate Shift".to_string()));
                }
                mods |= MOD_SHIFT;
                names.push("Shift".to_string());
            }
            "WIN" | "SUPER" => {
                if mods & MOD_WIN != 0 {
                    return Err(HotkeyParseError("duplicate Win".to_string()));
                }
                mods |= MOD_WIN;
                names.push("Win".to_string());
            }
            other => {
                if key.is_some() {
                    return Err(HotkeyParseError(format!(
                        "exactly one non-modifier key is allowed: '{value}'"
                    )));
                }
                key = Some(parse_key(other)?);
                names.push(other.to_string());
            }
        }
    }

    let vk = key.ok_or_else(|| HotkeyParseError(format!("missing key in '{value}'")))?;
    Ok(Hotkey {
        vk,
        mods,
        name: names.join("+"),
    })
}

fn parse_key(token: &str) -> Result<u32, HotkeyParseError> {
    match token {
        "F1" => Ok(0x70),
        "F2" => Ok(0x71),
        "F3" => Ok(0x72),
        "F4" => Ok(0x73),
        "F5" => Ok(0x74),
        "F6" => Ok(0x75),
        "F7" => Ok(0x76),
        "F8" => Ok(0x77),
        "F9" => Ok(0x78),
        "F10" => Ok(0x79),
        "F11" => Ok(0x7A),
        "F12" => Ok(0x7B),
        other if other.len() == 1 => {
            let byte = other.as_bytes()[0];
            if byte.is_ascii_uppercase() || byte.is_ascii_digit() {
                Ok(byte as u32)
            } else {
                Err(HotkeyParseError(format!("unsupported key: '{other}'")))
            }
        }
        other => Err(HotkeyParseError(format!("unsupported key: '{other}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_alt_x() {
        let hotkey = parse("Alt+X").expect("Alt+X should parse");
        assert_eq!(hotkey.vk, 0x58);
        assert_eq!(hotkey.mods, MOD_ALT);
        assert_eq!(hotkey.name, "Alt+X");
    }

    #[test]
    fn parses_ctrl_f1_and_ctrl_alt_j() {
        let ctrl_f1 = parse("ctrl+f1").unwrap();
        assert_eq!(ctrl_f1.vk, 0x70);
        assert_eq!(ctrl_f1.mods, MOD_CONTROL);
        assert_eq!(ctrl_f1.name, "Ctrl+F1");

        let combo = parse("Ctrl + Alt + J").unwrap();
        assert_eq!(combo.vk, 0x4A);
        assert_eq!(combo.mods, MOD_CONTROL | MOD_ALT);
        assert_eq!(combo.name, "Ctrl+Alt+J");
    }

    #[test]
    fn parses_bare_function_key() {
        let hotkey = parse("F8").unwrap();
        assert_eq!(hotkey.vk, 0x77);
        assert_eq!(hotkey.mods, 0);
        assert_eq!(hotkey.name, "F8");
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse("Alt+NotAKey").is_err());
    }

    #[test]
    fn rejects_multiple_keys() {
        assert!(parse("Alt+X+Y").is_err());
    }

    #[test]
    fn rejects_missing_key_or_empty_segment() {
        assert!(parse("Ctrl+Alt").is_err());
        assert!(parse("Ctrl++X").is_err());
    }

    #[test]
    fn rejects_duplicate_modifier() {
        assert!(parse("Ctrl+Ctrl+X").is_err());
    }
}
