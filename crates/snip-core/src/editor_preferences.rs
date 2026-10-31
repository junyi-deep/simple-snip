use crate::annotation::{ArrowStyle, Tool};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolStyle {
    pub width: f32,
    pub color: [u8; 4],
    pub arrow: ArrowStyle,
}
impl Default for ToolStyle {
    fn default() -> Self {
        Self {
            width: 3.,
            color: [240, 60, 70, 255],
            arrow: ArrowStyle::default(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPreferences {
    pub tool: Tool,
    pub shape: Tool,
    pub line: Tool,
    pub next_number: u32,
    pub toast_enabled: bool,
    pub toast_delay_ms: u32,
    #[serde(default)]
    pub arrow_style_version: u32,
    pub styles: BTreeMap<Tool, ToolStyle>,
}
impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            shape: Tool::Rectangle,
            line: Tool::Line,
            next_number: 1,
            toast_enabled: true,
            toast_delay_ms: 2000,
            arrow_style_version: 1,
            styles: BTreeMap::new(),
        }
    }
}
impl EditorPreferences {
    pub fn style(&self, tool: Tool) -> ToolStyle {
        self.styles.get(&tool).cloned().unwrap_or_default()
    }
    pub fn load(path: &Path) -> Self {
        let mut prefs: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        if !prefs.shape.is_shape() {
            prefs.shape = Tool::Rectangle;
        }
        if !prefs.line.is_line() {
            prefs.line = Tool::Line;
        }
        if prefs.arrow_style_version == 0 {
            if let Some(style) = prefs.styles.get_mut(&Tool::Arrow)
                && style.arrow == ArrowStyle::Uniform
            {
                style.arrow = ArrowStyle::Swallow;
            }
            prefs.arrow_style_version = 1;
        }
        prefs.toast_delay_ms = prefs.toast_delay_ms.clamp(500, 10000);
        prefs.next_number = prefs.next_number.max(1);
        prefs.styles.retain(|tool, style| {
            let (min, max) = if matches!(tool, Tool::Text | Tool::Number) {
                (1. / 6., 500. / 6.)
            } else {
                (0.5, 128.)
            };
            style.width.is_finite() && (min..=max).contains(&style.width)
        });
        prefs
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        crate::config::atomic_write(path, toml::to_string(self)?.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_remembers_separate_tools_and_recovers_invalid_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("editor.toml");
        let mut prefs = EditorPreferences {
            tool: Tool::Diamond,
            shape: Tool::Diamond,
            line: Tool::Curve,
            next_number: 7,
            toast_enabled: false,
            toast_delay_ms: 5000,
            ..Default::default()
        };
        prefs.styles.insert(
            Tool::Diamond,
            ToolStyle {
                width: 9.,
                ..Default::default()
            },
        );
        prefs.styles.insert(
            Tool::Text,
            ToolStyle {
                width: 8.,
                ..Default::default()
            },
        );
        prefs.styles.insert(
            Tool::Arrow,
            ToolStyle {
                width: 11.,
                arrow: ArrowStyle::DiamondHollow,
                ..Default::default()
            },
        );
        prefs.save(&path).unwrap();
        assert_eq!(EditorPreferences::load(&path), prefs);
        prefs.styles.get_mut(&Tool::Text).unwrap().width = f32::NAN;
        prefs.save(&path).unwrap();
        assert_eq!(
            EditorPreferences::load(&path).style(Tool::Text),
            ToolStyle::default()
        );
        std::fs::write(&path, "broken = [").unwrap();
        assert_eq!(EditorPreferences::load(&path), EditorPreferences::default());
    }
}
