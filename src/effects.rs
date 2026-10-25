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
    origin: ImagePoint,
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
        let origin = ImagePoint::new(f32::from(bounds.origin.x), f32::from(bounds.origin.y));
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
                let view = cx.new(|_| TrailView {
                    trail: Trail::new(config.lifetime_ms as f64 / 1000.),
                    origin,
                    start: Instant::now(),
                    width: config.width,
                });
                let weak = view.downgrade();
                let fps = config.fps;
                cx.spawn(async move |cx| {
                    loop {
                        let active = weak.update(cx, |v, cx| {
                            let had_points = !v.trail.points.is_empty();
                            if let Ok(p) = snip_platform::cursor_position() {
                                v.trail.update(
                                    ImagePoint::new(p.x - v.origin.x, p.y - v.origin.y),
                                    v.start.elapsed().as_secs_f64(),
                                );
                            }
                            let active = v.trail.points.len() > 1;
                            if had_points || !v.trail.points.is_empty() {
                                cx.notify();
                            }
                            active
                        });
                        let Ok(active) = active else {
                            break;
                        };
                        smol::Timer::after(Duration::from_millis(if active {
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
            Ok((h, _)) => handles.push(h),
            Err(e) => app::status(cx, format!("鼠标拖尾窗口创建失败: {e}")),
        }
    }
    cx.global_mut::<Runtime>().trails = handles;
}
pub fn stop(cx: &mut App) {
    for h in std::mem::take(&mut cx.global_mut::<Runtime>().trails) {
        let _ = h.update(cx, |_, w, cx| {
            crate::window_pool::park(crate::window_pool::Role::Trail, w, cx)
        });
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
