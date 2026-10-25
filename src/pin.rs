use crate::{
    app::{self, Event, Runtime},
    editor::{EditorHost, EditorView},
};
use gpui_kit::{prelude::*, *};
use image::RgbaImage;
use snip_core::{annotation::Document, geometry::ImageTransform};
use std::sync::Arc;
pub struct PinView {
    pub id: u64,
    pub doc: Document,
    shown: Arc<RgbaImage>,
    texture: Arc<RenderImage>,
    scale: f32,
    rotation: u8,
    flip_x: bool,
    flip_y: bool,
    opacity: f32,
    pub click_through: bool,
    focus: FocusHandle,
    editor: Option<Entity<EditorView>>,
    toolbar: Option<AnyWindowHandle>,
    handle: AnyWindowHandle,
    generation: u64,
    notice: Option<String>,
    notice_id: u64,
}
impl PinView {
    pub fn open(
        id: u64,
        image: Arc<RgbaImage>,
        edit: bool,
        cx: &mut App,
    ) -> anyhow::Result<(AnyWindowHandle, Entity<Self>)> {
        let scale = (680. / image.width() as f32)
            .min(500. / image.height() as f32)
            .min(1.);
        let bounds = Bounds::centered(
            None,
            size(
                px(image.width() as f32 * scale),
                px(image.height() as f32 * scale),
            ),
            cx,
        );
        let (handle, view) = crate::window_pool::open(
            crate::window_pool::Role::Pin,
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::Normal,
                app_owns_titlebar_drag: true,
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            cx,
            |window, cx| {
                window.set_window_title(&format!("Simple Snip · 贴图 {id}"));
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                app::apply_style(window, snip_platform::WindowStyle::default());
                window.on_window_should_close(cx, move |w, cx| {
                    crate::editor::hide_toolbar(id, cx);
                    cx.global_mut::<Runtime>().pins.remove(&id);
                    crate::window_pool::park(crate::window_pool::Role::Pin, w, cx);
                    false
                });
                let doc = Document::new(image.clone());
                let editor = if edit {
                    Some(cx.new(|cx| {
                        EditorView::new(
                            doc.clone(),
                            crate::editor::identity(&image),
                            EditorHost::Pin(id),
                            image.clone(),
                            Bounds::new(point(px(0.), px(0.)), window.viewport_size()),
                            window,
                            cx,
                        )
                    }))
                } else {
                    None
                };
                cx.new(|_| Self {
                    id,
                    doc,
                    shown: image.clone(),
                    texture: app::texture(&image),
                    scale,
                    rotation: 0,
                    flip_x: false,
                    flip_y: false,
                    opacity: 1.,
                    click_through: false,
                    focus,
                    editor,
                    toolbar: None,
                    handle: window.window_handle(),
                    generation: 0,
                    notice: None,
                    notice_id: 0,
                })
            },
        )?;
        if edit {
            let _ = handle.update(cx, |_, w, cx| {
                view.update(cx, |v, cx| v.open_toolbar(w, cx))
            });
        }
        Ok((handle, view))
    }
    fn open_toolbar(&mut self, w: &Window, cx: &mut Context<Self>) {
        if let Some(editor) = &self.editor {
            match crate::editor::open_toolbar(editor.clone(), w, cx) {
                Ok(handle) => self.toolbar = Some(handle),
                Err(e) => app::status(cx, e.to_string()),
            }
        }
    }
    fn remove_toolbar(&mut self, cx: &mut App) -> bool {
        self.toolbar = None;
        crate::editor::hide_toolbar(self.id, cx)
    }
    fn restore_focus(&self, cx: &mut App) {
        let focus = self.focus.clone();
        let _ = self.handle.update(cx, |_, w, cx| {
            focus.focus(w, cx);
            w.activate_window();
        });
    }
    pub fn focus_canvas(&self, cx: &mut App) {
        if let Some(editor) = self.editor.clone() {
            let _ = self.handle.update(cx, |_, w, cx| {
                editor.update(cx, |v, cx| v.focus_canvas(w, cx));
                w.activate_window();
            });
        }
    }
    pub fn focus_toolbar(&self, cx: &mut App) {
        if self.toolbar.is_some()
            && let Some((handle, view)) = cx.global::<Runtime>().annotation_toolbar.clone()
        {
            let _ = handle.update(cx, |_, w, cx| {
                view.update(cx, |v, cx| v.focus_tools(w, cx));
            });
        }
    }
    pub fn show_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.notice = Some(message);
        self.notice_id += 1;
        let id = self.notice_id;
        cx.spawn(async move |this, cx| {
            smol::Timer::after(std::time::Duration::from_secs(4)).await;
            let _ = this.update(cx, |v, cx| {
                if v.notice_id == id {
                    v.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
    pub fn style(&self, w: &Window) {
        app::apply_style(
            w,
            snip_platform::WindowStyle {
                opacity: self.opacity,
                click_through: self.click_through,
                ..Default::default()
            },
        );
    }
    fn transform(&self) -> ImageTransform {
        ImageTransform {
            width: self.doc.original.width() as f32,
            height: self.doc.original.height() as f32,
            scale: 1.,
            quarter_turns: self.rotation,
            flip_x: self.flip_x,
            flip_y: self.flip_y,
        }
    }
    fn edit(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        tracing::debug!(pin = self.id, "enter pin annotation");
        let transform = self.transform();
        let doc = self.doc.clone();
        let image = self.shown.clone();
        let id = self.id;
        self.editor = Some(cx.new(|cx| {
            EditorView::new(
                doc,
                transform,
                EditorHost::Pin(id),
                image,
                Bounds::new(point(px(0.), px(0.)), w.viewport_size()),
                w,
                cx,
            )
        }));
        self.open_toolbar(w, cx);
        cx.notify();
    }
    pub fn apply_document(&mut self, doc: Document, image: Arc<RgbaImage>, cx: &mut Context<Self>) {
        self.doc = doc;
        self.shown = image;
        self.texture = app::texture(&self.shown);
        self.editor = None;
        if self.remove_toolbar(cx) {
            self.restore_focus(cx);
        }
        cx.notify();
    }
    pub fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        if self.remove_toolbar(cx) {
            self.restore_focus(cx);
        }
        cx.notify();
    }
    fn transformed(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let doc = self.doc.clone();
        let transform = self.transform();
        let handle = self.handle;
        cx.spawn(async move |this, cx| {
            let result =
                smol::unblock(move || crate::editor::render_document(doc, transform)).await;
            let _ = this.update(cx, |v, cx| {
                if generation != v.generation {
                    return;
                }
                match result {
                    Ok((doc, image)) => {
                        v.doc = doc;
                        v.shown = image;
                        v.texture = app::texture(&v.shown);
                        let dimensions = size(
                            px(v.shown.width() as f32 * v.scale),
                            px(v.shown.height() as f32 * v.scale),
                        );
                        let _ = handle.update(cx, |_, w, _| w.resize(dimensions));
                    }
                    Err(e) => app::status(cx, e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for PinView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(editor) = &self.editor {
            return div().size_full().child(editor.clone()).into_any_element();
        }
        let notice = self.notice.clone();
        div()
            .id(("pin", self.id))
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .bg(gpui_kit::transparent_black())
            .child(
                img(self.texture.clone())
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .children(notice.map(|text| {
                div()
                    .absolute()
                    .bottom_2()
                    .right_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(rgba(0x15212de8))
                    .text_color(rgb(0xffffff))
                    .text_xs()
                    .child(text)
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|v, e: &MouseDownEvent, w, cx| {
                    tracing::debug!(pin = v.id, "pin received mouse down");
                    v.focus.focus(w, cx);
                    if e.click_count == 2 {
                        if let Ok(raw) = app::raw(w) {
                            let _ = snip_platform::show_window(raw, false);
                        }
                    } else {
                        w.start_window_move();
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|v, e: &ScrollWheelEvent, w, cx| {
                let delta = f32::from(e.delta.pixel_delta(px(20.)).y);
                if e.modifiers.control || e.modifiers.platform {
                    v.opacity = (v.opacity + delta.signum() * 0.05).clamp(0.1, 1.);
                    v.style(w);
                } else {
                    v.scale = (v.scale * (1. + delta.signum() * 0.05)).clamp(0.05, 6.);
                    w.resize(size(
                        px(v.shown.width() as f32 * v.scale),
                        px(v.shown.height() as f32 * v.scale),
                    ));
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                if matches!(e.keystroke.key.as_str(), "space" | "escape") {
                    tracing::debug!(
                        pin = v.id,
                        key = e.keystroke.key.as_str(),
                        "pin received edit key"
                    );
                }
                let modifier = if cfg!(target_os = "macos") {
                    e.keystroke.modifiers.platform
                } else {
                    e.keystroke.modifiers.control
                };
                match e.keystroke.key.as_str() {
                    "escape" => {
                        cx.global_mut::<Runtime>().pins.remove(&v.id);
                        crate::editor::hide_toolbar(v.id, cx);
                        crate::window_pool::park(crate::window_pool::Role::Pin, w, cx);
                    }
                    "space" => v.edit(w, cx),
                    "1" => {
                        v.rotation = (v.rotation + 1) % 4;
                        v.transformed(cx);
                    }
                    "2" => {
                        v.flip_x = !v.flip_x;
                        v.transformed(cx);
                    }
                    "3" => {
                        v.flip_y = !v.flip_y;
                        v.transformed(cx);
                    }
                    "t" => {
                        v.click_through = !v.click_through;
                        v.style(w);
                        app::status(cx, "穿透贴图可从托盘「恢复全部贴图交互」恢复");
                    }
                    "h" => {
                        if let Ok(raw) = app::raw(w) {
                            let _ = snip_platform::show_window(raw, false);
                        }
                    }
                    "s" if modifier => app::save_image(v.shown.clone(), cx),
                    "c" if modifier => {
                        let image = v.shown.clone();
                        let tx = cx.global::<Runtime>().tx.clone();
                        let id = v.id;
                        std::thread::spawn(move || {
                            use snip_platform::ClipboardProvider;
                            let message = match snip_platform::NativeClipboard.write_image(&image) {
                                Ok(()) => "贴图已复制".into(),
                                Err(e) => format!("复制失败: {e}"),
                            };
                            let _ = tx.send_blocking(Event::PinNotice(id, message));
                        });
                    }
                    _ => {}
                }
                cx.notify();
            }))
            .into_any_element()
    }
}
