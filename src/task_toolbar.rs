//! Live capture controls attached to the selected desktop region.
use crate::app::{self, Event, Runtime};
use gpui_kit::{
    component::{button::*, *},
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
    mut display: CapturedDisplay,
    rect: Rect,
    gif: bool,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
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
            focus: false,
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
            cx.new(|_| TaskToolbar {
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
                stitch: None,
                busy: false,
                auto: false,
                watch: false,
                generation: 0,
                empty_frames: 0,
                cancel: Default::default(),
                excluded: 0,
                capture_allowed: true,
            })
        },
    )?;
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
    Ok(handle)
}

fn toolbar_placement(selected: Rect, screen: Rect, gif: bool) -> Rect {
    let width = if gif { 440. } else { 520. };
    place_toolbar(
        selected,
        screen,
        width,
        if screen.width < width + 16. {
            128.
        } else {
            96.
        },
    )
}

struct TaskToolbar {
    display: CapturedDisplay,
    rect: Rect,
    gif: bool,
    handle: AnyWindowHandle,
    control: RecordingControl,
    recording: bool,
    message: String,
    path: Option<PathBuf>,
    stitch: Option<Stitcher>,
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
    let idle = cx.global::<Runtime>().capture_task.is_none()
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
    fn close(&mut self, cx: &mut Context<Self>) {
        self.control.cancel();
        self.cancel.cancel();
        self.watch = false;
        self.auto = false;
        self.generation += 1;
        app::send(cx, Event::CloseTask(self.handle));
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
        let config = cx.global::<Runtime>().config.clone();
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
                            v.message = "GIF 已保存，可预览或再次录制".into();
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
                match result {
                    Ok(changed) => {
                        v.empty_frames = if changed { 0 } else { v.empty_frames + 1 };
                        let motion = v
                            .stitch
                            .as_ref()
                            .and_then(|s| s.last_motion)
                            .unwrap_or((0, 0));
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
                smol::Timer::after(Duration::from_millis(if automatic { 900 } else { 450 })).await;
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
        } else if let Some(s) = &self.stitch {
            format!(
                "长截图 · {} × {} · {} 段{}",
                s.image.width(),
                s.image.height(),
                s.frames,
                if self.busy { " · 采集中" } else { "" }
            )
        } else {
            "长截图 · 正在采集首帧".into()
        };
        let mut controls = div().h_flex().gap_1().flex_wrap();
        if self.gif {
            controls = controls
                .child(
                    Button::new("record-start")
                        .small()
                        .primary()
                        .label("开始")
                        .disabled(self.recording || !self.capture_allowed)
                        .on_click(cx.listener(|v, _, _, cx| v.start_recording(cx))),
                )
                .child(
                    Button::new("record-pause")
                        .small()
                        .label(if self.control.paused.load(Ordering::Relaxed) {
                            "继续"
                        } else {
                            "暂停"
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
                        .label("停止并保存")
                        .disabled(!self.recording)
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.control.finish();
                            v.message = "正在完成编码…".into();
                            cx.notify();
                        })),
                );
            if let Some(path) = &self.path {
                let path = path.clone();
                controls = controls.child(
                    Button::new("record-preview")
                        .small()
                        .label("预览 GIF")
                        .on_click(move |_, _, cx| cx.open_with_system(&path)),
                );
            }
        } else {
            controls = controls
                .child(
                    Button::new("scroll-monitor")
                        .small()
                        .primary()
                        .label(if self.watch {
                            "暂停检测"
                        } else {
                            "继续检测"
                        })
                        .disabled(!self.capture_allowed)
                        .on_click(cx.listener(|v, _, _, cx| v.monitor(false, cx))),
                )
                .child(
                    Button::new("scroll-auto")
                        .small()
                        .label(if self.auto {
                            "停止自动滚动"
                        } else {
                            "自动向下滚动"
                        })
                        .disabled(!self.capture_allowed)
                        .on_click(cx.listener(|v, _, _, cx| v.monitor(true, cx))),
                )
                .child(
                    Button::new("scroll-sample")
                        .small()
                        .label("采一帧")
                        .disabled(self.busy || !self.capture_allowed)
                        .on_click(cx.listener(|v, _, _, cx| v.sample(cx))),
                )
                .child(
                    Button::new("scroll-finish")
                        .small()
                        .label("完成 / 编辑")
                        .disabled(self.stitch.is_none() || self.busy)
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.watch = false;
                            v.auto = false;
                            v.generation += 1;
                            if let Some(stitch) = v.stitch.take() {
                                app::send(cx, Event::Edit(Arc::new(stitch.image)));
                            }
                            v.close(cx);
                        })),
                );
        }
        controls = controls.child(
            Button::new("capture-task-close")
                .small()
                .label(if self.recording {
                    "取消录制"
                } else {
                    "关闭"
                })
                .on_click(cx.listener(|v, _, _, cx| v.close(cx))),
        );
        div()
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

pub fn closed(cx: &mut App) {
    restore_trails(cx);
}

#[cfg(test)]
mod tests {
    use super::toolbar_placement;
    use snip_core::geometry::Rect;
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
        assert_eq!(controls.height, 128.);
        assert_eq!(controls.intersect(screen), controls);
    }
}
