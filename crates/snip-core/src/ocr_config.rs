use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const PRESETS: [&str; 3] = ["快速", "均衡", "精确"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OcrConfig {
    pub profiles: Vec<OcrProfile>,
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrProfile {
    pub name: String,
    #[serde(default)]
    pub rapid: RapidOptions,
    #[serde(default)]
    pub vision: VisionOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RapidOptions {
    pub max_side: u32,
    pub threads: usize,
    pub orientation: bool,
    pub detection_threshold: f32,
    pub box_score: f32,
    pub text_score: f32,
    pub unclip_ratio: f32,
}

impl Default for RapidOptions {
    fn default() -> Self {
        Self {
            max_side: 1600,
            threads: 4,
            orientation: true,
            detection_threshold: 0.3,
            box_score: 0.5,
            text_score: 0.5,
            unclip_ratio: 1.6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisionOptions {
    pub accurate: bool,
    pub correction: bool,
    pub auto_language: bool,
    pub languages: String,
    pub min_height: f32,
    pub custom_words: String,
}

impl Default for VisionOptions {
    fn default() -> Self {
        Self {
            accurate: true,
            correction: true,
            auto_language: false,
            languages: "zh-Hans, zh-Hant, en-US".into(),
            min_height: 0.,
            custom_words: String::new(),
        }
    }
}

impl OcrConfig {
    pub fn preset(index: usize) -> OcrProfile {
        let mut rapid = RapidOptions::default();
        let mut vision = VisionOptions::default();
        match index {
            0 => {
                rapid.max_side = 960;
                rapid.orientation = false;
                vision.accurate = false;
                vision.correction = false;
                vision.min_height = 0.015;
            }
            2 => {
                rapid.max_side = 2560;
                rapid.box_score = 0.4;
                vision.auto_language = true;
            }
            _ => {}
        }
        OcrProfile {
            name: PRESETS[index].into(),
            rapid,
            vision,
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            (3..=16).contains(&self.profiles.len()) && self.active < self.profiles.len(),
            "OCR 模式配置无效"
        );
        for p in &self.profiles {
            ensure!(
                !p.name.trim().is_empty() && p.name.chars().count() <= 32,
                "OCR 模式名称须为 1–32 个字符"
            );
            p.rapid.validate()?;
            p.vision.validate()?;
        }
        Ok(())
    }
}

impl VisionOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (0. ..=0.25).contains(&self.min_height)
                && self.languages.len() <= 256
                && self.custom_words.len() <= 2048,
            "Vision 参数超出范围"
        );
        Ok(())
    }
}

impl RapidOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (320..=3200).contains(&self.max_side) && (1..=16).contains(&self.threads),
            "RapidOCR 边长须为 320–3200，线程数须为 1–16"
        );
        ensure!(
            [self.detection_threshold, self.box_score, self.text_score]
                .into_iter()
                .all(|n| (0.01..=0.99).contains(&n)),
            "RapidOCR 阈值须为 0.01–0.99"
        );
        ensure!(
            (1. ..=4.).contains(&self.unclip_ratio),
            "RapidOCR 扩框系数须为 1–4"
        );
        Ok(())
    }
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            profiles: (0..PRESETS.len()).map(Self::preset).collect(),
            active: 1,
        }
    }
}
