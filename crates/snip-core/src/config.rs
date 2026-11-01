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
    pub pin: PinConfig,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PinConfig {
    pub border_color: String,
    pub border_width: f32,
    pub shadow: bool,
}
impl Default for PinConfig {
    fn default() -> Self {
        Self {
            border_color: "#1688FF".into(),
            border_width: 1.,
            shadow: true,
        }
    }
}
impl PinConfig {
    pub fn color(&self) -> Result<u32> {
        let text = self
            .border_color
            .strip_prefix('#')
            .context("边框颜色须为 #RRGGBB")?;
        ensure!(text.len() == 6, "边框颜色须为 #RRGGBB");
        u32::from_str_radix(text, 16).context("边框颜色无效")
    }
    pub fn margin(&self) -> f32 {
        if self.shadow { 12. } else { 2. }
    }
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
    pub animation: TrailAnimation,
    pub color_mode: TrailColorMode,
    pub primary_color: String,
    pub secondary_color: String,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrailAnimation {
    #[default]
    Ribbon,
    Comet,
    Particles,
    Ripple,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrailColorMode {
    #[default]
    Rainbow,
    Solid,
    Gradient,
}
impl Default for TrailConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 9.,
            lifetime_ms: 300,
            fps: 60,
            animation: TrailAnimation::Ribbon,
            color_mode: TrailColorMode::Rainbow,
            primary_color: "#FF5A8A".into(),
            secondary_color: "#38BDF8".into(),
        }
    }
}
impl TrailConfig {
    pub fn parse_color(value: &str) -> Result<[u8; 4]> {
        let text = value
            .strip_prefix('#')
            .context("拖尾颜色须为 #RRGGBB 或 #RRGGBBAA")?;
        ensure!(
            [6, 8].contains(&text.len()),
            "拖尾颜色须为 #RRGGBB 或 #RRGGBBAA"
        );
        let color = u32::from_str_radix(text, 16).context("拖尾颜色无效")?;
        if text.len() == 8 {
            Ok(color.to_be_bytes())
        } else {
            Ok([(color >> 16) as u8, (color >> 8) as u8, color as u8, 255])
        }
    }
    pub fn color_at(&self, phase: f32) -> [u8; 4] {
        let first = || Self::parse_color(&self.primary_color).unwrap_or([255, 90, 138, 255]);
        match self.color_mode {
            TrailColorMode::Rainbow => {
                let [r, g, b] = crate::trail::rainbow(phase);
                [r, g, b, 255]
            }
            TrailColorMode::Solid => first(),
            TrailColorMode::Gradient => {
                let first = first();
                let second =
                    Self::parse_color(&self.secondary_color).unwrap_or([56, 189, 248, 255]);
                let t = phase.clamp(0., 1.);
                std::array::from_fn(|i| {
                    (first[i] as f32 + (second[i] as f32 - first[i] as f32) * t).round() as u8
                })
            }
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
            pin: PinConfig::default(),
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
        self.pin.color()?;
        ensure!(
            self.pin.border_width.is_finite() && (0.0..=8.0).contains(&self.pin.border_width),
            "贴图边框粗细应为 0–8 像素"
        );
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
        TrailConfig::parse_color(&self.trail.primary_color)?;
        TrailConfig::parse_color(&self.trail.secondary_color)?;
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
    fn trail_styles_and_rgba_palettes_roundtrip_without_changing_legacy_defaults() {
        let mut c: Config =
            toml::from_str("version=1\n[trail]\nwidth=9\nlifetime_ms=300\n").unwrap();
        assert_eq!(c.trail.animation, TrailAnimation::Ribbon);
        assert_eq!(c.trail.color_mode, TrailColorMode::Rainbow);
        c.trail.animation = TrailAnimation::Particles;
        c.trail.color_mode = TrailColorMode::Gradient;
        c.trail.primary_color = "#FF000080".into();
        c.trail.secondary_color = "#0000FF40".into();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        c.save(&path).unwrap();
        let (restored, warning) = Config::load(&path);
        assert!(warning.is_none());
        assert_eq!(restored.trail.animation, TrailAnimation::Particles);
        assert_eq!(restored.trail.color_at(0.), [255, 0, 0, 128]);
        assert_eq!(restored.trail.color_at(0.5), [128, 0, 128, 96]);
        assert_eq!(restored.trail.color_at(0.25), [191, 0, 64, 112]);
        assert_eq!(restored.trail.color_at(1.), [0, 0, 255, 64]);
        c.trail.color_mode = TrailColorMode::Solid;
        assert_eq!(c.trail.color_at(0.), c.trail.color_at(0.75));
        c.trail.primary_color = "#oops".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn defaults_only_two_global_keys() {
        let c = Config::default();
        assert!(
            c.trail.enabled,
            "new configurations start with rainbow trail enabled"
        );
        assert_eq!(c.hotkeys.len(), 2);
        assert_eq!(c.hotkeys["screenshot"], "F1");
        assert_eq!(c.hotkeys["paste_image"], "F3");
    }
    #[test]
    fn pin_frame_settings_validate_and_old_config_gets_defaults() {
        let old: Config =
            toml::from_str("version = 1\n[hotkeys]\nscreenshot = 'F1'\npaste_image = 'F3'\n")
                .unwrap();
        assert_eq!(old.pin.color().unwrap(), 0x1688ff);
        assert_eq!(old.pin.border_width, 1.);
        let mut config = old;
        config.pin.border_color = "#bad-color".into();
        assert!(config.validate().is_err());
        config.pin.border_color = "#ff8800".into();
        config.pin.border_width = 9.;
        assert!(config.validate().is_err());
        config.pin.border_width = 0.;
        assert!(config.validate().is_ok());
    }
    #[test]
    fn atomic_roundtrip_and_recovery() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        let mut config = Config::default();
        config.trail.enabled = false;
        config.save(&p).unwrap();
        let (loaded, warning) = Config::load(&p);
        assert!(warning.is_none());
        assert!(
            !loaded.trail.enabled,
            "an explicit saved opt-out must survive the new default"
        );
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
