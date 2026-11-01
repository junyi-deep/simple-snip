//! Opt-in native stress fixture. Never reads the user's clipboard or settings.
use crate::{
    app::{self, PinRecord, PreparedImage, Runtime},
    pin::PinView,
};
use gpui_kit::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub fn run(cx: &mut App) {
    crate::effects::stop(cx);
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    cx.spawn(async move |cx| {
        while !stop.load(Ordering::Relaxed) {
            let started = Instant::now();
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            tracing::info!(
                late_ms = started.elapsed().as_millis().saturating_sub(100),
                "PERF heartbeat"
            );
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        let result = async {
            let fixtures = smol::unblock(make_gifs).await?;
            tracing::info!("PERF phase=idle static=0 gif=0");
            smol::Timer::after(Duration::from_secs(4)).await;
            for round in 0..2 {
                let started = Instant::now();
                for index in 0..50 {
                    let prepared = smol::unblock(move || {
                        PreparedImage::new(Arc::new(image::RgbaImage::from_fn(
                            1920,
                            1080,
                            |x, y| {
                                // Fifty independent buffers, including screenshot-like
                                // text rows, gradients and unique content per image.
                                let ink = y % 28 < 3 && x > 45 && x < 1500;
                                image::Rgba(if ink {
                                    [index as u8, 30, 50, 255]
                                } else {
                                    [
                                        ((x / 8 + index * 3) % 240 + 15) as u8,
                                        ((y / 5) % 230 + 25) as u8,
                                        220,
                                        255,
                                    ]
                                })
                            },
                        )))
                    })
                    .await;
                    cx.update(|cx| -> anyhow::Result<()> {
                        let id = cx.global::<Runtime>().next_pin;
                        cx.global_mut::<Runtime>().next_pin += 1;
                        let screen = cx.displays().into_iter().next().unwrap().bounds();
                        let width = f32::from(screen.size.width) / 8. - 8.;
                        let height = width * 1080. / 1920.;
                        let bounds = Bounds::new(
                            screen.origin
                                + point(
                                    px((index % 8) as f32 * (width + 8.)),
                                    px((index / 8) as f32 * (height + 8.)),
                                ),
                            size(px(width), px(height)),
                        );
                        let (handle, view) =
                            PinView::open_prepared(id, prepared, false, Some(bounds), cx)?;
                        cx.global_mut::<Runtime>()
                            .pins
                            .insert(id, PinRecord { handle, view });
                        Ok(())
                    })?;
                    smol::Timer::after(Duration::from_millis(40)).await;
                }
                tracing::info!(
                    round,
                    open_ms = started.elapsed().as_millis(),
                    "PERF phase=static50 static=50 gif=0"
                );
                smol::Timer::after(Duration::from_secs(8)).await;
                for index in 0..10 {
                    let path = fixtures.path().join(format!("{index}.gif"));
                    let first = path.clone();
                    let prepared = smol::unblock(move || {
                        image::open(first).map(|i| PreparedImage::new(Arc::new(i.into_rgba8())))
                    })
                    .await?;
                    let source = crate::animation::Source {
                        path,
                        temporary: Some(fixtures.clone()),
                    };
                    cx.update(|cx| -> anyhow::Result<()> {
                        let id = cx.global::<Runtime>().next_pin;
                        cx.global_mut::<Runtime>().next_pin += 1;
                        let screen = cx.displays().into_iter().next().unwrap().bounds();
                        let bounds = Bounds::new(
                            screen.origin
                                + point(
                                    px((index % 5) as f32 * 210.),
                                    px(400. + (index / 5) as f32 * 150.),
                                ),
                            size(px(200.), px(112.5)),
                        );
                        let (handle, view) =
                            PinView::open_prepared(id, prepared, false, Some(bounds), cx)?;
                        view.update(cx, |v, cx| v.play_gif(source, cx));
                        cx.global_mut::<Runtime>()
                            .pins
                            .insert(id, PinRecord { handle, view });
                        Ok(())
                    })?;
                    smol::Timer::after(Duration::from_millis(50)).await;
                }
                tracing::info!(round, "PERF phase=mixed60 static=50 gif=10");
                smol::Timer::after(Duration::from_secs(10)).await;
                close_range(0, 50, cx);
                tracing::info!(round, "PERF phase=gif10 static=0 gif=10");
                smol::Timer::after(Duration::from_secs(8)).await;
                close_range(0, usize::MAX, cx);
                tracing::info!(round, "PERF phase=closed static=0 gif=0");
                smol::Timer::after(Duration::from_secs(8)).await;
            }
            anyhow::Ok(())
        }
        .await;
        stopped.store(true, Ordering::Relaxed);
        match result {
            Ok(()) => tracing::info!("PERF PASS"),
            Err(e) => tracing::error!("PERF FAILED: {e:#}"),
        }
        cx.update(|cx| app::command(cx, app::Command::Quit));
    })
    .detach();
}
fn close_range(start: usize, count: usize, cx: &mut AsyncApp) {
    cx.update(|cx| {
        let mut ids: Vec<_> = cx.global::<Runtime>().pins.keys().copied().collect();
        ids.sort();
        for id in ids.into_iter().skip(start).take(count) {
            let Some(pin) = cx.global_mut::<Runtime>().pins.remove(&id) else {
                continue;
            };
            let _ = pin.handle.update(cx, |_, w, cx| {
                crate::window_pool::park(crate::window_pool::Role::Pin, w, cx)
            });
        }
    });
}
fn make_gifs() -> anyhow::Result<Arc<tempfile::TempDir>> {
    let dir = Arc::new(tempfile::tempdir()?);
    for index in 0..10u32 {
        let mut encoder = image::codecs::gif::GifEncoder::new_with_speed(
            std::fs::File::create(dir.path().join(format!("{index}.gif")))?,
            30,
        );
        encoder.set_repeat(image::codecs::gif::Repeat::Infinite)?;
        for frame in 0..30u32 {
            let pixels = image::RgbaImage::from_fn(640, 360, |x, y| {
                let moving = x.abs_diff(frame * 18) < 45 && y.abs_diff(120 + index * 8) < 50;
                image::Rgba(if moving {
                    [240, 40, 80, 255]
                } else {
                    [
                        (index * 20) as u8,
                        90,
                        ((x / 8 + y / 8) % 8 * 25) as u8,
                        255,
                    ]
                })
            });
            encoder.encode_frame(image::Frame::from_parts(
                pixels,
                0,
                0,
                image::Delay::from_numer_denom_ms(70, 1),
            ))?;
        }
    }
    Ok(dir)
}
