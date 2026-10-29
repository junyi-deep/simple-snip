use crate::app::{self, Runtime};
use gpui_kit::prelude::*;
use gpui_kit::*;
use snip_core::{
    geometry::Point as ImagePoint,
    trail::{Trail, rainbow},
};
use std::time::{Duration, Instant};
pub struct TrailView {
    trail: Trail,
    visible: bool,
    capture_excluded: bool,
    start: Instant,
    width: f32,
}
pub fn start(cx: &mut App) {
    if !cx.global::<Runtime>().trails.is_empty() {
        return;
    }
    let config = cx.global::<Runtime>().config.trail.clone();
    let mut handles = vec![];
    for display in cx.displays() {
        let bounds = display.bounds();
        let result = crate::window_pool::open(
            crate::window_pool::Role::Trail,
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::PopUp,
                focus: false,
                is_resizable: false,
                is_movable: false,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Transparent,
                display_id: Some(display.id()),
                ..Default::default()
            },
            cx,
            |window, cx| {
                app::apply_style(
                    window,
                    snip_platform::WindowStyle {
                        click_through: true,
                        no_activate: true,
                        desktop_overlay: true,
                        ..Default::default()
                    },
                );
                #[cfg(target_os = "windows")]
                let capture_excluded = app::raw(window)
                    .and_then(snip_platform::exclude_capture_controls)
                    .is_ok();
                #[cfg(target_os = "macos")]
                let capture_excluded = false;
                let view = cx.new(|_| TrailView {
                    trail: Trail::new(config.lifetime_ms as f64 / 1000.),
                    visible: true,
                    capture_excluded,
                    start: Instant::now(),
                    width: config.width,
                });
                let weak = view.downgrade();
                let fps = config.fps;
                let handle = window.window_handle();
                cx.spawn(async move |cx| {
                    loop {
                        let visible = weak.read_with(cx, |v, _| v.visible).unwrap_or(false);
                        let position = visible.then(|| {
                            handle.update(cx, |_, w, _| {
                                let p = cursor_in_window(w)?;
                                #[cfg(target_os = "windows")]
                                if p.x >= 0.
                                    && p.y >= 0.
                                    && p.x < f32::from(w.viewport_size().width)
                                    && p.y < f32::from(w.viewport_size().height)
                                {
                                    // Settings, taskbar and other topmost windows can be
                                    // raised without activating our app. Follow their Z order.
                                    snip_platform::raise_overlay(app::raw(w)?)?;
                                }
                                Ok::<_, anyhow::Error>(p)
                            })
                        });
                        let active = weak.update(cx, |v, cx| {
                            if !v.visible {
                                return false;
                            }
                            let had_points = !v.trail.points.is_empty();
                            if let Some(Ok(Ok(p))) = position {
                                v.trail.update(p, v.start.elapsed().as_secs_f64());
                            }
                            let active = v.trail.points.len() > 1;
                            if had_points || active {
                                cx.notify();
                            }
                            active
                        });
                        if active.is_err() {
                            break;
                        }
                        smol::Timer::after(Duration::from_millis(if visible {
                            1000 / fps as u64
                        } else {
                            80
                        }))
                        .await;
                    }
                })
                .detach();
                view
            },
        );
        match result {
            Ok((h, view)) => handles.push((h, view)),
            Err(e) => app::status(cx, format!("鼠标拖尾窗口创建失败: {e}")),
        }
    }
    cx.global_mut::<Runtime>().trails = handles;
    if cx.global::<Runtime>().capture_task.is_some()
        || cx.global::<Runtime>().capture_busy
        || !cx.global::<Runtime>().overlays.is_empty()
    {
        app::set_trails_visible(cx, false);
    }
}

pub fn stop(cx: &mut App) {
    for (h, _) in std::mem::take(&mut cx.global_mut::<Runtime>().trails) {
        let _ = h.update(cx, |_, w, cx| {
            crate::window_pool::park(crate::window_pool::Role::Trail, w, cx)
        });
    }
}
fn cursor_in_window(window: &Window) -> anyhow::Result<ImagePoint> {
    #[cfg(target_os = "windows")]
    {
        snip_platform::cursor_in_window(app::raw(window)?, window.scale_factor())
    }
    #[cfg(target_os = "macos")]
    {
        let p = snip_platform::cursor_position()?;
        let origin = window.bounds().origin;
        Ok(ImagePoint::new(
            p.x - f32::from(origin.x),
            p.y - f32::from(origin.y),
        ))
    }
}

pub fn raise(cx: &mut App) {
    #[cfg(target_os = "windows")]
    if let Some(runtime) = cx.try_global::<Runtime>() {
        let trails = runtime.trails.clone();
        for (handle, view) in trails {
            if view.read(cx).visible {
                let _ = handle.update(cx, |_, w, _| {
                    if let Ok(raw) = app::raw(w) {
                        let _ = snip_platform::raise_overlay(raw);
                    }
                });
            }
        }
    }
    #[cfg(target_os = "macos")]
    let _ = cx;
}

impl TrailView {
    #[cfg(target_os = "windows")]
    pub fn is_visible(&self) -> bool {
        self.visible
    }
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        // WDA_EXCLUDEFROMCAPTURE keeps the effect visible to the user while
        // omitting it from screenshots/recordings. Fall back to suspension if
        // Windows refused that capability; never bake a trail into the image.
        let visible = visible || self.capture_excluded;
        if self.visible != visible {
            self.visible = visible;
            self.trail = Trail::new(self.trail.lifetime);
            cx.notify();
        }
    }
}
impl Render for TrailView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let ribbon = self.trail.ribbon();
        let time = self.start.elapsed().as_secs_f64();
        let life = self.trail.lifetime;
        let width = self.width;
        canvas(
            move |_, _, _| (),
            move |_, _, window, _| {
                for pair in ribbon.windows(2) {
                    let color = |time_at: f64, opacity: f32| {
                        let fade = (1. - (time - time_at) / life).clamp(0., 1.) as f32;
                        let [r, g, b] = rainbow((time_at / life + time * 0.15) as f32);
                        rgba(
                            ((r as u32) << 24)
                                | ((g as u32) << 16)
                                | ((b as u32) << 8)
                                | ((fade * opacity * 255.) as u32),
                        )
                    };
                    for (thickness, opacity) in [(width * 2., 0.08), (width, 0.8)] {
                        let edge = |p: &snip_core::trail::RibbonPoint, sign: f32| {
                            let fade = (1. - (time - p.time) / life).clamp(0., 1.) as f32;
                            point(
                                px(p.point.x + p.normal.x * sign * thickness * fade / 2.),
                                px(p.point.y + p.normal.y * sign * thickness * fade / 2.),
                            )
                        };
                        let mut path = PathBuilder::fill();
                        path.move_to(edge(&pair[0], 1.));
                        path.line_to(edge(&pair[1], 1.));
                        path.line_to(edge(&pair[1], -1.));
                        path.line_to(edge(&pair[0], -1.));
                        path.close();
                        if let Ok(path) = path.build() {
                            let a = point(px(pair[0].point.x), px(pair[0].point.y));
                            let b = point(px(pair[1].point.x), px(pair[1].point.y));
                            let gradient = ribbon_gradient(
                                path.bounds,
                                a,
                                b,
                                color(pair[0].time, opacity),
                                color(pair[1].time, opacity),
                            );
                            window.paint_path(path, gradient);
                        }
                    }
                }
            },
        )
        .size_full()
    }
}

// Match GPUI's gradient projection so both ends of adjoining ribbon pieces
// have exactly the same color/alpha, including steep and reverse directions.
fn ribbon_gradient(
    bounds: Bounds<Pixels>,
    a: Point<Pixels>,
    b: Point<Pixels>,
    from: Rgba,
    to: Rgba,
) -> Background {
    let width = f32::from(bounds.size.width).max(0.001);
    let height = f32::from(bounds.size.height).max(0.001);
    let dx = f32::from(b.x - a.x);
    let dy = f32::from(b.y - a.y);
    let length = dx.hypot(dy).max(0.001);
    let (ux, uy) = (dx / length, dy / length);
    let (sx, sy) = if width > height {
        (1., height / width)
    } else {
        (width / height, 1.)
    };
    let angle = ((uy / sy).atan2(ux / sx).to_degrees() + 90.).rem_euclid(360.);
    let size = if ux.abs() > uy.abs() { width } else { height };
    let cx = f32::from(bounds.origin.x) + width / 2.;
    let cy = f32::from(bounds.origin.y) + height / 2.;
    let at = |p: Point<Pixels>| {
        (((f32::from(p.x) - cx) * ux + (f32::from(p.y) - cy) * uy) / size + 0.5).clamp(0., 1.)
    };
    linear_gradient(
        angle,
        linear_color_stop(from, at(a)),
        linear_color_stop(to, at(b).max(at(a) + 0.0001)),
    )
}

#[cfg(test)]
mod tests {
    use super::{ImagePoint, Instant, Trail, TrailView};
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn capture_exclusion_keeps_trail_visible_during_screenshot(cx: &mut TestAppContext) {
        let view = cx.new(|_| TrailView {
            trail: Trail::new(0.3),
            visible: true,
            capture_excluded: true,
            start: Instant::now(),
            width: 9.,
        });
        view.update(cx, |v, cx| {
            v.trail.update(ImagePoint::new(20., 30.), 0.);
            v.set_visible(false, cx);
            assert!(v.visible);
            assert_eq!(v.trail.points.len(), 1);
        });
    }

    #[gpui_kit::test]
    fn capture_pause_and_resume_do_not_join_to_old_trail(cx: &mut TestAppContext) {
        let view = cx.new(|_| TrailView {
            trail: Trail::new(0.3),
            visible: true,
            capture_excluded: false,
            start: Instant::now(),
            width: 9.,
        });
        view.update(cx, |v, cx| {
            v.trail.update(ImagePoint::new(20., 30.), 0.);
            v.trail.update(ImagePoint::new(80., 40.), 0.01);
            assert!(!v.trail.ribbon().is_empty());
            v.set_visible(false, cx);
            assert!(v.trail.ribbon().is_empty());
            v.set_visible(true, cx);
            v.trail.update(ImagePoint::new(300., 400.), 0.02);
            assert_eq!(v.trail.points.len(), 1);
            assert!(v.trail.ribbon().is_empty());
            v.trail.update(ImagePoint::new(320., 410.), 0.03);
            assert_eq!(v.trail.curves()[0].start, ImagePoint::new(300., 400.));
        });
    }
}
