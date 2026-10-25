use crate::app::{Command, Event};
use anyhow::{Context, Result, ensure};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use snip_core::config::Config;
use std::collections::HashMap;
/// Registration is transactional: failed replacements restore the old bindings.
pub struct HotkeyManager {
    manager: GlobalHotKeyManager,
    registered: Vec<(HotKey, Command)>,
}
impl HotkeyManager {
    pub fn new(tx: async_channel::Sender<Event>) -> Result<Self> {
        let manager = GlobalHotKeyManager::new()?;
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                let _ = tx.try_send(Event::Hotkey(e.id));
            }
        }));
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
