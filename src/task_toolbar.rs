//! Live capture controls attached to the selected desktop region.
use crate::app::{self, Event, Runtime};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputEvent, InputState},
        menu::*,
        *,
    },
    prelude::*,
    *,
};
use image::RgbaImage;
use snip_core::{
    geometry::{Point as ImagePoint, Rect, place_toolbar},
    ocr::Cancellation,
    recording::{RecordingControl, record_gif},
    stitch::{StitchError, Stitcher},
};
use snip_platform::CapturedDisplay;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

pub fn open(
    display: CapturedDisplay,
    rect: Rect,
    gif: bool,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    build(display, rect, gif, cx).map(|(handle, _)| handle)
}
fn build(
    mut display: CapturedDisplay,
    rect: Rect,
    gif: bool,
    cx: &mut App,
) -> anyhow::Result<(AnyWindowHandle, Entity<TaskToolbar>)> {
    let (bounds, display_id) = app::display_bounds(&display, cx);
    let screen = Rect::new(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    );
    let selected = Rect::new(
        screen.x + rect.x / display.map.pixel_width as f32 * screen.width,
        screen.y + rect.y / display.map.pixel_height as f32 * screen.height,
        rect.width / display.map.pixel_width as f32 * screen.width,
        rect.height / display.map.pixel_height as f32 * screen.height,
    );
    let toolbar = toolbar_placement(selected, screen, gif);
    // A live session needs display metadata, not the original frozen desktop.
    display.image = Arc::new(RgbaImage::new(0, 0));
    let (handle, view) = crate::window_pool::open(
        crate::window_pool::Role::Task,
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(toolbar.x), px(toolbar.y)),
                size(px(toolbar.width), px(toolbar.height)),
            ))),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: true,
            is_resizable: false,
            is_movable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            display_id,
            ..Default::default()
        },
        cx,
        |w, cx| {
            w.set_window_title(if gif {
                "Simple Snip · GIF 控制栏"
            } else {
                "Simple Snip · 长截图控制栏"
            });
            app::apply_style(w, snip_platform::WindowStyle::default());
            w.on_window_should_close(cx, |w, cx| {
                app::send(cx, Event::CloseTask(w.window_handle()));
                false
            });
            cx.new(|cx| {
                let focus = cx.focus_handle();
                focus.focus(w, cx);
                let rate = if gif {
                    cx.global::<Runtime>().config.gif.fps
                } else {
                    450
                };
                let rate_input =
                    cx.new(|cx| InputState::new(w, cx).default_value(rate.to_string()));
                let rate_subscription =
                    cx.subscribe(&rate_input, |v: &mut TaskToolbar, _, _: &InputEvent, cx| {
                        if let Ok(rate) = v.rate_input.read(cx).value().parse::<u32>() {
                            let rate = rate
                                .clamp(if v.gif { 1 } else { 100 }, if v.gif { 30 } else { 5000 });
                            if rate != v.rate && !v.recording {
                                v.rate = rate;
                                cx.notify();
                            }
                        }
                    });
                TaskToolbar {
                    rate_input,
                    _rate_subscription: rate_subscription,
                    focus,
                    rate: if gif {
                        cx.global::<Runtime>().config.gif.fps
                    } else {
                        450
                    },
                    display,
                    rect,
                    gif,
                    handle: w.window_handle(),
                    control: Default::default(),
                    recording: false,
                    message: if gif {
                        "准备就绪；开始后控制栏保持可用"
                    } else {
                        "自动检测左右 / 上下滚动；请小幅、缓慢滚动"
                    }
                    .into(),
                    path: None,
                    temporary: None,
                    preview: None,
                    stitch: None,
                    summary: None,
                    finish_requested: false,
                    busy: false,
                    auto: false,
                    watch: false,
                    generation: 0,
                    empty_frames: 0,
                    cancel: Default::default(),
                    excluded: 0,
                    capture_allowed: true,
                }
            })
        },
    )?;
    if let Err(error) = open_region_frame(selected, display_id, cx) {
        let _ = handle.update(cx, |_, w, cx| {
            crate::window_pool::park(crate::window_pool::Role::Task, w, cx)
        });
        return Err(error);
    }
    cx.global_mut::<Runtime>().capture_task = Some(handle);
    let exclusion = handle.update(cx, |_, w, _| {
        app::raw(w).and_then(snip_platform::exclude_capture_controls)
    })?;
    view.update(cx, |v, cx| {
        match exclusion {
            Ok(id) => v.excluded = id,
            Err(e)
                if toolbar.intersect(selected).width > 0.
                    && toolbar.intersect(selected).height > 0. =>
            {
                v.capture_allowed = false;
                v.message = format!("无法排除控制栏：{e}。请缩小区域，在下方留出控制栏空间。");
            }
            Err(e) => {
                tracing::warn!("capture toolbar exclusion unavailable outside selection: {e}")
            }
        }
        if !gif && v.capture_allowed {
            app::set_trails_visible(cx, false);
            v.monitor(false, cx);
        }
        cx.notify();
    });
    Ok((handle, view))
}

fn toolbar_placement(selected: Rect, screen: Rect, gif: bool) -> Rect {
    let width = if gif { 900. } else { 740. };
    place_toolbar(
        selected,
        screen,
        width,
        if screen.width < width + 16. {
            192.
        } else {
            128.
        },
    )
}

struct TaskToolbar {
    display: CapturedDisplay,
    rect: Rect,
    gif: bool,
    handle: AnyWindowHandle,
    control: RecordingControl,
    focus: FocusHandle,
    rate: u32,
    rate_input: Entity<InputState>,
    _rate_subscription: Subscription,
    recording: bool,
    message: String,
    path: Option<PathBuf>,
    temporary: Option<Arc<tempfile::TempDir>>,
    preview: Option<AnyWindowHandle>,
    stitch: Option<Stitcher>,
    summary: Option<(u32, u32, usize)>,
    finish_requested: bool,
    busy: bool,
    auto: bool,
    watch: bool,
    generation: u64,
    empty_frames: u32,
    cancel: Cancellation,
    excluded: u32,
    capture_allowed: bool,
}
impl Drop for TaskToolbar {
    fn drop(&mut self) {
        self.control.cancel();
        self.cancel.cancel();
    }
}
fn restore_trails(cx: &mut App) {
    let idle = !cx.global::<Runtime>().capture_busy
        && cx.global::<Runtime>().overlays.is_empty()
        && cx.global::<Runtime>().capture_task.is_none()
        && !cx
            .global::<Runtime>()
            .recordings
            .iter()
            .any(|c| !c.done.load(Ordering::Relaxed));
    if idle {
        app::set_trails_visible(cx, true);
    }
}
impl TaskToolbar {
    fn finish_long_capture(&mut self, cx: &mut Context<Self>) {
        self.watch = false;
        self.auto = false;
        self.generation += 1;
        if self.busy {
            self.finish_requested = true;
            self.message = "正在完成当前帧…".into();
        } else if let Some(stitch) = self.stitch.take() {
            app::send(cx, Event::Edit(Arc::new(stitch.image)));
            self.close(cx);
        }
        cx.notify();
    }
    fn close(&mut self, cx: &mut Context<Self>) {
        self.close_preview(cx);
        self.control.cancel();
        self.cancel.cancel();
        self.watch = false;
        self.auto = false;
        self.generation += 1;
        app::send(cx, Event::CloseTask(self.handle));
    }
    fn close_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.preview.take() {
            if cx
                .try_global::<PreviewWindow>()
                .is_some_and(|v| v.0 == Some(handle))
            {
                cx.global_mut::<PreviewWindow>().0 = None;
            }
            let _ = handle.update(cx, |_, w, cx| {
                crate::window_pool::park(crate::window_pool::Role::Preview, w, cx)
            });
            cx.notify();
        }
    }
    fn preview_gif(&mut self, cx: &mut Context<Self>) {
        if self.preview.is_some() {
            self.close_preview(cx);
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        match open_preview(path, self.temporary.clone(), &self.display, self.rect, cx) {
            Ok(handle) => self.preview = Some(handle),
            Err(error) => self.message = format!("预览失败：{error}"),
        }
        cx.notify();
    }
    fn save_gif(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.path.clone() else {
            return;
        };
        let temporary = self.temporary.clone();
        let directory = cx.global::<Runtime>().config.save_directory.clone();
        let picker = cx.prompt_for_new_path(
            &directory,
            Some(&format!(
                "Snip_{}.gif",
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            )),
        );
        cx.spawn(async move |this, cx| {
            let _keep_temporary = temporary;
            let Ok(Ok(Some(mut path))) = picker.await else {
                return;
            };
            if path.extension().is_none() {
                path.set_extension("gif");
            }
            if !path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("gif"))
            {
                let _ = this.update(cx, |v, cx| {
                    v.message = "请选择 .gif 文件".into();
                    cx.notify();
                });
                return;
            }
            let destination = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { save_gif_file(&source, &destination) })
                .await;
            if result.is_ok() {
                cx.update(|cx| {
                    if let Some(parent) = path.parent() {
                        cx.global_mut::<Runtime>().config.save_directory = parent.to_path_buf();
                        let _ = cx
                            .global::<Runtime>()
                            .config
                            .save(&snip_core::config::Config::path());
                    }
                });
            }
            let _ = this.update(cx, |v, cx| {
                v.message = match result {
                    Ok(()) => format!("GIF 已保存：{}", path.display()),
                    Err(e) => format!("保存失败：{e}"),
                };
                cx.notify();
            });
        })
        .detach();
    }
    fn sync_rate(&self, w: &mut Window, cx: &mut Context<Self>) {
        self.rate_input
            .update(cx, |v, cx| v.set_value(self.rate.to_string(), w, cx));
    }
    fn start_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording || !self.capture_allowed {
            return;
        }
        if cx
            .global::<Runtime>()
            .recordings
            .iter()
            .any(|c| !c.done.load(Ordering::Relaxed))
        {
            self.message = "已有录制任务，请先停止它".into();
            cx.notify();
            return;
        }
        self.close_preview(cx);
        let temporary = match tempfile::tempdir() {
            Ok(dir) => Arc::new(dir),
            Err(e) => {
                self.message = format!("无法创建录制临时目录：{e}");
                cx.notify();
                return;
            }
        };
        self.temporary = Some(temporary.clone());
        self.control = Default::default();
        cx.global_mut::<Runtime>()
            .recordings
            .retain(|c| !c.done.load(Ordering::Relaxed));
        cx.global_mut::<Runtime>()
            .recordings
            .push(self.control.clone());
        self.recording = true;
        self.path = None;
        app::set_trails_visible(cx, false);
        app::refresh_tray(cx);
        let mut config = cx.global::<Runtime>().config.clone();
        config.gif.fps = self.rate;
        config.save_directory = temporary.path().to_path_buf();
        let (display, rect, excluded) = (self.display.id, self.rect, self.excluded);
        let control = self.control.clone();
        let timer = control.clone();
        cx.spawn(async move |this, cx| {
            while !timer.done.load(Ordering::Relaxed) {
                smol::Timer::after(Duration::from_millis(250)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let _keep_temporary = temporary;
            for remaining in (1..=config.gif.countdown).rev() {
                if control.stop.load(Ordering::Relaxed) {
                    break;
                }
                let _ = this.update(cx, |v, cx| {
                    v.message = format!("{remaining} 秒后开始");
                    cx.notify();
                });
                smol::Timer::after(Duration::from_secs(1)).await;
            }
            if control.stop.load(Ordering::Relaxed) {
                control.done.store(true, Ordering::Relaxed);
                cx.update(|cx| {
                    restore_trails(cx);
                    app::refresh_tray(cx);
                });
                let _ = this.update(cx, |v, cx| {
                    v.recording = false;
                    v.message = "已取消录制".into();
                    cx.notify();
                });
                return;
            }
            let path = config.save_directory.join(format!(
                "Snip_{}.gif",
                chrono::Local::now().format("%Y%m%d_%H%M%S_%3f")
            ));
            let directory = config.save_directory;
            let directory_result = cx
                .background_executor()
                .spawn(async move { std::fs::create_dir_all(directory) })
                .await;
            let (tx, rx) = async_channel::bounded(1);
            if let Err(error) = directory_result {
                control.done.store(true, Ordering::Relaxed);
                let _ = tx.send(Err(error.into())).await;
            } else {
                let _ = this.update(cx, |v, cx| {
                    v.message = "录制中；控制栏不会进入画面".into();
                    cx.notify();
                });
                record_gif(
                    config.gif,
                    path,
                    control,
                    move || snip_platform::capture_region_excluding(display, rect, excluded),
                    move |result| {
                        let _ = tx.send_blocking(result);
                    },
                );
            }
            if let Ok(result) = rx.recv().await {
                cx.update(|cx| {
                    restore_trails(cx);
                    app::refresh_tray(cx);
                });
                let _ = this.update(cx, |v, cx| {
                    v.recording = false;
                    match result {
                        Ok(path) => {
                            v.message = "录制已停止，临时文件就绪；可预览或另存为".into();
                            v.path = Some(path);
                        }
                        Err(e) => {
                            if e.is::<snip_platform::ScreenPermissionDenied>() {
                                app::send(cx, Event::ScreenPermissionDenied);
                            }
                            v.message = format!("录制失败：{e}");
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }
    fn sample(&mut self, cx: &mut Context<Self>) {
        if self.busy || !self.capture_allowed || self.cancel.cancelled() {
            return;
        }
        self.busy = true;
        let (display, rect, excluded) = (self.display.id, self.rect, self.excluded);
        let mut stitch = self.stitch.take();
        let cancel = self.cancel.clone();
        let worker = cx.background_executor().spawn(async move {
            let result = (|| -> anyhow::Result<bool> {
                anyhow::ensure!(!cancel.cancelled(), "任务已取消");
                let image = snip_platform::capture_region_excluding(display, rect, excluded)?;
                anyhow::ensure!(!cancel.cancelled(), "任务已取消");
                if let Some(s) = &mut stitch {
                    s.append(&image)
                } else {
                    stitch = Some(Stitcher::new(image, 256 * 1024 * 1024)?);
                    Ok(true)
                }
            })();
            (stitch, result)
        });
        cx.spawn(async move |this, cx| {
            let (stitch, result) = worker.await;
            let _ = this.update(cx, |v, cx| {
                v.busy = false;
                v.stitch = stitch;
                v.summary = v
                    .stitch
                    .as_ref()
                    .map(|s| (s.image.width(), s.image.height(), s.frames));
                match result {
                    Ok(changed) => {
                        let motion = v
                            .stitch
                            .as_ref()
                            .and_then(|s| s.last_motion)
                            .unwrap_or((0, 0));
                        v.empty_frames = if changed || motion != (0, 0) {
                            0
                        } else {
                            v.empty_frames + 1
                        };
                        let direction = match motion {
                            (x, _) if x > 0 => "向右",
                            (x, _) if x < 0 => "向左",
                            (_, y) if y > 0 => "向下",
                            (_, y) if y < 0 => "向上",
                            _ => "等待滚动",
                        };
                        v.message = if changed {
                            format!("已拼接 · {direction}；换方向后未扫过的区域保留透明")
                        } else {
                            format!("{direction} · 已有内容保留")
                        };
                        if v.auto && v.empty_frames >= 3 {
                            v.auto = false;
                            v.watch = false;
                            v.message = "连续未变化，自动滚动已停止；可完成导出".into();
                        }
                    }
                    Err(e) => {
                        let retryable =
                            matches!(e.downcast_ref::<StitchError>(), Some(StitchError::NoMatch))
                                && v.watch;
                        if !retryable {
                            v.auto = false;
                            v.watch = false;
                        }
                        if e.is::<snip_platform::ScreenPermissionDenied>() {
                            app::send(cx, Event::ScreenPermissionDenied);
                        }
                        v.message = e.to_string();
                    }
                }
                if v.finish_requested {
                    v.finish_requested = false;
                    v.finish_long_capture(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn monitor(&mut self, automatic: bool, cx: &mut Context<Self>) {
        let active = if automatic { self.auto } else { self.watch };
        self.generation += 1;
        self.auto = false;
        self.watch = false;
        if active {
            self.message = "已暂停；可继续或导出已有结果".into();
            cx.notify();
            return;
        }
        if automatic {
            self.auto = true;
        } else {
            self.watch = true;
        }
        self.empty_frames = 0;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            for _ in 0..600 {
                let active = this
                    .update(cx, |v, cx| {
                        if v.generation != generation
                            || (!v.auto && !v.watch)
                            || v.cancel.cancelled()
                        {
                            return false;
                        }
                        if started.elapsed() >= Duration::from_secs(300)
                            || v.stitch.as_ref().is_some_and(|s| s.frames >= 600)
                        {
                            v.auto = false;
                            v.watch = false;
                            v.message = "已达到采集时长 / 帧数上限，请完成导出".into();
                            cx.notify();
                            return false;
                        }
                        v.sample(cx);
                        true
                    })
                    .unwrap_or(false);
                if !active {
                    break;
                }
                let interval = this.read_with(cx, |v, _| v.rate).unwrap_or(450);
                smol::Timer::after(Duration::from_millis(interval as u64)).await;
                if automatic {
                    let target = this
                        .update(cx, |v, _| {
                            (v.generation == generation
                                && v.auto
                                && !v.busy
                                && !v.cancel.cancelled())
                            .then(|| {
                                v.display.map.to_desktop(ImagePoint::new(
                                    v.rect.x + v.rect.width / 2.,
                                    v.rect.y + v.rect.height / 2.,
                                ))
                            })
                        })
                        .ok()
                        .flatten();
                    if let Some(target) = target {
                        let result = cx
                            .background_executor()
                            .spawn(async move { snip_platform::scroll_at(target) })
                            .await;
                        if let Err(e) = result {
                            let _ = this.update(cx, |v, cx| {
                                v.auto = false;
                                v.watch = false;
                                v.message = e.to_string();
                                cx.notify();
                            });
                            break;
                        }
                        smol::Timer::after(Duration::from_millis(250)).await;
                    }
                }
            }
            let _ = this.update(cx, |v, cx| {
                if v.generation == generation && (v.watch || v.auto) {
                    v.watch = false;
                    v.auto = false;
                    v.message = "本轮监测已结束；可继续或导出".into();
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}
impl Render for TaskToolbar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = if self.gif {
            format!(
                "GIF · {} × {} · {:.1} 秒",
                self.rect.width as u32,
                self.rect.height as u32,
                self.control.elapsed_ms.load(Ordering::Relaxed) as f64 / 1000.
            )
        } else if let Some((width, height, frames)) = self.summary {
            format!(
                "长截图 · {} × {} · {} 段{}",
                width,
                height,
                frames,
                if self.busy { " · 采集中" } else { "" }
            )
        } else if self.busy {
            "长截图 · 正在采集首帧".into()
        } else {
            "长截图 · 等待采集".into()
        };
        let mut controls = div().h_flex().gap_1().flex_wrap();
        let display = self.display.clone();
        let rect = self.rect;
        let handle = self.handle;
        controls = controls.child(
            Button::new("session-mode")
                .small()
                .label(if self.gif { "GIF ▾" } else { "长截图 ▾" })
                .disabled(self.recording)
                .dropdown_menu(move |menu, _, _| {
                    let other = display.clone();
                    let gif_display = display.clone();
                    menu.item(PopupMenuItem::new("长截图 [L]").on_click(move |_, _, cx| {
                        app::send(cx, Event::CloseTask(handle));
                        app::send(
                            cx,
                            Event::Selection(
                                other.clone(),
                                rect,
                                crate::overlay::CaptureMode::Long,
                            ),
                        );
                    }))
                    .item(
                        PopupMenuItem::new("录制 GIF [G]").on_click(move |_, _, cx| {
                            app::send(cx, Event::CloseTask(handle));
                            app::send(
                                cx,
                                Event::Selection(
                                    gif_display.clone(),
                                    rect,
                                    crate::overlay::CaptureMode::Gif,
                                ),
                            );
                        }),
                    )
                    .item(PopupMenuItem::new("重新截图 F1").on_click(move |_, _, cx| {
                        app::send(cx, Event::CloseTask(handle));
                        app::command(cx, app::Command::Capture);
                    }))
                }),
        );
        controls = controls
            .child(
                Button::new("rate-minus")
                    .small()
                    .label("−")
                    .disabled(self.recording)
                    .on_click(cx.listener(|v, _, w, cx| {
                        v.rate = v
                            .rate
                            .saturating_sub(if v.gif { 1 } else { 100 })
                            .max(if v.gif { 1 } else { 100 });
                        v.sync_rate(w, cx);
                        cx.notify();
                    })),
            )
            .child(
                div().w(px(72.)).child(
                    Input::new(&self.rate_input)
                        .small()
                        .readonly(self.recording)
                        .aria_label(if self.gif {
                            "录制帧率 FPS"
                        } else {
                            "采集间隔毫秒"
                        }),
                ),
            )
            .child(
                Button::new("rate-plus")
                    .small()
                    .label("+")
                    .disabled(self.recording)
                    .on_click(cx.listener(|v, _, w, cx| {
                        v.rate = (v.rate + if v.gif { 1 } else { 100 }).min(if v.gif {
                            30
                        } else {
                            5000
                        });
                        v.sync_rate(w, cx);
                        cx.notify();
                    })),
            );
        if self.gif {
            controls = controls
                .child(
                    Button::new("record-start")
                        .small()
                        .primary()
                        .label("开始 [1]")
                        .disabled(self.recording || !self.capture_allowed)
                        .on_click(cx.listener(|v, _, _, cx| v.start_recording(cx))),
                )
                .child(
                    Button::new("record-pause")
                        .small()
                        .label(if self.control.paused.load(Ordering::Relaxed) {
                            "继续 [2]"
                        } else {
                            "暂停 [2]"
                        })
                        .disabled(!self.recording)
                        .on_click(cx.listener(|v, _, _, cx| {
                            let paused = !v.control.paused.load(Ordering::Relaxed);
                            v.control.pause(paused);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("record-stop")
                        .small()
                        .label("停止 [3]")
                        .disabled(!self.recording)
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.control.finish();
                            v.message = "正在完成编码…".into();
                            cx.notify();
                        })),
                );
            if self.path.is_some() {
                controls = controls
                    .child(
                        Button::new("record-preview")
                            .small()
                            .label(if self.preview.is_some() {
                                "关闭预览 [4]"
                            } else {
                                "预览 [4]"
                            })
                            .on_click(cx.listener(|v, _, _, cx| v.preview_gif(cx))),
                    )
                    .child(
                        Button::new("record-save")
                            .small()
                            .label("保存 [5]")
                            .on_click(cx.listener(|v, _, _, cx| v.save_gif(cx))),
                    );
            }
        } else {
            controls = controls
                .child(
                    Button::new("scroll-monitor")
                        .small()
                        .primary()
                        .label(if self.watch {
                            "暂停检测 [1]"
                        } else {
                            "继续检测 [1]"
                        })
                        .disabled(!self.capture_allowed)
                        .on_click(cx.listener(|v, _, _, cx| v.monitor(false, cx))),
                )
                .child(
                    Button::new("scroll-auto")
                        .small()
                        .label(if self.auto {
                            "停止自动滚动 [2]"
                        } else {
                            "自动向下滚动 [2]"
                        })
                        .disabled(!self.capture_allowed)
                        .on_click(cx.listener(|v, _, _, cx| v.monitor(true, cx))),
                )
                .child(
                    Button::new("scroll-sample")
                        .small()
                        .label("采一帧 [3]")
                        .disabled(self.busy || !self.capture_allowed)
                        .on_click(cx.listener(|v, _, _, cx| v.sample(cx))),
                )
                .child(
                    Button::new("scroll-finish")
                        .small()
                        .label(if self.finish_requested {
                            "完成中…"
                        } else {
                            "完成 / 编辑 [4]"
                        })
                        .disabled(self.summary.is_none() || self.finish_requested)
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.finish_long_capture(cx);
                        })),
                );
        }
        controls = controls.child(
            Button::new("capture-task-close")
                .small()
                .label(if self.recording {
                    "取消录制 Esc"
                } else {
                    "关闭 Esc"
                })
                .on_click(cx.listener(|v, _, _, cx| v.close(cx))),
        );
        div()
            .id("capture-controls")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                if v.rate_input.read(cx).focus_handle(cx).is_focused(w)
                    && e.keystroke.key != "escape"
                {
                    if e.keystroke.key == "enter" {
                        v.focus.focus(w, cx);
                        cx.stop_propagation();
                    }
                    return;
                }
                match e.keystroke.key.as_str() {
                    "l" | "g" if !v.recording => {
                        app::send(cx, Event::CloseTask(v.handle));
                        app::send(
                            cx,
                            Event::Selection(
                                v.display.clone(),
                                v.rect,
                                if e.keystroke.key == "l" {
                                    crate::overlay::CaptureMode::Long
                                } else {
                                    crate::overlay::CaptureMode::Gif
                                },
                            ),
                        );
                    }
                    "escape" => v.close(cx),
                    "1" if v.gif => v.start_recording(cx),
                    "2" if v.gif && v.recording => {
                        v.control.pause(!v.control.paused.load(Ordering::Relaxed))
                    }
                    "3" if v.gif && v.recording => v.control.finish(),
                    "4" if v.gif && !v.recording => v.preview_gif(cx),
                    "5" if v.gif && !v.recording => v.save_gif(cx),
                    "1" if !v.gif => v.monitor(false, cx),
                    "2" if !v.gif => v.monitor(true, cx),
                    "3" if !v.gif => v.sample(cx),
                    "4" if !v.gif => v.finish_long_capture(cx),
                    _ => {}
                }
                cx.notify();
            }))
            .size_full()
            .min_w_0()
            .v_flex()
            .gap_1()
            .p_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .text_xs()
            .whitespace_normal()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(status))
            .child(controls)
            .child(
                div()
                    .id("capture-task-status")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.message.clone()),
            )
    }
}

struct PreviewWindow(Option<AnyWindowHandle>);
impl Global for PreviewWindow {}
pub fn closed(cx: &mut App) {
    if let Some(handle) = cx.try_global::<PreviewWindow>().and_then(|v| v.0) {
        cx.global_mut::<PreviewWindow>().0 = None;
        let _ = handle.update(cx, |_, w, cx| {
            crate::window_pool::park(crate::window_pool::Role::Preview, w, cx)
        });
    }
    if let Some(frame) = cx.try_global::<CaptureFrame>().and_then(|f| f.0) {
        cx.global_mut::<CaptureFrame>().0 = None;
        let _ = frame.update(cx, |_, w, cx| {
            crate::window_pool::park(crate::window_pool::Role::Task, w, cx)
        });
    }
    restore_trails(cx);
}

fn save_gif_file(source: &std::path::Path, destination: &std::path::Path) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("保存目录无效"))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    let mut input = std::fs::File::open(source)?;
    std::io::copy(&mut input, &mut staged)?;
    staged.flush()?;
    staged.as_file().sync_all()?;
    staged.persist(destination).map_err(|e| e.error)?;
    Ok(())
}
struct GifPreview {
    texture: Option<Arc<RenderImage>>,
    message: String,
    stopped: Arc<std::sync::atomic::AtomicBool>,
    receiver: async_channel::Receiver<PreviewFrame>,
    _temporary: Option<Arc<tempfile::TempDir>>,
}
impl Drop for GifPreview {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // Wake a producer blocked on a full queue even if the current GIF
        // frame has a long delay and the playback task is still sleeping.
        self.receiver.close();
    }
}
impl Render for GifPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x202020))
            .text_color(rgb(0xffffff))
            .when_some(self.texture.clone(), |d, t| {
                d.child(img(t).size_full().object_fit(ObjectFit::Fill))
            })
            .when(!self.message.is_empty(), |d| d.child(self.message.clone()))
    }
}
type PreviewFrame = Result<(Arc<RenderImage>, Duration), String>;
fn preview_stream(
    path: PathBuf,
    temporary: Option<Arc<tempfile::TempDir>>,
) -> (
    async_channel::Receiver<PreviewFrame>,
    Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = stopped.clone();
    let (tx, rx) = async_channel::bounded(1);
    let keep = temporary.clone();
    let worker = std::thread::spawn(move || {
        use image::AnimationDecoder;
        let _keep = keep;
        while !stop.load(Ordering::Relaxed) && !tx.is_closed() {
            let result = (|| -> anyhow::Result<()> {
                let reader = std::io::BufReader::new(std::fs::File::open(&path)?);
                let decoder = image::codecs::gif::GifDecoder::new(reader)?;
                let mut count = 0;
                for frame in decoder.into_frames() {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let frame = frame?;
                    let (n, d) = frame.delay().numer_denom_ms();
                    let delay = Duration::from_millis((u64::from(n) / u64::from(d.max(1))).max(10));
                    let texture = app::texture(frame.buffer());
                    count += 1;
                    if tx.send_blocking(Ok((texture, delay))).is_err() {
                        return Ok(());
                    }
                }
                anyhow::ensure!(count > 0, "GIF 没有帧");
                Ok(())
            })();
            if let Err(error) = result {
                let _ = tx.send_blocking(Err(error.to_string()));
                break;
            }
        }
    });
    (rx, stopped, worker)
}
fn open_preview(
    path: PathBuf,
    temporary: Option<Arc<tempfile::TempDir>>,
    display: &CapturedDisplay,
    rect: Rect,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let (screen, display_id) = app::display_bounds(display, cx);
    let bounds = Bounds::new(
        screen.origin
            + point(
                px(rect.x / display.map.pixel_width as f32 * f32::from(screen.size.width)),
                px(rect.y / display.map.pixel_height as f32 * f32::from(screen.size.height)),
            ),
        size(
            px(rect.width / display.map.pixel_width as f32 * f32::from(screen.size.width)),
            px(rect.height / display.map.pixel_height as f32 * f32::from(screen.size.height)),
        ),
    );
    let (rx, stopped, _worker) = preview_stream(path, temporary.clone());
    let (handle, _) = crate::window_pool::open(
        crate::window_pool::Role::Preview,
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            display_id,
            is_resizable: false,
            is_movable: false,
            ..Default::default()
        },
        cx,
        |w, cx| {
            app::apply_style(
                w,
                snip_platform::WindowStyle {
                    no_activate: true,
                    ..Default::default()
                },
            );
            #[cfg(target_os = "windows")]
            if let Ok(raw) = app::raw(w) {
                let _ = snip_platform::configure_borderless(
                    raw,
                    Rect::new(
                        f32::from(bounds.origin.x),
                        f32::from(bounds.origin.y),
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                    ),
                    w.scale_factor(),
                );
            }
            cx.new(|cx| {
                let receiver = rx.clone();
                cx.spawn(async move |this, cx| {
                    while let Ok(result) = rx.recv().await {
                        let delay = match result {
                            Ok((texture, delay)) => {
                                if this
                                    .update(cx, |v: &mut GifPreview, cx| {
                                        v.texture = Some(texture);
                                        cx.notify();
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                                delay
                            }
                            Err(error) => {
                                let _ = this.update(cx, |v: &mut GifPreview, cx| {
                                    v.message = error;
                                    cx.notify();
                                });
                                break;
                            }
                        };
                        cx.background_executor().timer(delay).await;
                    }
                })
                .detach();
                GifPreview {
                    texture: None,
                    message: String::new(),
                    stopped,
                    receiver,
                    _temporary: temporary,
                }
            })
        },
    )?;
    cx.set_global(PreviewWindow(Some(handle)));
    Ok(handle)
}

struct CaptureFrame(Option<AnyWindowHandle>);
impl Global for CaptureFrame {}
struct RegionFrame;
impl Render for RegionFrame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .border_2()
            .border_color(rgb(0x28d4de))
            .bg(transparent_black())
    }
}
fn open_region_frame(
    region: Rect,
    display_id: Option<DisplayId>,
    cx: &mut App,
) -> anyhow::Result<()> {
    // Paint entirely outside the source rectangle; even backends that only
    // exclude one control window never record this border in the selected area.
    let (handle, _) = crate::window_pool::open(
        crate::window_pool::Role::Task,
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(region.x - 2.), px(region.y - 2.)),
                size(px(region.width + 4.), px(region.height + 4.)),
            ))),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            is_resizable: false,
            is_movable: false,
            is_minimizable: false,
            display_id,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        },
        cx,
        |w, cx| {
            app::apply_style(
                w,
                snip_platform::WindowStyle {
                    click_through: true,
                    no_activate: true,
                    ..Default::default()
                },
            );
            #[cfg(target_os = "windows")]
            if let Ok(raw) = app::raw(w) {
                let _ = snip_platform::exclude_capture_controls(raw);
            }
            cx.new(|_| RegionFrame)
        },
    )?;
    cx.set_global(CaptureFrame(Some(handle)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::toolbar_placement;
    use snip_core::geometry::Rect;
    #[test]
    fn gif_save_is_explicit_atomic_and_preserves_temporary_recording() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("recording.gif");
        let destination = temp.path().join("saved.gif");
        std::fs::write(&source, b"temporary recording bytes").unwrap();
        assert!(!destination.exists());
        super::save_gif_file(&source, &destination).unwrap();
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&destination).unwrap()
        );
        std::fs::write(&destination, b"old saved data").unwrap();
        super::save_gif_file(&source, &destination).unwrap();
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"temporary recording bytes"
        );
    }

    #[test]
    fn preview_stream_decodes_frames_with_timing_and_stops_on_close() {
        use image::{Delay, Frame, Rgba, RgbaImage};
        let temporary = std::sync::Arc::new(tempfile::tempdir().unwrap());
        let path = temporary.path().join("preview.gif");
        {
            let mut encoder =
                image::codecs::gif::GifEncoder::new(std::fs::File::create(&path).unwrap());
            for (color, delay) in [([255, 0, 0, 255], 40), ([0, 0, 255, 255], 80)] {
                encoder
                    .encode_frame(Frame::from_parts(
                        RgbaImage::from_pixel(4, 4, Rgba(color)),
                        0,
                        0,
                        Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .unwrap();
            }
        }
        let (receiver, stopped, worker) = super::preview_stream(path, Some(temporary));
        let (first, delay) = receiver.recv_blocking().unwrap().unwrap();
        assert_eq!(delay.as_millis(), 40);
        assert_eq!(&first.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
        let (second, delay) = receiver.recv_blocking().unwrap().unwrap();
        assert_eq!(delay.as_millis(), 80);
        assert_eq!(&second.as_bytes(0).unwrap()[..4], &[255, 0, 0, 255]);
        stopped.store(true, std::sync::atomic::Ordering::Relaxed);
        receiver.close();
        worker.join().unwrap();
    }

    #[gpui_kit::test]
    fn gif_shortcuts_pause_and_stop_without_saving(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(crate::window_pool::init);
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
        let (handle, view) = cx.update(|cx| {
            super::build(
                snip_platform::CapturedDisplay {
                    id: 1,
                    image: std::sync::Arc::new(image::RgbaImage::new(800, 600)),
                    map: snip_core::geometry::DisplayMap {
                        desktop: Rect::new(0., 0., 800., 600.),
                        pixel_width: 800,
                        pixel_height: 600,
                    },
                    windows: vec![],
                },
                Rect::new(20., 20., 200., 100.),
                true,
                cx,
            )
            .unwrap()
        });
        cx.update(|cx| {
            handle
                .update(cx, |_, w, cx| {
                    view.update(cx, |v, _| v.recording = true);
                    let _ = w.draw(cx);
                })
                .unwrap()
        });
        cx.simulate_keystrokes(handle, "2");
        cx.update(|cx| {
            assert!(
                view.read(cx)
                    .control
                    .paused
                    .load(std::sync::atomic::Ordering::Relaxed)
            )
        });
        cx.simulate_keystrokes(handle, "3");
        cx.update(|cx| {
            let v = view.read(cx);
            assert!(v.control.stop.load(std::sync::atomic::Ordering::Relaxed));
            assert!(v.path.is_none());
        });
    }

    #[test]
    fn controls_attach_to_region_right_edge_and_move_inside_at_screen_bottom() {
        let screen = Rect::new(-1440., 0., 1440., 900.);
        let region = Rect::new(-1000., 100., 600., 400.);
        let controls = toolbar_placement(region, screen, false);
        assert_eq!(controls.right(), region.right());
        assert!(controls.y > region.bottom());
        let region = Rect::new(-1000., 400., 600., 490.);
        let controls = toolbar_placement(region, screen, true);
        assert_eq!(controls.right(), region.right());
        assert!(controls.bottom() <= region.bottom());
        assert_eq!(controls.intersect(screen), controls);
    }
    #[test]
    fn controls_fit_narrow_displays_without_cutting_off_wrapped_rows() {
        let screen = Rect::new(0., 0., 360., 640.);
        let controls = toolbar_placement(screen, screen, false);
        assert!(controls.width <= 344.);
        assert_eq!(controls.height, 192.);
        assert_eq!(controls.intersect(screen), controls);
    }
}
