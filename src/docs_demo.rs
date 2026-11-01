//! Debug-only, privacy-safe fixtures for real UI documentation screenshots.
use crate::{
    app::{self, Runtime},
    overlay::{CaptureMode, Overlay},
    pin::PinView,
};
use gpui_kit::*;
use image::{Rgba, RgbaImage};
use snip_core::{
    annotation::{Annotation, Document, Tool},
    config::{TrailAnimation, TrailColorMode},
    geometry::{DisplayMap, Point as ImagePoint, Rect},
};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU32, Ordering},
};

pub fn run(cx: &mut App) {
    crate::effects::stop(cx);
    let mode = std::env::args()
        .skip_while(|a| a != "--docs-demo")
        .nth(1)
        .unwrap_or_else(|| "capture".into());
    let screen = cx
        .displays()
        .into_iter()
        .next()
        .expect("documentation needs a display");
    let bounds = screen.bounds();
    let id = u64::from(screen.id()) as u32;
    cx.spawn(async move |cx| {
        let result = async {
            let image = smol::unblock(|| make_page(1440, 960)).await?;
            cx.update(|cx| -> anyhow::Result<()> {
                match mode.as_str() {
                    "trail" => {
                        cx.global_mut::<Runtime>().config.trail.animation = TrailAnimation::Comet;
                        cx.global_mut::<Runtime>().config.trail.color_mode =
                            TrailColorMode::Gradient;
                        crate::effects::start(cx);
                        let handle = crate::settings::Settings::open_section(6, cx)?;
                        cx.global_mut::<Runtime>().settings = Some(handle);
                    }
                    "pin" | "long" => {
                        backdrop(image.clone(), bounds, cx)?;
                        let image = if mode == "long" {
                            let page = make_page(900, 1600)?;
                            let mut stitch = snip_core::stitch::Stitcher::new(
                                image::imageops::crop_imm(page.as_ref(), 0, 0, 900, 800).to_image(),
                                64 * 1024 * 1024,
                            )?;
                            for top in [400, 800] {
                                stitch.append(
                                    &image::imageops::crop_imm(page.as_ref(), 0, top, 900, 800)
                                        .to_image(),
                                )?;
                            }
                            Arc::new(stitch.image)
                        } else {
                            image.clone()
                        };
                        let pin_id = 1;
                        let placement = if mode == "long" {
                            Some(Bounds::new(
                                bounds.origin + point(px(420.), px(60.)),
                                size(px(450.), px(800.)),
                            ))
                        } else {
                            None
                        };
                        let (handle, view) = PinView::open_prepared(
                            pin_id,
                            app::PreparedImage::new(image),
                            false,
                            placement,
                            cx,
                        )?;
                        cx.global_mut::<Runtime>()
                            .pins
                            .insert(pin_id, app::PinRecord { handle, view });
                    }
                    "gif" => {
                        backdrop(image.clone(), bounds, cx)?;
                        let display = snip_platform::CapturedDisplay {
                            id,
                            image: image.clone(),
                            map: DisplayMap {
                                desktop: Rect::new(
                                    f32::from(bounds.origin.x),
                                    f32::from(bounds.origin.y),
                                    f32::from(bounds.size.width),
                                    f32::from(bounds.size.height),
                                ),
                                pixel_width: 1440,
                                pixel_height: 960,
                            },
                            windows: vec![],
                        };
                        cx.global_mut::<Runtime>().config.gif.countdown = 0;
                        cx.global_mut::<Runtime>().config.gif.max_seconds = 60;
                        crate::task_toolbar::open(
                            display,
                            Rect::new(260., 180., 640., 360.),
                            true,
                            cx,
                        )?;
                    }
                    _ => {
                        let display = snip_platform::CapturedDisplay {
                            id,
                            image: image.clone(),
                            map: DisplayMap {
                                desktop: Rect::new(
                                    f32::from(bounds.origin.x),
                                    f32::from(bounds.origin.y),
                                    f32::from(bounds.size.width),
                                    f32::from(bounds.size.height),
                                ),
                                pixel_width: 1440,
                                pixel_height: 960,
                            },
                            windows: vec![],
                        };
                        let handle = Overlay::open(display, CaptureMode::Screenshot, cx)?;
                        cx.global_mut::<Runtime>().overlays.push(handle);
                    }
                }
                Ok(())
            })?;
            anyhow::Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::error!("documentation fixture: {error:#}");
        }
    })
    .detach();
}

struct Backdrop(Arc<RenderImage>);
impl Render for Backdrop {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xf0f5fb))
            .child(img(self.0.clone()).size_full().object_fit(ObjectFit::Fill))
    }
}
fn backdrop(image: Arc<RgbaImage>, bounds: Bounds<Pixels>, cx: &mut App) -> anyhow::Result<()> {
    let texture = app::texture(&image);
    crate::window_pool::open(
        crate::window_pool::Role::Preview,
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            is_resizable: false,
            is_movable: false,
            ..Default::default()
        },
        cx,
        |w, cx| {
            w.set_window_title("Simple Snip · 示例内容");
            app::apply_style(
                w,
                snip_platform::WindowStyle {
                    no_activate: true,
                    ..Default::default()
                },
            );
            cx.new(|_| Backdrop(texture))
        },
    )?;
    Ok(())
}

/// The recording fixture never reads the desktop or clipboard.
pub fn capture(rect: Rect) -> RgbaImage {
    static PAGE: OnceLock<Arc<RgbaImage>> = OnceLock::new();
    static FRAME: AtomicU32 = AtomicU32::new(0);
    let page = PAGE.get_or_init(|| {
        make_page(900, 1600).unwrap_or_else(|_| {
            Arc::new(RgbaImage::from_pixel(900, 1600, Rgba([240, 245, 250, 255])))
        })
    });
    let frame = FRAME.fetch_add(1, Ordering::Relaxed);
    let (w, h) = (rect.width as u32, rect.height as u32);
    let top = (frame * 4) % 600;
    image::imageops::resize(
        &image::imageops::crop_imm(&**page, 0, top, 900, 700).to_image(),
        w.max(1),
        h.max(1),
        image::imageops::FilterType::Triangle,
    )
}

fn make_page(width: u32, height: u32) -> anyhow::Result<Arc<RgbaImage>> {
    let sx = width as f32 / 1440.;
    let source = Arc::new(RgbaImage::from_fn(width, height, |x, y| {
        let card =
            (x > (72. * sx) as u32 && x < (1368. * sx) as u32) && y > 200 && (y - 200) % 360 < 312;
        Rgba(if card {
            [255, 255, 255, 255]
        } else {
            [240, 245, 251, 255]
        })
    }));
    let mut doc = Document::new(source.clone());
    let mut text = |value: &str, x: f32, y: f32, font: f32, color: [u8; 4]| {
        doc.add(Annotation {
            tool: Tool::Text,
            points: vec![
                ImagePoint::new(x * sx, y),
                ImagePoint::new((x + 700.) * sx, y + font * 1.3),
            ],
            color,
            width: (font - 12.) / 6.,
            text: value.into(),
            arrow_style: Default::default(),
        });
    };
    text(
        "Simple Snip · 灵感工作台",
        72.,
        54.,
        42. * sx,
        [24, 42, 68, 255],
    );
    text(
        "Capture ideas. Keep your flow.",
        72.,
        124.,
        25. * sx,
        [71, 85, 105, 255],
    );
    for row in 0..((height.saturating_sub(200)) / 360 + 1) {
        let y = 230. + row as f32 * 360.;
        text(
            &format!(
                "0{}   {}",
                row + 1,
                ["本周待办", "灵感速记", "项目里程碑", "设计备忘"][row as usize % 4]
            ),
            108.,
            y,
            32. * sx,
            [24, 42, 68, 255],
        );
        text(
            [
                "完善截图与贴图交互",
                "让重点一目了然",
                "在原图上记录想法",
                "保留完整的上下文",
            ][row as usize % 4],
            108.,
            y + 74.,
            27. * sx,
            [51, 65, 85, 255],
        );
        text(
            "自由框选  /  独立贴图  /  原位标注",
            108.,
            y + 138.,
            24. * sx,
            [71, 85, 105, 255],
        );
        text(
            "本地处理，专注当下。",
            108.,
            y + 198.,
            23. * sx,
            [0, 143, 135, 255],
        );
    }
    crate::editor::render_document(doc, crate::editor::identity(&source)).map(|(_, image)| image)
}
