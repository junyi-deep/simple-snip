use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub show_settings_on_start: bool,
    pub theme: String,
    pub save_directory: PathBuf,
    pub format: String,
    pub history_limit: usize,
    pub hotkeys: BTreeMap<String, String>,
    pub gif: GifConfig,
    pub trail: TrailConfig,
    pub updates: UpdateConfig,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    System,
    Direct,
    Http,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    pub proxy_mode: ProxyMode,
    pub proxy_url: String,
    pub proxy_username: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GifConfig {
    pub fps: u32,
    pub max_seconds: u32,
    pub max_width: u32,
    pub countdown: u32,
    pub repeat: u16,
}
impl Default for GifConfig {
    fn default() -> Self {
        Self {
            fps: 15,
            max_seconds: 60,
            max_width: 1280,
            countdown: 3,
            repeat: 0,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrailConfig {
    pub enabled: bool,
    pub width: f32,
    pub lifetime_ms: u32,
    pub fps: u32,
}
impl Default for TrailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 9.,
            lifetime_ms: 300,
            fps: 60,
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            show_settings_on_start: true,
            theme: "system".into(),
            save_directory: dirs::picture_dir()
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
                .join("Simple Snip"),
            format: "png".into(),
            history_limit: 100,
            hotkeys: BTreeMap::from([
                ("screenshot".into(), "F1".into()),
                ("paste_image".into(), "F3".into()),
            ]),
            gif: GifConfig::default(),
            trail: TrailConfig::default(),
            updates: UpdateConfig::default(),
        }
    }
}
impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .expect("user configuration directory")
            .join("Simple Snip")
            .join("config.toml")
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "不支持此配置版本 {}", self.version);
        ensure!(
            [5, 10, 15, 20, 30].contains(&self.gif.fps),
            "GIF 帧率必须为 5/10/15/20/30"
        );
        ensure!(
            (1..=600).contains(&self.gif.max_seconds),
            "录制时长应为 1–600 秒"
        );
        ensure!(
            (64..=4096).contains(&self.gif.max_width),
            "GIF 宽度应为 64–4096"
        );
        ensure!(self.gif.countdown <= 10, "倒计时不能超过 10 秒");
        ensure!(self.history_limit <= 1000, "历史数量不能超过 1000");
        ensure!(
            ["png", "jpg"].contains(&self.format.as_str()),
            "图片格式须为 png 或 jpg"
        );
        ensure!(
            ["system", "light", "dark"].contains(&self.theme.as_str()),
            "未知主题"
        );
        ensure!(
            self.trail.width.is_finite() && (1.0..=60.0).contains(&self.trail.width),
            "拖尾宽度无效"
        );
        ensure!(
            (100..=3000).contains(&self.trail.lifetime_ms) && (15..=120).contains(&self.trail.fps),
            "拖尾生命周期或帧率无效"
        );
        let mut seen = std::collections::HashSet::new();
        for binding in self.hotkeys.values().filter(|s| !s.is_empty()) {
            ensure!(seen.insert(binding.to_lowercase()), "快捷键重复: {binding}");
        }
        Ok(())
    }
    /// A corrupt file is preserved, never silently overwritten.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        if !path.exists() {
            return (Self::default(), None);
        }
        let result = (|| -> Result<Self> {
            let s = fs::read_to_string(path)?;
            let mut c: Self = toml::from_str(&s)?;
            // v0 used the same hotkey table; new fields acquire serde defaults.
            if c.version == 0 {
                c.version = 1;
            }
            c.validate()?;
            Ok(c)
        })();
        match result {
            Ok(c) => (c, None),
            Err(e) => (
                Self::default(),
                Some(format!(
                    "配置加载失败，已使用默认值。原文件保留于 {}：{e}",
                    path.display()
                )),
            ),
        }
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        atomic_write(path, toml::to_string_pretty(self)?.as_bytes())
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("文件需要父目录")?;
    fs::create_dir_all(parent)?;
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    f.write_all(bytes)?;
    f.as_file().sync_all()?;
    f.persist(path).map_err(|e| e.error)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_only_two_global_keys() {
        let c = Config::default();
        assert_eq!(c.hotkeys.len(), 2);
        assert_eq!(c.hotkeys["screenshot"], "F1");
        assert_eq!(c.hotkeys["paste_image"], "F3");
    }
    #[test]
    fn atomic_roundtrip_and_recovery() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        Config::default().save(&p).unwrap();
        assert!(Config::load(&p).1.is_none());
        fs::write(&p, "bad [").unwrap();
        assert!(Config::load(&p).1.is_some());
        assert_eq!(fs::read_to_string(&p).unwrap(), "bad [");
    }
    #[test]
    fn conflict_and_limits() {
        let mut c = Config::default();
        c.hotkeys.insert("ocr".into(), "f1".into());
        assert!(c.validate().is_err());
        c.hotkeys.remove("ocr");
        c.gif.fps = 120;
        assert!(c.validate().is_err());
    }
}
