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
/// Registration is transactional: failed replacements restore the old bindings.
pub struct HotkeyManager {
    manager: Backend,
    registered: Vec<(HotKey, Command)>,
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
        })
    }
    pub fn apply(&mut self, config: &Config) -> Result<()> {
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
        for (key, _) in &self.registered {
            self.manager.unregister(*key)?;
        }
        let mut added = vec![];
        for (key, _) in &parsed {
            if let Err(error) = self.manager.register(*key) {
                for k in added {
                    let _ = self.manager.unregister(k);
                }
                for (k, _) in &self.registered {
                    if let Err(e) = self.manager.register(*k) {
                        tracing::error!("restoring hotkey failed: {e}");
                    }
                }
                anyhow::bail!(
                    "快捷键 {key:?} 注册失败：{error}。macOS 上请检查 Fn/Globe 和系统功能键设置，或改用其他组合键。"
                );
            }
            added.push(*key);
        }
        self.registered = parsed;
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
