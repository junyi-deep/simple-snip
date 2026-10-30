use crate::app::{self, Event};
use gpui_kit::prelude::*;
use gpui_kit::{
    component::{button::*, *},
    *,
};
use snip_core::geometry::{Point as ImagePoint, Rect, SelectionHandle, resize_selection, snap_at};
use snip_platform::CapturedDisplay;
#[derive(Clone, Copy, Debug)]
pub enum CaptureMode {
    Screenshot,
    Long,
    Gif,
}
pub struct Overlay {
    display: CapturedDisplay,
    texture: std::sync::Arc<RenderImage>,
    mode: CaptureMode,
    anchor: Option<ImagePoint>,
    moving: Option<(ImagePoint, Rect)>,
    selection: Option<Rect>,
    hover: ImagePoint,
    focus: FocusHandle,
    editor: Option<Entity<crate::editor::EditorView>>,
    resizing: Option<(SelectionHandle, Rect)>,
    editing_rect: Option<Rect>,
}
impl Overlay {
    pub fn open(
        display: CapturedDisplay,
        mode: CaptureMode,
        cx: &mut App,
    ) -> anyhow::Result<AnyWindowHandle> {
        let texture = app::texture(&display.image);
        Self::open_prepared(display, texture, mode, cx)
    }
    pub fn open_prepared(
        display: CapturedDisplay,
        texture: std::sync::Arc<RenderImage>,
        mode: CaptureMode,
        cx: &mut App,
    ) -> anyhow::Result<AnyWindowHandle> {
        let d = display.map.desktop;
        let cursor = snip_platform::cursor_position().unwrap_or_default();
        let (bounds, display_id) = app::display_bounds(&display, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id,
            titlebar: None,
            kind: WindowKind::Normal,
            focus: d.contains(cursor),
            is_resizable: false,
            is_minimizable: false,
            is_movable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        if d.contains(cursor) {
            cx.activate(true);
        }
        let (handle, _) = crate::window_pool::open(
            crate::window_pool::Role::Capture,
            options,
            cx,
            |window, cx| {
                app::apply_style(
                    window,
                    snip_platform::WindowStyle {
                        desktop_overlay: true,
                        ..Default::default()
                    },
                );
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                cx.new(|_| Self {
                    texture,
                    display,
                    mode,
                    anchor: None,
                    moving: None,
                    selection: None,
                    hover: ImagePoint::default(),
                    focus,
                    editor: None,
                    resizing: None,
                    editing_rect: None,
                })
            },
        )?;
        Ok(handle)
    }
    fn begin_edit(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        let r = self.selection.unwrap_or_else(|| self.candidate());
        self.selection = Some(r);
        self.editing_rect = Some(r);
        let (x, y, width, height) =
            r.pixel_bounds(self.display.image.width(), self.display.image.height());
        if width == 0 || height == 0 {
            return;
        }
        let image = std::sync::Arc::new(
            image::imageops::crop_imm(&*self.display.image, x, y, width, height).to_image(),
        );
        let transform = crate::editor::identity(&image);
        let doc = snip_core::annotation::Document::new(image.clone());
        let size = size(
            px(r.width / self.display.image.width() as f32 * f32::from(w.viewport_size().width)),
            px(r.height / self.display.image.height() as f32 * f32::from(w.viewport_size().height)),
        );
        self.editor = Some(cx.new(|cx| {
            crate::editor::EditorView::new(
                doc,
                transform,
                crate::editor::EditorHost::Capture,
                image,
                Bounds::new(
                    point(
                        px(r.x / self.display.image.width() as f32
                            * f32::from(w.viewport_size().width)),
                        px(r.y / self.display.image.height() as f32
                            * f32::from(w.viewport_size().height)),
                    ),
                    size,
                ),
                w,
                cx,
            )
        }));
        if let Some(editor) = &self.editor {
            editor.update(cx, |v, _| v.set_capture_context(self.display.clone(), r));
        }
        cx.notify();
    }
    fn apply_resized_selection(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.selection else {
            return;
        };
        let (x, y, width, height) =
            r.pixel_bounds(self.display.image.width(), self.display.image.height());
        if width == 0 || height == 0 {
            return;
        }
        let r = Rect::new(x as f32, y as f32, width as f32, height as f32);
        self.selection = Some(r);
        if let Some(editor) = &self.editor {
            let old = self.editing_rect.unwrap_or(r);
            let (old_x, old_y, _, _) =
                old.pixel_bounds(self.display.image.width(), self.display.image.height());
            let image = std::sync::Arc::new(
                image::imageops::crop_imm(&*self.display.image, x, y, width, height).to_image(),
            );
            let sx = f32::from(w.viewport_size().width) / self.display.image.width() as f32;
            let sy = f32::from(w.viewport_size().height) / self.display.image.height() as f32;
            let bounds = Bounds::new(
                point(px(r.x * sx), px(r.y * sy)),
                size(px(r.width * sx), px(r.height * sy)),
            );
            editor.update(cx, |v, cx| {
                v.set_capture_context(self.display.clone(), r);
                v.reframe_capture(
                    image,
                    old_x as f32 - x as f32,
                    old_y as f32 - y as f32,
                    bounds,
                    w,
                    cx,
                )
            });
            self.editing_rect = Some(r);
        }
        cx.notify();
    }
    fn resize_handles(&self, ui: Rect, cx: &mut Context<Self>) -> Vec<AnyElement> {
        use SelectionHandle::*;
        [
            (NorthWest, ui.x, ui.y, CursorStyle::ResizeUpLeftDownRight),
            (North, ui.x + ui.width / 2., ui.y, CursorStyle::ResizeUpDown),
            (
                NorthEast,
                ui.right(),
                ui.y,
                CursorStyle::ResizeUpRightDownLeft,
            ),
            (
                East,
                ui.right(),
                ui.y + ui.height / 2.,
                CursorStyle::ResizeLeftRight,
            ),
            (
                SouthEast,
                ui.right(),
                ui.bottom(),
                CursorStyle::ResizeUpLeftDownRight,
            ),
            (
                South,
                ui.x + ui.width / 2.,
                ui.bottom(),
                CursorStyle::ResizeUpDown,
            ),
            (
                SouthWest,
                ui.x,
                ui.bottom(),
                CursorStyle::ResizeUpRightDownLeft,
            ),
            (
                West,
                ui.x,
                ui.y + ui.height / 2.,
                CursorStyle::ResizeLeftRight,
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (handle, x, y, cursor))| {
            div()
                .id(("selection-resize", i))
                .absolute()
                .left(px(x - 5.))
                .top(px(y - 5.))
                .size(px(10.))
                .cursor(cursor)
                .child(
                    div()
                        .absolute()
                        .inset(px(2.))
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(0x1688ff)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, _, _, cx| {
                        if let Some(r) = v.selection {
                            v.resizing = Some((handle, r));
                            v.anchor = None;
                            v.moving = None;
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        })
        .collect()
    }
    fn image_point(&self, p: Point<Pixels>, window: &Window) -> ImagePoint {
        let s = window.viewport_size();
        ImagePoint::new(
            f32::from(p.x) * self.display.image.width() as f32 / f32::from(s.width),
            f32::from(p.y) * self.display.image.height() as f32 / f32::from(s.height),
        )
    }
    fn candidate(&self) -> Rect {
        let desktop = self.display.map.to_desktop(self.hover);
        let r = snap_at(desktop, &self.display.windows, self.display.map.desktop);
        let a = self.display.map.to_image(ImagePoint::new(r.x, r.y));
        let b = self
            .display
            .map
            .to_image(ImagePoint::new(r.right(), r.bottom()));
        Rect::from_points(a, b)
    }
    fn finish(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.mode, CaptureMode::Screenshot) {
            self.begin_edit(w, cx);
            return;
        }
        let r = self.selection.unwrap_or_else(|| self.candidate());
        app::send(cx, Event::Selection(self.display.clone(), r, self.mode));
    }
}
impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = window.viewport_size();
        let sx = f32::from(s.width) / self.display.image.width() as f32;
        let sy = f32::from(s.height) / self.display.image.height() as f32;
        let r = self.selection.unwrap_or_else(|| self.candidate());
        let ui = Rect::new(r.x * sx, r.y * sy, r.width * sx, r.height * sy);
        let color = self.display.image.get_pixel(
            (self.hover.x.max(0.) as u32).min(self.display.image.width() - 1),
            (self.hover.y.max(0.) as u32).min(self.display.image.height() - 1),
        );
        let guide = format!(
            "{} × {} px   #{:02X}{:02X}{:02X}   RGB {}, {}, {}",
            r.width as u32,
            r.height as u32,
            color[0],
            color[1],
            color[2],
            color[0],
            color[1],
            color[2]
        );
        let mut root = div()
            .id("capture-overlay")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .cursor_crosshair()
            .child(
                img(self.texture.clone())
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .child(
                canvas(
                    move |_, _, _| (),
                    move |bounds, _, w, _| {
                        let wdt = f32::from(bounds.size.width);
                        let hgt = f32::from(bounds.size.height);
                        let dark = rgba(0x06101b88);
                        for (x, y, width, height) in [
                            (0., 0., wdt, ui.y),
                            (0., ui.bottom(), wdt, hgt - ui.bottom()),
                            (0., ui.y, ui.x, ui.height),
                            (ui.right(), ui.y, wdt - ui.right(), ui.height),
                        ] {
                            if width > 0. && height > 0. {
                                w.paint_quad(fill(
                                    Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
                                    dark,
                                ));
                            }
                        }
                        w.paint_quad(outline(
                            Bounds::new(
                                point(px(ui.x), px(ui.y)),
                                size(px(ui.width), px(ui.height)),
                            ),
                            rgb(0x28d4de),
                            BorderStyle::Solid,
                        ));
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, w, cx| {
                    if this.editor.is_some() {
                        return;
                    }
                    let p = this.image_point(e.position, w);
                    if let Some(r) = this.selection.filter(|r| r.contains(p)) {
                        this.moving = Some((p, r));
                        this.anchor = None;
                    } else {
                        this.moving = None;
                        this.anchor = Some(p);
                        this.selection = None;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, w, cx| {
                let p = this.image_point(e.position, w);
                if let Some((handle, r)) = this.resizing {
                    this.selection = Some(resize_selection(
                        r,
                        handle,
                        p,
                        Rect::new(
                            0.,
                            0.,
                            this.display.image.width() as f32,
                            this.display.image.height() as f32,
                        ),
                    ));
                    cx.notify();
                    return;
                }
                if this.editor.is_some() {
                    return;
                }
                this.hover = p;
                if e.dragging() {
                    if let Some((a, r)) = this.moving {
                        let x = (r.x + p.x - a.x)
                            .clamp(0., this.display.image.width() as f32 - r.width);
                        let y = (r.y + p.y - a.y)
                            .clamp(0., this.display.image.height() as f32 - r.height);
                        this.selection = Some(Rect::new(x, y, r.width, r.height));
                    } else if let Some(a) = this.anchor {
                        this.selection = Some(Rect::from_points(a, p).intersect(Rect::new(
                            0.,
                            0.,
                            this.display.image.width() as f32,
                            this.display.image.height() as f32,
                        )));
                    }
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    if this.resizing.take().is_some() {
                        this.apply_resized_selection(w, cx);
                        return;
                    }
                    if this.editor.is_some() {
                        return;
                    }
                    this.anchor = None;
                    this.moving = None;
                    if this.selection.is_none_or(|r| r.width < 3. || r.height < 3.) {
                        this.selection = Some(this.candidate());
                    }
                    if matches!(this.mode, CaptureMode::Screenshot) {
                        this.begin_edit(w, cx);
                    }
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| {
                if this.editor.is_some() {
                    if e.keystroke.modifiers.shift
                        && this
                            .editor
                            .as_ref()
                            .is_some_and(|v| v.read(cx).canvas_focused(w))
                        && let Some(r) = this.selection
                    {
                        let p = match e.keystroke.key.as_str() {
                            "left" => Some(ImagePoint::new(r.right() - 1., r.bottom())),
                            "right" => Some(ImagePoint::new(r.right() + 1., r.bottom())),
                            "up" => Some(ImagePoint::new(r.right(), r.bottom() - 1.)),
                            "down" => Some(ImagePoint::new(r.right(), r.bottom() + 1.)),
                            _ => None,
                        };
                        if let Some(p) = p {
                            this.selection = Some(resize_selection(
                                r,
                                SelectionHandle::SouthEast,
                                p,
                                Rect::new(
                                    0.,
                                    0.,
                                    this.display.image.width() as f32,
                                    this.display.image.height() as f32,
                                ),
                            ));
                            this.apply_resized_selection(w, cx);
                            cx.stop_propagation();
                        }
                    }
                    return;
                }
                match e.keystroke.key.as_str() {
                    "escape" => app::close_overlays(cx),
                    "enter" => this.finish(w, cx),
                    "c" => {
                        let x = (this.hover.x.max(0.) as u32).min(this.display.image.width() - 1);
                        let y = (this.hover.y.max(0.) as u32).min(this.display.image.height() - 1);
                        let p = this.display.image.get_pixel(x, y);
                        let _ = snip_platform::write_text(format!(
                            "#{:02X}{:02X}{:02X}",
                            p[0], p[1], p[2]
                        ));
                    }
                    "a" => {
                        this.selection = Some(Rect::new(
                            0.,
                            0.,
                            this.display.image.width() as f32,
                            this.display.image.height() as f32,
                        ));
                        cx.notify();
                    }
                    "left" | "right" | "up" | "down" => {
                        let mut r = this.selection.unwrap_or_else(|| this.candidate());
                        let (dx, dy) = match e.keystroke.key.as_str() {
                            "left" => (-1., 0.),
                            "right" => (1., 0.),
                            "up" => (0., -1.),
                            _ => (0., 1.),
                        };
                        if e.keystroke.modifiers.shift {
                            r.width = (r.width + dx).max(1.);
                            r.height = (r.height + dy).max(1.);
                        } else {
                            r.x += dx;
                            r.y += dy;
                        }
                        this.selection = Some(r.intersect(Rect::new(
                            0.,
                            0.,
                            this.display.image.width() as f32,
                            this.display.image.height() as f32,
                        )));
                        cx.notify();
                    }
                    _ => {}
                }
            }));
        let toolbar = snip_core::geometry::place_toolbar(
            ui,
            Rect::new(0., 0., f32::from(s.width), f32::from(s.height)),
            490.,
            70.,
        );
        let toolbar_x = toolbar.x;
        let toolbar_y = toolbar.y;
        if self.selection.is_some() && self.anchor.is_none() && self.editor.is_none() {
            root = root.child(
                div()
                    .absolute()
                    .left(px(toolbar_x))
                    .top(px(toolbar_y))
                    .v_flex()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .gap_1()
                    .bg(rgb(0x15212d))
                    .text_color(rgb(0xf5fafc))
                    .rounded_md()
                    .p_2()
                    .child(div().text_xs().child(guide.clone()))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("cancel")
                                    .small()
                                    .label("取消 Esc")
                                    .on_click(|_, _, cx| app::close_overlays(cx)),
                            )
                            .child(Button::new("full").small().label("全屏 A").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.selection = Some(Rect::new(
                                        0.,
                                        0.,
                                        this.display.image.width() as f32,
                                        this.display.image.height() as f32,
                                    ));
                                    cx.notify();
                                }),
                            ))
                            .child(
                                Button::new("confirm")
                                    .small()
                                    .primary()
                                    .label(match self.mode {
                                        CaptureMode::Screenshot => "编辑选区 ↵",
                                        CaptureMode::Long => "开始长截图 ↵",
                                        CaptureMode::Gif => "设置录制 ↵",
                                    })
                                    .on_click(cx.listener(|this, _, w, cx| this.finish(w, cx))),
                            ),
                    ),
            );
        }
        if self.editor.is_some() || self.resizing.is_some() {
            if self.resizing.is_none()
                && let Some(editor) = &self.editor
            {
                root = root.child(
                    div()
                        .absolute()
                        .left(px(ui.x))
                        .top(px(ui.y))
                        .w(px(ui.width))
                        .h(px(ui.height))
                        .child(editor.clone()),
                );
            }
            return root.children(self.resize_handles(ui, cx)).child(
                div()
                    .absolute()
                    .left(px(ui.x))
                    .top(px((ui.y - 22.).max(0.)))
                    .text_xs()
                    .bg(rgb(0x15212d))
                    .text_color(rgb(0xffffff))
                    .px_1()
                    .child(guide),
            );
        }
        if self.selection.is_some() && self.anchor.is_none() {
            root = root.children(self.resize_handles(ui, cx));
        }
        let x = (self.hover.x.max(0.) as u32).min(self.display.image.width() - 1);
        let y = (self.hover.y.max(0.) as u32).min(self.display.image.height() - 1);
        let crop = image::imageops::crop_imm(
            &*self.display.image,
            x.saturating_sub(6),
            y.saturating_sub(6),
            13.min(self.display.image.width() - x.saturating_sub(6)),
            13.min(self.display.image.height() - y.saturating_sub(6)),
        )
        .to_image();
        let zoom = image::imageops::resize(&crop, 104, 104, image::imageops::FilterType::Nearest);
        let left = (self.hover.x * sx + 24.)
            .min(f32::from(s.width) - 120.)
            .max(8.);
        let top = (self.hover.y * sy + 24.)
            .min(f32::from(s.height) - 145.)
            .max(8.);
        root.child(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .v_flex()
                .bg(rgb(0x15212d))
                .text_color(rgb(0xf5fafc))
                .p_1()
                .child(img(app::texture(&zoom)).w(px(104.)).h(px(104.)))
                .child(div().text_xs().child("C 复制 HEX")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureMode, Overlay};
    use gpui_kit::{
        AppContext, Bounds, MouseButton, TestAppContext, VisualTestContext, WindowBounds,
        WindowOptions, point, px, size,
    };
    use snip_core::geometry::{DisplayMap, Rect};
    #[gpui_kit::test]
    fn selected_capture_resizes_through_real_handle_events(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (window, view) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(800.), px(600.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |w, cx| {
                    let image = std::sync::Arc::new(image::RgbaImage::new(1600, 1200));
                    let view = cx.new(|cx| Overlay {
                        display: snip_platform::CapturedDisplay {
                            id: 0,
                            map: DisplayMap {
                                desktop: Rect::new(0., 0., 800., 600.),
                                pixel_width: 1600,
                                pixel_height: 1200,
                            },
                            image: image.clone(),
                            windows: vec![],
                        },
                        texture: crate::app::texture(&image),
                        mode: CaptureMode::Screenshot,
                        anchor: None,
                        moving: None,
                        selection: Some(Rect::new(100., 100., 400., 300.)),
                        hover: Default::default(),
                        focus: cx.focus_handle(),
                        editor: None,
                        resizing: None,
                        editing_rect: None,
                    });
                    view.update(cx, |v, cx| v.begin_edit(w, cx));
                    view
                },
            )
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(window, cx);
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        visual.simulate_mouse_down(
            point(px(250.), px(200.)),
            MouseButton::Left,
            Default::default(),
        );
        cx.update(|cx| {
            assert!(
                view.read(cx).resizing.is_some(),
                "bottom-right handle must win over annotation canvas"
            )
        });
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        visual.simulate_mouse_move(
            point(px(300.), px(250.)),
            Some(MouseButton::Left),
            Default::default(),
        );
        visual.simulate_mouse_up(
            point(px(300.), px(250.)),
            MouseButton::Left,
            Default::default(),
        );
        cx.update(|cx| {
            let v = view.read(cx);
            assert!(v.resizing.is_none());
            assert_eq!(v.selection, Some(Rect::new(100., 100., 500., 400.)));
            assert_eq!(
                v.editor
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .doc
                    .original
                    .dimensions(),
                (500, 400)
            );
        });
        visual.update(|w, cx| {
            let _ = w.draw(cx);
            w.simulate_next_frame(cx);
            let _ = w.draw(cx);
        });
        cx.simulate_keystrokes(window, "shift-right");
        cx.update(|cx| {
            assert_eq!(
                view.read(cx)
                    .editor
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .doc
                    .original
                    .dimensions(),
                (501, 400)
            )
        });
    }
}
