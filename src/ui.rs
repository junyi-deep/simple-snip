use gpui_kit::component::{ActiveTheme, Theme, ThemeConfig, ThemeConfigColors, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{borrow::Cow, rc::Rc};

gpui_kit::assets::icon_assets!(
    EditingIcons,
    [
        ArrowUpRight,
        Circle,
        Droplet,
        Eraser,
        Grid2x2,
        Highlighter,
        ListOrdered,
        MousePointer2,
        Pencil,
        Pin,
        Save,
        ScanText,
        Slash,
        Type,
    ]
);

pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = EditingIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(EditingIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

pub const BODY: f32 = 13.;
pub const DETAIL: f32 = 12.;
pub const HEADING: f32 = 22.;
pub const SPACE: f32 = 8.;
pub const CONTROL_RADIUS: f32 = 6.;
pub const PANEL_RADIUS: f32 = 10.;
pub const LABEL_RADIUS: f32 = 4.;
pub const CAPTURE_SCRIM: u32 = 0x161c2880;
pub const IMAGE_FOCUS: u32 = 0x4797ff;
pub const IMAGE_HANDLE: u32 = 0xf4f8ff;
pub const IMAGE_LABEL: u32 = 0x1b2534ef;
pub const IMAGE_LABEL_TEXT: u32 = 0xf0f5ff;
pub const OCR_HOVER: u32 = 0x4797ff55;
pub const OCR_SELECTED: u32 = 0x4797ff99;

#[derive(Clone, Copy)]
struct Palette {
    background: &'static str,
    sidebar: &'static str,
    surface: &'static str,
    text: &'static str,
    detail: &'static str,
    border: &'static str,
    input: &'static str,
    hover: &'static str,
    selected: &'static str,
    primary: &'static str,
    on_primary: &'static str,
    primary_hover: &'static str,
    primary_active: &'static str,
    danger_text: u32,
}

const LIGHT: Palette = Palette {
    background: "#f8f9fc",
    sidebar: "#eef1f6",
    surface: "#fdfeff",
    text: "#202938",
    detail: "#505e71",
    border: "#d7dfea",
    input: "#8291a5",
    hover: "#e7edf6",
    selected: "#dfeafc",
    primary: "#275fc2",
    on_primary: "#fdfeff",
    primary_hover: "#2356ae",
    primary_active: "#1d4694",
    danger_text: 0x9e293d,
};

const DARK: Palette = Palette {
    background: "#191d24",
    sidebar: "#151920",
    surface: "#222832",
    text: "#eef2f8",
    detail: "#aab8cc",
    border: "#3b4657",
    input: "#718199",
    hover: "#2c3544",
    selected: "#2b4163",
    primary: "#91b7ff",
    on_primary: "#13233b",
    primary_hover: "#a5c4ff",
    primary_active: "#7ca7f5",
    danger_text: 0xff97a4,
};

impl Palette {
    fn theme(&self, mode: ThemeMode) -> Rc<ThemeConfig> {
        let color = |value: &'static str| Some(SharedString::from(value));
        let mut colors = ThemeConfigColors::default();
        macro_rules! assign {
            ($($key:ident: $value:expr),* $(,)?) => { $(colors.$key = $value;)* };
        }
        assign! {
            background: color(self.background),
            foreground: color(self.text),
            popover: color(self.surface),
            popover_foreground: color(self.text),
            muted: color(self.hover),
            muted_foreground: color(self.detail),
            sidebar: color(self.sidebar),
            sidebar_foreground: color(self.text),
            sidebar_border: color(self.border),
            sidebar_accent: color(self.selected),
            sidebar_accent_foreground: color(self.text),
            border: color(self.border),
            input: color(self.input),
            ring: color(self.primary),
            button: color(self.surface),
            button_foreground: color(self.text),
            button_hover: color(self.hover),
            button_active: color(self.selected),
            primary: color(self.primary),
            primary_foreground: color(self.on_primary),
            primary_hover: color(self.primary_hover),
            primary_active: color(self.primary_active),
            secondary: color(self.surface),
            secondary_foreground: color(self.text),
            secondary_hover: color(self.hover),
            secondary_active: color(self.selected),
            accent: color(self.hover),
            accent_foreground: color(self.text),
            list_hover: color(self.hover),
            list_active: color(self.selected),
            selection: color(self.selected),
            progress_bar: color(self.primary),
            link: color(self.primary),
            link_hover: color(self.primary_hover),
            link_active: color(self.primary_active),
            switch: color(self.input),
        }
        Rc::new(ThemeConfig {
            name: if mode.is_dark() {
                "Snip Dark"
            } else {
                "Snip Light"
            }
            .into(),
            mode,
            font_size: Some(BODY),
            radius: Some(CONTROL_RADIUS as usize),
            radius_lg: Some(PANEL_RADIUS as usize),
            shadow: Some(false),
            colors,
            ..Default::default()
        })
    }
}

pub fn install_theme(preference: &str, cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.light_theme = LIGHT.theme(ThemeMode::Light);
        theme.dark_theme = DARK.theme(ThemeMode::Dark);
    });
    apply_theme(preference, cx);
}

pub fn apply_theme(preference: &str, cx: &mut App) {
    let appearance = match preference {
        "light" => Some(WindowAppearance::Light),
        "dark" => Some(WindowAppearance::Dark),
        _ => None,
    };
    cx.set_window_appearance(appearance);
    Theme::change(
        appearance.unwrap_or_else(|| cx.window_appearance()),
        None,
        cx,
    );
}

pub fn watch_appearance(window: &Window) {
    window
        .observe_window_appearance(|_, cx| cx.defer(sync_system_theme))
        .detach();
}

pub fn sync_system_theme(cx: &mut App) {
    if cx
        .try_global::<crate::app::Runtime>()
        .is_some_and(|runtime| runtime.config.theme == "system")
    {
        let mode = ThemeMode::from(cx.window_appearance());
        if mode != cx.theme().mode {
            Theme::change(mode, None, cx);
        }
    }
}

pub fn danger_text(cx: &App) -> Hsla {
    let palette = if cx.theme().is_dark() { DARK } else { LIGHT };
    rgb(palette.danger_text).into()
}

pub fn panel(cx: &App) -> Div {
    div()
        .rounded(px(PANEL_RADIUS))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .text_size(px(BODY))
        .line_height(px(19.))
}

pub fn detail(cx: &App) -> Div {
    div()
        .text_size(px(DETAIL))
        .line_height(px(17.))
        .text_color(cx.theme().muted_foreground)
}

pub fn image_label() -> Div {
    div()
        .rounded(px(LABEL_RADIUS))
        .bg(rgba(IMAGE_LABEL))
        .text_color(rgb(IMAGE_LABEL_TEXT))
        .text_size(px(DETAIL))
        .line_height(px(17.))
}

pub fn divider(cx: &App) -> Div {
    div().w(px(1.)).h(px(16.)).mx_1().bg(cx.theme().border)
}

pub fn shortcut(windows: &str, macos: &str) -> String {
    if cfg!(target_os = "macos") {
        macos.into()
    } else {
        format!("Ctrl+{windows}")
    }
}

pub fn hotkey_label(label: &str, binding: Option<&String>) -> String {
    match binding.filter(|key| !key.trim().is_empty()) {
        Some(key) => format!("{label}  {key}"),
        None => label.into(),
    }
}
