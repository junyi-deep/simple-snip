use crate::app::{self, Event, Runtime};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        menu::*,
        *,
    },
    prelude::*,
    *,
};
use image::RgbaImage;
use snip_core::{
    annotation::{Annotation, ArrowStyle, Document, Tool},
    geometry::{ImageTransform, Point as ImagePoint, Rect},
    ocr::{Cancellation, OcrEngine, OcrResult},
};
use snip_platform::{CapturedDisplay, ClipboardProvider};
use std::sync::Arc;

#[derive(Clone, Default)]
struct PreferencesStore {
    value: snip_core::editor_preferences::EditorPreferences,
    writer: Option<std::sync::mpsc::Sender<snip_core::editor_preferences::EditorPreferences>>,
}
impl Global for PreferencesStore {}
pub fn init_preferences(cx: &mut App) {
    let path = snip_core::config::Config::path().with_file_name("editor.toml");
    let value = snip_core::editor_preferences::EditorPreferences::load(&path);
    let (writer, receiver) =
        std::sync::mpsc::channel::<snip_core::editor_preferences::EditorPreferences>();
    std::thread::spawn(move || {
        while let Ok(mut value) = receiver.recv() {
            for newer in receiver.try_iter() {
                value = newer;
            }
            if let Err(error) = value.save(&path) {
                tracing::error!("保存编辑偏好失败: {error}");
            }
        }
    });
    cx.set_global(PreferencesStore {
        value,
        writer: Some(writer),
    });
}

#[derive(Clone, Copy)]
pub enum EditorHost {
    Capture,
    Pin(u64),
}
/// One image canvas, embedded in the capture overlay or the existing pin window.
/// It never opens an application editor window.
pub struct EditorView {
    pub doc: Document,
    preferences: snip_core::editor_preferences::EditorPreferences,
    toolbar_rows: [f32; 3],
    transform: ImageTransform,
    host: EditorHost,
    rendered: Arc<RgbaImage>,
    texture: Arc<RenderImage>,
    tool: Tool,
    color: [u8; 4],
    width: f32,
    arrow_style: ArrowStyle,
    pointer: Option<Point<Pixels>>,
    text_anchor: Option<ImagePoint>,
    next_number: u32,
    size_input: Entity<InputState>,
    _size_subscription: Subscription,
    capture_context: Option<(CapturedDisplay, Rect)>,
    ocr_input: Entity<TextareaState>,
    ocr_pending: Option<String>,
    text_index: Option<usize>,
    _text_subscription: Subscription,
    last_wheel: Option<std::time::Instant>,
    draft: Option<Annotation>,
    selected: Option<usize>,
    last: Option<ImagePoint>,
    scale: f32,
    origin: Point<Pixels>,
    view_size: gpui_kit::Size<Pixels>,
    desktop_origin: Point<Pixels>,
    focus: FocusHandle,
    text: Entity<InputState>,
    message: String,
    generation: u64,
    rendering: bool,
    dirty: bool,
    ocr: Option<OcrResult>,
    ocr_start: Option<(usize, usize)>,
    ocr_end: Option<(usize, usize)>,
    ocr_hover: Option<(usize, usize)>,
    ocr_drag: bool,
    ocr_busy: bool,
    cancel: Cancellation,
    session_cancel: Cancellation,
}
impl Drop for EditorView {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.session_cancel.cancel();
    }
}
impl EditorView {
    pub fn new(
        doc: Document,
        transform: ImageTransform,
        host: EditorHost,
        initial: Arc<RgbaImage>,
        view_bounds: Bounds<Pixels>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if cx.try_global::<PreferencesStore>().is_none() {
            cx.set_global(PreferencesStore::default());
        }
        let preferences = cx.global::<PreferencesStore>().value.clone();
        let style = preferences.style(preferences.tool);
        let focus = cx.focus_handle();
        focus.focus(w, cx);
        let text = cx.new(|cx| InputState::new(w, cx));
        let window_handle = w.window_handle();
        let text_focus = focus.clone();
        let text_subscription = cx.subscribe(&text, move |v, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) && v.text_anchor.is_some() {
                v.commit_text(cx);
                let focus = text_focus.clone();
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, w, cx| focus.focus(w, cx));
                });
            } else if v.text_anchor.is_some() {
                cx.notify();
            }
        });
        let size_input = cx.new(|cx| {
            InputState::new(w, cx).default_value(format!(
                "{:.1}",
                if matches!(preferences.tool, Tool::Text | Tool::Number) {
                    style.width * 6. + 12.
                } else {
                    style.width
                }
            ))
        });
        let size_subscription = cx.subscribe(&size_input, |v, _, _: &InputEvent, cx| {
            if let Ok(size) = v.size_input.read(cx).value().trim().parse::<f32>() {
                v.set_size(size, cx);
            }
        });
        let ocr_input = cx.new(|cx| TextareaState::new(w, cx));
        let next_number = doc
            .objects
            .iter()
            .filter(|a| a.tool == Tool::Number)
            .filter_map(|a| a.text.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(preferences.next_number);
        Self {
            preferences: preferences.clone(),
            toolbar_rows: [0.; 3],
            size_input,
            _size_subscription: size_subscription,
            capture_context: None,
            ocr_input,
            ocr_pending: None,
            next_number,
            doc,
            transform,
            host,
            rendered: initial.clone(),
            texture: app::texture(&initial),
            tool: preferences.tool,
            color: style.color,
            width: style.width,
            arrow_style: style.arrow,
            pointer: None,
            text_anchor: None,
            text_index: None,
            _text_subscription: text_subscription,
            last_wheel: None,
            draft: None,
            selected: None,
            last: None,
            scale: f32::from(view_bounds.size.width) / initial.width() as f32,
            origin: view_bounds.origin,
            view_size: view_bounds.size,
            desktop_origin: w
                .display(cx)
                .map(|d| d.bounds().origin)
                .unwrap_or(w.bounds().origin),
            focus,
            text,
            message: String::new(),
            generation: 0,
            rendering: false,
            dirty: false,
            ocr: None,
            ocr_start: None,
            ocr_end: None,
            ocr_hover: None,
            ocr_drag: false,
            ocr_busy: false,
            cancel: Cancellation::default(),
            session_cancel: Cancellation::default(),
        }
    }
    fn remember(&mut self, cx: &mut Context<Self>) {
        let tool = self.option_tool();
        self.preferences = cx.global::<PreferencesStore>().value.clone();
        self.preferences.tool = self.tool;
        if self.tool.is_shape() {
            self.preferences.shape = self.tool;
        }
        if self.tool.is_line() {
            self.preferences.line = self.tool;
        }
        self.preferences.next_number = self.next_number;
        self.preferences.styles.insert(
            tool,
            snip_core::editor_preferences::ToolStyle {
                width: self.width,
                color: self.color,
                arrow: self.arrow_style,
            },
        );
        let store = cx.global_mut::<PreferencesStore>();
        if store.value != self.preferences {
            store.value = self.preferences.clone();
            if let Some(writer) = &store.writer {
                let _ = writer.send(store.value.clone());
            }
        }
    }
    fn choose_tool(&mut self, tool: Tool, w: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(cx);
        self.finish_path(cx);
        self.remember(cx);
        self.tool = tool;
        self.selected = None;
        let style = self.preferences.style(tool);
        self.width = style.width;
        self.color = style.color;
        self.arrow_style = style.arrow;
        self.clear_ocr();
        self.message.clear();
        self.sync_size(w, cx);
        self.remember(cx);
        cx.notify();
    }
    pub fn set_capture_context(&mut self, display: CapturedDisplay, rect: Rect) {
        self.capture_context = Some((display, rect));
    }
    fn display_size(&self) -> f32 {
        if matches!(self.option_tool(), Tool::Text | Tool::Number) {
            self.width * 6. + 12.
        } else {
            self.width
        }
    }
    fn set_size(&mut self, size: f32, cx: &mut Context<Self>) {
        if !size.is_finite() || size <= 0. || size > 512. {
            return;
        }
        let width = if matches!(self.option_tool(), Tool::Text | Tool::Number) {
            (size.max(13.) - 12.) / 6.
        } else {
            size.clamp(0.5, 128.)
        };
        if (width - self.width).abs() < 0.001 {
            return;
        }
        self.width = width;
        self.remember(cx);
        if let Some(i) = self.selected.filter(|i| *i < self.doc.objects.len()) {
            self.doc.checkpoint();
            self.doc.objects[i].width = width;
            self.refresh(cx);
        }
        cx.notify();
    }
    fn sync_size(&self, w: &mut Window, cx: &mut Context<Self>) {
        self.size_input.update(cx, |input, cx| {
            input.set_value(format!("{:.1}", self.display_size()), w, cx)
        });
    }
    fn finish_path(&mut self, cx: &mut Context<Self>) {
        if let Some(mut draft) = self.draft.take() {
            if draft.tool == Tool::Polyline && draft.points.len() > 1 {
                draft.points.dedup();
            }
            if draft.points.len() > 1 {
                self.doc.add(draft);
                self.refresh(cx);
            }
        }
    }
    fn paint_filter(&mut self, p: ImagePoint, checkpoint: bool, cx: &mut Context<Self>) {
        let radius = (self.width * 3.).clamp(3., 256.);
        let a = Annotation {
            tool: self.tool,
            points: vec![
                ImagePoint::new(p.x - radius, p.y - radius),
                ImagePoint::new(p.x + radius, p.y + radius),
            ],
            color: self.color,
            width: self.width,
            text: String::new(),
            arrow_style: self.arrow_style,
        };
        if checkpoint {
            self.doc.add(a);
        } else {
            self.doc.objects.push(a);
        }
        self.last = Some(p);
        self.refresh(cx);
    }
    pub fn focus_canvas(&self, w: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(w, cx);
    }
    pub fn canvas_focused(&self, w: &Window) -> bool {
        self.focus.is_focused(w) && self.text_anchor.is_none()
    }
    pub fn reframe_capture(
        &mut self,
        image: Arc<RgbaImage>,
        dx: f32,
        dy: f32,
        bounds: Bounds<Pixels>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_text(cx);
        self.clear_ocr();
        self.doc.reframe(image.clone(), dx, dy);
        self.transform = identity(&image);
        self.rendered = image.clone();
        self.texture = app::texture(&image);
        self.origin = bounds.origin;
        self.view_size = bounds.size;
        self.scale = f32::from(bounds.size.width) / image.width() as f32;
        self.draft = None;
        self.last = None;
        self.selected = None;
        self.pointer = None;
        self.refresh(cx);
        let editor = cx.entity().downgrade();
        // The resize preview temporarily unmounts the editor.
        w.on_next_frame(move |w, cx| {
            let _ = editor.update(cx, |v, cx| v.focus_canvas(w, cx));
        });
    }
    fn image_point(&self, p: Point<Pixels>) -> ImagePoint {
        self.transform.inverse(ImagePoint::new(
            f32::from(p.x - self.origin.x) / self.scale,
            f32::from(p.y - self.origin.y) / self.scale,
        ))
    }
    fn begin_text(
        &mut self,
        p: ImagePoint,
        index: Option<usize>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_text(cx);
        let existing = index.and_then(|i| self.doc.objects.get(i)).cloned();
        self.text_anchor = Some(existing.as_ref().map_or(p, |a| a.points[0]));
        self.text_index = index;
        let value = existing.map_or_else(String::new, |a| {
            self.width = a.width;
            self.color = a.color;
            a.text
        });
        self.text.update(cx, |input, cx| {
            input.set_value(value, w, cx);
        });
        // The field is mounted by the next render. Focusing it during this
        // mouse event leaves the native text-input handler on the canvas.
        let editor = cx.entity().downgrade();
        w.on_next_frame(move |w, cx| {
            let _ = editor.update(cx, |v, cx| {
                if v.text_anchor.is_some() {
                    v.text.update(cx, |input, cx| input.focus(w, cx));
                }
            });
        });
        self.message.clear();
        if index.is_some() {
            self.refresh(cx);
        }
        cx.notify();
    }
    fn commit_text(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.text_anchor.take() else {
            return;
        };
        let text = self.text.read(cx).value().to_string();
        let index = self.text_index.take();
        if !text.trim().is_empty() {
            let font = self.width * 6. + 12.;
            let a = Annotation {
                tool: Tool::Text,
                points: vec![
                    p,
                    ImagePoint::new(
                        p.x + text
                            .chars()
                            .map(|c| if c.is_ascii() { 0.7 } else { 1.0 })
                            .sum::<f32>()
                            * font,
                        p.y + font * 1.3,
                    ),
                ],
                color: self.color,
                width: self.width,
                text,
                arrow_style: ArrowStyle::default(),
            };
            if let Some(i) = index.filter(|i| *i < self.doc.objects.len()) {
                self.doc.checkpoint();
                self.doc.objects[i] = a;
                self.selected = Some(i);
            } else {
                self.doc.add(a);
                self.selected = Some(self.doc.objects.len() - 1);
            }
            self.refresh(cx);
        } else if let Some(i) = index.filter(|i| *i < self.doc.objects.len()) {
            self.doc.checkpoint();
            self.doc.objects.remove(i);
            self.selected = None;
            self.refresh(cx);
        }
        cx.notify();
    }
    fn resize_selected(&mut self, delta: f32, cx: &mut Context<Self>) {
        if !delta.is_finite() || delta.abs() < 0.01 {
            return;
        }
        if let Some(i) = self.selected.filter(|i| *i < self.doc.objects.len()) {
            let now = std::time::Instant::now();
            if self
                .last_wheel
                .is_none_or(|last| now.duration_since(last).as_millis() > 250)
            {
                self.doc.checkpoint();
            }
            self.last_wheel = Some(now);
            self.doc.objects[i].width = (self.doc.objects[i].width
                + delta.signum()
                    * if matches!(self.doc.objects[i].tool, Tool::Text | Tool::Number) {
                        1. / 6.
                    } else {
                        1.
                    })
            .clamp(
                if matches!(self.doc.objects[i].tool, Tool::Text | Tool::Number) {
                    1. / 6.
                } else {
                    0.5
                },
                if matches!(self.doc.objects[i].tool, Tool::Text | Tool::Number) {
                    (512. - 12.) / 6.
                } else {
                    128.
                },
            );
            self.width = self.doc.objects[i].width;
            self.refresh(cx);
        } else if self.tool != Tool::Select {
            self.width = (self.width
                + delta.signum()
                    * if matches!(self.tool, Tool::Text | Tool::Number) {
                        1. / 6.
                    } else {
                        1.
                    })
            .clamp(
                if matches!(self.tool, Tool::Text | Tool::Number) {
                    1. / 6.
                } else {
                    0.5
                },
                if matches!(self.tool, Tool::Text | Tool::Number) {
                    (512. - 12.) / 6.
                } else {
                    128.
                },
            );
            cx.notify();
        }
        self.remember(cx);
    }
    fn cursor_indicator(&self) -> Option<AnyElement> {
        let p = self.pointer?;
        if self.text_anchor.is_some()
            || matches!(self.tool, Tool::Select | Tool::Text)
            || self.ocr.is_some()
        {
            return None;
        }
        let radius = (self.width * 2. + 4.) * self.scale;
        let base = div().absolute().text_color(rgb(0x1688ff));
        Some(
            if matches!(self.tool, Tool::Mosaic | Tool::Blur | Tool::Eraser) {
                let radius = radius.max(4.);
                base.left(p.x - px(radius))
                    .top(p.y - px(radius))
                    .size(px(radius * 2.))
                    .border_1()
                    .border_color(rgb(0xffffff))
                    .bg(rgba(0x1688ff15))
                    .rounded_full()
                    .child(
                        div()
                            .size_full()
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(0x15212d))
                            .when(self.tool == Tool::Mosaic, |d| {
                                d.child(tool_icon(Tool::Mosaic).size(px(14.)))
                            })
                            .when(self.tool == Tool::Eraser, |d| {
                                d.child(tool_icon(Tool::Eraser).size(px(14.)))
                            }),
                    )
                    .into_any_element()
            } else {
                base.left(p.x + px(5.))
                    .top(p.y + px(5.))
                    .size(px(20.))
                    .bg(rgba(0xffffffb0))
                    .rounded_sm()
                    .child(tool_icon(self.tool).size(px(18.)))
                    .into_any_element()
            },
        )
    }
    fn option_tool(&self) -> Tool {
        self.selected
            .and_then(|i| self.doc.objects.get(i))
            .map_or(self.tool, |a| a.tool)
    }
    fn clear_ocr(&mut self) {
        self.cancel.cancel();
        self.ocr = None;
        self.ocr_start = None;
        self.ocr_end = None;
        self.ocr_hover = None;
        self.ocr_drag = false;
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.rendering {
            self.dirty = true;
            return;
        }
        self.rendering = true;
        let source = self.doc.original.clone();
        let mut doc = self.doc.clone();
        if let Some(i) = self.text_index.filter(|i| *i < doc.objects.len()) {
            doc.objects.remove(i);
        }
        let transform = self.transform;
        let render = cx
            .background_executor()
            .spawn(async move { render_document(doc, transform) });
        cx.spawn(async move |this, cx| {
            let result = render.await;
            let _ = this.update(cx, |v, cx| {
                v.rendering = false;
                if Arc::ptr_eq(&v.doc.original, &source) {
                    match result {
                        Ok((doc, image)) => {
                            v.doc.font = doc.font;
                            v.texture = app::texture(&image);
                            v.rendered = image;
                        }
                        Err(e) => v.message = e.to_string(),
                    }
                }
                if v.dirty {
                    v.dirty = false;
                    v.refresh(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn export(&mut self, action: u8, cx: &mut Context<Self>) {
        self.commit_text(cx);
        self.finish_path(cx);
        let doc = self.doc.clone();
        let transform = self.transform;
        let host = self.host;
        let placement = self.pin_placement();
        let cancel = self.session_cancel.clone();
        let tx = cx.global::<Runtime>().tx.clone();
        self.message = "正在合成…".into();
        cx.spawn(async move |this, cx| {
            let result = smol::unblock(move || render_document(doc, transform)).await;
            if cancel.cancelled() {
                return;
            }
            let (doc, image) = match result {
                Ok(value) => value,
                Err(e) => {
                    let _ = this.update(cx, |v, cx| {
                        v.message = e.to_string();
                        cx.notify();
                    });
                    return;
                }
            };
            match action {
                0 => {
                    let result =
                        smol::unblock(move || snip_platform::NativeClipboard.write_image(&image))
                            .await;
                    if cancel.cancelled() {
                        return;
                    }
                    let success = result.is_ok();
                    let message = match result {
                        Ok(()) => "已复制图片，可按 F3 贴图".into(),
                        Err(e) => format!("复制失败: {e}"),
                    };
                    let _ = this.update(cx, |v, cx| {
                        v.message = message;
                        cx.notify();
                    });
                    if success && matches!(host, EditorHost::Capture) {
                        cx.update(app::close_overlays);
                    }
                }
                1 => cx.update(|cx| app::save_image(image, cx)),
                2 => {
                    let prepared = smol::unblock(move || app::PreparedImage::new(image)).await;
                    if cancel.cancelled() {
                        return;
                    }
                    if matches!(host, EditorHost::Capture) {
                        cx.update(app::close_overlays);
                    }
                    let _ = tx.send(Event::Pin(prepared, placement)).await;
                }
                _ => match host {
                    EditorHost::Pin(id) => {
                        let _ = tx.send(Event::ApplyPin(id, doc, image, cancel)).await;
                    }
                    EditorHost::Capture => {}
                },
            }
        })
        .detach();
        cx.notify();
    }
    fn pin_placement(&self) -> Option<Bounds<Pixels>> {
        matches!(self.host, EditorHost::Capture)
            .then(|| Bounds::new(self.desktop_origin + self.origin, self.view_size))
    }
    fn cancel(&self, cx: &mut App) {
        self.cancel.cancel();
        self.session_cancel.cancel();
        match self.host {
            EditorHost::Capture => app::close_overlays(cx),
            EditorHost::Pin(id) => app::send(cx, Event::ClosePinEditor(id)),
        }
    }
    fn recognize(&mut self, cx: &mut Context<Self>) {
        if self.ocr_busy {
            return;
        }
        self.clear_ocr();
        self.ocr_busy = true;
        self.cancel = Cancellation::default();
        let cancel = self.cancel.clone();
        let document = self.doc.clone();
        let generation = self.generation;
        self.message = "正在本地识别…".into();
        cx.spawn(async move |this, cx| {
            let result = smol::unblock(move || {
                let image = document.flatten();
                snip_platform::NativeOcr.recognize(&image, &cancel)
            })
            .await;
            let _ = this.update(cx, |v, cx| {
                v.ocr_busy = false;
                if v.generation != generation {
                    return;
                }
                match result {
                    Ok(ocr) => {
                        v.message = format!("{} 行文字 · 拖动选择后复制", ocr.lines.len());
                        v.ocr_pending = Some(ocr.text());
                        v.ocr = Some(ocr);
                        v.ocr_start = None;
                        v.ocr_end = None;
                        v.tool = Tool::Select;
                    }
                    Err(e) => v.message = e.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn copy_text(&mut self, all: bool, table: bool) {
        if let Some(ocr) = &self.ocr {
            let text = if table {
                ocr.tsv()
            } else if all {
                ocr.text()
            } else if let (Some(a), Some(b)) = (self.ocr_start, self.ocr_end) {
                ocr.selected_text(a, b)
            } else {
                String::new()
            };
            if text.is_empty() {
                self.message = "请先拖动选择文字".into();
                return;
            }
            self.message = match snip_platform::write_text(text) {
                Ok(()) => "文字已复制".into(),
                Err(e) => format!("复制文字失败: {e}"),
            };
        }
    }
    fn down(&mut self, e: &MouseDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(w, cx);
        let p = self.image_point(e.position);
        if !Rect::new(
            0.,
            0.,
            self.doc.original.width() as f32,
            self.doc.original.height() as f32,
        )
        .contains(p)
        {
            return;
        }
        if let Some(ocr) = &self.ocr {
            if let Some(hit) = ocr.hit(p) {
                self.ocr_start = Some(hit);
                self.ocr_end = Some(hit);
                self.ocr_drag = true;
                cx.notify();
            }
            return;
        }
        if self.text_anchor.is_some() {
            self.commit_text(cx);
        }
        self.last_wheel = None;
        match self.tool {
            Tool::Select => {
                self.selected = self.doc.hit(p);
                if let Some(i) = self.selected {
                    self.width = self.doc.objects[i].width;
                    self.color = self.doc.objects[i].color;
                    self.arrow_style = self.doc.objects[i].arrow_style;
                    if e.click_count == 2 && self.doc.objects[i].tool == Tool::Text {
                        self.begin_text(p, Some(i), w, cx);
                        return;
                    }
                }
                self.sync_size(w, cx);
                if self.selected.is_some() {
                    self.doc.checkpoint();
                    self.last = Some(p);
                }
            }
            Tool::Eraser => {
                self.selected = None;
                self.doc.remove_at_radius(p, self.width * 2. + 4.);
                self.refresh(cx);
            }
            Tool::Text => {
                let index = self
                    .doc
                    .hit(p)
                    .filter(|i| self.doc.objects[*i].tool == Tool::Text);
                self.begin_text(p, index, w, cx);
            }
            Tool::Mosaic | Tool::Blur => {
                self.paint_filter(p, true, cx);
            }
            Tool::Polyline => {
                if e.click_count == 2 {
                    self.finish_path(cx);
                } else if let Some(draft) = &mut self.draft {
                    draft.points.push(p);
                } else {
                    self.draft = Some(Annotation {
                        tool: self.tool,
                        points: vec![p, p],
                        color: self.color,
                        width: self.width,
                        text: String::new(),
                        arrow_style: self.arrow_style,
                    });
                }
            }
            Tool::Number => {
                let number = self.next_number;
                self.next_number = number.saturating_add(1);
                self.remember(cx);
                self.doc.add(Annotation {
                    tool: Tool::Number,
                    points: vec![p],
                    color: self.color,
                    width: self.width,
                    text: number.to_string(),
                    arrow_style: ArrowStyle::default(),
                });
                self.refresh(cx);
            }
            _ => {
                self.draft = Some(Annotation {
                    tool: self.tool,
                    points: vec![p, p],
                    color: self.color,
                    width: self.width,
                    text: String::new(),
                    arrow_style: self.arrow_style,
                })
            }
        }
        cx.notify();
    }
    fn moving(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let local = e.position - self.origin;
        self.pointer = if local.x >= px(0.)
            && local.y >= px(0.)
            && local.x < self.view_size.width
            && local.y < self.view_size.height
        {
            Some(local)
        } else {
            None
        };
        cx.notify();
        let p = self.image_point(e.position);
        if let Some(ocr) = &self.ocr {
            self.ocr_hover = ocr.hit(p);
            if self.ocr_drag
                && e.dragging()
                && let Some(hit) = self.ocr_hover
            {
                self.ocr_end = Some(hit);
            }
            cx.notify();
            return;
        }
        if let Some(draft) = &mut self.draft
            && draft.tool == Tool::Polyline
        {
            if let Some(last) = draft.points.last_mut() {
                *last = p;
            }
            cx.notify();
            return;
        }
        if e.dragging() && matches!(self.tool, Tool::Mosaic | Tool::Blur) {
            if self
                .last
                .is_none_or(|last| (p.x - last.x).hypot(p.y - last.y) >= (self.width * 1.5).max(2.))
            {
                self.paint_filter(p, false, cx);
            }
            return;
        }
        if !e.dragging() {
            self.last = None;
            return;
        }
        if let Some(draft) = &mut self.draft {
            if matches!(draft.tool, Tool::Pen | Tool::Highlight | Tool::Curve) {
                draft.points.push(p);
            } else {
                draft.points[1] = p;
            }
            cx.notify();
        }
        if let (Some(i), Some(last)) = (self.selected, self.last)
            && let Some(object) = self.doc.objects.get_mut(i)
        {
            object.translate(p.x - last.x, p.y - last.y);
            self.last = Some(p);
            self.refresh(cx);
        }
    }
    fn toolbar_width(&self) -> f32 {
        if self.ocr.is_some() {
            return 520.;
        }
        let base = if self.toolbar_rows[0] > 0. {
            self.toolbar_rows[0]
        } else {
            650.
        };
        let options = if self.tool != Tool::Select || self.selected.is_some() {
            self.toolbar_rows[1]
        } else {
            0.
        };
        let arrows = if self.option_tool() == Tool::Arrow {
            self.toolbar_rows[2]
        } else {
            0.
        };
        base.max(options).max(arrows) + 8.
    }
    fn toolbar_height(&self) -> f32 {
        34. + if self.tool != Tool::Select || self.selected.is_some() || self.ocr.is_some() {
            38.
        } else {
            0.
        } + if self.option_tool() == Tool::Arrow {
            30.
        } else {
            0.
        } + if self.message.is_empty() { 0. } else { 32. }
    }
    fn toolbar(&mut self, w: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let row_measure = |index: usize| {
            let weak = cx.entity().downgrade();
            move |bounds: Vec<Bounds<Pixels>>, _: &mut Window, cx: &mut App| {
                let left = bounds
                    .iter()
                    .map(|b| f32::from(b.origin.x))
                    .fold(f32::INFINITY, f32::min);
                let right = bounds
                    .iter()
                    .map(|b| f32::from(b.origin.x + b.size.width))
                    .fold(f32::NEG_INFINITY, f32::max);
                let width = (right - left).ceil();
                if width.is_finite() && width > 0. {
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |v, cx| {
                            if (v.toolbar_rows[index] - width).abs() > 1. {
                                v.toolbar_rows[index] = width;
                                cx.notify();
                            }
                        });
                    });
                }
            }
        };
        let tools = [
            ("选择 / 移动标注", Tool::Select),
            ("矩形", Tool::Rectangle),
            ("椭圆", Tool::Ellipse),
            ("直线", Tool::Line),
            ("箭头", Tool::Arrow),
            ("自由画笔", Tool::Pen),
            ("荧光笔", Tool::Highlight),
            ("文字", Tool::Text),
            ("标号", Tool::Number),
            ("马赛克", Tool::Mosaic),
            ("高斯模糊", Tool::Blur),
            ("橡皮擦", Tool::Eraser),
        ];
        let mut strip = div()
            .on_children_prepainted(row_measure(0))
            .id("annotation-tools-strip")
            .h_flex()
            .gap(px(1.))
            .justify_end()
            .flex_nowrap()
            .overflow_x_scroll();
        if let Some((display, rect)) = self.capture_context.clone() {
            strip = strip.child(
                Button::new("capture-mode")
                    .xsmall()
                    .flex_shrink_0()
                    .label("截图 ▾")
                    .dropdown_menu(move |menu, _, _| {
                        let long_display = display.clone();
                        let gif_display = display.clone();
                        menu.item(PopupMenuItem::new("长截图 [L]").on_click(move |_, _, cx| {
                            app::send(
                                cx,
                                Event::Selection(
                                    long_display.clone(),
                                    rect,
                                    crate::overlay::CaptureMode::Long,
                                ),
                            )
                        }))
                        .item(
                            PopupMenuItem::new("录制 GIF [G]").on_click(move |_, _, cx| {
                                app::send(
                                    cx,
                                    Event::Selection(
                                        gif_display.clone(),
                                        rect,
                                        crate::overlay::CaptureMode::Gif,
                                    ),
                                )
                            }),
                        )
                    }),
            );
        }
        for (i, (label, tool)) in tools.into_iter().enumerate() {
            strip = strip.child(
                Button::new(("tool", i))
                    .xsmall()
                    .flex_shrink_0()
                    .ghost()
                    .icon(tool_icon(tool))
                    .child(
                        div()
                            .text_size(px(9.))
                            .child(["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "B", "E"][i]),
                    )
                    .when(tool == Tool::Number, |b| {
                        b.label(self.next_number.to_string())
                    })
                    .accessibility_label(label)
                    .tooltip(format!(
                        "{} [{}]",
                        label,
                        ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "B", "E"][i]
                    ))
                    .selected(
                        (self.tool == tool
                            || tool == Tool::Rectangle && self.tool.is_shape()
                            || tool == Tool::Line && self.tool.is_line())
                            && self.ocr.is_none(),
                    )
                    .on_click(cx.listener(move |v, _, w, cx| {
                        let tool = if tool == Tool::Rectangle {
                            v.preferences.shape
                        } else if tool == Tool::Line {
                            v.preferences.line
                        } else {
                            tool
                        };
                        v.choose_tool(tool, w, cx);
                        if let EditorHost::Pin(id) = v.host {
                            app::send(cx, Event::FocusPinCanvas(id));
                        }
                        cx.notify();
                    })),
            );
        }
        let actions = [
            ("撤销", "undo"),
            ("重做", "redo"),
            ("本地 OCR", "ocr"),
            ("保存图片", "save"),
            ("复制图片", "copy"),
            ("贴到桌面", "pin"),
            ("完成编辑", "done"),
            ("取消编辑", "close"),
        ];
        strip = strip.child(div().h_5().w(px(1.)).mx_1().bg(cx.theme().border));
        for (i, (label, name)) in actions.into_iter().enumerate() {
            if name == "done" && matches!(self.host, EditorHost::Capture) {
                continue;
            }
            strip = strip.child(
                Button::new(("action", i))
                    .xsmall()
                    .flex_shrink_0()
                    .ghost()
                    .icon(action_icon(name))
                    .accessibility_label(label)
                    .tooltip(format!(
                        "{} · {}",
                        label,
                        match name {
                            "undo" => "Ctrl/Cmd+Z",
                            "redo" => "Ctrl/Cmd+Shift+Z",
                            "save" => "Ctrl/Cmd+S",
                            "copy" => "Ctrl/Cmd+C",
                            "close" => "Esc",
                            "done" => "Space",
                            "ocr" => "O",
                            _ => "P",
                        }
                    ))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        match name {
                            "undo" => {
                                v.selected = None;
                                v.clear_ocr();
                                v.doc.undo();
                                v.refresh(cx);
                            }
                            "redo" => {
                                v.selected = None;
                                v.clear_ocr();
                                v.doc.redo();
                                v.refresh(cx);
                            }
                            "ocr" => v.recognize(cx),
                            "save" => v.export(1, cx),
                            "copy" => v.export(0, cx),
                            "pin" => v.export(2, cx),
                            "done" => v.export(3, cx),
                            _ => v.cancel(cx),
                        }
                        cx.notify();
                    })),
            );
        }
        let width = self
            .toolbar_width()
            .min(f32::from(w.viewport_size().width) - 16.)
            .max(260.);
        let has_options =
            self.tool != Tool::Select || self.selected.is_some() || self.ocr.is_some();
        let height = 34.
            + if has_options { 38. } else { 0. }
            + if self.option_tool() == Tool::Arrow {
                30.
            } else {
                0.
            }
            + if self.message.is_empty() { 0. } else { 26. };
        let image_w = f32::from(self.view_size.width);
        let image_h = f32::from(self.view_size.height);
        let ox = f32::from(self.origin.x);
        let oy = f32::from(self.origin.y);
        let placement = snip_core::geometry::place_toolbar(
            Rect::new(ox, oy, image_w, image_h),
            Rect::new(
                0.,
                0.,
                f32::from(w.viewport_size().width),
                f32::from(w.viewport_size().height),
            ),
            width,
            height,
        );
        let global_x = placement.x;
        let global_y = placement.y;
        let mut panel = div()
            .id("floating-annotation-toolbar")
            .when(matches!(self.host, EditorHost::Capture), |d| {
                d.absolute()
                    .left(px(global_x - ox))
                    .top(px(global_y - oy))
                    .w(px(width))
            })
            .when(matches!(self.host, EditorHost::Pin(_)), |d| d.w_full())
            .v_flex()
            .gap_1()
            .p_1()
            .rounded_md()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_md()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(strip);
        if self.ocr.is_some() {
            panel =
                panel.child(
                    div()
                        .h_flex()
                        .gap_1()
                        .child(
                            Button::new("selection-text")
                                .xsmall()
                                .flex_shrink_0()
                                .label("复制选中")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    let text = v.ocr_input.read(cx).selected_text().to_string();
                                    if !text.is_empty() {
                                        let _ = snip_platform::write_text(text);
                                    } else {
                                        v.copy_text(false, false);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(Button::new("all-text").xsmall().label("全部").on_click(
                            cx.listener(|v, _, _, cx| {
                                v.copy_text(true, false);
                                cx.notify();
                            }),
                        ))
                        .child(
                            Button::new("tsv")
                                .xsmall()
                                .flex_shrink_0()
                                .label("TSV")
                                .tooltip("仅适用于简单表格")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.copy_text(true, true);
                                    cx.notify();
                                })),
                        )
                        .child(div().flex_1().child(Input::new(&self.text).xsmall()))
                        .child(Button::new("correct").xsmall().label("修正行").on_click(
                            cx.listener(|v, _, _, cx| {
                                let offset = v.ocr_input.read(cx).selected_range().start;
                                let value = v.ocr_input.read(cx).value().to_string();
                                let index = value
                                    .get(..offset)
                                    .unwrap_or("")
                                    .bytes()
                                    .filter(|b| *b == b'\n')
                                    .count();
                                if let Some(ocr) = &mut v.ocr
                                    && let Some(line) = ocr.lines.get_mut(index)
                                {
                                    let text = v.text.read(cx).value().to_string();
                                    if !text.is_empty() {
                                        line.text = text.clone();
                                        line.words = vec![snip_core::ocr::OcrWord {
                                            text,
                                            bounds: line.bounds,
                                            confidence: None,
                                        }];
                                        v.ocr_start = Some((index, 0));
                                        v.ocr_end = Some((index, 0));
                                        v.ocr_pending = Some(ocr.text());
                                    }
                                }
                                cx.notify();
                            }),
                        )),
                );
        } else if has_options {
            let mut options = div()
                .on_children_prepainted(row_measure(1))
                .id("annotation-options")
                .h_flex()
                .gap_1()
                .items_center()
                .justify_end()
                .flex_nowrap()
                .overflow_x_scroll();
            if self.tool == Tool::Number {
                options = options
                    .child(
                        Button::new("number-minus")
                            .xsmall()
                            .flex_shrink_0()
                            .label("−")
                            .accessibility_label("减小下一个序号")
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.next_number = v.next_number.saturating_sub(1).max(1);
                                v.remember(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .child(format!("下一个 {}", self.next_number)),
                    )
                    .child(
                        Button::new("number-plus")
                            .xsmall()
                            .flex_shrink_0()
                            .label("+")
                            .accessibility_label("增大下一个序号")
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.next_number = v.next_number.saturating_add(1);
                                v.remember(cx);
                                cx.notify();
                            })),
                    );
            }
            for (i, (label, color)) in [
                ("红色", [240, 60, 70, 255]),
                ("黄色", [255, 180, 30, 255]),
                ("绿色", [30, 180, 110, 255]),
                ("蓝色", [30, 130, 240, 255]),
                ("白色", [255, 255, 255, 255]),
                ("黑色", [20, 20, 20, 255]),
            ]
            .into_iter()
            .enumerate()
            {
                options = options.child(
                    Button::new(("color", i))
                        .xsmall()
                        .flex_shrink_0()
                        .ghost()
                        .child(
                            div()
                                .size(px(12.))
                                .rounded_full()
                                .bg(rgb(((color[0] as u32) << 16)
                                    | ((color[1] as u32) << 8)
                                    | color[2] as u32))
                                .border_1()
                                .border_color(rgb(0x7b8790)),
                        )
                        .accessibility_label(label)
                        .tooltip(label)
                        .selected(self.color == color)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.color = color;
                            v.remember(cx);
                            if let Some(i) = v.selected.filter(|i| *i < v.doc.objects.len()) {
                                v.doc.checkpoint();
                                v.doc.objects[i].color = color;
                                v.refresh(cx);
                            }
                            cx.notify();
                        })),
                );
            }
            options = options
                .child(
                    Button::new("size-minus")
                        .xsmall()
                        .flex_shrink_0()
                        .label("−")
                        .on_click(cx.listener(|v, _, w, cx| {
                            v.set_size(v.display_size() - 1., cx);
                            v.sync_size(w, cx);
                        })),
                )
                .child(
                    div().w(px(64.)).child(
                        Input::new(&self.size_input)
                            .xsmall()
                            .flex_shrink_0()
                            .aria_label("字体大小 / 线宽像素"),
                    ),
                )
                .child(
                    Button::new("size-plus")
                        .xsmall()
                        .flex_shrink_0()
                        .label("+")
                        .on_click(cx.listener(|v, _, w, cx| {
                            v.set_size(v.display_size() + 1., cx);
                            v.sync_size(w, cx);
                        })),
                )
                .on_scroll_wheel(cx.listener(|v, e: &ScrollWheelEvent, w, cx| {
                    v.set_size(
                        v.display_size() + f32::from(e.delta.pixel_delta(px(20.)).y).signum(),
                        cx,
                    );
                    v.sync_size(w, cx);
                    cx.stop_propagation();
                }));
            if self.tool.is_shape() || self.tool.is_line() {
                let variants = if self.tool.is_shape() {
                    vec![
                        ("矩形", Tool::Rectangle),
                        ("圆角", Tool::RoundedRectangle),
                        ("椭圆", Tool::Ellipse),
                        ("菱形", Tool::Diamond),
                        ("三角", Tool::Triangle),
                    ]
                } else {
                    vec![
                        ("直线", Tool::Line),
                        ("折线", Tool::Polyline),
                        ("曲线", Tool::Curve),
                    ]
                };
                for (i, (label, tool)) in variants.into_iter().enumerate() {
                    options = options.child(
                        Button::new(("variant", i))
                            .xsmall()
                            .flex_shrink_0()
                            .label(label)
                            .selected(self.tool == tool)
                            .on_click(cx.listener(move |v, _, w, cx| v.choose_tool(tool, w, cx))),
                    );
                }
            }
            panel = panel.child(options);
            if self.option_tool() == Tool::Arrow {
                let mut styles = div()
                    .on_children_prepainted(row_measure(2))
                    .h_flex()
                    .gap_1()
                    .justify_end();
                for (i, (label, style)) in [
                    ("头尾等宽", ArrowStyle::Uniform),
                    ("头宽尾细", ArrowStyle::HeadWide),
                    ("头细尾宽", ArrowStyle::TailWide),
                ]
                .into_iter()
                .enumerate()
                {
                    styles = styles.child(
                        Button::new(("arrow-style", i))
                            .xsmall()
                            .flex_shrink_0()
                            .ghost()
                            .label(label)
                            .selected(self.arrow_style == style)
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.arrow_style = style;
                                v.remember(cx);
                                if let Some(i) = v.selected.filter(|i| {
                                    *i < v.doc.objects.len()
                                        && v.doc.objects[*i].tool == Tool::Arrow
                                }) {
                                    v.doc.checkpoint();
                                    v.doc.objects[i].arrow_style = style;
                                    v.refresh(cx);
                                }
                                cx.notify();
                            })),
                    );
                }
                panel = panel.child(styles);
            }
        }
        if !self.message.is_empty() {
            panel = panel.child(div().px_1().text_xs().child(self.message.clone()));
        }
        panel
    }
}
impl Render for EditorView {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(text) = self.ocr_pending.take() {
            self.ocr_input.update(cx, |v, cx| v.set_value(text, w, cx));
        }
        let weak = cx.entity().downgrade();
        let scale = self.scale;
        let transform = self.transform;
        let draft = self.draft.clone();
        let ocr = self.ocr.clone();
        let selection = self.ocr_start.zip(self.ocr_end);
        let hover = self.ocr_hover;
        let selected_bounds = self
            .selected
            .filter(|_| self.text_anchor.is_none())
            .and_then(|i| self.doc.objects.get(i))
            .map(|a| a.bounds());
        let canvas = canvas(
            move |bounds, _, cx| {
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |v, cx| {
                        let changed = v.origin != bounds.origin || v.view_size != bounds.size;
                        v.origin = bounds.origin;
                        v.view_size = bounds.size;
                        v.scale = f32::from(bounds.size.width) / v.rendered.width() as f32;
                        if changed {
                            cx.notify();
                        }
                    });
                });
            },
            move |bounds, _, window, _| {
                let map = |p: ImagePoint| {
                    let p = transform.forward(p);
                    bounds.origin + point(px(p.x * scale), px(p.y * scale))
                };
                if let Some(a) = &draft {
                    let mut points = a.path_points();
                    let r = a.bounds();
                    if a.tool == Tool::Rectangle {
                        points = vec![
                            ImagePoint::new(r.x, r.y),
                            ImagePoint::new(r.right(), r.y),
                            ImagePoint::new(r.right(), r.bottom()),
                            ImagePoint::new(r.x, r.bottom()),
                            ImagePoint::new(r.x, r.y),
                        ];
                    } else if a.tool == Tool::Ellipse {
                        points = (0..=40)
                            .map(|i| {
                                let t = i as f32 / 40. * std::f32::consts::TAU;
                                ImagePoint::new(
                                    r.x + r.width / 2. + r.width / 2. * t.cos(),
                                    r.y + r.height / 2. + r.height / 2. * t.sin(),
                                )
                            })
                            .collect();
                    }
                    if a.tool == Tool::Arrow {
                        points = a.arrow_outline();
                    }
                    let mut path = if a.tool == Tool::Arrow {
                        PathBuilder::fill()
                    } else {
                        PathBuilder::stroke(px(a.width * scale))
                    };
                    if let Some(p) = points.first() {
                        path.move_to(map(*p));
                        for p in points.iter().skip(1) {
                            path.line_to(map(*p));
                        }
                        if a.tool == Tool::Arrow {
                            path.line_to(map(*p));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(
                                path,
                                rgba(
                                    ((a.color[0] as u32) << 24)
                                        | ((a.color[1] as u32) << 16)
                                        | ((a.color[2] as u32) << 8)
                                        | a.color[3] as u32,
                                ),
                            );
                        }
                    }
                }
                if let Some(r) = selected_bounds {
                    let corners = [
                        map(ImagePoint::new(r.x, r.y)),
                        map(ImagePoint::new(r.right(), r.y)),
                        map(ImagePoint::new(r.right(), r.bottom())),
                        map(ImagePoint::new(r.x, r.bottom())),
                    ];
                    let mut border = PathBuilder::stroke(px(1.));
                    border.move_to(corners[0]);
                    for p in corners.iter().skip(1) {
                        border.line_to(*p);
                    }
                    border.line_to(corners[0]);
                    if let Ok(path) = border.build() {
                        window.paint_path(path, rgb(0x1688ff));
                    }
                    for p in corners {
                        window.paint_quad(
                            fill(
                                Bounds::new(p - point(px(2.5), px(2.5)), size(px(5.), px(5.))),
                                rgb(0xffffff),
                            )
                            .border_widths(px(1.))
                            .border_color(rgb(0x1688ff)),
                        );
                    }
                }
                if let Some(ocr) = &ocr {
                    for (i, line) in ocr.lines.iter().enumerate() {
                        for (j, word) in line.words.iter().enumerate() {
                            let r = word.bounds;
                            let a = map(ImagePoint::new(r.x, r.y));
                            let b = map(ImagePoint::new(r.right(), r.bottom()));
                            let selected = selection.is_some_and(|(a, b)| {
                                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                                (i, j) >= a && (i, j) <= b
                            });
                            let color = if selected {
                                rgba(0x2597f785)
                            } else if hover == Some((i, j)) {
                                rgba(0x2597f74c)
                            } else {
                                rgba(0x49d9dc18)
                            };
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(a.x.min(b.x), a.y.min(b.y)),
                                    size((a.x - b.x).abs(), (a.y - b.y).abs()),
                                ),
                                color,
                            ));
                        }
                    }
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();
        let toolbar = if matches!(self.host, EditorHost::Capture) {
            Some(self.toolbar(w, cx))
        } else {
            None
        };
        div()
            .id("inline-image-editor")
            .key_context("ImageAnnotation")
            .on_action(cx.listener(|v, _: &app::FocusAnnotationTools, _, cx| {
                if let EditorHost::Pin(id) = v.host {
                    app::send(cx, Event::FocusPinToolbar(id));
                }
            }))
            .track_focus(&self.focus)
            .relative()
            .cursor(match self.tool {
                Tool::Select => CursorStyle::Arrow,
                Tool::Text => CursorStyle::IBeam,
                _ => CursorStyle::Crosshair,
            })
            .on_hover(cx.listener(|v, inside, _, cx| {
                if !inside {
                    v.pointer = None;
                    cx.notify();
                }
            }))
            .size_full()
            .child(
                img(self.texture.clone())
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .child(canvas)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|v, e, w, cx| v.down(e, w, cx)),
            )
            .on_mouse_move(cx.listener(|v, e, _, cx| v.moving(e, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|v, _, _, cx| {
                    v.ocr_drag = false;
                    v.last = None;
                    if v.draft.as_ref().is_some_and(|a| a.tool == Tool::Polyline) {
                        return;
                    }
                    if let Some(a) = v.draft.take() {
                        v.doc.add(a);
                        v.refresh(cx);
                    }
                    cx.notify();
                }),
            )
            .children(self.cursor_indicator())
            .children(self.text_anchor.map(|p| {
                let p = self.transform.forward(p);
                let font = (self.width * 6. + 12.) * self.scale;
                let content_width = self
                    .text
                    .read(cx)
                    .value()
                    .chars()
                    .map(|c| if c.is_ascii() { 0.7 } else { 1.0 })
                    .sum::<f32>()
                    * font;
                div()
                    .id("image-text-input")
                    .absolute()
                    .left(px(p.x * self.scale))
                    .top(px(p.y * self.scale))
                    .w(px((content_width + font).max(48.)))
                    .text_size(px(font))
                    .line_height(px(font * 1.3))
                    .border_1()
                    .border_color(rgb(0x1688ff))
                    .bg(transparent_black())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Input::new(&self.text)
                            .id("annotation-text-field")
                            .aria_label("图片文字")
                            .appearance(false)
                            .bordered(false)
                            .p_0()
                            .text_size(px(font))
                            .h(px(font * 1.3))
                            .line_height(px(font * 1.3))
                            .text_color(rgba(
                                ((self.color[0] as u32) << 24)
                                    | ((self.color[1] as u32) << 16)
                                    | ((self.color[2] as u32) << 8)
                                    | self.color[3] as u32,
                            )),
                    )
            }))
            .children(self.ocr.as_ref().map(|_| {
                div()
                    .absolute()
                    .inset_0()
                    .p_2()
                    .bg(rgba(0xfffffff0))
                    .cursor_text()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Textarea::new(&self.ocr_input)
                            .readonly(true)
                            .appearance(false)
                            .bordered(false)
                            .size_full()
                            .aria_label("OCR 文字，可直接选择复制"),
                    )
            }))
            .children(toolbar)
            .on_scroll_wheel(cx.listener(|v, e: &ScrollWheelEvent, w, cx| {
                if v.ocr.is_some() {
                    return;
                }
                v.resize_selected(f32::from(e.delta.pixel_delta(px(20.)).y), cx);
                v.sync_size(w, cx);
                cx.stop_propagation();
            }))
            .capture_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    v.cancel(cx);
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                if v.text_anchor.is_some() {
                    if e.keystroke.key == "escape" {
                        v.text_anchor = None;
                        v.text_index = None;
                        v.refresh(cx);
                        v.focus.focus(w, cx);
                        cx.notify();
                    }
                    return;
                }

                if !v.focus.is_focused(w) {
                    return;
                }
                let modifier = if cfg!(target_os = "macos") {
                    e.keystroke.modifiers.platform
                } else {
                    e.keystroke.modifiers.control
                };
                match e.keystroke.key.as_str() {
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "b" | "e"
                        if !modifier =>
                    {
                        let tool = match e.keystroke.key.as_str() {
                            "1" => Tool::Select,
                            "2" => Tool::Rectangle,
                            "3" => Tool::Ellipse,
                            "4" => Tool::Line,
                            "5" => Tool::Arrow,
                            "6" => Tool::Pen,
                            "7" => Tool::Highlight,
                            "8" => Tool::Text,
                            "9" => Tool::Number,
                            "0" => Tool::Mosaic,
                            "b" => Tool::Blur,
                            _ => Tool::Eraser,
                        };
                        let tool = if tool == Tool::Rectangle {
                            v.preferences.shape
                        } else if tool == Tool::Line {
                            v.preferences.line
                        } else {
                            tool
                        };
                        v.choose_tool(tool, w, cx);
                    }
                    "o" if !modifier => v.recognize(cx),
                    "p" if !modifier => v.export(2, cx),
                    "enter" if v.tool == Tool::Polyline => v.finish_path(cx),
                    "l" | "g" if v.capture_context.is_some() => {
                        let (display, rect) = v.capture_context.clone().unwrap();
                        app::send(
                            cx,
                            Event::Selection(
                                display,
                                rect,
                                if e.keystroke.key == "l" {
                                    crate::overlay::CaptureMode::Long
                                } else {
                                    crate::overlay::CaptureMode::Gif
                                },
                            ),
                        );
                    }
                    "escape" => v.cancel(cx),
                    "space" if matches!(v.host, EditorHost::Pin(_)) => v.export(3, cx),
                    "z" if modifier => {
                        v.selected = None;
                        v.clear_ocr();
                        if e.keystroke.modifiers.shift {
                            v.doc.redo();
                        } else {
                            v.doc.undo();
                        }
                        v.refresh(cx);
                    }
                    "c" if modifier => {
                        if v.ocr.is_some() {
                            v.copy_text(false, false);
                        } else {
                            v.export(0, cx);
                        }
                    }
                    "s" if modifier => v.export(1, cx),
                    "backspace" | "delete" => {
                        if let Some(i) = v.selected.take().filter(|i| *i < v.doc.objects.len()) {
                            v.doc.checkpoint();
                            v.doc.objects.remove(i);
                            v.refresh(cx);
                        }
                    }
                    _ => {}
                }
            }))
    }
}
fn svg_icon(path: &str) -> Icon {
    Icon::default().data(format!("<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20' viewBox='0 0 24 24' fill='none' stroke='currentColor' stroke-width='1.65' stroke-linecap='round' stroke-linejoin='round'>{path}</svg>").as_bytes())
}
fn tool_icon(tool: Tool) -> Icon {
    svg_icon(match tool {
        Tool::Select => "<path d='m5 3 14 9-7 1-4 7z'/>",
        Tool::Rectangle | Tool::RoundedRectangle | Tool::Diamond | Tool::Triangle => {
            "<rect x='4' y='5' width='16' height='14' rx='1'/>"
        }
        Tool::Ellipse => "<ellipse cx='12' cy='12' rx='8' ry='7'/>",
        Tool::Line | Tool::Polyline | Tool::Curve => "<path d='m4 20 16-16'/>",
        Tool::Arrow => "<path d='M5 19 19 5M8 5h11v11'/>",
        Tool::Pen => "<path d='m4 19 3-7L17 2l5 5-10 10zM4 19l8-2M14 5l5 5'/>",
        Tool::Highlight => "<path d='m8 15 8-11 5 4-9 10zM8 15l-4 6 8-3M3 22h17'/>",
        Tool::Number => "<circle cx='12' cy='12' r='9'/><path d='m10 8 2-1v10m-2 0h4'/>",
        Tool::Text => "<path d='M4 5h16M12 5v15M8 20h8'/>",
        Tool::Mosaic => "<path d='M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z'/>",
        Tool::Blur => {
            "<path d='M12 3C9 8 5 11 5 15a7 7 0 0 0 14 0c0-4-4-7-7-12zM8 15a4 4 0 0 0 4 4'/>"
        }
        Tool::Eraser => {
            "<path d='m4 14 9-10a2 2 0 0 1 3 0l5 5a2 2 0 0 1 0 3l-7 8H9zM7 11l9 9M13 21h8'/>"
        }
    })
}
fn action_icon(name: &str) -> Icon {
    svg_icon(match name {
        "undo" => "<path d='M8 5 3 10l5 5M3 10h10a7 7 0 0 1 7 7'/>",
        "redo" => "<path d='m16 5 5 5-5 5M21 10H11a7 7 0 0 0-7 7'/>",
        "ocr" => "<path d='M3 8V3h5M16 3h5v5M21 16v5h-5M8 21H3v-5M7 8h10M12 8v9'/>",
        "save" => "<path d='M5 3h12l4 4v14H3V3zM7 3v6h10V3M7 21v-8h10v8'/>",
        "copy" => "<rect x='8' y='8' width='13' height='13' rx='1'/><path d='M16 8V3H3v13h5'/>",
        "pin" => "<path d='m8 3 12 12M15 3l6 6-4 1-4 7-6-6 7-4zM10 14l-7 7'/>",
        "done" => "<path d='m4 12 5 5L20 6'/>",
        _ => "<path d='m5 5 14 14M19 5 5 19'/>",
    })
}

pub fn identity(image: &RgbaImage) -> ImageTransform {
    ImageTransform {
        width: image.width() as f32,
        height: image.height() as f32,
        scale: 1.,
        quarter_turns: 0,
        flip_x: false,
        flip_y: false,
    }
}
pub fn render_document(
    mut doc: Document,
    transform: ImageTransform,
) -> anyhow::Result<(Document, Arc<RgbaImage>)> {
    static FONT: std::sync::OnceLock<Result<Arc<snip_core::fontdue::Font>, String>> =
        std::sync::OnceLock::new();
    if doc.font.is_none()
        && doc
            .objects
            .iter()
            .any(|a| matches!(a.tool, Tool::Text | Tool::Number))
    {
        let font = FONT.get_or_init(|| {
            snip_platform::annotation_font()
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    snip_core::fontdue::Font::from_bytes(bytes, Default::default())
                        .map(Arc::new)
                        .map_err(|e| e.to_string())
                })
        });
        doc.font = Some(
            font.clone()
                .map_err(|e| anyhow::anyhow!("标注字体不可用: {e}"))?,
        );
    }
    let mut image = doc.flatten();
    if transform.flip_x {
        image::imageops::flip_horizontal_in_place(&mut image);
    }
    if transform.flip_y {
        image::imageops::flip_vertical_in_place(&mut image);
    }
    let image = match transform.quarter_turns % 4 {
        1 => image::imageops::rotate90(&image),
        2 => image::imageops::rotate180(&image),
        3 => image::imageops::rotate270(&image),
        _ => image,
    };
    Ok((doc, Arc::new(image)))
}
pub fn open_recording(display: CapturedDisplay, rect: Rect, cx: &mut App) {
    if let Err(e) = crate::task_toolbar::open(display, rect, true, cx) {
        app::status(cx, e.to_string());
    }
}
pub fn open_long_capture(display: CapturedDisplay, rect: Rect, cx: &mut App) {
    if let Err(e) = crate::task_toolbar::open(display, rect, false, cx) {
        app::status(cx, e.to_string());
    }
}
pub struct FloatingToolbar {
    focus: FocusHandle,
    editor: Option<Entity<EditorView>>,
    anchor: Rect,
    screen: Rect,
    last: Rect,
    subscription: Option<Subscription>,
}
impl FloatingToolbar {
    pub fn focus_tools(&self, w: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(w, cx);
        w.activate_window();
    }
}
impl Render for FloatingToolbar {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.clone() else {
            return div().size_full();
        };
        let height = editor.read(cx).toolbar_height();
        let bounds = snip_core::geometry::place_toolbar(
            self.anchor,
            self.screen,
            editor.read(cx).toolbar_width(),
            height,
        );
        if bounds != self.last {
            self.last = bounds;
            w.resize(size(px(bounds.width), px(bounds.height)));
            if let Ok(raw) = app::raw(w) {
                let _ = snip_platform::move_window(raw, bounds.x, bounds.y, w.scale_factor());
            }
        }
        let tools = editor.update(cx, |editor, cx| editor.toolbar(w, cx));
        div()
            .size_full()
            .track_focus(&self.focus)
            .key_context("AnnotationTools")
            .capture_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    if let Some(editor) = &v.editor {
                        editor.update(cx, |v, cx| v.cancel(cx));
                    }
                    cx.stop_propagation();
                }
            }))
            .child(tools)
            .on_action(cx.listener(|v, _: &app::CancelAnnotation, _, cx| {
                if let Some(editor) = &v.editor {
                    editor.update(cx, |editor, cx| editor.cancel(cx));
                }
            }))
            .on_action(cx.listener(|v, _: &app::CloseSelectedPin, _, cx| {
                if let Some(editor) = &v.editor
                    && let EditorHost::Pin(id) = editor.read(cx).host
                {
                    app::send(cx, Event::ClosePin(id));
                    cx.stop_propagation();
                }
            }))
    }
}
pub fn open_toolbar(
    editor: Entity<EditorView>,
    pin_window: &Window,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let b = pin_window.bounds();
    let view = editor.read(cx);
    let anchor = Rect::new(
        f32::from(b.origin.x + view.origin.x),
        f32::from(b.origin.y + view.origin.y),
        f32::from(view.view_size.width),
        f32::from(view.view_size.height),
    );
    let screen = pin_window.display(cx).map(|d| d.bounds()).unwrap_or(b);
    let screen = Rect::new(
        f32::from(screen.origin.x),
        f32::from(screen.origin.y),
        f32::from(screen.size.width),
        f32::from(screen.size.height),
    );
    let bounds = snip_core::geometry::place_toolbar(
        anchor,
        screen,
        editor.read(cx).toolbar_width(),
        editor.read(cx).toolbar_height(),
    );
    if let Some((handle, view)) = cx.global::<app::Runtime>().annotation_toolbar.clone() {
        if let Some(previous) = view.read(cx).editor.as_ref()
            && let EditorHost::Pin(id) = previous.read(cx).host
            && previous != &editor
        {
            app::send(cx, Event::ClosePinEditor(id));
        }
        view.update(cx, |v, cx| {
            v.subscription = Some(cx.observe(&editor, |_, _, cx| cx.notify()));
            v.editor = Some(editor);
            v.anchor = anchor;
            v.screen = screen;
            // Force a move even if a same-sized editor was opened on another display.
            v.last = Rect::new(0., 0., 0., 0.);
            cx.notify();
        });
        let _ = handle.update(cx, |_, w, _| {
            if let Ok(raw) = app::raw(w) {
                let _ = snip_platform::show_window(raw, true);
            }
        });
        return Ok(handle);
    }
    let (handle, view) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(bounds.x), px(bounds.y)),
                size(px(bounds.width), px(bounds.height)),
            ))),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            is_resizable: false,
            is_movable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            display_id: pin_window.display(cx).map(|d| d.id()),
            ..Default::default()
        },
        cx,
        |w, cx| {
            app::apply_style(w, snip_platform::WindowStyle::default());
            cx.new(|cx| {
                let subscription = cx.observe(&editor, |_, _, cx| cx.notify());
                cx.observe_window_activation(w, |_, _, cx| cx.refresh_windows())
                    .detach();
                FloatingToolbar {
                    focus: cx.focus_handle(),
                    editor: Some(editor),
                    anchor,
                    screen,
                    last: bounds,
                    subscription: Some(subscription),
                }
            })
        },
    )?;
    cx.global_mut::<app::Runtime>().annotation_toolbar = Some((handle, view));
    Ok(handle)
}

/// Keep one empty native toolbar window for reuse. Dropping the editor and its
/// subscription releases pixels, OCR jobs and document history while avoiding
/// AppKit's Touch Bar observer race when destroying a focused popup.
pub fn hide_toolbar(pin_id: u64, cx: &mut App) -> bool {
    let Some((handle, view)) = cx.global::<app::Runtime>().annotation_toolbar.clone() else {
        return false;
    };
    let owns = view
        .read(cx)
        .editor
        .as_ref()
        .is_some_and(|e| matches!(e.read(cx).host, EditorHost::Pin(id) if id == pin_id));
    if !owns {
        return false;
    }
    view.update(cx, |v, cx| {
        v.subscription = None;
        v.editor = None;
        cx.notify();
    });
    let _ = handle.update(cx, |_, w, _| {
        if let Ok(raw) = app::raw(w) {
            let _ = snip_platform::show_window(raw, false);
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::{Arc, Document, EditorHost, EditorView, ImagePoint, RgbaImage, identity};
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render,
        Styled, TestAppContext, Window, WindowBounds, WindowOptions, div, point, px, size,
    };

    struct Surface(Entity<EditorView>);
    impl Render for Surface {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().relative().child(
                div()
                    .absolute()
                    .left(px(450.))
                    .top(px(650.))
                    .w(px(620.))
                    .h(px(215.))
                    .child(self.0.clone()),
            )
        }
    }
    fn editor(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<EditorView>) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            let mut built = None;
            let (window, _) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1440.), px(900.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |w, cx| {
                    let image = Arc::new(RgbaImage::new(1240, 430));
                    let editor = cx.new(|cx| {
                        EditorView::new(
                            Document::new(image.clone()),
                            identity(&image),
                            EditorHost::Pin(1),
                            image,
                            Bounds::new(point(px(450.), px(650.)), size(px(620.), px(215.))),
                            w,
                            cx,
                        )
                    });
                    built = Some(editor.clone());
                    cx.new(|_| Surface(editor))
                },
            )
            .expect("test editor window");
            (window, built.unwrap())
        })
    }

    #[gpui_kit::test]
    fn new_editor_restores_tool_shape_and_independent_sizes(cx: &mut TestAppContext) {
        let (window, first) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    first.update(cx, |v, cx| {
                        v.choose_tool(super::Tool::Diamond, w, cx);
                        v.set_size(7., cx);
                        v.choose_tool(super::Tool::Text, w, cx);
                        v.set_size(42., cx);
                    })
                })
                .unwrap()
        });
        let (window, next) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    next.update(cx, |v, cx| {
                        assert_eq!(v.tool, super::Tool::Text);
                        assert_eq!(v.display_size(), 42.);
                        assert_eq!(v.preferences.shape, super::Tool::Diamond);
                        v.choose_tool(v.preferences.shape, w, cx);
                        assert_eq!(v.width, 7.);
                    })
                })
                .unwrap()
        });
    }

    #[gpui_kit::test]
    fn toolbar_width_follows_actual_button_content(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| editor.update(cx, |v, _| v.host = EditorHost::Capture));
        for _ in 0..4 {
            cx.update(|cx| {
                window
                    .update(cx, |_, w, cx| {
                        let _ = w.draw(cx);
                        w.simulate_next_frame(cx);
                    })
                    .unwrap()
            });
            cx.run_until_parked();
        }
        cx.update(|cx| {
            let v = editor.read(cx);
            assert!(v.toolbar_rows[0] > 0.);
            assert!(v.toolbar_width() < 800., "{}", v.toolbar_width());
            assert!((v.toolbar_width() - v.toolbar_rows[0] - 8.).abs() < 1.);
        });
    }

    #[gpui_kit::test]
    fn escape_cancels_editor_even_with_text_or_size_input_focus(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        let (tx, rx) = async_channel::bounded(8);
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
        for text in [true, false] {
            let key_window = if text {
                window
            } else {
                cx.update(|cx| {
                    window
                        .update(cx, |_, w, cx| super::open_toolbar(editor.clone(), w, cx))
                        .unwrap()
                        .unwrap()
                })
            };
            cx.update(|cx| {
                key_window
                    .update(cx, |_, w, cx| {
                        editor.update(cx, |v, cx| {
                            if text {
                                v.tool = super::Tool::Text;
                                v.begin_text(ImagePoint::new(20., 20.), None, w, cx);
                            } else {
                                v.text_anchor = None;
                                v.size_input.update(cx, |i, cx| i.focus(w, cx));
                            }
                        });
                        let _ = w.draw(cx);
                        w.simulate_next_frame(cx);
                        let _ = w.draw(cx);
                    })
                    .unwrap()
            });
            cx.run_until_parked();
            cx.simulate_keystrokes(key_window, "escape");
            assert!(
                matches!(rx.try_recv(), Ok(crate::app::Event::ClosePinEditor(1))),
                "text focus: {text}"
            );
            cx.update(|cx| assert!(editor.read(cx).doc.objects.is_empty()));
        }
    }

    #[gpui_kit::test]
    fn filter_paints_on_down_and_whole_gesture_undoes_once(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.tool = super::Tool::Mosaic;
                        let event = gpui_kit::MouseDownEvent {
                            button: gpui_kit::MouseButton::Left,
                            position: point(px(480.), px(680.)),
                            modifiers: Default::default(),
                            click_count: 1,
                            first_mouse: false,
                        };
                        v.down(&event, w, cx);
                        assert_eq!(v.doc.objects.len(), 1, "paint before releasing mouse");
                        v.paint_filter(ImagePoint::new(130., 100.), false, cx);
                        assert_eq!(v.doc.objects.len(), 2);
                        v.doc.undo();
                        assert!(v.doc.objects.is_empty(), "one undo per brush gesture");
                    })
                })
                .unwrap()
        });
    }

    #[gpui_kit::test]
    fn numeric_size_changes_selected_font_and_wheel_by_one_pixel(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.tool = super::Tool::Text;
                        v.begin_text(ImagePoint::new(100., 80.), None, w, cx);
                        v.text.update(cx, |i, cx| i.set_value("字", w, cx));
                        v.commit_text(cx);
                        v.set_size(25., cx);
                        assert!((v.doc.objects[0].width * 6. + 12. - 25.).abs() < 0.001);
                        v.resize_selected(1., cx);
                        assert!((v.display_size() - 26.).abs() < 0.001);
                        v.set_size(f32::NAN, cx);
                        assert!((v.display_size() - 26.).abs() < 0.001);
                        v.sync_size(w, cx);
                        assert_eq!(v.size_input.read(cx).value().as_str(), "26.0");
                    })
                })
                .unwrap()
        });
    }

    #[gpui_kit::test]
    fn polyline_finishes_with_enter_without_losing_last_segment(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.tool = super::Tool::Polyline;
                        v.draft = Some(super::Annotation {
                            tool: v.tool,
                            points: vec![
                                ImagePoint::new(10., 10.),
                                ImagePoint::new(50., 50.),
                                ImagePoint::new(80., 10.),
                                ImagePoint::new(80., 10.),
                            ],
                            color: v.color,
                            width: v.width,
                            text: String::new(),
                            arrow_style: v.arrow_style,
                        });
                        v.focus.focus(w, cx);
                    })
                })
                .unwrap()
        });
        cx.simulate_keystrokes(window, "enter");
        cx.update(|cx| {
            let v = editor.read(cx);
            assert!(v.draft.is_none());
            assert_eq!(v.doc.objects[0].points.len(), 3);
            assert_eq!(v.doc.objects[0].points[2], ImagePoint::new(80., 10.));
        });
    }

    #[gpui_kit::test]
    fn number_tool_uses_adjusted_next_value_and_advances(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.tool = super::Tool::Number;
                        v.next_number = 6;
                        let e = gpui_kit::MouseDownEvent {
                            button: gpui_kit::MouseButton::Left,
                            position: point(px(480.), px(680.)),
                            modifiers: Default::default(),
                            click_count: 1,
                            first_mouse: false,
                        };
                        v.down(&e, w, cx);
                        v.down(&e, w, cx);
                        v.next_number = 3;
                        v.down(&e, w, cx);
                        assert_eq!(
                            v.doc
                                .objects
                                .iter()
                                .map(|a| a.text.as_str())
                                .collect::<Vec<_>>(),
                            ["6", "7", "3"]
                        );
                        assert_eq!(v.next_number, 4);
                    })
                })
                .unwrap()
        });
    }

    #[gpui_kit::test]
    fn switching_from_ocr_to_annotation_keeps_export_session_alive(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        let session = cx.update(|cx| {
            editor.update(cx, |v, _| {
                let ocr = v.cancel.clone();
                let session = v.session_cancel.clone();
                v.ocr_start = Some((2, 3));
                v.ocr_end = Some((3, 1));
                v.clear_ocr();
                assert!(ocr.cancelled(), "switching tools must cancel stale OCR");
                assert!(
                    !session.cancelled(),
                    "drawing after OCR must still allow copy/save/done"
                );
                assert!(v.ocr_start.is_none() && v.ocr_end.is_none());
                session
            })
        });
        // Closing the editor must cancel pending export results as well.
        cx.update(|cx| window.update(cx, |_, w, _| w.remove_window()).unwrap());
        drop(editor);
        cx.update(|_| {});
        assert!(
            session.cancelled(),
            "closed editors must reject late results"
        );
    }

    #[gpui_kit::test]
    fn capture_pin_retains_latest_selection_on_negative_origin_display(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        assert!(
                            v.pin_placement().is_none(),
                            "clipboard pins keep automatic placement"
                        );
                        v.host = EditorHost::Capture;
                        v.desktop_origin = point(px(-1440.), px(-900.));
                        assert_eq!(
                            v.pin_placement(),
                            Some(Bounds::new(
                                point(px(-990.), px(-250.)),
                                size(px(620.), px(215.))
                            ))
                        );
                        v.reframe_capture(
                            Arc::new(RgbaImage::new(1600, 600)),
                            0.,
                            0.,
                            Bounds::new(point(px(300.), px(200.)), size(px(800.), px(300.))),
                            w,
                            cx,
                        );
                        assert_eq!(
                            v.pin_placement(),
                            Some(Bounds::new(
                                point(px(-1140.), px(-700.)),
                                size(px(800.), px(300.))
                            )),
                            "pinning must use the resized selection and Retina logical size"
                        );
                    });
                })
                .unwrap();
        });
    }

    #[gpui_kit::test]
    fn capture_editor_maps_nonzero_window_origin_to_source_pixels(cx: &mut TestAppContext) {
        let (_, editor) = editor(cx);
        cx.update(|cx| {
            editor.update(cx, |v, _| {
                assert_eq!(
                    v.image_point(point(px(450.), px(650.))),
                    ImagePoint::new(0., 0.)
                );
                assert_eq!(
                    v.image_point(point(px(500.), px(700.))),
                    ImagePoint::new(100., 100.)
                );
                assert_eq!(
                    v.image_point(point(px(1070.), px(865.))),
                    ImagePoint::new(1240., 430.)
                );
            })
        });
    }
    #[gpui_kit::test]
    fn capture_cancel_from_its_own_listener_replaces_window_root(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        let (tx, _) = async_channel::bounded(4);
        cx.set_global(crate::app::Runtime {
            recordings: vec![],
            config: snip_core::config::Config::default(),
            status: String::new(),
            tx,
            pins: Default::default(),
            next_pin: 1,
            overlays: vec![window],
            capture_busy: false,
            permission_prompt: false,
            settings: None,
            hotkeys: None,
            tray: None,
            trails: vec![],
            annotation_toolbar: None,
            capture_task: None,
        });
        cx.update(crate::window_pool::init);
        let before = cx.update(|cx| {
            window
                .update(cx, |_, w, _| {
                    w.root::<gpui_kit::base::Root>()
                        .flatten()
                        .unwrap()
                        .entity_id()
                })
                .unwrap()
        });
        cx.update(|cx| {
            window
                .update(cx, |_, _, cx| {
                    editor.update(cx, |view, cx| {
                        view.host = EditorHost::Capture;
                        view.cancel(cx);
                    });
                })
                .unwrap();
        });
        cx.update(|cx| {
            assert!(cx.global::<crate::app::Runtime>().overlays.is_empty());
            window
                .update(cx, |_, w, _| {
                    assert_ne!(
                        w.root::<gpui_kit::base::Root>()
                            .flatten()
                            .unwrap()
                            .entity_id(),
                        before
                    )
                })
                .unwrap();
        });
    }
    #[gpui_kit::test]
    fn ocr_text_supports_character_and_cross_line_selection_copy(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, _| {
                        v.ocr = Some(super::OcrResult::default());
                        v.ocr_pending = Some("Hello world\n第二行".into());
                    });
                    let _ = w.draw(cx);
                    w.simulate_next_frame(cx);
                    let _ = w.draw(cx);
                    editor.update(cx, |v, cx| {
                        v.ocr_input.update(cx, |i, cx| {
                            i.focus(w, cx);
                            i.set_selected_range(1..4, cx);
                            assert_eq!(i.selected_text().to_string(), "ell");
                            i.set_selected_range(6..21, cx);
                            assert_eq!(i.selected_text().to_string(), "world\n第二行");
                        })
                    });
                })
                .unwrap()
        });
        let copy = if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        };
        cx.simulate_keystrokes(window, copy);
        cx.update(|cx| {
            assert_eq!(
                cx.read_from_clipboard().and_then(|c| c.text()),
                Some("world\n第二行".into())
            )
        });
        cx.simulate_input(window, "must not replace OCR");
        cx.update(|cx| {
            assert_eq!(
                editor.read(cx).ocr_input.read(cx).value().to_string(),
                "Hello world\n第二行"
            )
        });
    }

    #[gpui_kit::test]
    fn new_inline_text_field_receives_keyboard_input_after_mount(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.tool = super::Tool::Text;
                        v.width = 8.;
                        v.begin_text(ImagePoint::new(100., 80.), None, w, cx);
                    });
                })
                .unwrap();
        });
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    let _ = w.draw(cx);
                    w.simulate_next_frame(cx);
                    let _ = w.draw(cx);
                })
                .unwrap();
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let v = editor.read(cx);
            let expected = (v.width * 6. + 12.) * v.scale * 1.3;
            assert!(
                (f32::from(v.text.read(cx).line_height().unwrap()) - expected).abs() < 0.1,
                "caret line height {:?} must follow scaled image font {expected}",
                v.text.read(cx).line_height()
            );
        });
        cx.simulate_input(window, "hello");
        cx.update(|cx| {
            assert_eq!(editor.read(cx).text.read(cx).value().as_str(), "hello");
        });
        cx.simulate_keystrokes(window, "enter");
        cx.update(|cx| {
            let view = editor.read(cx);
            assert!(view.text_anchor.is_none());
            assert_eq!(view.doc.objects[0].text, "hello");
        });
    }
    #[gpui_kit::test]
    fn inline_text_input_submits_at_image_point_and_edits_existing_text(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.tool = super::Tool::Text;
                        v.begin_text(ImagePoint::new(100., 80.), None, w, cx);
                        assert!(v.text_anchor.is_some());
                        v.text.update(cx, |input, cx| {
                            input.set_value("图片内输入", w, cx);
                            cx.emit(super::InputEvent::PressEnter {
                                secondary: false,
                                shift: false,
                            });
                        });
                    })
                })
                .unwrap()
        });
        cx.update(|cx| {
            editor.update(cx, |v, _| {
                assert!(v.text_anchor.is_none());
                assert_eq!(v.doc.objects.len(), 1);
                assert_eq!(v.doc.objects[0].text, "图片内输入");
                assert_eq!(v.doc.objects[0].points[0], ImagePoint::new(100., 80.));
            })
        });
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.begin_text(ImagePoint::new(100., 80.), Some(0), w, cx);
                        v.text
                            .update(cx, |input, cx| input.set_value("改字", w, cx));
                        v.commit_text(cx);
                        assert_eq!(v.doc.objects.len(), 1);
                        assert_eq!(v.doc.objects[0].text, "改字");
                        v.doc.undo();
                        assert_eq!(v.doc.objects[0].text, "图片内输入");
                    })
                })
                .unwrap()
        });
    }
    #[gpui_kit::test]
    fn selected_element_scroll_preserves_style_and_can_be_undone(cx: &mut TestAppContext) {
        let (_, editor) = editor(cx);
        cx.update(|cx| {
            editor.update(cx, |v, cx| {
                v.doc.add(super::Annotation {
                    tool: super::Tool::Arrow,
                    points: vec![ImagePoint::new(20., 20.), ImagePoint::new(90., 80.)],
                    color: [200, 0, 0, 255],
                    width: 3.,
                    text: String::new(),
                    arrow_style: super::ArrowStyle::HeadWide,
                });
                v.selected = Some(0);
                v.resize_selected(1., cx);
                v.resize_selected(1., cx);
                assert_eq!(v.doc.objects[0].width, 5.);
                assert_eq!(v.doc.objects[0].arrow_style, super::ArrowStyle::HeadWide);
                v.doc.undo();
                assert_eq!(v.doc.objects[0].width, 3.);
            })
        });
    }
    #[gpui_kit::test]
    fn trail_root_overrides_the_kit_background_with_transparency(cx: &mut TestAppContext) {
        let (window, _) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    crate::window_pool::transparent_root(w, cx);
                    w.root::<gpui_kit::base::Root>()
                        .flatten()
                        .unwrap()
                        .update(cx, |root, _| {
                            assert_eq!(
                                root.style().background,
                                Some(gpui_kit::transparent_black().into())
                            );
                        });
                })
                .unwrap()
        });
    }
}
