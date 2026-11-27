use crate::app::{self, Command, Runtime};
use anyhow::Context as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    component::{
        button::*,
        color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
        input::{Input, InputEvent, InputState},
        notification::{Notification, NotificationDelivery},
        scroll::ScrollableElement,
        switch::Switch,
        *,
    },
    *,
};
use snip_core::config::{
    Config, ProxyMode, TrailAnimation, TrailColorMode, TrailConfig, UpdateConfig,
};

fn picker_hex(picker: &Entity<ColorPickerState>, cx: &App) -> String {
    let color = picker
        .read(cx)
        .value()
        .unwrap_or(rgb(0xff5a8a).into())
        .to_rgb();
    let rgb = format!(
        "#{:02X}{:02X}{:02X}",
        (color.r * 255.).round() as u8,
        (color.g * 255.).round() as u8,
        (color.b * 255.).round() as u8
    );
    if color.a < 0.999 {
        format!("{rgb}{:02X}", (color.a * 255.).round() as u8)
    } else {
        rgb
    }
}

#[derive(Clone, Copy)]
enum OcrModeAction {
    Save,
    Default,
    Reset,
    Duplicate,
}

enum DownloadEvent {
    Progress(u64, u64),
    Installing,
    Done(anyhow::Result<()>),
}
// Apply only fields actually edited in this window. Saving a dialog elsewhere
// (or toggling the tray menu) must not be overwritten by an older settings form.
fn merge_settings(mut current: Config, baseline: &Config, draft: &Config) -> Config {
    macro_rules! merge {
        ($($($field:ident).+),+ $(,)?) => { $(
            if draft.$($field).+ != baseline.$($field).+ {
                current.$($field).+ = draft.$($field).+.clone();
            }
        )+ };
    }
    merge!(
        show_settings_on_start,
        theme,
        format,
        save_directory,
        hotkeys,
        gif.fps,
        gif.max_seconds,
        gif.max_width,
        trail.width,
        trail.show_in_capture,
        trail.lifetime_ms,
        trail.animation,
        trail.color_mode,
        trail.primary_color,
        trail.secondary_color,
        pin.border_color,
        pin.border_width,
        pin.shadow,
        updates.auto_check,
        updates.proxy_mode,
        updates.proxy_url,
        updates.proxy_username
    );
    current
}
pub struct Settings {
    section: usize,
    baseline: Config,
    config_path: std::path::PathBuf,
    show_on_start: bool,
    gif_fps: u32,
    theme: String,
    format: String,
    save_status: String,
    save_error: bool,
    pending: bool,
    autosave: Option<Task<()>>,
    screenshot_key: Entity<InputState>,
    paste_key: Entity<InputState>,
    save_path: Entity<InputState>,
    gif_seconds: Entity<InputState>,
    gif_width: Entity<InputState>,
    trail_width: Entity<InputState>,
    trail_life: Entity<InputState>,
    trail_animation: TrailAnimation,
    trail_show_in_capture: bool,
    trail_color_mode: TrailColorMode,
    trail_primary: Entity<ColorPickerState>,
    trail_secondary: Entity<ColorPickerState>,
    _subscriptions: Vec<Subscription>,
    pin_border_color: Entity<ColorPickerState>,
    pin_border_width: Entity<InputState>,
    pin_shadow: bool,
    proxy_mode: ProxyMode,
    proxy_url: Entity<InputState>,
    proxy_username: Entity<InputState>,
    proxy_password: Entity<InputState>,
    update_busy: bool,
    update_status: String,
    update_progress: Option<f32>,
    auto_check: bool,
    last_update_notice: Option<String>,
    page_transition: usize,
    optional: Vec<(String, Entity<InputState>)>,
    ocr_edit: usize,
    ocr_fields: Vec<Entity<InputState>>,
}
impl Settings {
    pub fn open(cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
        Self::build(cx).map(|(handle, _)| handle)
    }
    fn build(cx: &mut App) -> anyhow::Result<(AnyWindowHandle, Entity<Self>)> {
        crate::updates::init(cx);
        crate::ui::sync_system_theme(cx);
        let bounds = Bounds::centered(
            None,
            size(
                px(820.),
                px(if crate::is_docs_demo() { 690. } else { 574. }),
            ),
            cx,
        );
        let (h, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Simple Snip · 偏好设置".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(720.), px(490.))),
                ..Default::default()
            },
            cx,
            |w, cx| {
                crate::ui::watch_appearance(w);
                app::prepare_window(w);
                app::apply_style(
                    w,
                    snip_platform::WindowStyle {
                        topmost: false,
                        app_window: true,
                        ..Default::default()
                    },
                );
                let config = cx.global::<Runtime>().config.clone();
                let ocr_fields = Self::ocr_values(&config.ocr.profiles[config.ocr.active])
                    .into_iter()
                    .map(|value| cx.new(|cx| InputState::new(w, cx).default_value(value)))
                    .collect();
                let mut input =
                    |value: String| cx.new(|cx| InputState::new(w, cx).default_value(value));
                let screenshot_key = input(
                    config
                        .hotkeys
                        .get("screenshot")
                        .cloned()
                        .unwrap_or_default(),
                );
                let paste_key = input(
                    config
                        .hotkeys
                        .get("paste_image")
                        .cloned()
                        .unwrap_or_default(),
                );
                let save_path = input(config.save_directory.to_string_lossy().to_string());
                let gif_seconds = input(config.gif.max_seconds.to_string());
                let gif_width = input(config.gif.max_width.to_string());
                let trail_width = input(config.trail.width.to_string());
                let trail_life = input(config.trail.lifetime_ms.to_string());
                let pin_border_width = input(config.pin.border_width.to_string());
                let proxy_url = input(config.updates.proxy_url.clone());
                let proxy_username = input(config.updates.proxy_username.clone());
                let optional: Vec<_> = ["long_capture", "gif", "ocr", "mouse_trail", "settings"]
                    .into_iter()
                    .map(|key| {
                        (
                            key.to_string(),
                            input(config.hotkeys.get(key).cloned().unwrap_or_default()),
                        )
                    })
                    .collect();
                let pin_color = config.pin.color().unwrap_or(0x1688ff);
                let pin_border_color =
                    cx.new(|cx| ColorPickerState::new(w, cx).default_value(rgb(pin_color)));
                let color = |text: &str| {
                    let color = TrailConfig::parse_color(text).unwrap_or([255, 90, 138, 255]);
                    rgba(u32::from_be_bytes(color))
                };
                let trail_primary = cx.new(|cx| {
                    ColorPickerState::new(w, cx).default_value(color(&config.trail.primary_color))
                });
                let trail_secondary = cx.new(|cx| {
                    ColorPickerState::new(w, cx).default_value(color(&config.trail.secondary_color))
                });
                let password = cx
                    .try_global::<crate::updates::Updates>()
                    .map(|v| v.password.clone())
                    .unwrap_or_default();
                let proxy_password =
                    cx.new(|cx| InputState::new(w, cx).masked(true).default_value(password));
                cx.new(|cx| {
                    let mut subscriptions = vec![
                        cx.subscribe(
                            &proxy_password,
                            |_: &mut Self, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    let value = input.read(cx).value().to_string();
                                    cx.global_mut::<crate::updates::Updates>().password = value;
                                }
                            },
                        ),
                        cx.observe(&trail_primary, |_: &mut Self, _, cx| cx.notify()),
                        cx.observe(&trail_secondary, |_, _, cx| cx.notify()),
                        cx.observe(&trail_width, |_, _, cx| cx.notify()),
                        cx.observe(&trail_life, |_, _, cx| cx.notify()),
                    ];
                    for input in [
                        &screenshot_key,
                        &paste_key,
                        &save_path,
                        &gif_seconds,
                        &gif_width,
                        &trail_width,
                        &trail_life,
                        &pin_border_width,
                        &proxy_url,
                        &proxy_username,
                    ]
                    .into_iter()
                    .chain(optional.iter().map(|(_, input)| input))
                    {
                        subscriptions.push(cx.subscribe(input, |v, _, event: &InputEvent, cx| {
                            if matches!(event, InputEvent::Change) {
                                v.schedule_save(cx);
                            }
                        }));
                    }
                    for picker in [&trail_primary, &trail_secondary, &pin_border_color] {
                        subscriptions.push(cx.subscribe(
                            picker,
                            |v, _, event: &ColorPickerEvent, cx| {
                                if matches!(event, ColorPickerEvent::Change(_)) {
                                    v.schedule_save(cx);
                                }
                            },
                        ));
                    }
                    Self {
                        section: 0,
                        baseline: config.clone(),
                        config_path: Config::path(),
                        show_on_start: config.show_settings_on_start,
                        gif_fps: config.gif.fps,
                        theme: config.theme.clone(),
                        format: config.format.clone(),
                        save_status: "修改后自动保存".into(),
                        save_error: false,
                        pending: false,
                        autosave: None,
                        screenshot_key,
                        paste_key,
                        save_path,
                        gif_seconds,
                        gif_width,
                        trail_width,
                        trail_life,
                        trail_animation: config.trail.animation,
                        trail_show_in_capture: config.trail.show_in_capture,
                        trail_color_mode: config.trail.color_mode,
                        trail_primary,
                        trail_secondary,
                        _subscriptions: subscriptions,
                        pin_border_color,
                        pin_border_width,
                        pin_shadow: config.pin.shadow,
                        proxy_mode: config.updates.proxy_mode,
                        proxy_url,
                        proxy_username,
                        proxy_password,
                        update_busy: false,
                        update_progress: None,
                        update_status: match std::fs::read_to_string(
                            Config::path()
                                .with_file_name("updates")
                                .join("last-install.txt"),
                        )
                        .as_deref()
                        {
                            Ok("ok") => "上次升级安装完成；可检查新版本。".into(),
                            Ok("failed") => {
                                "上次升级失败，已尝试恢复原版本；请检查安装目录中的升级日志。"
                                    .into()
                            }
                            _ => "手动检查 GitHub Release；截图和识别内容不会发送。".into(),
                        },
                        auto_check: config.updates.auto_check,
                        last_update_notice: None,
                        page_transition: 0,
                        optional,
                        ocr_edit: config.ocr.active,
                        ocr_fields,
                    }
                })
            },
        )?;
        h.update(cx, |_, window, cx| {
            let settings = view.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                let _ = settings.update(cx, |v, cx| v.save(cx));
                if let Ok(raw) = app::raw(window) {
                    let _ = snip_platform::show_window(raw, false);
                }
                false
            });
        })?;
        let settings = view.downgrade();
        let on_quit = cx.on_app_quit(move |cx| {
            let _ = settings.update(cx, |v, cx| {
                if v.pending {
                    v.save(cx);
                }
            });
            async {}
        });
        view.update(cx, |v, _| v._subscriptions.push(on_quit));
        crate::updates::attach(h, &view, cx);
        Ok((h, view))
    }
    #[cfg(debug_assertions)]
    pub(crate) fn open_section(section: usize, cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
        let (handle, view) = Self::build(cx)?;
        view.update(cx, |v, cx| {
            v.section = section;
            cx.notify();
        });
        Ok(handle)
    }
    #[inline(never)]
    fn trail_options(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut options = div()
            .v_flex()
            .gap_3()
            .child(div().text_sm().child("动画效果"));
        let mut styles = div().h_flex().gap_2().flex_wrap();
        for (label, style) in [
            ("柔光丝带", TrailAnimation::Ribbon),
            ("彗星", TrailAnimation::Comet),
            ("光点", TrailAnimation::Particles),
            ("涟漪", TrailAnimation::Ripple),
        ] {
            styles = styles.child(
                Button::new(label)
                    .label(label)
                    .selected(self.trail_animation == style)
                    .on_click(cx.listener(move |v, _, _, cx| {
                        v.trail_animation = style;
                        v.save(cx);
                    })),
            );
        }
        options = options
            .child(styles)
            .child(div().text_sm().child("配色模式"));
        let mut modes = div().h_flex().gap_2();
        for (label, mode) in [
            ("彩虹", TrailColorMode::Rainbow),
            ("单色", TrailColorMode::Solid),
            ("渐变", TrailColorMode::Gradient),
        ] {
            modes = modes.child(
                Button::new(label)
                    .label(label)
                    .selected(self.trail_color_mode == mode)
                    .on_click(cx.listener(move |v, _, _, cx| {
                        v.trail_color_mode = mode;
                        v.save(cx);
                    })),
            );
        }
        options = options.child(modes);
        if self.trail_color_mode != TrailColorMode::Rainbow {
            let mut colors =
                div()
                    .h_flex()
                    .gap_3()
                    .child(ColorPicker::new(&self.trail_primary).label(
                        if self.trail_color_mode == TrailColorMode::Solid {
                            "拖尾颜色"
                        } else {
                            "起始色"
                        },
                    ));
            if self.trail_color_mode == TrailColorMode::Gradient {
                colors = colors.child(ColorPicker::new(&self.trail_secondary).label("结束色"));
            }
            options = options.child(colors);
        }
        let mut config = cx.global::<Runtime>().config.trail.clone();
        config.animation = self.trail_animation;
        config.color_mode = self.trail_color_mode;
        config.primary_color = picker_hex(&self.trail_primary, cx);
        config.secondary_color = picker_hex(&self.trail_secondary, cx);
        config.width = self
            .trail_width
            .read(cx)
            .value()
            .parse::<f32>()
            .unwrap_or(9.)
            .clamp(1., 60.);
        config.lifetime_ms = self
            .trail_life
            .read(cx)
            .value()
            .parse::<u32>()
            .unwrap_or(300)
            .clamp(100, 3000);
        options
            .child(crate::effects::preview(config))
            .child(
                div()
                    .text_xs()
                    .child("效果预览 · 修改后自动保存并应用到桌面"),
            )
            .into_any_element()
    }
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.pending = true;
        self.save_error = false;
        self.autosave = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            let _ = this.update(cx, |v, cx| v.save(cx));
        }));
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        self.pending = false;
        let result = (|| -> anyhow::Result<Config> {
            let mut c = self.baseline.clone();
            c.show_settings_on_start = self.show_on_start;
            c.gif.fps = self.gif_fps;
            c.theme = self.theme.clone();
            c.format = self.format.clone();
            c.save_directory = self.save_path.read(cx).value().to_string().into();
            c.hotkeys.insert(
                "screenshot".into(),
                self.screenshot_key.read(cx).value().to_string(),
            );
            c.hotkeys.insert(
                "paste_image".into(),
                self.paste_key.read(cx).value().to_string(),
            );
            for (name, input) in &self.optional {
                let value = input.read(cx).value().to_string();
                if value.is_empty() {
                    c.hotkeys.remove(name);
                } else {
                    c.hotkeys.insert(name.clone(), value);
                }
            }
            c.gif.max_seconds = self
                .gif_seconds
                .read(cx)
                .value()
                .parse()
                .context("最长录制时间须为整数")?;
            c.gif.max_width = self
                .gif_width
                .read(cx)
                .value()
                .parse()
                .context("输出最大宽度须为整数")?;
            c.trail.width = self
                .trail_width
                .read(cx)
                .value()
                .parse()
                .context("拖尾宽度须为数字")?;
            c.trail.lifetime_ms = self
                .trail_life
                .read(cx)
                .value()
                .parse()
                .context("拖尾生命周期须为整数")?;
            c.trail.animation = self.trail_animation;
            c.trail.show_in_capture = self.trail_show_in_capture;
            c.trail.color_mode = self.trail_color_mode;
            c.trail.primary_color = picker_hex(&self.trail_primary, cx);
            c.trail.secondary_color = picker_hex(&self.trail_secondary, cx);
            if let Some(color) = self.pin_border_color.read(cx).value() {
                let c_rgb = color.to_rgb();
                c.pin.border_color = format!(
                    "#{:02X}{:02X}{:02X}",
                    (c_rgb.r * 255.).round() as u8,
                    (c_rgb.g * 255.).round() as u8,
                    (c_rgb.b * 255.).round() as u8
                );
            }
            c.pin.border_width = self
                .pin_border_width
                .read(cx)
                .value()
                .parse()
                .context("贴图边框粗细须为数字")?;
            c.pin.shadow = self.pin_shadow;
            c.updates = self.proxy_config(cx);
            snip_update::validate_proxy(&c.updates)?;
            c.validate()?;
            crate::hotkeys::validate(&c)?;
            Ok(c)
        })();
        match result {
            Ok(draft) => {
                let previous = cx.global::<Runtime>().config.clone();
                let config = merge_settings(previous.clone(), &self.baseline, &draft);
                if config != previous
                    && !crate::is_docs_demo()
                    && let Err(error) = config.save(&self.config_path)
                {
                    self.save_status = format!("自动保存失败：{error}。请检查配置目录权限后重试。");
                    self.save_error = true;
                    self.notify_save_error(cx);
                    cx.notify();
                    return;
                }
                let refresh_pins = config.pin != previous.pin;
                let restart_trail =
                    config.trail != previous.trail && !cx.global::<Runtime>().trails.is_empty();
                let update_hotkeys = config.hotkeys != previous.hotkeys;
                cx.global_mut::<Runtime>().config = config.clone();
                self.baseline = draft;
                if config.theme != previous.theme {
                    crate::ui::apply_theme(&config.theme, cx);
                }
                let pins: Vec<_> = cx
                    .global::<Runtime>()
                    .pins
                    .values()
                    .map(|p| (p.handle, p.view.clone()))
                    .collect();
                for (handle, view) in pins.into_iter().filter(|_| refresh_pins) {
                    let _ = handle.update(cx, |_, w, cx| {
                        view.update(cx, |v, cx| v.refresh_frame(w, cx))
                    });
                }
                if restart_trail {
                    crate::effects::stop(cx);
                    crate::effects::start(cx);
                }
                self.save_status = "已自动保存".into();
                self.save_error = false;
                if update_hotkeys
                    && let Some(keys) = cx.global_mut::<Runtime>().hotkeys.as_mut()
                    && let Err(error) = keys.apply(&config)
                {
                    self.save_status = format!("已保存；快捷键尚未生效：{error}");
                    self.save_error = true;
                }
                if update_hotkeys {
                    app::refresh_tray(cx);
                }
                cx.refresh_windows();
            }
            Err(e) => {
                self.save_status = format!("尚未保存：{e}；请修正输入，原设置仍然有效。");
                self.save_error = true;
            }
        }
        if self.save_error {
            self.notify_save_error(cx);
        }
        cx.notify();
    }
    fn notify_save_error(&self, cx: &mut Context<Self>) {
        let message = self.save_status.clone();
        if let Some(handle) = cx.global::<Runtime>().settings {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, w, cx| crate::ui::notify(w, message, true, cx));
            });
        }
    }
    pub(crate) fn show_section(&mut self, section: usize, cx: &mut Context<Self>) {
        if self.section != section {
            self.section = section;
            self.page_transition = self.page_transition.wrapping_add(1);
            cx.notify();
        }
    }
    pub(crate) fn notify_update(&mut self, tag: &str, w: &mut Window, cx: &mut Context<Self>) {
        if self.last_update_notice.as_deref() == Some(tag) {
            return;
        }
        self.last_update_notice = Some(tag.into());
        tracing::debug!(tag, "showing update notification");
        let view = cx.entity().downgrade();
        let detail = format!("{tag} 已发布 · 点此查看");
        w.push_notification(
            Notification::new()
                .content(move |_, _, _| {
                    div()
                        .debug_selector(|| "update-notice-body".to_string())
                        .child(detail.clone())
                        .into_any_element()
                })
                .id::<crate::updates::Updates>()
                .title("发现新版本")
                .delivery(NotificationDelivery::InApp)
                .placement(Anchor::TopRight)
                .autohide(false)
                .on_click(move |_, _, cx| {
                    let _ = view.update(cx, |v, cx| v.show_section(10, cx));
                }),
            cx,
        );
    }
    fn set_automatic_updates(&mut self, enabled: bool, w: &mut Window, cx: &mut Context<Self>) {
        let mut config = cx.global::<Runtime>().config.clone();
        if config.updates.auto_check == enabled {
            return;
        }
        config.updates.auto_check = enabled;
        if !crate::is_docs_demo()
            && let Err(error) = config.save(&self.config_path)
        {
            crate::ui::notify(w, format!("无法保存自动更新设置：{error}"), true, cx);
            return;
        }
        cx.global_mut::<Runtime>().config = config;
        self.auto_check = enabled;
        self.baseline.updates.auto_check = enabled;
        if enabled && !crate::is_docs_demo() {
            crate::updates::automatic(cx);
        }
        cx.notify();
    }
    fn ocr_values(profile: &snip_core::ocr_config::OcrProfile) -> Vec<String> {
        let yes = |value: bool| {
            if value {
                "1".to_string()
            } else {
                "0".to_string()
            }
        };
        let mut values = vec![profile.name.clone()];
        #[cfg(target_os = "windows")]
        {
            let p = &profile.rapid;
            values.extend([
                p.max_side.to_string(),
                p.threads.to_string(),
                yes(p.orientation),
                p.detection_threshold.to_string(),
                p.box_score.to_string(),
                p.text_score.to_string(),
                p.unclip_ratio.to_string(),
            ]);
        }
        #[cfg(target_os = "macos")]
        {
            let p = &profile.vision;
            values.extend([
                yes(p.accurate),
                yes(p.correction),
                yes(p.auto_language),
                p.languages.clone(),
                p.min_height.to_string(),
                p.custom_words.clone(),
            ]);
        }
        values
    }

    fn load_ocr_fields(&mut self, index: usize, w: &mut Window, cx: &mut Context<Self>) {
        self.ocr_edit = index;
        let values = Self::ocr_values(&cx.global::<Runtime>().config.ocr.profiles[index]);
        for (field, value) in self.ocr_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value, w, cx));
        }
        cx.notify();
    }

    fn read_ocr_fields(&self, cx: &App) -> anyhow::Result<snip_core::ocr_config::OcrProfile> {
        let mut profile = cx.global::<Runtime>().config.ocr.profiles[self.ocr_edit].clone();
        let values: Vec<_> = self
            .ocr_fields
            .iter()
            .map(|field| field.read(cx).value())
            .collect();
        let value = |index: usize| values[index].trim();
        let boolean = |index| match value(index) {
            "1" => Ok(true),
            "0" => Ok(false),
            _ => anyhow::bail!("开关参数须为 0 或 1"),
        };
        profile.name = value(0).into();
        #[cfg(target_os = "windows")]
        {
            let p = &mut profile.rapid;
            p.max_side = value(1).parse().context("最大边长须为整数")?;
            p.threads = value(2).parse().context("线程数须为整数")?;
            p.orientation = boolean(3)?;
            p.detection_threshold = value(4).parse().context("检测阈值无效")?;
            p.box_score = value(5).parse().context("框置信度无效")?;
            p.text_score = value(6).parse().context("识别置信度无效")?;
            p.unclip_ratio = value(7).parse().context("扩框系数无效")?;
        }
        #[cfg(target_os = "macos")]
        {
            let p = &mut profile.vision;
            p.accurate = boolean(1)?;
            p.correction = boolean(2)?;
            p.auto_language = boolean(3)?;
            p.languages = value(4).into();
            p.min_height = value(5).parse().context("最小文字高度无效")?;
            p.custom_words = value(6).into();
        }
        Ok(profile)
    }

    fn change_ocr_config(
        &mut self,
        operation: OcrModeAction,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = (|| -> anyhow::Result<usize> {
            let mut config = cx.global::<Runtime>().config.clone();
            let mut index = self.ocr_edit;
            match operation {
                OcrModeAction::Save => config.ocr.profiles[index] = self.read_ocr_fields(cx)?,
                OcrModeAction::Default => config.ocr.active = index,
                OcrModeAction::Reset => {
                    config.ocr.profiles[index] = snip_core::ocr_config::OcrConfig::preset(index)
                }
                OcrModeAction::Duplicate => {
                    let mut profile = self.read_ocr_fields(cx)?;
                    profile.name =
                        format!("{} 副本", profile.name.chars().take(28).collect::<String>());
                    index = config.ocr.profiles.len();
                    config.ocr.profiles.push(profile);
                }
            }
            config.ocr.validate()?;
            config.save(&self.config_path)?;
            self.baseline.ocr = config.ocr.clone();
            cx.global_mut::<Runtime>().config = config;
            Ok(index)
        })();
        match result {
            Ok(index) => {
                if matches!(operation, OcrModeAction::Default) {
                    crate::editor::use_default_ocr_mode(cx);
                }
                self.load_ocr_fields(index, w, cx);
                crate::ui::notify(w, "OCR 模式已保存", false, cx);
            }
            Err(error) => crate::ui::notify(w, format!("无法保存 OCR 模式：{error}"), true, cx),
        }
    }

    fn ocr_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use menu::{DropdownMenu, PopupMenuItem};
        let modes = cx.global::<Runtime>().config.ocr.clone();
        let active = modes.active;
        let weak = cx.entity().downgrade();
        let name = modes.profiles[self.ocr_edit].name.clone();
        let selector = Button::new("ocr-mode-settings")
            .small()
            .label(name)
            .accessibility_label("选择要编辑的 OCR 模式")
            .dropdown_menu(move |mut menu, _, _| {
                for (index, profile) in modes.profiles.iter().enumerate() {
                    let weak = weak.clone();
                    let name = if index == active {
                        format!("{} · 默认", profile.name)
                    } else {
                        profile.name.clone()
                    };
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, w, cx| {
                        let _ = weak.update(cx, |v, cx| v.load_ocr_fields(index, w, cx));
                    }));
                }
                menu
            });
        #[cfg(target_os = "windows")]
        let labels = [
            ("模式名称", "1–32 个字符"),
            ("最大边长", "320–3200 像素"),
            ("CPU 线程", "1–16"),
            ("方向校正", "1 开启，0 关闭"),
            ("检测阈值", "0.01–0.99"),
            ("框置信度", "0.01–0.99"),
            ("识别置信度", "0.01–0.99"),
            ("扩框系数", "1–4"),
        ];
        #[cfg(target_os = "macos")]
        let labels = [
            ("模式名称", "1–32 个字符"),
            ("优先准确度", "1 精确，0 快速"),
            ("语言纠正", "1 开启，0 关闭"),
            ("自动检测语言", "1 开启，0 关闭"),
            ("识别语言", "逗号分隔的语言代码，如 zh-Hans, en-US"),
            ("最小文字高度", "相对图片高度的比例，0–0.25"),
            ("自定义词汇", "逗号分隔，开启语言纠正后生效"),
        ];
        let engine = if cfg!(target_os = "windows") {
            "RapidOCR · 本地识别"
        } else {
            "Apple Vision · 本地识别"
        };
        div()
            .v_flex()
            .gap_2()
            .child(row("识别模式", engine, selector).icon("scan-text"))
            .children(
                labels
                    .into_iter()
                    .zip(&self.ocr_fields)
                    .map(|((label, help), field)| {
                        row(
                            label,
                            help,
                            div()
                                .w(px(190.))
                                .child(Input::new(field).small().aria_label(label)),
                        )
                    }),
            )
            .child(
                div().flex().flex_wrap().gap_2().children(
                    [
                        ("保存模式", OcrModeAction::Save, true),
                        (
                            "设为默认",
                            OcrModeAction::Default,
                            self.ocr_edit != active
                                || crate::editor::remembered_ocr_mode(cx).is_some(),
                        ),
                        ("还原预设", OcrModeAction::Reset, self.ocr_edit < 3),
                        ("复制为自定义", OcrModeAction::Duplicate, true),
                    ]
                    .into_iter()
                    .map(|(label, action, enabled)| {
                        Button::new(label)
                            .small()
                            .label(label)
                            .disabled(!enabled)
                            .on_click(
                                cx.listener(move |v, _, w, cx| v.change_ocr_config(action, w, cx)),
                            )
                    }),
                ),
            )
    }

    fn proxy_config(&self, cx: &App) -> UpdateConfig {
        UpdateConfig {
            auto_check: self.auto_check,
            proxy_mode: self.proxy_mode,
            proxy_url: self.proxy_url.read(cx).value().trim().to_string(),
            proxy_username: self.proxy_username.read(cx).value().to_string(),
        }
    }
    fn check_updates(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.update_busy {
            return;
        }
        let config = self.proxy_config(cx);
        if let Err(error) = snip_update::validate_proxy(&config) {
            crate::ui::notify(w, error.to_string(), true, cx);
            return;
        }
        cx.global_mut::<crate::updates::Updates>().password =
            self.proxy_password.read(cx).value().to_string();
        crate::updates::check(config, true, cx);
    }
    fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.update_busy {
            return;
        }
        let Some(release) = cx.global::<crate::updates::Updates>().release.clone() else {
            return;
        };
        let config = self.proxy_config(cx);
        let password = self.proxy_password.read(cx).value().to_string();
        let directory = Config::path().with_file_name("updates");
        let (tx, rx) = async_channel::bounded(8);
        self.update_busy = true;
        self.update_progress = Some(0.);
        self.update_status = "正在下载并校验更新包…".into();
        let app_tx = cx.global::<Runtime>().tx.clone();
        std::thread::spawn(move || {
            let mut last_progress = std::time::Instant::now();
            let result =
                snip_update::download(&release, &config, &password, &directory, |done, total| {
                    // Keep UI work bounded even on fast connections. Never block the
                    // download for intermediate progress; stage changes are reliable.
                    if (done == total
                        || last_progress.elapsed() >= std::time::Duration::from_millis(100))
                        && tx.try_send(DownloadEvent::Progress(done, total)).is_ok()
                    {
                        last_progress = std::time::Instant::now();
                    }
                })
                .and_then(|path| {
                    let _ = tx.send_blocking(DownloadEvent::Installing);
                    snip_update::prepare_install(&path)
                })
                .and_then(snip_update::launch_install);
            if result.is_ok() {
                let _ = app_tx.send_blocking(app::Event::Command(Command::Quit));
            }
            let _ = tx.send_blocking(DownloadEvent::Done(result));
        });
        cx.spawn(async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                let finished = matches!(event, DownloadEvent::Done(_));
                if this
                    .update(cx, |v, cx| {
                        let failed = matches!(&event, DownloadEvent::Done(Err(_)));
                        v.apply_download_event(event);
                        if failed && let Some(handle) = cx.global::<Runtime>().settings {
                            let message = v.update_status.clone();
                            cx.defer(move |cx| {
                                let _ = handle
                                    .update(cx, |_, w, cx| crate::ui::notify(w, message, true, cx));
                            });
                        }
                        cx.notify();
                    })
                    .is_err()
                    || finished
                {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }
    fn apply_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Progress(done, total) => {
                let percent = (done as f64 / total.max(1) as f64 * 100.).clamp(0., 100.) as f32;
                self.update_progress = Some(percent);
                self.update_status = if total > 0 && done >= total {
                    "下载完成，正在校验 SHA256…".into()
                } else {
                    format!("正在下载：{percent:.0}%")
                };
            }
            DownloadEvent::Installing => {
                self.update_progress = Some(100.);
                self.update_status = "校验通过，正在准备安装…".into();
            }
            DownloadEvent::Done(result) => {
                self.update_busy = false;
                match result {
                    Ok(()) => {
                        self.update_progress = Some(100.);
                        self.update_status = "升级助手已就绪，正在退出并安装新版本…".into();
                    }
                    Err(e) => {
                        self.update_progress = None;
                        self.update_status = format!("升级失败：{e}");
                    }
                }
            }
        }
    }
}
#[derive(IntoElement)]
struct SettingsRow {
    icon: Option<&'static str>,
    title: SharedString,
    description: SharedString,
    control: AnyElement,
}

fn row(title: &str, description: &str, control: impl IntoElement) -> SettingsRow {
    SettingsRow {
        icon: None,
        title: title.to_string().into(),
        description: description.to_string().into(),
        control: control.into_any_element(),
    }
}

impl SettingsRow {
    fn icon(mut self, glyph: &'static str) -> Self {
        self.icon = Some(glyph);
        self
    }
}
fn glyph(name: &str, size: f32) -> Icon {
    Icon::default()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
}
fn key_hint(key: &'static str, label: &'static str, cx: &App) -> Div {
    div()
        .h_flex()
        .gap_1()
        .items_center()
        .child(
            div()
                .px_1()
                .rounded(px(4.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .text_size(px(10.))
                .child(key),
        )
        .child(crate::ui::detail(cx).child(label))
}
fn appearance_sample(mode: &str) -> impl IntoElement {
    let panel = |dark: bool| {
        let (surface, rail, text, accent) = if dark {
            (0x222832, 0x151920, 0xaab8cc, 0x91b7ff)
        } else {
            (0xf8f9fc, 0xe9eef7, 0x64728a, 0x275fc2)
        };
        div()
            .flex_1()
            .h_full()
            .h_flex()
            .overflow_hidden()
            .bg(rgb(surface))
            .child(div().w_1_4().h_full().bg(rgb(rail)))
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .gap_1()
                    .p_2()
                    .child(div().w_1_2().h(px(4.)).rounded_full().bg(rgb(text)))
                    .child(
                        div()
                            .w_3_4()
                            .h(px(3.))
                            .rounded_full()
                            .bg(rgb(text))
                            .opacity(0.48),
                    )
                    .child(div().w_1_3().h(px(6.)).rounded(px(2.)).bg(rgb(accent))),
            )
    };
    div()
        .w_full()
        .h(px(48.))
        .h_flex()
        .overflow_hidden()
        .rounded(px(5.))
        .when(mode != "dark", |d| d.child(panel(false)))
        .when(mode != "light", |d| d.child(panel(true)))
}
impl RenderOnce for SettingsRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .h_flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .when_some(self.icon, |d, icon| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(28.))
                        .flex_shrink_0()
                        .rounded_md()
                        .bg(cx.theme().muted)
                        .text_color(cx.theme().muted_foreground)
                        .child(glyph(icon, 14.)),
                )
            })
            .child(
                div()
                    .v_flex()
                    .gap(px(2.))
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .child(div().font_weight(FontWeight::MEDIUM).child(self.title))
                    .when(!self.description.is_empty(), |d| {
                        d.child(crate::ui::detail(cx).child(self.description))
                    }),
            )
            .child(div().flex_shrink_0().child(self.control))
    }
}
impl Render for Settings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = cx.global::<Runtime>().config.clone();
        let names = [
            "通用",
            "截图与标注",
            "贴图",
            "长截图",
            "GIF 录制",
            "文字识别",
            "鼠标拖尾",
            "快捷键",
            "保存与历史",
            "外观",
            "软件更新",
        ];
        let icons = [
            "settings-2",
            "frame",
            "square",
            "gallery-vertical-end",
            "play",
            "case-sensitive",
            "asterisk",
            "square-terminal",
            "folder-closed",
            "palette",
            "download",
        ];
        let mut nav = div()
            .id("settings-navigation")
            .v_flex()
            .w(px(158.))
            .flex_shrink_0()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .p_3()
            .gap(px(2.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .px_2()
                    .py_2()
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(25.))
                                    .rounded_md()
                                    .bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                                    .child(Icon::default().path("icons/frame.svg").size(px(15.))),
                            )
                            .child("Simple Snip"),
                    ),
            );
        for (i, name) in names.iter().enumerate() {
            if let Some(group) = match i {
                0 => Some("工作流"),
                6 => Some("偏好"),
                10 => Some("应用"),
                _ => None,
            } {
                nav = nav.child(crate::ui::detail(cx).px_2().mt_2().child(group));
            }
            nav = nav.child(
                Button::new(("section", i))
                    .ghost()
                    .w_full()
                    .h(px(29.))
                    .tab_stop(true)
                    .accessibility_label(*name)
                    .child(
                        div()
                            .h_flex()
                            .flex_1()
                            .items_center()
                            .gap_2()
                            .relative()
                            .px_2()
                            .when(self.section == i, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .h(px(13.))
                                        .w(px(2.))
                                        .rounded_full()
                                        .bg(cx.theme().primary)
                                        .with_animation(
                                            ("nav-indicator", self.page_transition),
                                            Animation::new(std::time::Duration::from_millis(170))
                                                .with_easing(|t| 1. - (1. - t).powi(3)),
                                            |marker, t| {
                                                marker.h(px(5. + t * 8.)).opacity(0.4 + t * 0.6)
                                            },
                                        ),
                                )
                            })
                            .child(
                                Icon::default()
                                    .path(format!("icons/{}.svg", icons[i]))
                                    .size(px(15.)),
                            )
                            .child(div().min_w_0().text_ellipsis().child(*name)),
                    )
                    .selected(self.section == i)
                    .when(self.section == i, |b| b.font_weight(FontWeight::SEMIBOLD))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_section(i, cx);
                    })),
            );
        }
        let mut content = div()
            .id(("settings-content", self.section))
            .v_flex()
            .gap_1()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .whitespace_normal()
            .px_5()
            .py_4()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .mb_3()
                    .child(
                        Icon::default()
                            .path(format!("icons/{}.svg", icons[self.section]))
                            .size(px(19.))
                            .text_color(cx.theme().primary),
                    )
                    .child(
                        div()
                            .text_size(px(crate::ui::HEADING))
                            .line_height(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(names[self.section]),
                    ),
            );
        let updates = cx.global::<crate::updates::Updates>();
        let checking = updates.checking;
        let release = updates
            .release
            .as_ref()
            .map(|release| release.tag_name.clone());
        match self.section {
            0 => {
                let (toast_enabled, toast_delay) = crate::editor::toast_preferences(cx);
                content = content
                    .child(
                        row(
                            "标注提示",
                            "显示时长",
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    Switch::new("annotation-toast")
                                        .accessibility_label("图片内提示")
                                        .checked(toast_enabled)
                                        .on_click(move |enabled, _, cx| {
                                            crate::editor::set_toast_preferences(
                                                *enabled,
                                                toast_delay,
                                                cx,
                                            )
                                        }),
                                )
                                .children([1000, 2000, 3000, 5000].into_iter().map(|delay| {
                                    Button::new(("toast-delay", delay))
                                        .small()
                                        .label(format!("{} 秒", delay / 1000))
                                        .selected(toast_delay == delay)
                                        .on_click(move |_, _, cx| {
                                            crate::editor::set_toast_preferences(
                                                toast_enabled,
                                                delay,
                                                cx,
                                            )
                                        })
                                })),
                        )
                        .icon("bell"),
                    )
                    .child(
                        row(
                            "启动时打开设置",
                            "",
                            Switch::new("show-settings")
                                .accessibility_label("启动时显示偏好设置")
                                .checked(self.show_on_start)
                                .on_click(cx.listener(|v, checked, _, cx| {
                                    v.show_on_start = *checked;
                                    v.save(cx);
                                })),
                        )
                        .icon("settings-2"),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .mt_4()
                            .child(
                                Button::new("capture")
                                    .primary()
                                    .small()
                                    .icon(glyph("frame", 14.))
                                    .label(crate::ui::hotkey_label(
                                        "区域截图",
                                        config.hotkeys.get("screenshot"),
                                    ))
                                    .on_click(|_, _, cx| app::command(cx, Command::Capture)),
                            )
                            .child(
                                Button::new("paste")
                                    .small()
                                    .icon(glyph("pin", 14.))
                                    .label(crate::ui::hotkey_label(
                                        "剪贴板贴图",
                                        config.hotkeys.get("paste_image"),
                                    ))
                                    .on_click(|_, _, cx| app::command(cx, Command::Paste)),
                            ),
                    );
            }
            1 => {
                content = content
                    .child(
                        row(
                            "区域截图",
                            "单击选窗或拖动框选",
                            Button::new("take")
                                .primary()
                                .small()
                                .icon(glyph("frame", 14.))
                                .label("开始截图")
                                .on_click(|_, _, cx| app::command(cx, Command::Capture)),
                        )
                        .icon("frame"),
                    )
                    .child(
                        div()
                            .h_flex()
                            .flex_wrap()
                            .gap_3()
                            .mt_2()
                            .child(key_hint("A", "选择当前屏幕", cx))
                            .child(key_hint("Shift + ↑↓←→", "微调宽高", cx))
                            .child(key_hint("Esc", "取消", cx)),
                    );
            }

            2 => {
                content = content
                    .child(row(
                        "选中边框颜色",
                        "",
                        div()
                            .w_40()
                            .child(ColorPicker::new(&self.pin_border_color).label("边框颜色")),
                    ))
                    .child(row(
                        "选中边框粗细",
                        "0–8 px",
                        div()
                            .w_24()
                            .child(Input::new(&self.pin_border_width).aria_label("贴图边框粗细")),
                    ))
                    .child(row(
                        "边框阴影",
                        "",
                        Switch::new("pin-shadow")
                            .accessibility_label("贴图边框阴影")
                            .checked(self.pin_shadow)
                            .on_click(cx.listener(|v, checked, _, cx| {
                                v.pin_shadow = *checked;
                                v.save(cx);
                            })),
                    ))
                    .child(
                        div()
                            .mt_2()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("桌面贴图"),
                    )
                    .child(
                        div()
                            .h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("hide")
                                    .small()
                                    .icon(glyph("eye-off", 14.))
                                    .label("隐藏全部")
                                    .on_click(|_, _, cx| app::command(cx, Command::HidePins)),
                            )
                            .child(
                                Button::new("show")
                                    .small()
                                    .icon(glyph("eye", 14.))
                                    .label("显示全部")
                                    .on_click(|_, _, cx| app::command(cx, Command::ShowPins)),
                            )
                            .child(
                                Button::new("restore")
                                    .small()
                                    .icon(glyph("mouse-pointer-2", 14.))
                                    .label("恢复全部交互")
                                    .on_click(|_, _, cx| app::command(cx, Command::RestorePins)),
                            ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .flex_wrap()
                            .gap_3()
                            .mt_2()
                            .child(key_hint("Space", "标注", cx))
                            .child(key_hint("T", "穿透", cx))
                            .child(key_hint("Esc", "关闭", cx)),
                    );
            }
            3 => {
                content = content.child(
                    row(
                        "滚动采集",
                        "支持横向 / 纵向",
                        Button::new("long")
                            .primary()
                            .small()
                            .icon(glyph("gallery-vertical-end", 14.))
                            .label("选择区域")
                            .tooltip("框选后缓慢滚动，避开固定页头")
                            .on_click(|_, _, cx| app::command(cx, Command::LongCapture)),
                    )
                    .icon("gallery-vertical-end"),
                );
            }

            4 => {
                let mut rates = div().h_flex().gap_1();
                for fps in [5, 10, 15, 20, 30] {
                    rates = rates.child(
                        Button::new(("fps", fps as usize))
                            .small()
                            .label(format!("{fps}"))
                            .selected(self.gif_fps == fps)
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.gif_fps = fps;
                                v.save(cx);
                            })),
                    );
                }
                content = content
                    .child(row("采集帧率", "帧 / 秒，默认 15", rates))
                    .child(row(
                        "最长录制（秒）",
                        "1–600，达到上限后自动完成",
                        div()
                            .w_24()
                            .child(Input::new(&self.gif_seconds).aria_label("最长录制秒数")),
                    ))
                    .child(row(
                        "输出最大宽度",
                        "64–4096 像素，按比例缩小",
                        div()
                            .w_24()
                            .child(Input::new(&self.gif_width).aria_label("GIF 最大宽度")),
                    ))
                    .child(
                        div().h_flex().mt_2().child(
                            Button::new("gif")
                                .primary()
                                .small()
                                .icon(glyph("play", 14.))
                                .label("选择录制区域")
                                .on_click(|_, _, cx| app::command(cx, Command::Gif)),
                        ),
                    );
            }
            5 => {
                content =
                    content
                        .child(
                            row(
                                "本地文字识别",
                                "支持普通文本和简单表格",
                                Button::new("ocr")
                                    .primary()
                                    .small()
                                    .icon(glyph("scan-text", 14.))
                                    .label("识别剪贴板图片")
                                    .on_click(|_, _, cx| app::command(cx, Command::Ocr)),
                            )
                            .icon("scan-text"),
                        )
                        .child(div().h_flex().gap_2().mt_2().child(key_hint(
                            "O",
                            "在标注中识别",
                            cx,
                        )));
                content = content.child(self.ocr_settings(cx));
            }
            6 => {
                content = content
                    .when(cfg!(target_os = "windows"), |d| {
                        d.child(row(
                            "在录屏和屏幕共享中显示",
                            "会议演示请共享整个屏幕；本工具截图、长截图和 GIF 采集期间自动排除拖尾",
                            Switch::new("trail-show-in-capture")
                                .accessibility_label("在录屏和屏幕共享中显示拖尾")
                                .checked(self.trail_show_in_capture)
                                .on_click(cx.listener(|v, checked, _, cx| {
                                    v.trail_show_in_capture = *checked;
                                    v.schedule_save(cx);
                                })),
                        ))
                    })
                    .child(row(
                        "鼠标拖尾",
                        "独立透明穿透窗口；支持动画与自定义配色",
                        Switch::new("trail")
                            .accessibility_label("鼠标拖尾")
                            .checked(!cx.global::<Runtime>().trails.is_empty())
                            .on_click(|_, _, cx| app::command(cx, Command::Trail)),
                    ))
                    .child(row(
                        "宽度",
                        "1–60 逻辑像素",
                        div()
                            .w_24()
                            .child(Input::new(&self.trail_width).aria_label("鼠标拖尾宽度")),
                    ))
                    .child(row(
                        "生命周期（毫秒）",
                        "100–3000，静止时自然消失",
                        div()
                            .w_24()
                            .child(Input::new(&self.trail_life).aria_label("鼠标拖尾生命周期")),
                    ))
                    .child(self.trail_options(cx));
            }
            7 => {
                content = content
                    .child(row(
                        "区域截图",
                        "F1 · 留空停用",
                        div()
                            .w_48()
                            .child(Input::new(&self.screenshot_key).aria_label("区域截图快捷键")),
                    ))
                    .child(row(
                        "剪贴板贴图",
                        "F3 · 留空停用",
                        div()
                            .w_48()
                            .child(Input::new(&self.paste_key).aria_label("剪贴板贴图快捷键")),
                    ));
                for (name, input) in &self.optional {
                    let label = match name.as_str() {
                        "long_capture" => "长截图",
                        "gif" => "GIF 录制",
                        "ocr" => "文字识别",
                        "mouse_trail" => "鼠标拖尾",
                        "settings" => "偏好设置",
                        _ => name,
                    };
                    content = content.child(row(
                        label,
                        "",
                        div()
                            .w_48()
                            .child(Input::new(input).aria_label(label.to_owned())),
                    ));
                }
                if let Some(error) = cx
                    .global::<Runtime>()
                    .hotkeys
                    .as_ref()
                    .and_then(|keys| keys.registration_error())
                {
                    let error = error.to_owned();
                    content = content.child(
                        div().h_flex().mt_2().child(
                            Button::new("hotkey-diagnostics")
                                .small()
                                .ghost()
                                .icon(glyph("info", 14.))
                                .label("查看注册问题")
                                .on_click(move |_, w, cx| {
                                    crate::ui::notify(w, error.clone(), true, cx);
                                }),
                        ),
                    );
                }
                #[cfg(target_os = "macos")]
                {
                    content = content.child(div().mt_2().child(key_hint("Fn", "标准功能键", cx)));
                }
            }
            8 => {
                content = content
                    .child(row(
                        "保存目录",
                        "截图自动使用带毫秒时间戳的文件名",
                        div()
                            .w(px(280.))
                            .max_w_full()
                            .child(Input::new(&self.save_path).aria_label("保存目录")),
                    ))
                    .child(row(
                        "图片格式",
                        "适用于普通截图和贴图的保存。",
                        div().h_flex().gap_1().children(
                            [("PNG", "png"), ("JPEG", "jpg")]
                                .into_iter()
                                .map(|(label, format)| {
                                    Button::new(format)
                                        .small()
                                        .label(label)
                                        .selected(config.format == format)
                                        .on_click(cx.listener(move |v, _, _, cx| {
                                            v.format = format.into();
                                            v.save(cx);
                                        }))
                                }),
                        ),
                    ))
                    .child(
                        div()
                            .mt_3()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("最近保存"),
                    );
                let entries = if crate::is_docs_demo() {
                    Err(std::io::ErrorKind::NotFound.into())
                } else {
                    std::fs::read_dir(&config.save_directory)
                };
                if let Ok(entries) = entries {
                    let mut paths: Vec<_> = entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.extension().is_some_and(|e| {
                                ["png", "jpg", "jpeg", "gif"]
                                    .iter()
                                    .any(|s| e.eq_ignore_ascii_case(s))
                            })
                        })
                        .collect();
                    paths.sort();
                    paths.reverse();
                    if paths.is_empty() {
                        content = content.child(
                            crate::ui::detail(cx)
                                .child("尚无记录。保存截图或录制后，可从这里将其重新贴到桌面。"),
                        );
                    }
                    for (i, path) in paths.into_iter().take(30).enumerate() {
                        let label = path.file_name().unwrap().to_string_lossy().to_string();
                        content = content.child(
                            Button::new(("history", i))
                                .ghost()
                                .w_full()
                                .accessibility_label(label.clone())
                                .tooltip(label.clone())
                                .child(div().flex_1().min_w_0().text_ellipsis().child(label))
                                .on_click(move |_, _, cx| {
                                    let path = path.clone();
                                    let tx = cx.global::<Runtime>().tx.clone();
                                    std::thread::spawn(move || {
                                        let event = match image::open(&path) {
                                            Ok(i) => {
                                                let prepared = app::PreparedImage::new(
                                                    std::sync::Arc::new(i.into_rgba8()),
                                                );
                                                if path
                                                    .extension()
                                                    .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
                                                {
                                                    app::Event::AnimatedPin(
                                                        prepared,
                                                        crate::animation::Source::file(path),
                                                    )
                                                } else {
                                                    app::Event::Pin(prepared, None)
                                                }
                                            }
                                            Err(e) => app::Event::Status(e.to_string()),
                                        };
                                        let _ = tx.send_blocking(event);
                                    });
                                }),
                        );
                    }
                } else {
                    let message = match entries.unwrap_err().kind() {
                        std::io::ErrorKind::NotFound => {
                            "尚无记录。第一次保存图片时会创建保存目录。"
                        }
                        _ => "无法打开保存目录。请检查路径和读取权限。",
                    };
                    content = content.child(crate::ui::detail(cx).child(message));
                }
            }
            9 => {
                content = content.child(
                    div().h_flex().gap_2().children(
                        [
                            ("跟随系统", "system", "monitor"),
                            ("浅色", "light", "sun"),
                            ("深色", "dark", "moon"),
                        ]
                        .into_iter()
                        .map(|(label, mode, icon)| {
                            Button::new(mode)
                                .w(px(125.))
                                .h(px(81.))
                                .px_2()
                                .py_1()
                                .accessibility_label(label)
                                .selected(self.theme == mode)
                                .child(
                                    div()
                                        .v_flex()
                                        .w_full()
                                        .items_center()
                                        .gap_1()
                                        .child(appearance_sample(mode))
                                        .child(
                                            div()
                                                .h_flex()
                                                .items_center()
                                                .gap_1()
                                                .child(glyph(icon, 12.))
                                                .child(label),
                                        ),
                                )
                                .on_click(cx.listener(move |v, _, _, cx| {
                                    v.theme = mode.into();
                                    v.save(cx);
                                }))
                        }),
                    ),
                );
            }

            _ => {
                let proxy_modes = div().h_flex().gap_1().children(
                    [
                        ("系统代理", ProxyMode::System),
                        ("无代理", ProxyMode::Direct),
                        ("HTTP 代理", ProxyMode::Http),
                    ]
                    .into_iter()
                    .map(|(label, mode)| {
                        Button::new(label)
                            .small()
                            .label(label)
                            .selected(self.proxy_mode == mode)
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.proxy_mode = mode;
                                v.save(cx);
                            }))
                    }),
                );
                content = content
                    .child(row(
                        "自动检查更新",
                        "启动时和每小时",
                        Switch::new("auto-update-check")
                            .accessibility_label("自动检查更新")
                            .checked(self.auto_check)
                            .on_click(cx.listener(|v, checked, w, cx| {
                                v.set_automatic_updates(*checked, w, cx);
                            })),
                    ))
                    .child(row(
                        "当前版本",
                        concat!("v", env!("CARGO_PKG_VERSION")),
                        Button::new("check-updates")
                            .small()
                            .icon(Icon::default().path("icons/refresh-cw.svg").size(px(13.)))
                            .label(if checking {
                                "检查中…"
                            } else {
                                "检查更新"
                            })
                            .disabled(checking || self.update_busy)
                            .on_click(cx.listener(|v, _, w, cx| v.check_updates(w, cx))),
                    ))
                    .when_some(release, |d, version| {
                        d.child(row(
                            "可用版本",
                            &version,
                            Button::new("download-update")
                                .small()
                                .primary()
                                .icon(Icon::default().path("icons/download.svg").size(px(13.)))
                                .label(if self.update_busy {
                                    format!("{}%", self.update_progress.unwrap_or(0.) as u32)
                                } else {
                                    "安装并重启".into()
                                })
                                .tooltip(if self.update_busy {
                                    self.update_status.clone()
                                } else {
                                    "重启前请保存当前采集".into()
                                })
                                .disabled(self.update_busy || checking)
                                .on_click(cx.listener(|v, _, _, cx| v.download_update(cx))),
                        ))
                    })
                    .child(row("更新代理", "", proxy_modes));
                if self.proxy_mode == ProxyMode::Http {
                    content = content
                        .child(row(
                            "代理地址",
                            "例如 http://127.0.0.1:7890",
                            div()
                                .w_64()
                                .child(Input::new(&self.proxy_url).aria_label("代理地址")),
                        ))
                        .child(row(
                            "用户名（可选）",
                            "仅用于代理认证",
                            div()
                                .w_64()
                                .child(Input::new(&self.proxy_username).aria_label("代理用户名")),
                        ))
                        .child(row(
                            "密码（可选）",
                            "仅本次运行有效，不保存到磁盘",
                            div()
                                .w_64()
                                .child(Input::new(&self.proxy_password).aria_label("代理密码")),
                        ));
                }
                content = content.child(
                    div().h_flex().child(
                        Button::new("github-releases")
                            .small()
                            .ghost()
                            .icon(
                                Icon::default()
                                    .path("icons/external-link.svg")
                                    .size(px(13.)),
                            )
                            .label("GitHub 发布页")
                            .on_click(|_, _, cx| {
                                cx.open_url("https://github.com/junyi-deep/simple-snip/releases")
                            }),
                    ),
                );
                if let Some(url) = snip_platform::screen_permission_settings_url() {
                    content = content
                        .child(
                            div()
                                .mt_3()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("屏幕录制权限"),
                        )
                        .child(
                            div()
                                .h_flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new("permission")
                                        .label("重新验证")
                                        .on_click(|_, _, cx| app::check_screen_permission(cx)),
                                )
                                .child(
                                    Button::new("permission-settings")
                                        .label("打开系统设置")
                                        .on_click(move |_, _, cx| cx.open_url(url)),
                                ),
                        );
                }
                content = content
                    .child(
                        div()
                            .mt_3()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("关于 Simple Snip"),
                    )
                    .child(crate::ui::detail(cx).child("开源许可 · Apache 2.0"));
            }
        }
        div()
            .v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(crate::ui::BODY))
            .line_height(px(19.))
            .child(
                div()
                    .h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(nav.overflow_y_scrollbar())
                    .child(
                        content
                            .overflow_y_scrollbar()
                            .id(("settings-scroll", self.section))
                            .with_animation(
                                ("settings-page-in", self.page_transition),
                                Animation::new(std::time::Duration::from_millis(190))
                                    .with_easing(|t| 1. - (1. - t).powi(3)),
                                |body, t| body.opacity(0.78 + t * 0.22),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, DownloadEvent, Runtime, Settings, merge_settings};
    use gpui_kit::component::WindowExt;
    use gpui_kit::{AnyWindowHandle, Entity, TestAppContext};
    use std::time::Duration;

    fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Settings>, tempfile::TempDir) {
        cx.update(gpui_kit::init);
        let (tx, _) = async_channel::bounded(8);
        cx.set_global(Runtime {
            recordings: vec![],
            config: Config::default(),
            status: String::new(),
            tx,
            pins: Default::default(),
            next_pin: 1,
            overlays: vec![],
            capture_busy: false,
            permission_prompt: false,
            settings: None,
            hotkeys: None,
            tray: None,
            trails: vec![],
            annotation_toolbar: None,
            capture_task: None,
        });
        let directory = tempfile::tempdir().unwrap();
        let (handle, settings) = cx.update(|cx| Settings::build(cx).unwrap());
        cx.update(|cx| {
            settings.update(cx, |v, _| {
                v.config_path = directory.path().join("config.toml")
            })
        });
        (handle, settings, directory)
    }
    fn type_lifetime(
        handle: AnyWindowHandle,
        settings: &Entity<Settings>,
        value: &str,
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            handle
                .update(cx, |_, w, cx| {
                    let input = settings.read(cx).trail_life.clone();
                    input.update(cx, |i, cx| i.replace_all(value.to_string(), w, cx));
                })
                .unwrap()
        });
        cx.run_until_parked();
    }
    fn wait_for_save(cx: &mut TestAppContext) {
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
    }
    #[gpui_kit::test]
    fn update_notice_is_deduplicated_and_click_opens_update_page(cx: &mut TestAppContext) {
        use gpui_kit::{VisualTestContext, point, px};
        let (handle, view, _) = fixture(cx);
        let mut visual = VisualTestContext::from_window(handle, cx);
        visual.update(|w, cx| {
            view.update(cx, |v, cx| {
                v.notify_update("v9.9.9", w, cx);
                v.notify_update("v9.9.9", w, cx);
                assert_eq!(v.section, 0);
            });
            assert_eq!(w.notifications(cx).len(), 1);
            let _ = w.draw(cx);
        });
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        visual.update(|w, cx| {
            w.simulate_next_frame(cx);
            let _ = w.draw(cx);
        });
        let bounds = visual
            .debug_bounds("update-notice-body")
            .expect("rendered update toast");
        let position = point(bounds.left() + px(20.), bounds.top() + px(6.));
        visual.simulate_mouse_move(position, None, Default::default());
        visual.simulate_click(position, Default::default());
        cx.update(|cx| assert_eq!(view.read(cx).section, 10));
    }

    #[gpui_kit::test]
    fn disabling_automatic_updates_saves_even_if_a_draft_field_is_invalid(cx: &mut TestAppContext) {
        let (handle, view, directory) = fixture(cx);
        type_lifetime(handle, &view, "", cx);
        cx.update(|cx| {
            handle
                .update(cx, |_, w, cx| {
                    view.update(cx, |v, cx| v.set_automatic_updates(false, w, cx));
                })
                .unwrap();
            assert!(!cx.global::<Runtime>().config.updates.auto_check);
        });
        wait_for_save(cx);
        let (config, error) = Config::load(&directory.path().join("config.toml"));
        assert!(error.is_none());
        assert!(!config.updates.auto_check);
        assert_eq!(
            config.trail.lifetime_ms,
            Config::default().trail.lifetime_ms
        );
    }

    #[gpui_kit::test]
    fn update_progress_renders_without_notifications_and_recovers_from_failure(
        cx: &mut TestAppContext,
    ) {
        let (handle, view, _directory) = fixture(cx);
        cx.update(|cx| {
            handle
                .update(cx, |_, window, cx| {
                    view.update(cx, |v, cx| {
                        v.section = 10;
                        v.update_busy = true;
                        v.update_progress = Some(0.);
                        cx.notify();
                    });
                    for done in 0..=100 {
                        view.update(cx, |v, cx| {
                            v.apply_download_event(DownloadEvent::Progress(done, 100));
                            assert_eq!(v.update_progress, Some(done as f32));
                            assert!(v.update_busy);
                            cx.notify();
                        });
                        let _ = window.draw(cx);
                        assert!(window.notifications(cx).is_empty());
                    }
                    view.update(cx, |v, cx| {
                        assert!(v.update_status.contains("校验"));
                        v.apply_download_event(DownloadEvent::Installing);
                        assert_eq!(v.update_progress, Some(100.));
                        assert!(v.update_status.contains("准备安装"));
                        v.apply_download_event(DownloadEvent::Done(Err(anyhow::anyhow!(
                            "校验失败"
                        ))));
                        assert!(!v.update_busy, "failure must enable retry");
                        assert!(v.update_progress.is_none());
                        assert!(v.update_status.contains("升级失败"));
                        cx.notify();
                    });
                    let _ = window.draw(cx);
                    assert!(window.notifications(cx).is_empty());
                    // The same indicator also renders on another tab during a retry.
                    view.update(cx, |v, cx| {
                        v.section = 0;
                        v.update_busy = true;
                        v.apply_download_event(DownloadEvent::Progress(25, 100));
                        cx.notify();
                    });
                    let _ = window.draw(cx);
                    assert!(window.notifications(cx).is_empty());
                    view.update(cx, |v, cx| {
                        v.apply_download_event(DownloadEvent::Done(Ok(())));
                        assert!(!v.update_busy);
                        assert_eq!(v.update_progress, Some(100.));
                        cx.notify();
                    });
                    let _ = window.draw(cx);
                    assert!(window.notifications(cx).is_empty());
                })
                .unwrap();
        });
    }
    #[gpui_kit::test]
    fn trail_sharing_setting_autosaves_and_updates_runtime(cx: &mut TestAppContext) {
        let (_handle, view, directory) = fixture(cx);
        for enabled in [false, true] {
            cx.update(|cx| {
                view.update(cx, |v, cx| {
                    v.trail_show_in_capture = enabled;
                    v.schedule_save(cx);
                })
            });
            wait_for_save(cx);
            let (loaded, warning) = Config::load(&directory.path().join("config.toml"));
            assert!(warning.is_none());
            assert_eq!(loaded.trail.show_in_capture, enabled);
            cx.update(|cx| {
                assert_eq!(cx.global::<Runtime>().config.trail.show_in_capture, enabled)
            });
        }
    }

    #[gpui_kit::test]
    fn appearance_and_export_format_apply_and_survive_reloading(cx: &mut TestAppContext) {
        use gpui_kit::component::ActiveTheme;
        let (_handle, view, directory) = fixture(cx);
        cx.update(|cx| {
            crate::ui::install_theme("light", cx);
            assert!(!cx.theme().is_dark());
            view.update(cx, |v, cx| {
                v.theme = "dark".into();
                v.format = "jpg".into();
                v.save(cx);
            });
            assert!(cx.theme().is_dark());
            assert_eq!(cx.global::<Runtime>().config.format, "jpg");
        });
        let (loaded, error) = Config::load(&directory.path().join("config.toml"));
        assert!(error.is_none());
        assert_eq!(loaded.theme, "dark");
        assert_eq!(loaded.format, "jpg");
    }

    #[gpui_kit::test]
    fn input_changes_are_debounced_and_survive_reloading(cx: &mut TestAppContext) {
        let (h, v, directory) = fixture(cx);
        type_lifetime(h, &v, "4", cx);
        type_lifetime(h, &v, "450", cx);
        assert!(!directory.path().join("config.toml").exists());
        wait_for_save(cx);
        let (loaded, error) = Config::load(&directory.path().join("config.toml"));
        assert!(error.is_none());
        assert_eq!(loaded.trail.lifetime_ms, 450);
        assert_eq!(loaded.hotkeys.len(), 2);
        cx.update(|cx| assert_eq!(v.read(cx).save_status, "已自动保存"));
    }
    #[gpui_kit::test]
    fn invalid_draft_preserves_disk_and_recovers_after_correction(cx: &mut TestAppContext) {
        let (h, v, directory) = fixture(cx);
        type_lifetime(h, &v, "500", cx);
        wait_for_save(cx);
        let path = directory.path().join("config.toml");
        let original = std::fs::read(&path).unwrap();
        type_lifetime(h, &v, "", cx);
        wait_for_save(cx);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        cx.update(|cx| {
            assert!(v.read(cx).save_error);
            assert_eq!(cx.global::<Runtime>().config.trail.lifetime_ms, 500);
        });
        type_lifetime(h, &v, "750", cx);
        wait_for_save(cx);
        assert_eq!(Config::load(&path).0.trail.lifetime_ms, 750);
    }
    #[gpui_kit::test]
    fn save_failure_does_not_apply_or_discard_draft(cx: &mut TestAppContext) {
        let (h, v, directory) = fixture(cx);
        cx.update(|cx| v.update(cx, |v, _| v.config_path = directory.path().to_path_buf()));
        type_lifetime(h, &v, "600", cx);
        wait_for_save(cx);
        cx.update(|cx| {
            assert!(v.read(cx).save_error);
            assert_eq!(
                cx.global::<Runtime>().config.trail.lifetime_ms,
                Config::default().trail.lifetime_ms
            );
            v.update(cx, |v, cx| {
                v.config_path = directory.path().join("config.toml");
                v.save(cx);
                assert!(!v.save_error);
            });
        });
        assert_eq!(
            Config::load(&directory.path().join("config.toml"))
                .0
                .trail
                .lifetime_ms,
            600
        );
    }
    #[gpui_kit::test]
    fn closing_window_flushes_pending_edit(cx: &mut TestAppContext) {
        let (h, v, directory) = fixture(cx);
        type_lifetime(h, &v, "900", cx);
        let mut visual = gpui_kit::VisualTestContext::from_window(h, cx);
        assert!(
            !visual.simulate_close(),
            "closing preferences must keep the resident window alive"
        );
        assert_eq!(
            Config::load(&directory.path().join("config.toml"))
                .0
                .trail
                .lifetime_ms,
            900
        );
    }
    #[test]
    fn unrelated_edits_preserve_newer_tray_and_save_dialog_preferences() {
        let baseline = Config::default();
        let mut latest = baseline.clone();
        latest.trail.enabled = !baseline.trail.enabled;
        latest.save_directory = "/new/save/dialog/location".into();
        let mut draft = baseline.clone();
        draft.pin.shadow = !baseline.pin.shadow;
        let merged = merge_settings(latest.clone(), &baseline, &draft);
        assert_eq!(merged.trail.enabled, latest.trail.enabled);
        assert_eq!(merged.save_directory, latest.save_directory);
        assert_eq!(merged.pin.shadow, draft.pin.shadow);
    }
}
