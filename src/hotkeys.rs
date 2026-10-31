use crate::app::{Command, Event};
use anyhow::{Context, Result, ensure};
use global_hotkey::hotkey::HotKey;
#[cfg(target_os = "windows")]
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
#[cfg(target_os = "windows")]
type Backend = GlobalHotKeyManager;
#[cfg(target_os = "macos")]
struct Backend(snip_platform::NativeHotkeyManager);
#[cfg(target_os = "macos")]
impl Backend {
    fn register(&mut self, key: HotKey) -> Result<()> {
        use global_hotkey::hotkey::Modifiers as M;
        let code = mac_keycode(key.key)
            .context("macOS 不支持此快捷键按键，请选择 F1–F20、字母、数字或常见导航键")?;
        let mut modifiers = 0;
        for (flag, native) in [
            (M::SHIFT, 512),
            (M::META, 256),
            (M::ALT, 2048),
            (M::CONTROL, 4096),
        ] {
            if key.mods.contains(flag) {
                modifiers |= native;
            }
        }
        self.0.register(code, modifiers, key.id())
    }
    fn unregister(&mut self, key: HotKey) -> Result<()> {
        self.0.unregister(key.id())
    }
}
#[cfg(target_os = "macos")]
fn mac_keycode(key: global_hotkey::hotkey::Code) -> Option<u32> {
    use global_hotkey::hotkey::Code as C;
    // Apple Events.h virtual key constants, independent of input language.
    let letters = [
        C::KeyA,
        C::KeyS,
        C::KeyD,
        C::KeyF,
        C::KeyH,
        C::KeyG,
        C::KeyZ,
        C::KeyX,
        C::KeyC,
        C::KeyV,
    ];
    if let Some(i) = letters.iter().position(|k| *k == key) {
        return Some(i as u32);
    }
    let f = [
        122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90,
    ];
    let fs = [
        C::F1,
        C::F2,
        C::F3,
        C::F4,
        C::F5,
        C::F6,
        C::F7,
        C::F8,
        C::F9,
        C::F10,
        C::F11,
        C::F12,
        C::F13,
        C::F14,
        C::F15,
        C::F16,
        C::F17,
        C::F18,
        C::F19,
        C::F20,
    ];
    if let Some(i) = fs.iter().position(|k| *k == key) {
        return Some(f[i]);
    }
    Some(match key {
        C::KeyB => 11,
        C::KeyQ => 12,
        C::KeyW => 13,
        C::KeyE => 14,
        C::KeyR => 15,
        C::KeyY => 16,
        C::KeyT => 17,
        C::Digit1 => 18,
        C::Digit2 => 19,
        C::Digit3 => 20,
        C::Digit4 => 21,
        C::Digit6 => 22,
        C::Digit5 => 23,
        C::Digit9 => 25,
        C::Digit7 => 26,
        C::Digit8 => 28,
        C::Digit0 => 29,
        C::KeyO => 31,
        C::KeyU => 32,
        C::KeyI => 34,
        C::KeyP => 35,
        C::Enter => 36,
        C::KeyL => 37,
        C::KeyJ => 38,
        C::KeyK => 40,
        C::KeyN => 45,
        C::KeyM => 46,
        C::Tab => 48,
        C::Space => 49,
        C::Backspace => 51,
        C::Escape => 53,
        C::ArrowLeft => 123,
        C::ArrowRight => 124,
        C::ArrowDown => 125,
        C::ArrowUp => 126,
        C::Home => 115,
        C::End => 119,
        C::PageUp => 116,
        C::PageDown => 121,
        C::Delete => 117,
        _ => return None,
    })
}
use snip_core::config::Config;
use std::collections::HashMap;
pub fn validate(config: &Config) -> Result<()> {
    parse(config).map(|_| ())
}
fn parse(config: &Config) -> Result<Vec<(HotKey, Command)>> {
    let commands = HashMap::from([
        ("screenshot", Command::Capture),
        ("paste_image", Command::Paste),
        ("long_capture", Command::LongCapture),
        ("gif", Command::Gif),
        ("ocr", Command::Ocr),
        ("mouse_trail", Command::Trail),
        ("settings", Command::Settings),
    ]);
    let mut parsed = vec![];
    for (name, value) in &config.hotkeys {
        if value.is_empty() {
            continue;
        }
        let command = commands
            .get(name.as_str())
            .context(format!("未知快捷键动作: {name}"))?;
        let key = value
            .parse::<HotKey>()
            .with_context(|| format!("无法解析 {name} 的快捷键 {value}"))?;
        ensure!(
            !parsed.iter().any(|(k, _)| *k == key),
            "快捷键冲突: {value}"
        );
        parsed.push((key, *command));
    }
    Ok(parsed)
}

/// Retain successful registrations and retry only missing bindings.
pub struct HotkeyManager {
    manager: Backend,
    registered: Vec<(HotKey, Command)>,
    last_error: Option<String>,
}
impl HotkeyManager {
    pub fn new(tx: async_channel::Sender<Event>) -> Result<Self> {
        #[cfg(target_os = "windows")]
        let manager = {
            let manager = GlobalHotKeyManager::new()?;
            GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
                if e.state == HotKeyState::Pressed {
                    let _ = tx.try_send(Event::Hotkey(e.id));
                }
            }));
            manager
        };
        #[cfg(target_os = "macos")]
        let manager = Backend(snip_platform::NativeHotkeyManager::new(move |id| {
            let _ = tx.try_send(Event::Hotkey(id));
        })?);
        Ok(Self {
            manager,
            registered: vec![],
            last_error: None,
        })
    }
    pub fn apply(&mut self, config: &Config) -> Result<()> {
        let result = self.reconcile(config);
        self.last_error = result.as_ref().err().map(ToString::to_string);
        result
    }
    pub fn registration_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    fn reconcile(&mut self, config: &Config) -> Result<()> {
        let parsed = parse(config)?;
        let removed: Vec<_> = self
            .registered
            .iter()
            .filter(|(key, _)| !parsed.iter().any(|(wanted, _)| wanted == key))
            .map(|(key, _)| *key)
            .collect();
        for key in removed {
            self.manager.unregister(key)?;
            self.registered.retain(|(k, _)| *k != key);
        }
        let mut failures = vec![];
        for (key, command) in parsed {
            if let Some((_, registered_command)) =
                self.registered.iter_mut().find(|(k, _)| *k == key)
            {
                *registered_command = command;
                continue;
            }
            match self.manager.register(key) {
                Ok(()) => self.registered.push((key, command)),
                Err(error) => {
                    let label = config
                        .hotkeys
                        .iter()
                        .find(|(_, v)| v.parse::<HotKey>().ok() == Some(key))
                        .map(|(name, value)| format!("{name}: {value}"))
                        .unwrap_or_else(|| format!("{key:?}"));
                    failures.push(format!("{label} 注册失败：{error}"));
                }
            }
        }
        ensure!(
            failures.is_empty(),
            "{}。每 5 秒自动重试；系统未提供占用应用信息，可关闭冲突应用或更换快捷键。",
            failures.join("；")
        );
        Ok(())
    }
    pub fn command(&self, id: u32) -> Option<Command> {
        self.registered
            .iter()
            .find(|(k, _)| k.id() == id)
            .map(|(_, c)| *c)
    }
}
impl Drop for HotkeyManager {
    fn drop(&mut self) {
        for (key, _) in &self.registered {
            let _ = self.manager.unregister(*key);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn apple_function_and_custom_key_mapping() {
        use global_hotkey::hotkey::{Code, HotKey, Modifiers};
        assert_eq!(super::mac_keycode(Code::F1), Some(122));
        assert_eq!(super::mac_keycode(Code::F3), Some(99));
        let custom = "Command+Shift+S".parse::<HotKey>().unwrap();
        assert!(custom.mods.contains(Modifiers::META | Modifiers::SHIFT));
        assert_eq!(super::mac_keycode(custom.key), Some(1));
        assert_eq!(super::mac_keycode(Code::MediaStop), None);
    }
}

#[cfg(all(test, target_os = "windows"))]
mod registration_tests {
    use super::*;
    #[test]
    fn retries_conflicts_without_discarding_working_bindings() {
        let owner = GlobalHotKeyManager::new().unwrap();
        let blocked = "Control+Alt+Shift+F24".parse::<HotKey>().unwrap();
        owner.register(blocked).unwrap();
        let (tx, _) = async_channel::bounded(8);
        let mut keys = HotkeyManager::new(tx).unwrap();
        let mut config = Config::default();
        config.hotkeys.clear();
        config
            .hotkeys
            .insert("screenshot".into(), "Control+Alt+Shift+F24".into());
        config
            .hotkeys
            .insert("paste_image".into(), "Control+Alt+Shift+F23".into());
        let error = keys.apply(&config).unwrap_err().to_string();
        assert!(error.contains("Control+Alt+Shift+F24"));
        assert!(keys.registration_error().is_some());
        let working = "Control+Alt+Shift+F23".parse::<HotKey>().unwrap();
        assert!(matches!(keys.command(working.id()), Some(Command::Paste)));
        owner.unregister(blocked).unwrap();
        keys.apply(&config).unwrap();
        assert!(keys.registration_error().is_none());
        assert!(matches!(keys.command(blocked.id()), Some(Command::Capture)));
        config.hotkeys.remove("paste_image");
        keys.apply(&config).unwrap();
        assert!(keys.command(working.id()).is_none());
    }
}
