use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

pub struct HotkeyManager {
    #[allow(dead_code)]
    manager: GlobalHotKeyManager,
    push_to_talk_id: u32,
    always_listen_id: u32,
    push_to_talk_display: String,
    always_listen_display: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    PushToTalkPressed,
    PushToTalkReleased,
    AlwaysListenToggle,
}

impl HotkeyManager {
    pub fn from_config(push_to_talk_str: &str, always_listen_str: &str) -> Result<Self> {
        let manager = GlobalHotKeyManager::new()
            .map_err(|e| anyhow::anyhow!("Failed to create hotkey manager: {}", e))?;

        // Parse push-to-talk hotkey
        let push_to_talk = parse_hotkey(push_to_talk_str)?;
        let push_to_talk_id = push_to_talk.id();

        // Parse always-listen hotkey
        let always_listen = parse_hotkey(always_listen_str)?;
        let always_listen_id = always_listen.id();

        manager
            .register(push_to_talk)
            .map_err(|e| anyhow::anyhow!("Failed to register push-to-talk hotkey: {}", e))?;

        manager
            .register(always_listen)
            .map_err(|e| anyhow::anyhow!("Failed to register always-listen hotkey: {}", e))?;

        let push_to_talk_display = format_hotkey_display(push_to_talk_str);
        let always_listen_display = format_hotkey_display(always_listen_str);

        println!("Hotkeys registered:");
        println!("  {} - Push-to-talk toggle", push_to_talk_display);
        println!("  {} - Always-listening mode toggle", always_listen_display);

        Ok(Self {
            manager,
            push_to_talk_id,
            always_listen_id,
            push_to_talk_display,
            always_listen_display,
        })
    }

    pub fn push_to_talk_id(&self) -> u32 {
        self.push_to_talk_id
    }

    pub fn always_listen_id(&self) -> u32 {
        self.always_listen_id
    }

    #[allow(dead_code)]
    pub fn push_to_talk_display(&self) -> &str {
        &self.push_to_talk_display
    }

    #[allow(dead_code)]
    pub fn always_listen_display(&self) -> &str {
        &self.always_listen_display
    }

    pub fn receiver() -> crossbeam_channel::Receiver<GlobalHotKeyEvent> {
        GlobalHotKeyEvent::receiver().clone()
    }
}

/// Parse a hotkey string like "Control+Backquote" or "F2" into a HotKey
fn parse_hotkey(s: &str) -> Result<HotKey> {
    let parts: Vec<&str> = s.split('+').collect();

    let mut modifiers = Modifiers::empty();
    let mut key_code: Option<Code> = None;

    for part in parts {
        let part = part.trim();
        match part.to_lowercase().as_str() {
            "control" | "ctrl" => modifiers |= Modifiers::CONTROL,
            "alt" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "super" | "win" | "meta" => modifiers |= Modifiers::SUPER,
            _ => {
                // This should be the key code
                key_code = Some(parse_key_code(part)?);
            }
        }
    }

    let code = key_code.ok_or_else(|| anyhow::anyhow!("No key code found in hotkey string: {}", s))?;

    let mods = if modifiers.is_empty() { None } else { Some(modifiers) };
    Ok(HotKey::new(mods, code))
}

/// Parse a key name to a Code
fn parse_key_code(s: &str) -> Result<Code> {
    let code = match s {
        "Backquote" | "`" => Code::Backquote,
        "Digit1" | "1" => Code::Digit1,
        "Digit2" | "2" => Code::Digit2,
        "Digit3" | "3" => Code::Digit3,
        "Digit4" | "4" => Code::Digit4,
        "Digit5" | "5" => Code::Digit5,
        "Digit6" | "6" => Code::Digit6,
        "Digit7" | "7" => Code::Digit7,
        "Digit8" | "8" => Code::Digit8,
        "Digit9" | "9" => Code::Digit9,
        "Digit0" | "0" => Code::Digit0,
        "KeyA" | "A" => Code::KeyA,
        "KeyB" | "B" => Code::KeyB,
        "KeyC" | "C" => Code::KeyC,
        "KeyD" | "D" => Code::KeyD,
        "KeyE" | "E" => Code::KeyE,
        "KeyF" | "F" => Code::KeyF,
        "KeyG" | "G" => Code::KeyG,
        "KeyH" | "H" => Code::KeyH,
        "KeyI" | "I" => Code::KeyI,
        "KeyJ" | "J" => Code::KeyJ,
        "KeyK" | "K" => Code::KeyK,
        "KeyL" | "L" => Code::KeyL,
        "KeyM" | "M" => Code::KeyM,
        "KeyN" | "N" => Code::KeyN,
        "KeyO" | "O" => Code::KeyO,
        "KeyP" | "P" => Code::KeyP,
        "KeyQ" | "Q" => Code::KeyQ,
        "KeyR" | "R" => Code::KeyR,
        "KeyS" | "S" => Code::KeyS,
        "KeyT" | "T" => Code::KeyT,
        "KeyU" | "U" => Code::KeyU,
        "KeyV" | "V" => Code::KeyV,
        "KeyW" | "W" => Code::KeyW,
        "KeyX" | "X" => Code::KeyX,
        "KeyY" | "Y" => Code::KeyY,
        "KeyZ" | "Z" => Code::KeyZ,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "Space" => Code::Space,
        "Tab" => Code::Tab,
        "CapsLock" => Code::CapsLock,
        "Escape" | "Esc" => Code::Escape,
        "Insert" => Code::Insert,
        "Delete" | "Del" => Code::Delete,
        "Home" => Code::Home,
        "End" => Code::End,
        "PageUp" => Code::PageUp,
        "PageDown" => Code::PageDown,
        "ArrowUp" | "Up" => Code::ArrowUp,
        "ArrowDown" | "Down" => Code::ArrowDown,
        "ArrowLeft" | "Left" => Code::ArrowLeft,
        "ArrowRight" | "Right" => Code::ArrowRight,
        "Numpad0" => Code::Numpad0,
        "Numpad1" => Code::Numpad1,
        "Numpad2" => Code::Numpad2,
        "Numpad3" => Code::Numpad3,
        "Numpad4" => Code::Numpad4,
        "Numpad5" => Code::Numpad5,
        "Numpad6" => Code::Numpad6,
        "Numpad7" => Code::Numpad7,
        "Numpad8" => Code::Numpad8,
        "Numpad9" => Code::Numpad9,
        "NumpadAdd" => Code::NumpadAdd,
        "NumpadSubtract" => Code::NumpadSubtract,
        "NumpadMultiply" => Code::NumpadMultiply,
        "NumpadDivide" => Code::NumpadDivide,
        "NumpadEnter" => Code::NumpadEnter,
        "NumpadDecimal" => Code::NumpadDecimal,
        _ => return Err(anyhow::anyhow!("Unknown key code: {}", s)),
    };
    Ok(code)
}

/// Format hotkey for display (more user-friendly)
fn format_hotkey_display(s: &str) -> String {
    s.replace("Control", "Ctrl")
        .replace("Backquote", "`")
        .replace("Key", "")
        .replace("Digit", "")
}

/// Check hotkey event given the IDs
/// Push-to-talk: responds to both press and release
/// Always-listen: only responds to press (toggle)
pub fn check_hotkey_event(
    event: &GlobalHotKeyEvent,
    push_to_talk_id: u32,
    always_listen_id: u32,
) -> Option<HotkeyAction> {
    if event.id == push_to_talk_id {
        match event.state {
            HotKeyState::Pressed => Some(HotkeyAction::PushToTalkPressed),
            HotKeyState::Released => Some(HotkeyAction::PushToTalkReleased),
        }
    } else if event.id == always_listen_id {
        // Only toggle on press, ignore release
        if event.state == HotKeyState::Pressed {
            Some(HotkeyAction::AlwaysListenToggle)
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hotkey_simple() {
        let hotkey = parse_hotkey("Backquote").unwrap();
        // Just verify it parses successfully and has no modifiers
        assert!(hotkey.mods.is_empty());
    }

    #[test]
    fn test_parse_hotkey_with_modifier() {
        let hotkey = parse_hotkey("Control+Backquote").unwrap();
        assert!(hotkey.mods.contains(Modifiers::CONTROL));
    }

    #[test]
    fn test_parse_hotkey_multiple_modifiers() {
        let hotkey = parse_hotkey("Control+Shift+F1").unwrap();
        assert!(hotkey.mods.contains(Modifiers::CONTROL));
        assert!(hotkey.mods.contains(Modifiers::SHIFT));
    }

    #[test]
    fn test_parse_hotkey_alt_modifier() {
        let hotkey = parse_hotkey("Alt+Space").unwrap();
        assert!(hotkey.mods.contains(Modifiers::ALT));
    }

    #[test]
    fn test_parse_key_code_variations() {
        // Test both formats work
        assert_eq!(parse_key_code("Backquote").unwrap(), Code::Backquote);
        assert_eq!(parse_key_code("`").unwrap(), Code::Backquote);
        
        assert_eq!(parse_key_code("KeyA").unwrap(), Code::KeyA);
        assert_eq!(parse_key_code("A").unwrap(), Code::KeyA);
        
        assert_eq!(parse_key_code("Digit1").unwrap(), Code::Digit1);
        assert_eq!(parse_key_code("1").unwrap(), Code::Digit1);
    }

    #[test]
    fn test_parse_key_code_function_keys() {
        assert_eq!(parse_key_code("F1").unwrap(), Code::F1);
        assert_eq!(parse_key_code("F12").unwrap(), Code::F12);
    }

    #[test]
    fn test_parse_key_code_special_keys() {
        assert_eq!(parse_key_code("Space").unwrap(), Code::Space);
        assert_eq!(parse_key_code("Escape").unwrap(), Code::Escape);
        assert_eq!(parse_key_code("Esc").unwrap(), Code::Escape);
        assert_eq!(parse_key_code("Tab").unwrap(), Code::Tab);
        assert_eq!(parse_key_code("Delete").unwrap(), Code::Delete);
    }

    #[test]
    fn test_parse_key_code_arrows() {
        assert_eq!(parse_key_code("Up").unwrap(), Code::ArrowUp);
        assert_eq!(parse_key_code("ArrowUp").unwrap(), Code::ArrowUp);
        assert_eq!(parse_key_code("Down").unwrap(), Code::ArrowDown);
        assert_eq!(parse_key_code("Left").unwrap(), Code::ArrowLeft);
        assert_eq!(parse_key_code("Right").unwrap(), Code::ArrowRight);
    }

    #[test]
    fn test_parse_key_code_unknown() {
        assert!(parse_key_code("UnknownKey").is_err());
        assert!(parse_key_code("").is_err());
    }

    #[test]
    fn test_format_hotkey_display() {
        assert_eq!(format_hotkey_display("Control+Backquote"), "Ctrl+`");
        assert_eq!(format_hotkey_display("KeyA"), "A");
        assert_eq!(format_hotkey_display("Digit1"), "1");
        assert_eq!(format_hotkey_display("Control+Shift+F1"), "Ctrl+Shift+F1");
    }

    #[test]
    fn test_hotkey_action_equality() {
        assert_eq!(HotkeyAction::PushToTalk, HotkeyAction::PushToTalk);
        assert_eq!(HotkeyAction::AlwaysListen, HotkeyAction::AlwaysListen);
        assert_ne!(HotkeyAction::PushToTalk, HotkeyAction::AlwaysListen);
    }
}
