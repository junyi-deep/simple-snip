use crate::{
    hotkeys::HotkeyManager,
    overlay::{CaptureMode, Overlay},
    pin::PinView,
    settings::Settings,
};
use gpui_kit::*;
use raw_window_handle::HasWindowHandle;
use snip_core::{config::Config, geometry::Rect};
use snip_platform::{
    CapturedDisplay, ClipboardProvider, NativeCapture, NativeClipboard, ScreenCapture, WindowStyle,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub enum Command {
    Capture,
    LongCapture,
    Gif,
    Paste,
    Ocr,
    Trail,
    HidePins,
    ShowPins,
    RestorePins,
    History,
    Settings,
    About,
    RecordingPause,
    RecordingStop,
    RecordingCancel,
    Quit,
}
pub enum Event {
    Command(Command),
    Hotkey(u32),
    Captured(
        anyhow::Result<Vec<(CapturedDisplay, Arc<RenderImage>)>>,
        CaptureMode,
    ),
    Selection(CapturedDisplay, Rect, CaptureMode),
    Pin(PreparedImage, Option<Bounds<Pixels>>),
    Edit(Arc<image::RgbaImage>),
    ApplyPin(
        u64,
        snip_core::annotation::Document,
        Arc<image::RgbaImage>,
        snip_core::ocr::Cancellation,
    ),
    ClosePinEditor(u64),
    ClosePin(u64),
    CloseTask(AnyWindowHandle),
    FocusPinToolbar(u64),
    FocusPinCanvas(u64),
    PinNotice(u64, String),
    Status(String),
    ScreenPermissionDenied,
    Saved(anyhow::Result<PathBuf>),
}
/// Pixel conversion is CPU work; prepare it on the capture/clipboard worker.
pub struct PreparedImage {
    pub pixels: Arc<image::RgbaImage>,
    pub texture: Arc<RenderImage>,
}
impl PreparedImage {
    pub fn new(pixels: Arc<image::RgbaImage>) -> Self {
        let texture = texture(&pixels);
        Self { pixels, texture }
    }
}
pub struct PinRecord {
    pub handle: AnyWindowHandle,
    pub view: Entity<PinView>,
}
pub struct Runtime {
    pub recordings: Vec<snip_core::recording::RecordingControl>,
    pub config: Config,
    pub status: String,
    pub tx: async_channel::Sender<Event>,
    pub pins: HashMap<u64, PinRecord>,
    pub next_pin: u64,
    pub overlays: Vec<AnyWindowHandle>,
    pub capture_busy: bool,
    pub permission_prompt: bool,
    pub settings: Option<AnyWindowHandle>,
    pub hotkeys: Option<HotkeyManager>,
    pub tray: Option<gpui_tray::Tray>,
    pub trails: Vec<(AnyWindowHandle, Entity<crate::effects::TrailView>)>,
    pub annotation_toolbar: Option<(AnyWindowHandle, Entity<crate::editor::FloatingToolbar>)>,
    pub capture_task: Option<AnyWindowHandle>,
}
impl Global for Runtime {}
gpui_kit::actions!(
    simple_snip,
    [
        Capture,
        LongCapture,
        Gif,
        Paste,
        Ocr,
        Trail,
        HidePins,
        ShowPins,
        RestorePins,
        History,
        Preferences,
        About,
        RecordingPause,
        RecordingStop,
        RecordingCancel,
        FocusAnnotationTools,
        CancelAnnotation,
        CloseSelectedPin,
        Quit
    ]
);
pub fn send(cx: &App, event: Event) {
    let _ = cx.global::<Runtime>().tx.try_send(event);
}
pub fn command(cx: &App, command: Command) {
    send(cx, Event::Command(command));
}
pub fn status(cx: &mut App, message: impl Into<String>) {
    let message = message.into();
    tracing::info!("{message}");
    cx.global_mut::<Runtime>().status = message;
    cx.refresh_windows();
    refresh_tray(cx);
}
pub fn raw(window: &Window) -> anyhow::Result<raw_window_handle::RawWindowHandle> {
    Ok(HasWindowHandle::window_handle(window)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .as_raw())
}
pub fn apply_style(window: &Window, style: WindowStyle) {
    prepare_window(window);
    if let Err(e) = raw(window).and_then(|h| snip_platform::style_window(h, style)) {
        tracing::error!("window style: {e}");
    }
}
pub fn texture(image: &image::RgbaImage) -> Arc<RenderImage> {
    let mut bgra = image.clone();
    let swizzle = |bytes: &mut [u8]| {
        for pixel in bytes.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    };
    let workers =
        std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).clamp(1, 4));
    if bgra.as_raw().len() >= 8 * 1024 * 1024 && workers > 1 {
        let chunk = (bgra.as_raw().len() / 4).div_ceil(workers) * 4;
        std::thread::scope(|scope| {
            let data: &mut [u8] = bgra.as_mut();
            for bytes in data.chunks_mut(chunk) {
                scope.spawn(move || swizzle(bytes));
            }
        });
    } else {
        swizzle(bgra.as_mut());
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(bgra)]))
}
pub fn resize_pin(w: &mut Window, dimensions: Size<Pixels>, cx: &mut App) {
    #[cfg(target_os = "windows")]
    if raw(w).is_ok() {
        let handle = Window::window_handle(w);
        cx.spawn(async move |cx| {
            if let Ok(Ok((raw, scale))) =
                handle.update(cx, |_, w, _| raw(w).map(|raw| (raw, w.scale_factor())))
                && let Err(error) = snip_platform::resize_borderless(
                    raw,
                    f32::from(dimensions.width),
                    f32::from(dimensions.height),
                    scale,
                )
            {
                tracing::error!("贴图缩放失败: {error}");
            }
        })
        .detach();
        return;
    }
    let _ = cx;
    w.resize(dimensions);
}
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FocusAnnotationTools, Some("ImageAnnotation")),
        KeyBinding::new("escape", CancelAnnotation, Some("AnnotationTools")),
    ]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-w", CloseSelectedPin, Some("PinWindow")),
        KeyBinding::new("cmd-w", CloseSelectedPin, Some("AnnotationTools")),
    ]);
    #[cfg(target_os = "windows")]
    cx.bind_keys([
        KeyBinding::new("ctrl-w", CloseSelectedPin, Some("PinWindow")),
        KeyBinding::new("ctrl-w", CloseSelectedPin, Some("AnnotationTools")),
    ]);
}
pub fn start(cx: &mut App, smoke_test: bool) {
    crate::window_pool::init(cx);
    bind_keys(cx);
    let (config, warning) = if smoke_test {
        (
            Config {
                show_settings_on_start: false,
                ..Config::default()
            },
            None,
        )
    } else {
        Config::load(&Config::path())
    };
    let (tx, rx) = async_channel::bounded(64);
    // A smoke instance can coexist with the user's app without stealing F1/F3
    // or loading/saving their preferences.
    let mut hotkeys = if smoke_test {
        Err("原生窗口测试模式".into())
    } else {
        HotkeyManager::new(tx.clone()).map_err(|e| e.to_string())
    };
    let key_status = match &mut hotkeys {
        Ok(h) => h
            .apply(&config)
            .map(|_| "F1 截图 · F3 贴图 · 菜单栏常驻".to_string())
            .unwrap_or_else(|e| e.to_string()),
        Err(e) => e.clone(),
    };
    tracing::info!("Hotkeys: {key_status}");
    cx.set_global(Runtime {
        recordings: vec![],
        config,
        status: warning.unwrap_or(key_status),
        tx,
        pins: HashMap::new(),
        next_pin: 1,
        overlays: vec![],
        capture_busy: false,
        permission_prompt: false,
        settings: None,
        hotkeys: hotkeys.ok(),
        tray: None,
        trails: vec![],
        annotation_toolbar: None,
        capture_task: None,
    });
    #[cfg(target_os = "windows")]
    if !smoke_test {
        let (sender, wheel) = async_channel::bounded(64);
        match snip_platform::watch_pin_wheel(move |id, delta, opacity| {
            sender.try_send((id, delta, opacity)).is_ok()
        }) {
            Ok(()) => {
                cx.spawn(async move |cx| {
                    while let Ok((id, delta, opacity)) = wheel.recv().await {
                        cx.update(|cx| {
                            if let Some(record) = cx.global::<Runtime>().pins.get(&id) {
                                let (handle, view) = (record.handle, record.view.clone());
                                let _ = handle.update(cx, |_, w, cx| {
                                    view.update(cx, |v, cx| v.wheel(delta, opacity, w, cx))
                                });
                            }
                        });
                    }
                })
                .detach();
            }
            Err(error) => status(cx, format!("全局滚轮不可用：{error}")),
        }
    }
    cx.set_activation_policy(ActivationPolicy::Accessory);
    cx.on_app_quit(|cx| {
        #[cfg(target_os = "windows")]
        snip_platform::stop_pin_wheel();
        let controls = cx.global::<Runtime>().recordings.clone();
        for control in &controls {
            control.cancel();
        }
        cx.global_mut::<Runtime>().hotkeys.take();
        if let Some(tray) = cx.global_mut::<Runtime>().tray.take() {
            let _ = tray.close(cx);
        }
        async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while controls
                .iter()
                .any(|c| !c.done.load(std::sync::atomic::Ordering::Relaxed))
                && std::time::Instant::now() < deadline
            {
                smol::Timer::after(std::time::Duration::from_millis(30)).await;
            }
        }
    })
    .detach();
    macro_rules! action {
        ($ty:ty,$cmd:expr) => {
            cx.on_action(|_: &$ty, cx| command(cx, $cmd));
        };
    }
    action!(Capture, Command::Capture);
    action!(LongCapture, Command::LongCapture);
    action!(Gif, Command::Gif);
    action!(Paste, Command::Paste);
    action!(Ocr, Command::Ocr);
    action!(Trail, Command::Trail);
    action!(HidePins, Command::HidePins);
    action!(ShowPins, Command::ShowPins);
    action!(RestorePins, Command::RestorePins);
    action!(History, Command::History);
    action!(Preferences, Command::Settings);
    action!(About, Command::About);
    action!(RecordingPause, Command::RecordingPause);
    action!(RecordingStop, Command::RecordingStop);
    action!(RecordingCancel, Command::RecordingCancel);
    action!(Quit, Command::Quit);
    let icon = tray_icon();
    match gpui_tray::Tray::builder()
        .icon(icon)
        .tooltip("Simple Snip — F1 截图 / F3 贴图")
        .menu(menu)
        .build(cx)
    {
        Ok(tray) => {
            tracing::info!("Native tray created");
            cx.global_mut::<Runtime>().tray = Some(tray);
        }
        Err(e) => status(cx, format!("托盘创建失败: {e}")),
    }
    cx.spawn(async move |cx| {
        while let Ok(event) = rx.recv().await {
            cx.update(|cx| handle(event, cx));
        }
    })
    .detach();
    let theme = match cx.global::<Runtime>().config.theme.as_str() {
        "light" => Some(gpui_kit::component::ThemeMode::Light),
        "dark" => Some(gpui_kit::component::ThemeMode::Dark),
        _ => None,
    };
    if let Some(theme) = theme {
        gpui_kit::component::Theme::change(theme, None, cx);
    }
    if cx.global::<Runtime>().config.show_settings_on_start {
        open_settings(cx);
    }
    if cx.global::<Runtime>().config.trail.enabled {
        crate::effects::start(cx);
    }
}
fn tray_icon() -> gpui_tray::Icon {
    #[cfg(target_os = "macos")]
    let source = include_bytes!("../assets/icons/tray.png").as_slice();
    #[cfg(target_os = "windows")]
    let source = include_bytes!("../assets/icons/app.png").as_slice();
    let image = image::load_from_memory(source)
        .expect("valid built-in icon")
        .into_rgba8();
    #[cfg(target_os = "macos")]
    let size = 36;
    #[cfg(target_os = "windows")]
    let size = 32;
    let image = image::imageops::resize(&image, size, size, image::imageops::FilterType::Lanczos3);
    let builder = gpui_tray::Icon::from_rgba(image.into_raw(), size, size);
    #[cfg(target_os = "macos")]
    let builder = builder.scale_factor(2.).template(true);
    builder.build().expect("valid built-in icon")
}
fn menu(cx: &mut App) -> Vec<MenuItem> {
    let active = cx
        .global::<Runtime>()
        .recordings
        .iter()
        .any(|c| !c.done.load(std::sync::atomic::Ordering::Relaxed));
    vec![
        MenuItem::action("区域截图    F1", Capture),
        MenuItem::action("长截图…", LongCapture),
        MenuItem::action("GIF 录制…", Gif),
        MenuItem::action("暂停 / 继续当前录制", RecordingPause).disabled(!active),
        MenuItem::action("停止录制并保存", RecordingStop).disabled(!active),
        MenuItem::action("取消当前录制", RecordingCancel).disabled(!active),
        MenuItem::action("剪贴板贴图    F3", Paste),
        MenuItem::action("识别剪贴板图片文字…", Ocr),
        MenuItem::separator(),
        MenuItem::action("隐藏所有贴图", HidePins),
        MenuItem::action("显示所有贴图", ShowPins),
        MenuItem::action("恢复全部贴图交互", RestorePins),
        MenuItem::action("彩虹鼠标拖尾", Trail).checked(!cx.global::<Runtime>().trails.is_empty()),
        MenuItem::separator(),
        MenuItem::action("截图历史…", History),
        MenuItem::action("偏好设置…", Preferences),
        MenuItem::action("关于 Simple Snip", About),
        MenuItem::separator(),
        MenuItem::action("退出 Simple Snip", Quit),
    ]
}
fn handle(event: Event, cx: &mut App) {
    match event {
        Event::Command(cmd) => match cmd {
            Command::Capture => start_capture(CaptureMode::Screenshot, cx),
            Command::LongCapture => start_capture(CaptureMode::Long, cx),
            Command::Gif => start_capture(CaptureMode::Gif, cx),
            Command::Paste | Command::Ocr => {
                let tx = cx.global::<Runtime>().tx.clone();
                std::thread::spawn(move || match NativeClipboard.read_image() {
                    Ok(image) => {
                        let _ = tx.send_blocking(if matches!(cmd, Command::Paste) {
                            Event::Pin(PreparedImage::new(image), None)
                        } else {
                            Event::Edit(image)
                        });
                    }
                    Err(e) => {
                        let _ = tx.send_blocking(Event::Status(e.to_string()));
                    }
                });
            }
            Command::Trail => {
                if cx.global::<Runtime>().trails.is_empty() {
                    crate::effects::start(cx);
                } else {
                    crate::effects::stop(cx);
                }
                let enabled = !cx.global::<Runtime>().trails.is_empty();
                cx.global_mut::<Runtime>().config.trail.enabled = enabled;
                if let Err(e) = cx.global::<Runtime>().config.save(&Config::path()) {
                    status(cx, e.to_string());
                }
            }
            Command::HidePins | Command::ShowPins | Command::RestorePins => {
                let records: Vec<_> = cx
                    .global::<Runtime>()
                    .pins
                    .values()
                    .map(|p| (p.handle, p.view.clone()))
                    .collect();
                for (handle, view) in records {
                    if matches!(cmd, Command::HidePins) {
                        view.update(cx, |v, cx| v.close_editor(cx));
                    }
                    let _ = handle.update(cx, |_, window, cx| {
                        if matches!(cmd, Command::RestorePins) {
                            view.update(cx, |v, cx| {
                                v.click_through = false;
                                v.style(window);
                                cx.notify();
                            });
                        }
                        if let Ok(raw) = raw(window) {
                            let _ =
                                snip_platform::show_window(raw, !matches!(cmd, Command::HidePins));
                        }
                    });
                }
                crate::effects::raise(cx);
            }
            Command::History => {
                open_settings(cx);
                status(
                    cx,
                    format!(
                        "历史文件保存在 {}",
                        cx.global::<Runtime>().config.save_directory.display()
                    ),
                );
            }
            Command::Settings => open_settings(cx),
            Command::About => {
                open_settings(cx);
                status(
                    cx,
                    format!(
                        "Simple Snip {} · Rust + GPUI Kit 0.8 · 图片与 OCR 全程离线",
                        env!("CARGO_PKG_VERSION")
                    ),
                );
            }
            Command::RecordingPause | Command::RecordingStop | Command::RecordingCancel => {
                for control in &cx.global::<Runtime>().recordings {
                    if control.done.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    match cmd {
                        Command::RecordingPause => control
                            .pause(!control.paused.load(std::sync::atomic::Ordering::Relaxed)),
                        Command::RecordingStop => control.finish(),
                        _ => control.cancel(),
                    }
                }
                cx.refresh_windows();
            }
            Command::Quit => {
                close_overlays(cx);
                crate::effects::stop(cx);
                cx.global_mut::<Runtime>().hotkeys.take();
                if let Some(tray) = cx.global_mut::<Runtime>().tray.take() {
                    let _ = tray.close(cx);
                }
                cx.quit();
            }
        },
        Event::Hotkey(id) => {
            tracing::info!(id, "global hotkey received");
            if let Some(cmd) = cx
                .global::<Runtime>()
                .hotkeys
                .as_ref()
                .and_then(|h| h.command(id))
            {
                handle(Event::Command(cmd), cx);
            }
        }
        Event::Captured(result, mode) => {
            let opened_at = std::time::Instant::now();
            cx.global_mut::<Runtime>().capture_busy = false;
            match result {
                Ok(displays) => {
                    for (display, texture) in displays {
                        match Overlay::open_prepared(display, texture, mode, cx) {
                            Ok(handle) => cx.global_mut::<Runtime>().overlays.push(handle),
                            Err(e) => status(cx, e.to_string()),
                        }
                    }
                    tracing::debug!(
                        elapsed_ms = opened_at.elapsed().as_millis(),
                        "capture overlay roots ready"
                    );
                    if cx.global::<Runtime>().overlays.is_empty() {
                        set_trails_visible(cx, true);
                    }
                }
                Err(e) => {
                    set_trails_visible(cx, true);
                    if e.is::<snip_platform::ScreenPermissionDenied>() {
                        screen_permission_dialog(cx);
                    } else {
                        status(cx, format!("截图失败：{e:#}"));
                    }
                }
            }
        }
        Event::Selection(display, rect, mode) => {
            close_overlays(cx);
            match mode {
                CaptureMode::Screenshot => {
                    let (x, y, w, h) =
                        rect.pixel_bounds(display.image.width(), display.image.height());
                    if w > 0 && h > 0 {
                        let image = Arc::new(
                            image::imageops::crop_imm(&*display.image, x, y, w, h).to_image(),
                        );
                        open_editor(image, cx);
                    }
                }
                CaptureMode::Gif => crate::editor::open_recording(display, rect, cx),
                CaptureMode::Long => crate::editor::open_long_capture(display, rect, cx),
            }
        }
        Event::Pin(image, placement) => {
            cx.activate(true);
            let id = cx.global::<Runtime>().next_pin;
            cx.global_mut::<Runtime>().next_pin += 1;
            match PinView::open_prepared(id, image, false, placement, cx) {
                Ok((handle, view)) => {
                    cx.global_mut::<Runtime>()
                        .pins
                        .insert(id, PinRecord { handle, view });
                    status(
                        cx,
                        "已创建独立贴图。滚轮缩放 · 修饰键+滚轮透明度 · 空格编辑 · Esc 关闭",
                    );
                }
                Err(e) => status(cx, e.to_string()),
            }
        }
        Event::ClosePin(id) => close_pin(id, cx),
        Event::CloseTask(handle) => {
            if cx.global::<Runtime>().capture_task == Some(handle) {
                cx.global_mut::<Runtime>().capture_task = None;
            }
            let _ = handle.update(cx, |_, w, cx| {
                crate::window_pool::park(crate::window_pool::Role::Task, w, cx)
            });
            crate::task_toolbar::closed(cx);
        }
        Event::ApplyPin(id, doc, image, cancel) => {
            if cancel.cancelled() {
                return;
            }
            if let Some(view) = cx.global::<Runtime>().pins.get(&id).map(|p| p.view.clone()) {
                view.update(cx, |v, cx| v.apply_document(doc, image, cx));
            }
        }
        Event::PinNotice(id, message) => {
            if let Some(view) = cx.global::<Runtime>().pins.get(&id).map(|p| p.view.clone()) {
                view.update(cx, |v, cx| v.show_notice(message, cx));
            }
        }
        Event::FocusPinCanvas(id) => {
            if let Some(view) = cx.global::<Runtime>().pins.get(&id).map(|p| p.view.clone()) {
                view.update(cx, |v, cx| v.focus_canvas(cx));
            }
        }
        Event::FocusPinToolbar(id) => {
            if let Some(view) = cx.global::<Runtime>().pins.get(&id).map(|p| p.view.clone()) {
                view.update(cx, |v, cx| v.focus_toolbar(cx));
            }
        }
        Event::ClosePinEditor(id) => {
            if let Some(view) = cx.global::<Runtime>().pins.get(&id).map(|p| p.view.clone()) {
                view.update(cx, |v, cx| v.close_editor(cx));
            }
        }
        Event::Edit(image) => open_editor(image, cx),
        Event::ScreenPermissionDenied => screen_permission_dialog(cx),
        Event::Status(s) => status(cx, s),
        Event::Saved(result) => match result {
            Ok(path) => status(cx, format!("已保存 {}", path.display())),
            Err(e) => status(cx, format!("保存失败: {e}")),
        },
    }
}
fn start_capture(mode: CaptureMode, cx: &mut App) {
    if let Some(task) = cx.global::<Runtime>().capture_task {
        let _ = task.update(cx, |_, w, _| {
            if let Ok(raw) = raw(w) {
                let _ = snip_platform::show_window(raw, true);
            }
        });
        status(cx, "请先在当前控制栏完成或关闭采集，再开始新的截图");
        return;
    }
    if cx.global::<Runtime>().capture_busy || !cx.global::<Runtime>().overlays.is_empty() {
        return;
    }
    cx.global_mut::<Runtime>().capture_busy = true;
    set_trails_visible(cx, false);
    let tx = cx.global::<Runtime>().tx.clone();
    let has_trails = !cx.global::<Runtime>().trails.is_empty();
    // Give the compositor time to remove the trail before sampling the desktop.
    cx.spawn(async move |_| {
        if has_trails {
            smol::Timer::after(std::time::Duration::from_millis(
                if cfg!(target_os = "windows") { 35 } else { 70 },
            ))
            .await;
        }
        std::thread::spawn(move || {
            let captured_at = std::time::Instant::now();
            let result = NativeCapture.capture_desktop().map(|displays| {
                tracing::debug!(
                    elapsed_ms = captured_at.elapsed().as_millis(),
                    "desktop pixels captured"
                );
                displays
                    .into_iter()
                    .map(|d| {
                        let converted_at = std::time::Instant::now();
                        let texture = texture(&d.image);
                        tracing::debug!(
                            elapsed_ms = converted_at.elapsed().as_millis(),
                            "capture texture prepared"
                        );
                        (d, texture)
                    })
                    .collect()
            });
            let _ = tx.send_blocking(Event::Captured(result, mode));
        });
    })
    .detach();
}
pub fn close_overlays(cx: &mut App) {
    let overlays = std::mem::take(&mut cx.global_mut::<Runtime>().overlays);
    // Cancel can originate inside this same window's event listener. Defer the
    // root replacement until its mutable window/entity borrow has ended.
    cx.defer(move |cx| {
        for h in overlays {
            if let Err(error) = h.update(cx, |_, w, cx| {
                crate::window_pool::park(crate::window_pool::Role::Capture, w, cx)
            }) {
                tracing::warn!("关闭截图窗口失败: {error}");
            }
        }
        if !cx.global::<Runtime>().capture_busy
            && cx.global::<Runtime>().overlays.is_empty()
            && cx.global::<Runtime>().capture_task.is_none()
            && !cx
                .global::<Runtime>()
                .recordings
                .iter()
                .any(|c| !c.done.load(std::sync::atomic::Ordering::Relaxed))
        {
            set_trails_visible(cx, true);
        }
    });
}
pub fn set_trails_visible(cx: &mut App, show: bool) {
    let handles = cx.global::<Runtime>().trails.clone();
    for (h, view) in handles {
        view.update(cx, |v, cx| v.set_visible(show, cx));
        let _ = h.update(cx, |_, w, _| {
            // Windows' full-screen transparent swap chain can stop presenting
            // after native hide/show. Keep it mapped and paint an empty frame.
            #[cfg(target_os = "macos")]
            if let Ok(raw) = raw(w) {
                let _ = snip_platform::show_window(raw, show);
            }
            w.refresh();
        });
    }
    if show {
        crate::effects::raise(cx);
    }
}
pub(crate) fn close_pin(id: u64, cx: &mut App) {
    if let Some(pin) = cx.global_mut::<Runtime>().pins.remove(&id) {
        crate::editor::hide_toolbar(id, cx);
        let _ = pin.handle.update(cx, |_, w, cx| {
            crate::window_pool::park(crate::window_pool::Role::Pin, w, cx);
        });
        tracing::debug!(pin = id, "pin closed and native window released or pooled");
    }
}

pub fn open_settings(cx: &mut App) {
    cx.activate(true);
    if let Some(h) = cx.global::<Runtime>().settings
        && h.update(cx, |_, w, _| {
            if let Ok(raw) = raw(w) {
                let _ = snip_platform::style_window(
                    raw,
                    WindowStyle {
                        topmost: false,
                        app_window: true,
                        ..Default::default()
                    },
                );
                let _ = snip_platform::show_window(raw, true);
            }
            w.activate_window();
        })
        .is_ok()
    {
        crate::effects::raise(cx);
        return;
    }
    match Settings::open(cx) {
        Ok(h) => cx.global_mut::<Runtime>().settings = Some(h),
        Err(e) => status(cx, e.to_string()),
    }
    crate::effects::raise(cx);
}
pub fn open_editor(image: Arc<image::RgbaImage>, cx: &mut App) {
    cx.activate(true);
    let id = cx.global::<Runtime>().next_pin;
    cx.global_mut::<Runtime>().next_pin += 1;
    match PinView::open(id, image, true, cx) {
        Ok((handle, view)) => {
            cx.global_mut::<Runtime>()
                .pins
                .insert(id, PinRecord { handle, view });
        }
        Err(e) => status(cx, e.to_string()),
    }
}
pub fn save_image(image: Arc<image::RgbaImage>, cx: &App) {
    let directory = cx.global::<Runtime>().config.save_directory.clone();
    let name = format!(
        "Snip_{}.{}",
        chrono::Local::now().format("%Y%m%d_%H%M%S"),
        cx.global::<Runtime>().config.format
    );
    let picker = cx.prompt_for_new_path(&directory, Some(&name));
    cx.spawn(async move |cx| match picker.await {
        Ok(Ok(Some(path))) => cx.update(|cx| {
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(extension.as_str(), "png" | "jpg" | "jpeg") {
                status(cx, "请选择 .png 或 .jpg 文件名");
                return;
            }
            if let Some(parent) = path.parent() {
                cx.global_mut::<Runtime>().config.save_directory = parent.to_path_buf();
                if let Err(error) = cx.global::<Runtime>().config.save(&Config::path()) {
                    status(cx, format!("保存位置记忆失败：{error}"));
                }
            }
            write_image_to(image, path, cx);
        }),
        Ok(Ok(None)) => {}
        _ => cx.update(|cx| status(cx, "无法打开保存对话框")),
    })
    .detach();
}
fn write_image_to(image: Arc<image::RgbaImage>, path: PathBuf, cx: &App) {
    let config = cx.global::<Runtime>().config.clone();
    let tx = cx.global::<Runtime>().tx.clone();
    std::thread::spawn(move || {
        let result =
            (|| -> anyhow::Result<PathBuf> {
                std::fs::create_dir_all(&config.save_directory)?;
                let mut data = std::io::Cursor::new(Vec::new());
                if path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")
                }) {
                    image::DynamicImage::ImageRgba8((*image).clone())
                        .to_rgb8()
                        .write_to(&mut data, image::ImageFormat::Jpeg)?;
                } else {
                    image.write_to(&mut data, image::ImageFormat::Png)?;
                }
                snip_core::config::atomic_write(&path, data.get_ref())?;
                let history_path = snip_core::history::History::path();
                let mut history = snip_core::history::History::load(&history_path);
                history.record(
                    snip_core::history::Entry {
                        path: path.clone(),
                        created: chrono::Local::now().to_rfc3339(),
                    },
                    config.history_limit,
                );
                if let Err(e) = history.save(&history_path) {
                    tracing::warn!("历史索引保存失败: {e}");
                }
                Ok(path)
            })();
        let _ = tx.send_blocking(Event::Saved(result));
    });
}

/// Native integration probe. Does not synthesize input or grant permissions.
/// This verifies window properties and lifecycle, not visual interaction.
pub fn smoke_test(cx: &mut App) {
    cx.spawn(async move |cx| {
        smol::Timer::after(std::time::Duration::from_millis(800)).await;
        let result = async {
            #[cfg(target_os = "windows")]
            for cycle in 0..3 {
                let started = std::time::Instant::now();
                cx.update(|cx| start_capture(CaptureMode::Screenshot, cx));
                loop {
                    if !cx.read_global::<Runtime, _>(|r, _| r.capture_busy) {
                        break;
                    }
                    anyhow::ensure!(started.elapsed().as_secs() < 15, "capture did not finish");
                    smol::Timer::after(std::time::Duration::from_millis(20)).await;
                }
                // Let the real overlay present before cancelling it.
                smol::Timer::after(std::time::Duration::from_millis(100)).await;
                cx.update(|cx| -> anyhow::Result<()> {
                    anyhow::ensure!(!cx.global::<Runtime>().overlays.is_empty(), "capture failed: {}", cx.global::<Runtime>().status);
                    anyhow::ensure!(cx.global::<Runtime>().trails.iter().all(|(_, v)| v.read(cx).is_visible()), "capture-excluded trail disappeared during screenshot");
                    close_overlays(cx);
                    Ok(())
                })?;
                smol::Timer::after(std::time::Duration::from_millis(40)).await;
                cx.update(|cx| -> anyhow::Result<()> {
                    anyhow::ensure!(cx.global::<Runtime>().overlays.is_empty(), "capture overlay retained");
                    anyhow::ensure!(!cx.global::<Runtime>().trails.is_empty(), "no trail windows");
                    for (handle, view) in cx.global::<Runtime>().trails.clone() {
                        anyhow::ensure!(view.read(cx).is_visible(), "trail did not resume");
                        handle.update(cx, |_, w, _| -> anyhow::Result<()> {
                            let raw = raw(w)?;
                            anyhow::ensure!(snip_platform::window_visible(raw)?, "trail swap chain window hidden");
                            let (top, through, _) = snip_platform::inspect_window(raw)?;
                            anyhow::ensure!(top && through, "trail lost topmost/click-through style");
                            snip_platform::cursor_in_window(raw, w.scale_factor())?;
                            Ok(())
                        })??;
                    }
                    Ok(())
                })?;
                tracing::info!(cycle, elapsed_ms = started.elapsed().as_millis(), "SMOKE capture/cancel/trail-visible cycle passed");
            }
            #[cfg(target_os = "windows")]
            {
                let image = smol::unblock(|| PreparedImage::new(Arc::new(image::RgbaImage::from_fn(360, 220, |x, y| image::Rgba([(x % 255) as u8, (y % 255) as u8, 100, 255]))))).await;
                let id = cx.update(|cx| {
                    let id = cx.global::<Runtime>().next_pin;
                    handle(Event::Pin(image, Some(Bounds::new(point(px(100.), px(140.)), size(px(360.), px(220.))))), cx);
                    id
                });
                // Exercise the production pin event and actual presentation,
                // not only create/destroy native handles in one event loop turn.
                smol::Timer::after(std::time::Duration::from_millis(2000)).await;
                cx.update(|cx| -> anyhow::Result<()> {
                    let pin = cx.global::<Runtime>().pins.get(&id).ok_or_else(|| anyhow::anyhow!("pin event failed"))?.handle;
                    let margin = cx.global::<Runtime>().config.pin.margin();
                    pin.update(cx, |_, w, _| -> anyhow::Result<()> {
                        anyhow::ensure!(!snip_platform::window_has_system_buttons(raw(w)?)?, "pin retained native system buttons");
                        let rect = snip_platform::inspect_borderless_rect(raw(w)?)?;
                        let scale = w.scale_factor();
                        anyhow::ensure!((rect.x / scale - (100. - margin)).abs() < 1. && (rect.y / scale - (140. - margin)).abs() < 1.
                            && (rect.width / scale - (360. + margin * 2.)).abs() < 1. && (rect.height / scale - (220. + margin * 2.)).abs() < 1., "pin moved or resized after presentation: {rect:?}");
                        anyhow::ensure!((rect.width / w.scale_factor() - f32::from(w.viewport_size().width)).abs() < 1., "pin client size differs from native surface");
                        Ok(())
                    })??;
                    close_pin(id, cx);
                    Ok(())
                })?;
            }
            let mut previous = None;
            for cycle in 0..3 {
                let (handle, weak) = cx.update(|cx| -> anyhow::Result<_> {
                    anyhow::ensure!(cx.global::<Runtime>().tray.is_some(), "native tray missing");
                    let image = Arc::new(image::RgbaImage::from_fn(360, 220, |x, y| image::Rgba([(x % 255) as u8, (y % 255) as u8, 100, 255])));
                    let (handle, view) = PinView::open(999 + cycle, image, false, cx)?;
                    open_settings(cx);
                    if let Some(settings)=cx.global::<Runtime>().settings {
                        settings.update(cx, |_, w, _| -> anyhow::Result<()> {
                            anyhow::ensure!(!snip_platform::inspect_window(raw(w)?)?.0, "settings must not stay topmost");
                            Ok(())
                        })??;
                        settings.update(cx,|_,w,_| {snip_platform::show_window(raw(w)?,false)})??;
                        open_settings(cx);
                        settings.update(cx,|_,w,_| -> anyhow::Result<()> {anyhow::ensure!(snip_platform::window_visible(raw(w)?)?,"hidden settings did not reappear with a pin present");Ok(())})??;
                    }
                    handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                        let h = raw(window)?;
                        #[cfg(target_os = "windows")]
                        anyhow::ensure!(!snip_platform::window_has_system_buttons(h)?, "pin retained native system buttons");
                        let (top, through, _) = snip_platform::inspect_window(h)?;
                        anyhow::ensure!(top && !through, "pin not interactive/topmost");
                        snip_platform::style_window(h, WindowStyle { click_through: true, opacity: 0.5, ..Default::default() })?;
                        let (top, through, alpha) = snip_platform::inspect_window(h)?;
                        anyhow::ensure!(top && through && (alpha - 0.5).abs() < 0.01, "native style did not apply");
                        view.update(cx, |v, _| { v.click_through = false; v.style(window); });
                        let (_, through, _) = snip_platform::inspect_window(h)?;
                        anyhow::ensure!(!through, "click-through recovery failed");
                        snip_platform::show_window(h, false)?;
                        snip_platform::show_window(h, true)?;
                        crate::window_pool::park(crate::window_pool::Role::Pin, window, cx);
                        Ok(())
                    })??;
                    Ok((handle, view.downgrade()))
                })?;
                #[cfg(target_os = "macos")]
                if let Some(last) = previous { anyhow::ensure!(last == handle, "native pin window was not reused"); }
                previous = Some(handle);
                smol::Timer::after(std::time::Duration::from_millis(100)).await;
                cx.update(|_| -> anyhow::Result<()> { anyhow::ensure!(weak.upgrade().is_none(), "closed pin retained its document"); Ok(()) })?;
            }
            let _ = previous;
            #[cfg(target_os = "macos")]
            for _ in 0..2 {
                let (pin, expected) = cx.update(|cx| -> anyhow::Result<_> {
                    let image = Arc::new(image::RgbaImage::new(1600, 1000));
                    let selected = Bounds::new(point(px(100.), px(140.)), size(px(800.), px(500.)));
                    let margin = cx.global::<Runtime>().config.pin.margin();
                    let (pin, _) = PinView::open_at(1004, image, false, Some(selected), cx)?;
                    Ok((pin, Rect::new(100. - margin, 140. - margin, 800. + margin * 2., 500. + margin * 2.)))
                })?;
                smol::Timer::after(std::time::Duration::from_millis(150)).await;
                cx.update(|cx| -> anyhow::Result<()> {
                    pin.update(cx, |_, w, cx| -> anyhow::Result<()> {
                        let actual = snip_platform::inspect_window_rect(raw(w)?)?;
                        anyhow::ensure!((actual.x - expected.x).abs() < 1. && (actual.y - expected.y).abs() < 1.
                            && (actual.width - expected.width).abs() < 1. && (actual.height - expected.height).abs() < 1.,
                            "native capture pin moved or resized: {actual:?}, expected {expected:?}");
                        crate::window_pool::park(crate::window_pool::Role::Pin, w, cx);
                        Ok(())
                    })??;
                    Ok(())
                })?;
            }
            cx.update(|cx| -> anyhow::Result<()> {
                let image = Arc::new(image::RgbaImage::new(360, 220));
                let (editor, view) = PinView::open(1003, image.clone(), true, cx)?;
                view.update(cx, |v, cx| v.close_editor(cx));
                editor.update(cx, |_, w, cx| crate::window_pool::park(crate::window_pool::Role::Pin, w, cx))?;
                let display = CapturedDisplay { id: 0, map: snip_core::geometry::DisplayMap { desktop: Rect::new(0., 0., 360., 220.), pixel_width: 360, pixel_height: 220 }, image, windows: vec![] };
                let mut previous_overlay = None;
                for _ in 0..3 {
                    let overlay = Overlay::open(display.clone(), CaptureMode::Screenshot, cx)?;
                    #[cfg(target_os = "macos")]
                    if let Some(last) = previous_overlay { anyhow::ensure!(last == overlay, "native capture window was not reused"); }
                    previous_overlay = Some(overlay);
                    overlay.update(cx, |_, w, cx| crate::window_pool::park(crate::window_pool::Role::Capture, w, cx))?;
                }
                let _ = previous_overlay;
                let controls = crate::task_toolbar::open(display, Rect::new(0., 0., 300., 160.), true, cx)?;
                controls.update(cx, |_, w, _| -> anyhow::Result<()> {
                    let (top, through, _) = snip_platform::inspect_window(raw(w)?)?;
                    anyhow::ensure!(top && !through, "capture toolbar must be topmost and interactive");
                    Ok(())
                })??;
                handle(Event::CloseTask(controls), cx);
                anyhow::ensure!(cx.global::<Runtime>().capture_task.is_none(), "closed toolbar retained capture session");
                Ok(())
            })?;
            anyhow::Ok(())
        }.await;
        match result {
            Ok(()) => tracing::info!("SMOKE PASS: tray, transparent/topmost pin, opacity, click-through recovery, three pin and capture lifecycle cycles, native window reuse, pin document release and capture toolbar lifecycle"),
            Err(e) => tracing::error!("SMOKE FAILED: {e:#}"),
        }
        cx.update(|cx| command(cx, Command::Quit));
    }).detach();
}

pub fn display_bounds(display: &CapturedDisplay, cx: &App) -> (Bounds<Pixels>, Option<DisplayId>) {
    if let Some(found) = cx
        .displays()
        .into_iter()
        .find(|d| u64::from(d.id()) as u32 == display.id)
    {
        return (found.bounds(), Some(found.id()));
    }
    let r = display.map.desktop;
    (
        Bounds::new(point(px(r.x), px(r.y)), size(px(r.width), px(r.height))),
        None,
    )
}

pub fn prepare_window(window: &Window) {
    if let Err(e) = raw(window).and_then(snip_platform::prepare_window) {
        tracing::error!("native window setup: {e}");
    }
}

pub fn refresh_tray(cx: &mut App) {
    if let Some(tray) = cx.global::<Runtime>().tray.clone()
        && let Err(e) = tray.refresh_menu(cx)
    {
        tracing::warn!("托盘菜单更新失败: {e}");
    }
}

pub fn check_screen_permission(cx: &mut App) {
    let tx = cx.global::<Runtime>().tx.clone();
    std::thread::spawn(move || {
        let event = match snip_platform::verify_screen_permission() {
            Ok(()) => Event::Status("当前应用的屏幕录制权限验证通过，可以截图".into()),
            Err(e) if e.is::<snip_platform::ScreenPermissionDenied>() => {
                Event::ScreenPermissionDenied
            }
            Err(e) => Event::Status(format!("权限验证失败：{e:#}")),
        };
        let _ = tx.send_blocking(event);
    });
}
fn screen_permission_dialog(cx: &mut App) {
    if cx.global::<Runtime>().permission_prompt {
        return;
    }
    status(cx, "当前应用需要屏幕录制权限");
    open_settings(cx);
    let Some(handle) = cx.global::<Runtime>().settings else {
        return;
    };
    let executable = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let detail = format!(
        "截图仅在本地处理。请在「隐私与安全性 → 屏幕与系统音频录制」中允许当前这份 Simple Snip。\n\n如果开关已开启仍失败，请移除旧条目，再添加当前应用包；测试包更新后 ad-hoc 签名会变化，单纯重启无法修复旧签名的授权。\n\n当前程序：{executable}"
    );
    if let Ok(receiver) = handle.update(cx, |_, w, cx| {
        w.prompt(
            PromptLevel::Warning,
            "允许 Simple Snip 读取屏幕",
            Some(&detail),
            &[
                PromptButton::ok("打开系统设置"),
                PromptButton::cancel("稍后"),
            ],
            cx,
        )
    }) {
        cx.global_mut::<Runtime>().permission_prompt = true;
        cx.spawn(async move |cx| {
            let answer = receiver.await.ok();
            cx.update(|cx| {
                cx.global_mut::<Runtime>().permission_prompt = false;
                if answer == Some(0)
                    && let Some(url) = snip_platform::screen_permission_settings_url()
                {
                    cx.open_url(url);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod texture_tests {
    #[test]
    fn swizzle_preserves_alpha_source_and_odd_chunk_boundaries() {
        for (width, height) in [(3, 1), (2049, 1025)] {
            let source = image::RgbaImage::from_fn(width, height, |x, y| {
                image::Rgba([(x % 251) as u8, (y % 239) as u8, 71, 93])
            });
            let converted = super::texture(&source);
            let bytes = converted.as_bytes(0).unwrap();
            for index in [
                0,
                (width as usize * height as usize) / 2,
                width as usize * height as usize - 1,
            ] {
                let p = source
                    .get_pixel(index as u32 % width, index as u32 / width)
                    .0;
                assert_eq!(&bytes[index * 4..index * 4 + 4], &[p[2], p[1], p[0], p[3]]);
            }
            assert_eq!(source.get_pixel(0, 0).0, [0, 0, 71, 93]);
        }
    }
}
