use crate::app::{self, Command, Runtime};
use gpui_kit::prelude::*;
use gpui_kit::{
    component::{
        button::*,
        color_picker::{ColorPicker, ColorPickerState},
        input::{Input, InputState},
        notification::Notification,
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

struct UpdateNotification;
pub struct Settings {
    section: usize,
    screenshot_key: Entity<InputState>,
    paste_key: Entity<InputState>,
    save_path: Entity<InputState>,
    gif_seconds: Entity<InputState>,
    gif_width: Entity<InputState>,
    trail_width: Entity<InputState>,
    trail_life: Entity<InputState>,
    trail_animation: TrailAnimation,
    trail_color_mode: TrailColorMode,
    trail_primary: Entity<ColorPickerState>,
    trail_secondary: Entity<ColorPickerState>,
    _trail_observers: Vec<Subscription>,
    pin_border_color: Entity<ColorPickerState>,
    pin_border_width: Entity<InputState>,
    pin_shadow: bool,
    proxy_mode: ProxyMode,
    proxy_url: Entity<InputState>,
    proxy_username: Entity<InputState>,
    proxy_password: Entity<InputState>,
    update_busy: bool,
    update_status: String,
    notified_status: String,
    update_release: Option<snip_update::Release>,
    optional: Vec<(String, Entity<InputState>)>,
}
impl Settings {
    pub fn open(cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
        Self::build(cx).map(|(handle, _)| handle)
    }
    fn build(cx: &mut App) -> anyhow::Result<(AnyWindowHandle, Entity<Self>)> {
        let bounds = Bounds::centered(
            None,
            size(
                px(900.),
                px(if crate::is_docs_demo() { 800. } else { 660. }),
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
                window_min_size: Some(size(px(760.), px(540.))),
                ..Default::default()
            },
            cx,
            |w, cx| {
                app::prepare_window(w);
                app::apply_style(
                    w,
                    snip_platform::WindowStyle {
                        topmost: false,
                        app_window: true,
                        ..Default::default()
                    },
                );
                w.on_window_should_close(cx, |window, _| {
                    if let Ok(raw) = app::raw(window) {
                        let _ = snip_platform::show_window(raw, false);
                    }
                    false
                });
                let config = cx.global::<Runtime>().config.clone();
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
                let optional = ["long_capture", "gif", "ocr", "mouse_trail", "settings"]
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
                let proxy_password = cx.new(|cx| InputState::new(w, cx).masked(true));
                cx.new(|cx| {
                    let trail_observers = vec![
                        cx.observe(&trail_primary, |_: &mut Self, _, cx| cx.notify()),
                        cx.observe(&trail_secondary, |_, _, cx| cx.notify()),
                        cx.observe(&trail_width, |_, _, cx| cx.notify()),
                        cx.observe(&trail_life, |_, _, cx| cx.notify()),
                    ];
                    Self {
                        section: 0,
                        screenshot_key,
                        paste_key,
                        save_path,
                        gif_seconds,
                        gif_width,
                        trail_width,
                        trail_life,
                        trail_animation: config.trail.animation,
                        trail_color_mode: config.trail.color_mode,
                        trail_primary,
                        trail_secondary,
                        _trail_observers: trail_observers,
                        pin_border_color,
                        pin_border_width,
                        pin_shadow: config.pin.shadow,
                        proxy_mode: config.updates.proxy_mode,
                        proxy_url,
                        proxy_username,
                        proxy_password,
                        update_busy: false,
                        notified_status: String::new(),
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
                        update_release: None,
                        optional,
                    }
                })
            },
        )?;
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
                        cx.notify();
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
                        cx.notify();
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
                    .child("效果预览 · 保存设置后应用到桌面，颜色和动画会自动记住"),
            )
            .into_any_element()
    }
    fn save(&self, window: &mut Window, cx: &mut App) {
        let result = (|| -> anyhow::Result<Config> {
            let mut c = cx.global::<Runtime>().config.clone();
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
            c.gif.max_seconds = self.gif_seconds.read(cx).value().parse()?;
            c.gif.max_width = self.gif_width.read(cx).value().parse()?;
            c.trail.width = self.trail_width.read(cx).value().parse()?;
            c.trail.lifetime_ms = self.trail_life.read(cx).value().parse()?;
            c.trail.animation = self.trail_animation;
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
            c.pin.border_width = self.pin_border_width.read(cx).value().parse()?;
            c.pin.shadow = self.pin_shadow;
            c.updates = self.proxy_config(cx);
            snip_update::validate_proxy(&c.updates)?;
            c.validate()?;
            crate::hotkeys::validate(&c)?;
            if !crate::is_docs_demo() {
                c.save(&Config::path())?;
            }
            Ok(c)
        })();
        match result {
            Ok(config) => {
                let restart_trail = !cx.global::<Runtime>().trails.is_empty();
                cx.global_mut::<Runtime>().config = config;
                let pins: Vec<_> = cx
                    .global::<Runtime>()
                    .pins
                    .values()
                    .map(|p| (p.handle, p.view.clone()))
                    .collect();
                for (handle, view) in pins {
                    let _ = handle.update(cx, |_, w, cx| {
                        view.update(cx, |v, cx| v.refresh_frame(w, cx))
                    });
                }
                if restart_trail {
                    crate::effects::stop(cx);
                    crate::effects::start(cx);
                }
                let config = cx.global::<Runtime>().config.clone();
                if let Some(keys) = cx.global_mut::<Runtime>().hotkeys.as_mut()
                    && let Err(error) = keys.apply(&config)
                {
                    tracing::warn!("{error}");
                }
                window.push_notification(Notification::success("设置已保存"), cx);
                cx.refresh_windows();
            }
            Err(e) => window.push_notification(Notification::error(format!("设置未保存: {e}")), cx),
        }
    }
    fn proxy_config(&self, cx: &App) -> UpdateConfig {
        UpdateConfig {
            proxy_mode: self.proxy_mode,
            proxy_url: self.proxy_url.read(cx).value().trim().to_string(),
            proxy_username: self.proxy_username.read(cx).value().to_string(),
        }
    }
    fn check_updates(&mut self, cx: &mut Context<Self>) {
        if self.update_busy {
            return;
        }
        let config = self.proxy_config(cx);
        if let Err(e) = snip_update::validate_proxy(&config) {
            self.update_status = e.to_string();
            cx.notify();
            return;
        }
        let password = self.proxy_password.read(cx).value().to_string();
        self.update_busy = true;
        self.update_release = None;
        self.update_status = "正在检查 GitHub 发布版本…".into();
        cx.spawn(async move |this, cx| {
            let result = smol::unblock(move || {
                snip_update::check(&config, &password, env!("CARGO_PKG_VERSION"))
            })
            .await;
            let _ = this.update(cx, |v, cx| {
                v.update_busy = false;
                match result {
                    Ok(Some(release)) => {
                        v.update_status = format!("发现 {}，可一键升级并重启。", release.tag_name);
                        v.update_release = Some(release);
                    }
                    Ok(None) => v.update_status = "当前已是最新版本（或尚无公开稳定版本）。".into(),
                    Err(e) => v.update_status = format!("检查失败：{e}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.update_busy {
            return;
        }
        let Some(release) = self.update_release.clone() else {
            return;
        };
        let config = self.proxy_config(cx);
        let password = self.proxy_password.read(cx).value().to_string();
        let directory = Config::path().with_file_name("updates");
        enum DownloadEvent {
            Progress(u64, u64),
            Done(anyhow::Result<()>),
        }
        let (tx, rx) = async_channel::bounded(8);
        self.update_busy = true;
        self.update_status = "正在下载并校验更新包…".into();
        let app_tx = cx.global::<Runtime>().tx.clone();
        std::thread::spawn(move || {
            let result =
                snip_update::download(&release, &config, &password, &directory, |done, total| {
                    let _ = tx.try_send(DownloadEvent::Progress(done, total));
                })
                .and_then(|path| snip_update::prepare_install(&path))
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
                        match event {
                            DownloadEvent::Progress(done, total) => {
                                v.update_status = format!(
                                    "正在下载：{}%（完成后校验 SHA256）",
                                    done * 100 / total.max(1)
                                )
                            }
                            DownloadEvent::Done(result) => {
                                v.update_busy = false;
                                match result {
                                    Ok(()) => {
                                        v.update_status =
                                            "升级助手已就绪，正在退出并安装新版本…".into();
                                    }
                                    Err(e) => v.update_status = format!("升级失败：{e}"),
                                }
                            }
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
}
fn row(title: &str, description: &str, control: impl IntoElement) -> Div {
    div()
        .h_flex()
        .items_center()
        .justify_between()
        .gap_6()
        .py_3()
        .border_b_1()
        .border_color(rgb(0xdde3e8))
        .child(
            div()
                .v_flex()
                .gap_1()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .child(title.to_string())
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x596977))
                        .child(description.to_string()),
                ),
        )
        .child(div().flex_shrink_0().child(control))
}
impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.notified_status != self.update_status {
            if !self.notified_status.is_empty() {
                window.push_notification(
                    Notification::info(self.update_status.clone()).id::<UpdateNotification>(),
                    cx,
                );
            }
            self.notified_status = self.update_status.clone();
        }
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
            "关于、更新与权限",
        ];
        let mut nav = div()
            .id("settings-navigation")
            .v_flex()
            .w(px(200.))
            .flex_shrink_0()
            .min_h_0()
            .overflow_y_scroll()
            .bg(cx.theme().muted)
            .p_3()
            .gap_1()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .px_2()
                    .py_4()
                    .child("Simple Snip"),
            );
        for (i, name) in names.iter().enumerate() {
            nav = nav.child(
                Button::new(("section", i))
                    .label(*name)
                    .when(self.section == i, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.section = i;
                        cx.notify();
                    })),
            );
        }
        let mut content = div()
            .id("settings-content")
            .v_flex()
            .gap_3()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .whitespace_normal()
            .overflow_y_scroll()
            .p_6()
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .mb_3()
                    .child(names[self.section]),
            );
        match self.section {
            0 => {
                let (toast_enabled, toast_delay) = crate::editor::toast_preferences(cx);
                content = content
                    .child(row(
                        "图片内提示",
                        "提示开关和显示时间",
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Switch::new("annotation-toast")
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
                    ))
                    .child(row(
                        "启动时显示偏好设置",
                        "关闭窗口后继续驻留菜单栏 / 系统托盘",
                        Switch::new("show-settings")
                            .checked(config.show_settings_on_start)
                            .on_click(|checked, _, cx| {
                                cx.global_mut::<Runtime>().config.show_settings_on_start = *checked;
                                cx.refresh_windows();
                            }),
                    ))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .mt_4()
                            .child(
                                Button::new("capture")
                                    .primary()
                                    .label("区域截图 F1")
                                    .on_click(|_, _, cx| app::command(cx, Command::Capture)),
                            )
                            .child(
                                Button::new("paste")
                                    .label("剪贴板贴图 F3")
                                    .on_click(|_, _, cx| app::command(cx, Command::Paste)),
                            ),
                    )
                    .child(
                        div()
                            .mt_5()
                            .text_sm()
                            .child("图片保存在本地。截图、剪贴板与 OCR 不上传网络。"),
                    );
            }
            1 => {
                content=content.child("拖动创建选区，单击吸附窗口。选区生成后可拖动八个边缘手柄调整宽高；画布聚焦时 Shift + 方向键按 1 像素微调大小。A 选择当前屏幕。").child("编辑器保留原始图像与独立标注；复制和保存时合成。Esc 可取消截图。").child(Button::new("take").primary().label("开始截图").on_click(|_,_,cx|app::command(cx,Command::Capture)));
            }
            2 => {
                content=content.child("单击贴图选中并显示边框，失焦后隐藏边框。滚轮缩放；Command / Ctrl + 滚轮调整透明度；1 旋转、2 水平翻转、3 垂直翻转；空格编辑；T 切换穿透；双击隐藏；选中后 Esc 关闭（编辑时取消标注）。也可用 Command+W / Ctrl+W，或右键「关闭贴图」。这些操作仅在贴图内生效。").child(div().h_flex().gap_2().child(Button::new("hide").label("隐藏全部").on_click(|_,_,cx|app::command(cx,Command::HidePins))).child(Button::new("show").label("显示全部").on_click(|_,_,cx|app::command(cx,Command::ShowPins))).child(Button::new("restore").primary().label("恢复全部交互").on_click(|_,_,cx|app::command(cx,Command::RestorePins))));
                content = content
                    .child(row(
                        "选中边框颜色",
                        "#RRGGBB，默认蓝色；未选中时不显示",
                        div()
                            .w_40()
                            .child(ColorPicker::new(&self.pin_border_color).label("边框颜色")),
                    ))
                    .child(row(
                        "选中边框粗细",
                        "0–8 逻辑像素，0 表示隐藏选中边框",
                        div()
                            .w_24()
                            .child(Input::new(&self.pin_border_width).aria_label("贴图边框粗细")),
                    ))
                    .child(row(
                        "边框阴影",
                        "只改变桌面外观，不写入图片",
                        Switch::new("pin-shadow")
                            .checked(self.pin_shadow)
                            .on_click(cx.listener(|v, checked, _, cx| {
                                v.pin_shadow = *checked;
                                cx.notify();
                            })),
                    ));
            }
            3 => {
                content=content.child("框选稳定内容，避开固定页头。选择后自动检测左右或上下滚动，请缓慢、小幅滚动；在区域右下控制栏暂停、继续、自动向下滚动或完成编辑。匹配异常会保留已有结果，可回滚重试。内存预算 256 MiB。").child(Button::new("long").primary().label("选择长截图区域").on_click(|_,_,cx|app::command(cx,Command::LongCapture)));
            }
            4 => {
                content = content.child("框选后使用区域右下控制栏开始、暂停 / 继续、停止保存或取消。控制栏保持可用，并从录制画面中排除。");
                let mut rates = div().h_flex().gap_2();
                for fps in [5, 10, 15, 20, 30] {
                    rates = rates.child(
                        Button::new(("fps", fps as usize))
                            .label(format!("{fps}"))
                            .when(config.gif.fps == fps, |b| b.primary())
                            .on_click(move |_, _, cx| {
                                cx.global_mut::<Runtime>().config.gif.fps = fps;
                                cx.refresh_windows();
                            }),
                    );
                }
                content = content
                    .child(row(
                        "采集帧率",
                        "默认 15 FPS；编码按实际时间戳计算帧延迟",
                        rates,
                    ))
                    .child(row(
                        "最长录制（秒）",
                        "1–600，达到上限后自动完成",
                        div().w_24().child(Input::new(&self.gif_seconds)),
                    ))
                    .child(row(
                        "输出最大宽度",
                        "64–4096 像素，按比例缩小",
                        div().w_24().child(Input::new(&self.gif_width)),
                    ))
                    .child(
                        Button::new("gif")
                            .primary()
                            .label("选择 GIF 录制区域")
                            .on_click(|_, _, cx| app::command(cx, Command::Gif)),
                    );
            }
            5 => {
                content=content.child("使用系统原生离线 OCR。编辑器中点击「识别文字」，在图片上拖选文字后复制；支持跨行选择、全部复制与简单表格 TSV。").child("文字框以原图像素保存。系统识别语言不可用时会显示错误，不会阻塞其他图片操作。").child(Button::new("ocr").primary().label("识别剪贴板图片").on_click(|_,_,cx|app::command(cx,Command::Ocr)));
            }
            6 => {
                content = content
                    .child(row(
                        "鼠标拖尾",
                        "独立透明穿透窗口；支持动画与自定义配色",
                        Switch::new("trail")
                            .checked(!cx.global::<Runtime>().trails.is_empty())
                            .on_click(|_, _, cx| app::command(cx, Command::Trail)),
                    ))
                    .child(row(
                        "宽度",
                        "1–60 逻辑像素",
                        div().w_24().child(Input::new(&self.trail_width)),
                    ))
                    .child(row(
                        "生命周期（毫秒）",
                        "100–3000，静止时自然消失",
                        div().w_24().child(Input::new(&self.trail_life)),
                    ))
                    .child(self.trail_options(cx));
            }
            7 => {
                let registration = match cx.global::<Runtime>().hotkeys.as_ref() {
                    Some(keys) => keys
                        .registration_error()
                        .map(str::to_owned)
                        .unwrap_or_else(|| "快捷键注册正常".into()),
                    None => "快捷键服务暂未就绪，后台每 5 秒自动重试".into(),
                };
                content = content
                    .child(
                        div()
                            .text_sm()
                            .p_3()
                            .rounded_md()
                            .bg(cx.theme().muted)
                            .child(registration),
                    )
                    .child(row(
                        "区域截图",
                        "留空禁用。例：F1、Alt+Shift+S",
                        div().w_48().child(Input::new(&self.screenshot_key)),
                    ))
                    .child(row(
                        "剪贴板贴图",
                        "留空禁用。例：F3、Alt+Shift+P",
                        div().w_48().child(Input::new(&self.paste_key)),
                    ));
                for (name, input) in &self.optional {
                    content = content.child(row(
                        name,
                        "可选全局绑定，默认留空",
                        div().w_48().child(Input::new(input)),
                    ));
                }
                content=content.child(div().text_sm().mt_3().child("macOS 的 F1 / F3 可能执行亮度或 Mission Control。请使用 Fn / Globe，或在键盘设置启用标准功能键，也可以在此改用组合键。"));
            }
            8 => {
                content = content
                    .child(row(
                        "保存目录",
                        "截图自动使用带毫秒时间戳的文件名",
                        div().w(px(300.)).child(Input::new(&self.save_path)),
                    ))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("png")
                                    .label("PNG")
                                    .when(config.format == "png", |b| b.primary())
                                    .on_click(|_, _, cx| {
                                        cx.global_mut::<Runtime>().config.format = "png".into();
                                        cx.refresh_windows();
                                    }),
                            )
                            .child(
                                Button::new("jpg")
                                    .label("JPEG")
                                    .when(config.format == "jpg", |b| b.primary())
                                    .on_click(|_, _, cx| {
                                        cx.global_mut::<Runtime>().config.format = "jpg".into();
                                        cx.refresh_windows();
                                    }),
                            ),
                    );
                if let Ok(entries) = std::fs::read_dir(&config.save_directory) {
                    let mut paths: Vec<_> = entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.extension()
                                .is_some_and(|e| ["png", "jpg", "gif"].iter().any(|s| e == *s))
                        })
                        .collect();
                    paths.sort();
                    paths.reverse();
                    for (i, path) in paths.into_iter().take(30).enumerate() {
                        let label = path.file_name().unwrap().to_string_lossy().to_string();
                        content = content.child(Button::new(("history", i)).label(label).on_click(
                            move |_, _, cx| {
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
                            },
                        ));
                    }
                }
            }
            9 => {
                content = content.child("主题跟随 GPUI Kit。选择后立即应用。");
                for (label, mode) in [("浅色", ThemeMode::Light), ("深色", ThemeMode::Dark)] {
                    content = content.child(Button::new(label).label(label).on_click(
                        move |_, window, cx| {
                            Theme::change(mode, Some(window), cx);
                            cx.global_mut::<Runtime>().config.theme = if mode == ThemeMode::Dark {
                                "dark"
                            } else {
                                "light"
                            }
                            .into();
                        },
                    ));
                }
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
                                cx.notify();
                            }))
                    }),
                );
                content = content.child(row(
                    "更新代理",
                    "默认跟随系统代理；自定义 HTTP 代理用于检查和下载",
                    proxy_modes,
                ));
                if self.proxy_mode == ProxyMode::Http {
                    content = content
                        .child(row(
                            "代理地址",
                            "例如 http://127.0.0.1:7890",
                            div().w_64().child(Input::new(&self.proxy_url)),
                        ))
                        .child(row(
                            "用户名（可选）",
                            "仅用于代理认证",
                            div().w_64().child(Input::new(&self.proxy_username)),
                        ))
                        .child(row(
                            "密码（可选）",
                            "仅本次运行有效，不保存到磁盘",
                            div().w_64().child(Input::new(&self.proxy_password)),
                        ));
                }
                content = content
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("check-updates")
                                    .label("检查更新")
                                    .disabled(self.update_busy)
                                    .on_click(cx.listener(|v, _, _, cx| v.check_updates(cx))),
                            )
                            .when(self.update_release.is_some(), |d| {
                                d.child(
                                    Button::new("download-update")
                                        .label("升级并重启")
                                        .tooltip(
                                            "自动下载、校验并替换；重启会结束当前采集，请先保存",
                                        )
                                        .disabled(self.update_busy)
                                        .on_click(cx.listener(|v, _, _, cx| v.download_update(cx))),
                                )
                            }),
                    )
                    .child(
                        Button::new("github-releases")
                            .ghost()
                            .label("GitHub 发布页")
                            .on_click(|_, _, cx| {
                                cx.open_url("https://github.com/junyi-deep/simple-snip/releases")
                            }),
                    );
                content = content.child(format!("Simple Snip {}", env!("CARGO_PKG_VERSION")))
                    .child("Rust · GPUI Kit 0.8.0 · Apache-2.0")
                    .child(format!("配置：{}",Config::path().display()))
                    .child(if snip_platform::screen_permission() {"屏幕录制权限预检：已授权"} else {"系统预检尚未确认权限；可重新验证当前应用。"})
                    .child(Button::new("permission").label("重新验证屏幕录制权限").on_click(|_,_,cx|app::check_screen_permission(cx)))
                    .when(snip_platform::screen_permission_settings_url().is_some(),|d|d.child(Button::new("permission-settings").label("打开屏幕录制设置").on_click(|_,_,cx| {
                        if let Some(url)=snip_platform::screen_permission_settings_url(){cx.open_url(url);}
                    })))
                    .when(cfg!(target_os = "macos"), |d| d.child("本地 ad-hoc 测试包更新后签名会变化；已开启仍失败时，请在系统设置移除旧条目并重新添加当前应用包。"))
                    .child("本机实际测试状态见 docs/TESTING.md；Windows 需要原生设备独立验收。");
            }
        }
        div()
            .v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(
                div()
                    .h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(nav)
                    .child(content),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("save-settings")
                            .primary()
                            .label("保存设置")
                            .on_click(cx.listener(|this, _, w, cx| this.save(w, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Runtime, Settings};
    use gpui_kit::TestAppContext;
    use gpui_kit::component::WindowExt;
    #[gpui_kit::test]
    fn failed_save_uses_notification_without_writing_configuration(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (tx, _) = async_channel::bounded(8);
        cx.set_global(crate::app::Runtime {
            recordings: vec![],
            config: snip_core::config::Config::default(),
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
        cx.update(|cx| cx.global_mut::<Runtime>().config.gif.max_seconds = 0);
        let (handle, settings) = cx.update(|cx| Settings::build(cx).unwrap());
        cx.update(|cx| {
            handle
                .update(cx, |_, w, cx| {
                    let _ = w.draw(cx);
                    settings.update(cx, |v, cx| v.save(w, cx));
                    assert_eq!(w.notifications(cx).len(), 1);
                    let _ = w.draw(cx);
                })
                .unwrap()
        });
    }
}
