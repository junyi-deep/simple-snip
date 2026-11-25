use crate::app::{self, Event, Runtime};
use crate::ui as design;
use gpui_kit::{
    component::{
        button::*,
        color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
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
    geometry::{ImageTransform, Point as ImagePoint, Rect, SelectionHandle, resize_selection},
    ocr::{Cancellation, OcrResult},
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
    if crate::is_docs_demo() {
        cx.set_global(PreferencesStore::default());
        return;
    }
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

pub fn toast_preferences(cx: &App) -> (bool, u32) {
    let prefs = cx
        .try_global::<PreferencesStore>()
        .map(|s| s.value.clone())
        .unwrap_or_default();
    (prefs.toast_enabled, prefs.toast_delay_ms)
}
pub fn set_toast_preferences(enabled: bool, delay: u32, cx: &mut App) {
    if cx.try_global::<PreferencesStore>().is_none() {
        init_preferences(cx);
    }
    let store = cx.global_mut::<PreferencesStore>();
    store.value.toast_enabled = enabled;
    store.value.toast_delay_ms = delay.clamp(500, 10000);
    if let Some(writer) = &store.writer {
        let _ = writer.send(store.value.clone());
    }
    cx.refresh_windows();
}
#[derive(Clone, Copy)]
pub enum EditorHost {
    Capture,
    Pin(u64),
}
/// One image canvas, embedded in the capture overlay or the existing pin window.
pub struct ActiveCaptureEditor(pub WeakEntity<EditorView>);
impl Global for ActiveCaptureEditor {}

#[derive(Clone, Copy)]
enum GeometryHandle {
    Edge(SelectionHandle),
    Endpoint(usize),
}

struct GeometryDrag {
    handle: GeometryHandle,
    start: ImagePoint,
    bounds: Rect,
    points: Vec<ImagePoint>,
    checkpointed: bool,
}

/// The positions are in source pixels, so both drawing and hit testing can
/// use the same transform for zoomed, rotated, or mirrored pins.
fn geometry_handles(annotation: &Annotation) -> Vec<(GeometryHandle, ImagePoint)> {
    if matches!(annotation.tool, Tool::Line | Tool::Arrow) && annotation.points.len() == 2 {
        return annotation
            .points
            .iter()
            .copied()
            .enumerate()
            .map(|(i, p)| (GeometryHandle::Endpoint(i), p))
            .collect();
    }
    if !(annotation.tool.is_shape()
        || matches!(
            annotation.tool,
            Tool::Polyline | Tool::Curve | Tool::Pen | Tool::Highlight
        ))
        || annotation.points.len() < 2
    {
        return vec![];
    }
    let r = annotation.bounds();
    use SelectionHandle::*;
    [
        (NorthWest, r.x, r.y),
        (North, r.x + r.width * 0.5, r.y),
        (NorthEast, r.right(), r.y),
        (East, r.right(), r.y + r.height * 0.5),
        (SouthEast, r.right(), r.bottom()),
        (South, r.x + r.width * 0.5, r.bottom()),
        (SouthWest, r.x, r.bottom()),
        (West, r.x, r.y + r.height * 0.5),
    ]
    .into_iter()
    .map(|(handle, x, y)| (GeometryHandle::Edge(handle), ImagePoint::new(x, y)))
    .collect()
}

/// It never opens an application editor window.
pub struct EditorView {
    pub doc: Document,
    preferences: snip_core::editor_preferences::EditorPreferences,
    toolbar_rows: [f32; 2],
    transform: ImageTransform,
    host: EditorHost,
    rendered: Arc<RgbaImage>,
    texture: Arc<RenderImage>,
    texture_window: AnyWindowHandle,
    tool: Tool,
    color: [u8; 4],
    color_picker: Entity<ColorPickerState>,
    _color_subscription: Subscription,
    _color_observer: Subscription,
    pending_annotations: Vec<(Annotation, u64)>,
    popup_open: bool,
    width: f32,
    arrow_style: ArrowStyle,
    pointer: Option<Point<Pixels>>,
    text_anchor: Option<ImagePoint>,
    next_number: u32,
    size_input: Entity<InputState>,
    _size_subscription: Subscription,
    capture_context: Option<(CapturedDisplay, Rect)>,
    text_index: Option<usize>,
    text_tool: Tool,
    watermark: snip_core::annotation::WatermarkStyle,
    _text_subscription: Subscription,
    last_wheel: Option<std::time::Instant>,
    draft: Option<Annotation>,
    selected: Option<usize>,
    geometry_drag: Option<GeometryDrag>,
    moving_annotation: bool,
    last: Option<ImagePoint>,
    scale: f32,
    origin: Point<Pixels>,
    view_size: gpui_kit::Size<Pixels>,
    desktop_origin: Point<Pixels>,
    focus: FocusHandle,
    text: Entity<InputState>,
    message: String,
    toast_last: String,
    toast_visible: bool,
    toast_generation: u64,
    generation: u64,
    rendering: bool,
    dirty: bool,
    ocr: Option<Arc<OcrResult>>,
    ocr_text: SharedString,
    ocr_field: Entity<TextareaState>,
    ocr_mode: Option<usize>,
    ocr_size: gpui_kit::Size<Pixels>,
    ocr_resize: Option<(Point<Pixels>, gpui_kit::Size<Pixels>)>,
    ocr_request: u64,
    ocr_busy: bool,
    ocr_scanning: bool,
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
        let initial_tool = if matches!(host, EditorHost::Pin(_)) {
            Tool::Select
        } else {
            preferences.tool
        };
        let style = preferences.style(initial_tool);
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
                if preferences.tool.is_typographic() {
                    style.width * 6. + 12.
                } else {
                    style.width
                }
            ))
        });
        let size_subscription = cx.subscribe(&size_input, |v, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Ok(size) = v.size_input.read(cx).value().trim().parse::<f32>()
            {
                v.set_size(size, cx);
            }
        });
        let color_picker = cx.new(|cx| {
            ColorPickerState::new(w, cx).default_value(rgba(u32::from_be_bytes(style.color)))
        });
        let color_subscription =
            cx.subscribe(&color_picker, |v, _, event: &ColorPickerEvent, cx| {
                if let ColorPickerEvent::Change(Some(color)) = event {
                    let color = color.to_rgb();
                    v.color = [
                        (color.r * 255.).round() as u8,
                        (color.g * 255.).round() as u8,
                        (color.b * 255.).round() as u8,
                        (color.a * 255.).round() as u8,
                    ];
                    v.remember_fields(cx, 2);
                    if let Some(i) = v.selected.filter(|i| *i < v.doc.objects.len()) {
                        v.doc.checkpoint();
                        v.doc.objects[i].color = v.color;
                        v.refresh(cx);
                    }
                    cx.notify();
                }
            });
        let color_observer = cx.observe(&color_picker, |_, _, cx| cx.notify());
        let next_number = doc
            .objects
            .iter()
            .filter(|a| a.tool == Tool::Number)
            .filter_map(|a| a.text.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Self {
            preferences: preferences.clone(),
            toolbar_rows: [0.; 2],
            size_input,
            _size_subscription: size_subscription,
            capture_context: None,
            next_number,
            doc,
            transform,
            host,
            rendered: initial.clone(),
            texture: app::texture(&initial),
            tool: initial_tool,
            color: style.color,
            color_picker,
            _color_subscription: color_subscription,
            _color_observer: color_observer,
            pending_annotations: vec![],
            texture_window: w.window_handle(),
            popup_open: false,
            width: style.width,
            arrow_style: style.arrow,
            pointer: None,
            text_anchor: None,
            text_index: None,
            text_tool: Tool::Text,
            watermark: snip_core::annotation::WatermarkStyle {
                density: snip_core::annotation::WatermarkDensity::Medium,
                bold: false,
            },
            _text_subscription: text_subscription,
            last_wheel: None,
            draft: None,
            selected: None,
            geometry_drag: None,
            moving_annotation: false,
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
            toast_last: String::new(),
            toast_visible: false,
            toast_generation: 0,
            generation: 0,
            rendering: false,
            dirty: false,
            ocr: None,
            ocr_text: SharedString::default(),
            ocr_field: cx.new(|cx| TextareaState::new(w, cx).soft_wrap(false)),
            ocr_mode: None,
            ocr_size: size(px(0.), px(124.)),
            ocr_resize: None,
            ocr_request: 0,
            ocr_busy: false,
            ocr_scanning: false,
            cancel: Cancellation::default(),
            session_cancel: Cancellation::default(),
        }
    }
    fn remember(&mut self, cx: &mut Context<Self>) {
        self.remember_fields(cx, 7);
    }
    fn remember_fields(&mut self, cx: &mut Context<Self>, fields: u8) {
        let tool = self.option_tool();
        self.preferences = cx.global::<PreferencesStore>().value.clone();
        self.preferences.tool = self.tool;
        if self.tool.is_shape() {
            self.preferences.shape = self.tool;
        }
        if self.tool.is_line() {
            self.preferences.line = self.tool;
        }
        let mut style = self.preferences.style(tool);
        if fields & 1 != 0 {
            style.width = self.width;
        }
        if fields & 2 != 0 {
            style.color = self.color;
        }
        if fields & 4 != 0 {
            style.arrow = self.arrow_style;
        }
        self.preferences.styles.insert(tool, style);
        let store = cx.global_mut::<PreferencesStore>();
        if store.value != self.preferences {
            store.value = self.preferences.clone();
            if let Some(writer) = &store.writer {
                let _ = writer.send(store.value.clone());
            }
        }
    }
    pub(crate) fn choose_tool(&mut self, tool: Tool, w: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(cx);
        self.finish_path(cx);
        self.preferences = cx.global::<PreferencesStore>().value.clone();
        self.tool = tool;
        self.selected = None;
        self.geometry_drag = None;
        self.last = None;
        self.last_wheel = None;
        let style = self.preferences.style(tool);
        self.width = style.width;
        self.color = style.color;
        self.arrow_style = style.arrow;
        self.color_picker.update(cx, |v, cx| {
            v.set_value(rgba(u32::from_be_bytes(style.color)), w, cx)
        });
        self.clear_ocr();
        self.message.clear();
        self.sync_size(w, cx);
        self.remember(cx);
        cx.notify();
    }
    fn key_down(
        &mut self,
        e: &KeyDownEvent,
        w: &mut Window,
        cx: &mut Context<Self>,
        require_canvas: bool,
    ) {
        if self.text_anchor.is_some() {
            if e.keystroke.key == "escape" {
                self.text_anchor = None;
                self.text_index = None;
                self.refresh(cx);
                self.focus.focus(w, cx);
                cx.notify();
            }
            return;
        }

        if require_canvas && !self.focus.is_focused(w) {
            return;
        }
        let modifier = if cfg!(target_os = "macos") {
            e.keystroke.modifiers.platform
        } else {
            e.keystroke.modifiers.control
        };
        match e.keystroke.key.as_str() {
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "b" | "e" if !modifier => {
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
                let tool = if tool == Tool::Line {
                    self.preferences.line
                } else {
                    tool
                };
                self.choose_tool(tool, w, cx);
            }
            "o" if !modifier => self.recognize(cx),
            "w" if !modifier => self.choose_tool(Tool::Watermark, w, cx),
            "p" if !modifier => self.export(2, cx),
            "enter" if self.tool == Tool::Polyline => self.finish_path(cx),
            "l" | "g" if self.capture_context.is_some() => {
                let (display, rect) = self.capture_context.clone().unwrap();
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
            "escape" => self.cancel(cx),
            "space" if matches!(self.host, EditorHost::Pin(_)) => self.export(3, cx),
            "z" if modifier => {
                self.selected = None;
                self.clear_ocr();
                if e.keystroke.modifiers.shift {
                    self.doc.redo();
                } else {
                    self.doc.undo();
                }
                self.refresh(cx);
            }
            "c" if modifier && self.ocr.is_none() && !self.ocr_busy => self.export(0, cx),
            "s" if modifier => self.export(1, cx),
            "backspace" | "delete" => {
                if let Some(i) = self.selected.take().filter(|i| *i < self.doc.objects.len()) {
                    self.doc.checkpoint();
                    self.doc.objects.remove(i);
                    self.refresh(cx);
                }
            }
            _ => {}
        }
    }
    pub fn set_capture_context(&mut self, display: CapturedDisplay, rect: Rect) {
        self.capture_context = Some((display, rect));
    }
    fn display_size(&self) -> f32 {
        if self.option_tool().is_typographic() {
            self.width * 6. + 12.
        } else {
            self.width
        }
    }
    fn set_size(&mut self, size: f32, cx: &mut Context<Self>) {
        if !size.is_finite() || size <= 0. || size > 512. {
            return;
        }
        let width = if self.option_tool().is_typographic() {
            (size.max(13.) - 12.) / 6.
        } else {
            size.clamp(0.5, 128.)
        };
        if (width - self.width).abs() < 0.001 {
            return;
        }
        self.width = width;
        self.remember_fields(cx, 1);
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
                self.add_selected(draft, cx);
            }
        }
    }
    fn add_selected(&mut self, annotation: Annotation, cx: &mut Context<Self>) {
        self.pending_annotations
            .push((annotation.clone(), self.generation + 1));
        self.doc.add(annotation);
        self.selected = Some(self.doc.objects.len() - 1);
        self.last_wheel = None;
        self.refresh(cx);
    }
    fn geometry_handle_at(&self, mouse: Point<Pixels>) -> Option<GeometryHandle> {
        let object = self.doc.objects.get(self.selected?)?;
        geometry_handles(object)
            .into_iter()
            .find_map(|(handle, p)| {
                let p = self.transform.forward(p);
                let position = self.origin + point(px(p.x * self.scale), px(p.y * self.scale));
                ((position.x - mouse.x).abs() <= px(5.) && (position.y - mouse.y).abs() <= px(5.))
                    .then_some(handle)
            })
    }
    fn resize_geometry(&mut self, p: ImagePoint, cx: &mut Context<Self>) {
        let Some(drag) = self.geometry_drag.as_mut() else {
            return;
        };
        let Some(index) = self.selected.filter(|&i| i < self.doc.objects.len()) else {
            return;
        };
        let limit = Rect::new(
            0.,
            0.,
            self.doc.original.width() as f32,
            self.doc.original.height() as f32,
        );
        let dx = p.x - drag.start.x;
        let dy = p.y - drag.start.y;
        if !drag.checkpointed && dx == 0. && dy == 0. {
            return;
        }
        let points = match drag.handle {
            GeometryHandle::Endpoint(i) => {
                let mut points = drag.points.clone();
                points[i].x = (points[i].x + dx).clamp(0., limit.width);
                points[i].y = (points[i].y + dy).clamp(0., limit.height);
                points
            }
            GeometryHandle::Edge(handle) => {
                use SelectionHandle::*;
                let old = drag.bounds;
                let x = if matches!(handle, NorthWest | West | SouthWest) {
                    old.x
                } else {
                    old.right()
                };
                let y = if matches!(handle, NorthWest | North | NorthEast) {
                    old.y
                } else {
                    old.bottom()
                };
                let new = resize_selection(old, handle, ImagePoint::new(x + dx, y + dy), limit);
                drag.points
                    .iter()
                    .map(|p| {
                        ImagePoint::new(
                            new.x + (p.x - old.x) * (new.width / old.width),
                            new.y + (p.y - old.y) * (new.height / old.height),
                        )
                    })
                    .collect()
            }
        };
        if points == self.doc.objects[index].points {
            return;
        }
        if !drag.checkpointed {
            self.doc.checkpoint();
            self.pending_annotations.clear();
            drag.checkpointed = true;
        }
        self.doc.objects[index].points = points;
        self.refresh(cx);
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
            watermark: Default::default(),
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
        if self.text_anchor.is_some() {
            self.text.update(cx, |v, cx| v.focus(w, cx));
        } else {
            self.focus.focus(w, cx);
        }
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
        self.pending_annotations.clear();
        self.doc.reframe(image.clone(), dx, dy);
        self.transform = identity(&image);
        self.rendered = image.clone();
        let old = std::mem::replace(&mut self.texture, app::texture(&image));
        app::retire_texture(self.texture_window, old, cx);
        self.origin = bounds.origin;
        self.view_size = bounds.size;
        self.scale = f32::from(bounds.size.width) / image.width() as f32;
        self.draft = None;
        self.last = None;
        self.selected = None;
        self.geometry_drag = None;
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
        self.text_tool = existing.as_ref().map_or(
            if self.tool == Tool::Watermark {
                Tool::Watermark
            } else {
                Tool::Text
            },
            |a| a.tool,
        );
        let value = existing.map_or_else(String::new, |a| {
            self.width = a.width;
            self.color = a.color;
            if a.tool == Tool::Watermark {
                self.watermark = a.watermark;
            }
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
    #[cfg(debug_assertions)]
    pub(crate) fn fixture_watermark(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.choose_tool(Tool::Watermark, w, cx);
        self.begin_text(ImagePoint::new(290., 310.), None, w, cx);
        self.width = 4.;
        self.text
            .update(cx, |t, cx| t.set_value("Simple Snip", w, cx));
        self.commit_text(cx);
    }
    fn text_top_left(&self, anchor: ImagePoint) -> ImagePoint {
        let mut point = self.transform.forward(anchor);
        if self.text_index.is_none() {
            // For new text, keep the first line's center at the pointer even
            // if the font size changes while typing.
            point.y -= (self.width * 6. + 12.) * 1.3 / 2.;
        }
        point
    }

    fn apply_watermark_style(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.selected.filter(|i| {
            self.doc
                .objects
                .get(*i)
                .is_some_and(|a| a.tool == Tool::Watermark)
        }) {
            self.doc.checkpoint();
            self.doc.objects[index].watermark = self.watermark;
            self.refresh(cx);
        }
        cx.notify();
    }

    fn commit_text(&mut self, cx: &mut Context<Self>) {
        let Some(anchor) = self.text_anchor.take() else {
            return;
        };
        let p = if self.text_index.is_some() {
            anchor
        } else {
            self.transform.inverse(self.text_top_left(anchor))
        };
        let text = self.text.read(cx).value().to_string();
        let index = self.text_index.take();
        if !text.trim().is_empty() {
            let font = self.width * 6. + 12.;
            let a = Annotation {
                tool: self.text_tool,
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
                watermark: self.watermark,
            };
            self.pending_annotations
                .push((a.clone(), self.generation + 1));
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
                    * if self.doc.objects[i].tool.is_typographic() {
                        1. / 6.
                    } else {
                        1.
                    })
            .clamp(
                if self.doc.objects[i].tool.is_typographic() {
                    1. / 6.
                } else {
                    0.5
                },
                if self.doc.objects[i].tool.is_typographic() {
                    (512. - 12.) / 6.
                } else {
                    128.
                },
            );
            self.width = self.doc.objects[i].width;
            self.pending_annotations.clear();
            self.refresh(cx);
        } else if self.tool != Tool::Select {
            self.width = (self.width
                + delta.signum()
                    * if self.tool.is_typographic() {
                        1. / 6.
                    } else {
                        1.
                    })
            .clamp(
                if self.tool.is_typographic() {
                    1. / 6.
                } else {
                    0.5
                },
                if self.tool.is_typographic() {
                    (512. - 12.) / 6.
                } else {
                    128.
                },
            );
            cx.notify();
        }
        self.remember_fields(cx, 1);
    }
    fn over_geometry_handle(&self) -> bool {
        self.geometry_drag.is_some()
            || self
                .pointer
                .is_some_and(|p| self.geometry_handle_at(p + self.origin).is_some())
    }
    fn cursor_indicator(&self) -> Option<AnyElement> {
        let p = self.pointer?;
        if self.text_anchor.is_some()
            || (self.tool == Tool::Select || self.tool.is_text())
            || self.ocr.is_some()
            || self.over_geometry_handle()
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
        self.ocr_scanning = false;
        self.ocr_text = SharedString::default();
        self.ocr_resize = None;
        self.ocr_request = self.ocr_request.wrapping_add(1);
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.rendering {
            self.dirty = true;
            return;
        }
        self.rendering = true;
        let generation = self.generation;
        let source = self.doc.original.clone();
        let mut doc = self.doc.render_snapshot();
        if let Some(i) = self.text_index.filter(|i| *i < doc.objects.len()) {
            doc.objects.remove(i);
        }
        let transform = self.transform;
        let render = cx.background_executor().spawn(async move {
            render_document(doc, transform).map(|(doc, image)| {
                let texture = app::texture(&image);
                (doc, image, texture)
            })
        });
        cx.spawn(async move |this, cx| {
            let result = render.await;
            let _ = this.update(cx, |v, cx| {
                v.rendering = false;
                if Arc::ptr_eq(&v.doc.original, &source) && generation == v.generation {
                    match result {
                        Ok((doc, image, texture)) => {
                            v.doc.font = doc.font;
                            let old = std::mem::replace(&mut v.texture, texture);
                            app::retire_texture(v.texture_window, old, cx);
                            v.pending_annotations
                                .retain(|(_, revision)| *revision > generation);
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
    fn export_action(&self, action: u8) -> u8 {
        if action == 2 && matches!(self.host, EditorHost::Pin(_)) {
            4
        } else {
            action
        }
    }
    pub fn export(&mut self, action: u8, cx: &mut Context<Self>) {
        let action = self.export_action(action);
        self.commit_text(cx);
        self.finish_path(cx);
        let doc = if action == 3 || action == 4 {
            self.doc.clone()
        } else {
            self.doc.render_snapshot()
        };
        let transform = self.transform;
        let host = self.host;
        let placement = self.pin_placement();
        let cancel = self.session_cancel.clone();
        let tx = cx.global::<Runtime>().tx.clone();
        self.message = "正在合成…".into();
        let render = cx
            .background_executor()
            .spawn(async move { render_document(doc, transform) });
        cx.spawn(async move |this, cx| {
            let result = render.await;
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
                    let _ = this.update(cx, |v, cx| {
                        v.copied(result.map_err(|e| format!("复制失败: {e}")), cx);
                    });
                }
                1 => cx.update(|cx| app::save_image(image, cx)),
                2 => {
                    let prepared = cx
                        .background_executor()
                        .spawn(async move { app::PreparedImage::new(image) })
                        .await;
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
                        if action == 4 {
                            let _ = tx.send(Event::RevealPin(id)).await;
                        }
                    }
                    EditorHost::Capture => {}
                },
            }
        })
        .detach();
        cx.notify();
    }
    fn copied(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        if result.is_ok() && matches!(self.host, EditorHost::Capture) {
            app::close_overlays(cx);
        } else {
            self.toast_last.clear();
            self.message = result
                .err()
                .unwrap_or_else(|| "已复制图片，可按 F3 贴图".into());
            cx.notify();
        }
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
    pub(crate) fn recognize(&mut self, cx: &mut Context<Self>) {
        self.commit_text(cx);
        if self.ocr_busy {
            return;
        }
        self.clear_ocr();
        self.ocr_busy = true;
        self.ocr_scanning = true;
        self.cancel = Cancellation::default();
        let cancel = self.cancel.clone();
        let request = self.ocr_request;
        let document = self.doc.render_snapshot();
        let transform = self.transform;
        let generation = self.generation;
        let modes = &cx.global::<Runtime>().config.ocr;
        let profile = modes.profiles[self
            .ocr_mode
            .filter(|i| *i < modes.profiles.len())
            .unwrap_or(modes.active)]
        .clone();
        self.message.clear();
        cx.spawn(async move |this, cx| {
            let result = smol::unblock(move || -> anyhow::Result<OcrResult> {
                if cancel.cancelled() {
                    return Ok(OcrResult::default());
                }
                let (_, image) = render_document(document, transform)?;
                #[cfg(target_os = "windows")]
                {
                    crate::windows_ocr::recognize(&image, &profile.rapid, &cancel)
                }
                #[cfg(target_os = "macos")]
                {
                    snip_platform::recognize_with_vision(&image, &profile.vision, &cancel)
                }
            })
            .await;
            let _ = this.update(cx, |v, cx| {
                v.ocr_busy = false;
                v.ocr_scanning = false;
                if v.ocr_request != request || v.generation != generation {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(result) => {
                        tracing::debug!(lines = result.lines.len(), "OCR finished");
                        v.ocr_text = result.text().into();
                        v.ocr = Some(Arc::new(result));
                        v.tool = Tool::Select;
                        v.selected = None;
                    }
                    Err(error) => v.message = format!("识别失败：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    #[cfg(debug_assertions)]
    pub(crate) fn fixture_ocr_result(&mut self, cx: &mut Context<Self>) {
        self.ocr = Some(Arc::new(OcrResult::default()));
        self.ocr_text = "Capture ideas. Keep your flow.\n本周待办\n完善截图与贴图交互\n自由框选 / 独立贴图 / 原位标注\n本地处理，专注当下。".into();
        self.tool = Tool::Select;
        self.selected = None;
        cx.notify();
    }
    fn copy_ocr(&mut self, table: bool, cx: &mut Context<Self>) {
        let Some(result) = &self.ocr else {
            return;
        };
        let text = if table {
            result.tsv().into()
        } else {
            self.ocr_text.clone()
        };
        if !text.trim().is_empty() {
            self.toast_last.clear();
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
            self.message = "文字已复制".into();
            cx.notify();
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
        if self.ocr.is_some() || self.ocr_scanning {
            return;
        }
        if self.text_anchor.is_some() {
            self.commit_text(cx);
        }
        self.last_wheel = None;
        self.geometry_drag = None;
        self.last = None;
        self.moving_annotation = false;
        if let Some(handle) = self.geometry_handle_at(e.position)
            && let Some(object) = self.selected.and_then(|index| self.doc.objects.get(index))
        {
            self.geometry_drag = Some(GeometryDrag {
                handle,
                start: p,
                bounds: object.bounds(),
                points: object.points.clone(),
                checkpointed: false,
            });
            return;
        }
        match self.tool {
            Tool::Select => {
                self.selected = self.doc.hit(p);
                if let Some(i) = self.selected {
                    self.width = self.doc.objects[i].width;
                    self.color = self.doc.objects[i].color;
                    self.arrow_style = self.doc.objects[i].arrow_style;
                    self.color_picker.update(cx, |picker, cx| {
                        picker.set_value(rgba(u32::from_be_bytes(self.color)), w, cx)
                    });
                    if e.click_count == 2 && self.doc.objects[i].tool.is_text() {
                        self.begin_text(p, Some(i), w, cx);
                        return;
                    }
                }
                self.sync_size(w, cx);
                if self.selected.is_some() {
                    self.last = Some(p);
                }
            }
            Tool::Eraser => {
                self.selected = None;
                self.doc.remove_at_radius(p, self.width * 2. + 4.);
                self.refresh(cx);
            }
            Tool::Text | Tool::Watermark => {
                let index = self
                    .doc
                    .hit(p)
                    .filter(|i| self.doc.objects[*i].tool == self.tool);
                self.begin_text(p, index, w, cx);
            }
            Tool::Mosaic | Tool::Blur => {
                self.paint_filter(p, true, cx);
            }
            Tool::Polyline => {
                if e.click_count == 2 {
                    self.finish_path(cx);
                } else if let Some(draft) = &mut self.draft {
                    if draft.points.len() < 8192
                        && draft
                            .points
                            .last()
                            .is_none_or(|last| (p.x - last.x).hypot(p.y - last.y) >= 1.)
                    {
                        draft.points.push(p);
                    }
                } else {
                    self.selected = None;
                    self.draft = Some(Annotation {
                        tool: self.tool,
                        points: vec![p, p],
                        color: self.color,
                        width: self.width,
                        text: String::new(),
                        arrow_style: self.arrow_style,
                        watermark: Default::default(),
                    });
                }
            }
            Tool::Number => {
                let number = self.next_number;
                self.next_number = number.saturating_add(1);
                self.remember(cx);
                self.add_selected(
                    Annotation {
                        tool: Tool::Number,
                        points: vec![p],
                        color: self.color,
                        width: self.width,
                        text: number.to_string(),
                        arrow_style: ArrowStyle::default(),
                        watermark: Default::default(),
                    },
                    cx,
                );
            }
            _ => {
                self.selected = None;
                self.draft = Some(Annotation {
                    tool: self.tool,
                    points: vec![p, p],
                    color: self.color,
                    width: self.width,
                    text: String::new(),
                    arrow_style: self.arrow_style,
                    watermark: Default::default(),
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
        let p = self.image_point(e.position);
        if self.ocr.is_some() || self.ocr_scanning {
            return;
        }
        cx.notify();
        if self.geometry_drag.is_some() {
            if e.dragging() {
                self.resize_geometry(p, cx);
            } else {
                self.geometry_drag = None;
            }
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
            && i < self.doc.objects.len()
            && p != last
        {
            if !self.moving_annotation {
                self.doc.checkpoint();
                self.moving_annotation = true;
            }
            self.doc.objects[i].translate(p.x - last.x, p.y - last.y);
            self.last = Some(p);
            self.refresh(cx);
        }
    }
    fn pointer_up(&mut self, e: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.ocr_resize.take().is_some() {
            return;
        }
        self.last = None;
        self.moving_annotation = false;
        if self.geometry_drag.is_some() {
            self.resize_geometry(self.image_point(e.position), cx);
            self.geometry_drag = None;
            cx.notify();
        }
        if self
            .draft
            .as_ref()
            .is_some_and(|a| a.tool == Tool::Polyline)
        {
            return;
        }
        if let Some(draft) = self.draft.take() {
            self.add_selected(draft, cx);
            cx.notify();
        }
    }
    fn tools_width(&self) -> f32 {
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
        base.max(options) + 10.
    }
    fn ocr_width(&self) -> Pixels {
        if self.ocr_size.width > px(0.) {
            self.ocr_size.width
        } else {
            px(self.tools_width())
        }
    }
    pub fn has_ocr_result(&self) -> bool {
        self.ocr.is_some()
    }
    fn toolbar_width(&self) -> f32 {
        if self.ocr.is_some() {
            self.tools_width().max(f32::from(self.ocr_width()))
        } else {
            self.tools_width()
        }
    }
    fn host_height(&self, cx: &App) -> f32 {
        self.toolbar_height()
            + if self.toast_visible { 34. } else { 0. }
            + if self.popup_open || self.color_picker.read(cx).is_open() {
                420.
            } else {
                0.
            }
    }
    fn toolbar_height(&self) -> f32 {
        36. + if self.ocr.is_some() {
            4. + f32::from(self.ocr_size.height)
        } else if self.tool != Tool::Select || self.selected.is_some() {
            38.
        } else {
            0.
        }
    }
    fn toolbar(&mut self, w: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
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
                if width.is_finite()
                    && width > 0.
                    && weak
                        .upgrade()
                        .is_some_and(|v| (v.read(cx).toolbar_rows[index] - width).abs() > 1.)
                {
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
            (
                "模糊 / 马赛克",
                if self.tool == Tool::Mosaic {
                    Tool::Mosaic
                } else {
                    Tool::Blur
                },
            ),
            ("橡皮擦", Tool::Eraser),
            ("图片水印", Tool::Watermark),
        ];
        let mut strip = div()
            .on_children_prepainted(row_measure(0))
            .id("annotation-tools-strip")
            .h_flex()
            .gap(px(1.))
            .w_full()
            .justify_start()
            .flex_nowrap()
            .overflow_x_scroll();
        if let Some((display, rect)) = self.capture_context.clone() {
            strip = strip.child(capture_mode_button(display, rect));
            strip = strip.child(design::divider(cx));
        }
        let modes = cx
            .try_global::<Runtime>()
            .map(|runtime| runtime.config.ocr.clone())
            .unwrap_or_default();
        let mode = self
            .ocr_mode
            .filter(|index| *index < modes.profiles.len())
            .unwrap_or(modes.active);
        let shortcuts = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "B", "E", "W"];
        for (i, (label, tool)) in tools.into_iter().enumerate() {
            let shortcut = shortcuts[i];
            if matches!(
                tool,
                Tool::Rectangle | Tool::Pen | Tool::Blur | Tool::Mosaic
            ) {
                strip = strip.child(design::divider(cx));
            }
            strip = strip.child(
                Button::new(("tool", i))
                    .small()
                    .compact()
                    .w(px(28.))
                    .flex_shrink_0()
                    .ghost()
                    .child(
                        div()
                            .relative()
                            .size(px(23.))
                            .flex()
                            .justify_center()
                            .items_center()
                            .child(tool_icon(tool).size(px(14.)))
                            .child(
                                div()
                                    .absolute()
                                    .bottom_0()
                                    .right_0()
                                    .text_size(px(8.))
                                    .line_height(px(8.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(shortcut),
                            ),
                    )
                    .accessibility_label(label)
                    .tooltip(if tool == Tool::Number {
                        format!("{label} [{shortcut}] · 下一个 {}", self.next_number)
                    } else if tool == Tool::Watermark {
                        format!("{label} [{shortcut}] · 在图片上点击输入")
                    } else {
                        format!("{label} [{shortcut}]")
                    })
                    .selected(
                        (self.tool == tool
                            || tool == Tool::Rectangle
                                && matches!(
                                    self.tool,
                                    Tool::RoundedRectangle | Tool::Diamond | Tool::Triangle
                                )
                            || tool == Tool::Line && self.tool.is_line())
                            && self.ocr.is_none()
                            && !self.ocr_scanning,
                    )
                    .on_click(cx.listener(move |v, _, w, cx| {
                        let tool = if tool == Tool::Line {
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
            ("识别文字", "ocr"),
            ("保存图片", "save"),
            ("复制图片", "copy"),
            ("贴到桌面", "pin"),
            ("完成编辑", "done"),
            ("取消编辑", "close"),
        ];
        strip = strip.child(design::divider(cx));
        for (i, (label, name)) in actions.into_iter().enumerate() {
            if name == "done" && matches!(self.host, EditorHost::Capture) {
                continue;
            }
            if name == "ocr" || name == "save" {
                strip = strip.child(design::divider(cx));
            }
            if name == "ocr" {
                let editor = cx.entity().downgrade();
                let observer = editor.clone();
                let profiles = modes.profiles.clone();
                strip = strip.child(
                    Button::new("ocr-mode")
                        .xsmall()
                        .ghost()
                        .flex_shrink_0()
                        .label(profiles[mode].name.clone())
                        .accessibility_label("选择 OCR 模式")
                        .dropdown_menu(move |mut menu, _, _| {
                            for (index, profile) in profiles.iter().enumerate() {
                                let editor = editor.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(profile.name.clone())
                                        .checked(index == mode)
                                        .on_click(move |_, _, cx| {
                                            let _ = editor.update(cx, |v, cx| {
                                                v.ocr_mode = Some(index);
                                                cx.notify();
                                            });
                                        }),
                                );
                            }
                            menu.max_h(px(320.)).scrollable(true)
                        })
                        .on_open_change(move |open, _, cx| {
                            let editor = observer.clone();
                            let open = *open;
                            cx.defer(move |cx| {
                                let _ = editor.update(cx, |v, cx| {
                                    v.popup_open = open;
                                    cx.notify();
                                });
                            });
                        }),
                );
            }
            strip = strip.child(
                Button::new(("action", i))
                    .small()
                    .flex_shrink_0()
                    .ghost()
                    .when(
                        name == "copy" && matches!(self.host, EditorHost::Capture),
                        |b| b.primary(),
                    )
                    .icon(action_icon(name))
                    .disabled(name == "ocr" && self.ocr_busy)
                    .selected(name == "ocr" && (self.ocr_busy || self.ocr.is_some()))
                    .accessibility_label(label)
                    .tooltip(format!(
                        "{} ({})",
                        label,
                        match name {
                            "undo" => design::shortcut("Z", "⌘Z"),
                            "redo" => design::shortcut("Shift+Z", "⇧⌘Z"),
                            "save" => design::shortcut("S", "⌘S"),
                            "copy" => design::shortcut("C", "⌘C"),
                            "close" => "Esc".into(),
                            "done" => "Space".into(),
                            "ocr" => "O".into(),
                            _ => "P".into(),
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
            .min((f32::from(w.viewport_size().width) - 16.).max(1.));
        let has_options =
            self.ocr.is_none() && (self.tool != Tool::Select || self.selected.is_some());
        let height = self
            .toolbar_height()
            .min((f32::from(w.viewport_size().height) - 16.).max(1.));
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
        let mut panel = design::panel(cx)
            .id("floating-annotation-toolbar")
            .w(px(self.tools_width()))
            .max_w_full()
            .v_flex()
            .items_start()
            .gap_1()
            .p_1()
            .shadow_sm()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(strip);
        if has_options {
            let mut options = div()
                .on_children_prepainted(row_measure(1))
                .id("annotation-options")
                .h_flex()
                .gap_1()
                .items_center()
                .justify_start()
                .flex_nowrap()
                .when(
                    !self.color_picker.read(cx).is_open() && !self.popup_open,
                    |d| d.overflow_x_scroll(),
                );
            if matches!(self.tool, Tool::Blur | Tool::Mosaic) {
                for (tool, label) in [(Tool::Blur, "高斯模糊"), (Tool::Mosaic, "马赛克")] {
                    options = options.child(
                        Button::new(label)
                            .xsmall()
                            .label(label)
                            .selected(self.tool == tool)
                            .flex_shrink_0()
                            .on_click(cx.listener(move |v, _, w, cx| v.choose_tool(tool, w, cx))),
                    );
                }
            }
            if self.option_tool() == Tool::Watermark {
                use snip_core::annotation::WatermarkDensity;
                for (label, density) in [
                    ("单个", WatermarkDensity::Single),
                    ("疏", WatermarkDensity::Sparse),
                    ("中", WatermarkDensity::Medium),
                    ("密", WatermarkDensity::Dense),
                ] {
                    options = options.child(
                        Button::new(("watermark-density", density as usize))
                            .xsmall()
                            .label(label)
                            .selected(self.watermark.density == density)
                            .tooltip(format!("水印密度 · {label}"))
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.watermark.density = density;
                                v.apply_watermark_style(cx);
                            })),
                    );
                }
                options = options.child(
                    Button::new("watermark-bold")
                        .xsmall()
                        .label("加粗")
                        .selected(self.watermark.bold)
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.watermark.bold = !v.watermark.bold;
                            v.apply_watermark_style(cx);
                        })),
                );
            }
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
            options = options.child(
                ColorPicker::new(&self.color_picker)
                    .small()
                    .accessibility_label("标注颜色"),
            );
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
                            .icon(tool_icon(tool))
                            .tooltip(label)
                            .accessibility_label(label)
                            .selected(self.tool == tool)
                            .on_click(cx.listener(move |v, _, w, cx| v.choose_tool(tool, w, cx))),
                    );
                }
            }
            if self.option_tool() == Tool::Arrow {
                options = options.child(arrow_style_button(
                    self.arrow_style,
                    cx.entity().downgrade(),
                    cx,
                ));
            }
            panel = panel.child(options);
        }
        let result_height = if matches!(self.host, EditorHost::Capture) {
            self.ocr_size.height.min(px((height - 40.).max(32.)))
        } else {
            self.ocr_size.height
        };
        let toast = self.toast_visible.then(|| {
            design::panel(cx)
                .absolute()
                .right_0()
                .top(px(
                    if matches!(self.host, EditorHost::Capture)
                        && global_y + height + 36. > f32::from(w.viewport_size().height)
                    {
                        -33.
                    } else {
                        self.toolbar_height().min(height) + 4.
                    },
                ))
                .max_w_full()
                .px_2()
                .py_1()
                .shadow_sm()
                .child(design::detail(cx).child(self.message.clone()))
        });
        div()
            .id("annotation-anchor")
            .when(matches!(self.host, EditorHost::Capture), |d| {
                d.absolute()
                    .left(px(global_x - ox))
                    .top(px(global_y - oy))
                    .w(px(width))
            })
            .when(matches!(self.host, EditorHost::Pin(_)), |d| {
                d.w_full().relative()
            })
            .v_flex()
            .gap_1()
            .items_end()
            .child(panel)
            .when(self.ocr.is_some(), |d| {
                d.child(self.ocr_panel(result_height, w, cx))
            })
            .when(self.ocr_resize.is_some(), |d| {
                let moving = cx.entity().downgrade();
                let ending = moving.clone();
                d.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let moving = moving.clone();
                            window.on_mouse_event(move |e: &MouseMoveEvent, phase, w, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = moving.update(cx, |v, cx| v.resize_ocr(e, w, cx));
                                }
                            });
                            let ending = ending.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = ending.update(cx, |v, cx| {
                                        v.ocr_resize = None;
                                        cx.notify();
                                    });
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
            .children(toast)
    }
    fn ocr_panel(
        &self,
        height: Pixels,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let empty = self.ocr_text.trim().is_empty();
        let text = if empty {
            "未识别到文字"
        } else {
            self.ocr_text.as_ref()
        };
        if self.ocr_field.read(cx).value().as_ref() != text {
            self.ocr_field
                .update(cx, |field, cx| field.set_value(text.to_owned(), w, cx));
        }
        design::panel(cx)
            .id("ocr-result")
            .relative()
            .w(self.ocr_width())
            .max_w_full()
            .h(height)
            .v_flex()
            .overflow_hidden()
            .shadow_sm()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h_flex()
                    .h(px(30.))
                    .flex_shrink_0()
                    .px_2()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .items_center()
                            .child(action_icon("ocr").size(px(13.)))
                            .child("识别结果"),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(
                                Button::new("ocr-tsv")
                                    .xsmall()
                                    .ghost()
                                    .disabled(empty)
                                    .icon(Icon::new(gpui_kit::assets::IconName::Grid2x2))
                                    .accessibility_label("复制表格 TSV")
                                    .tooltip("复制表格 TSV")
                                    .on_click(cx.listener(|v, _, _, cx| v.copy_ocr(true, cx))),
                            )
                            .child(
                                Button::new("ocr-copy")
                                    .xsmall()
                                    .ghost()
                                    .disabled(empty)
                                    .icon(action_icon("copy"))
                                    .accessibility_label("复制识别结果")
                                    .tooltip("复制识别结果")
                                    .on_click(cx.listener(|v, _, _, cx| v.copy_ocr(false, cx))),
                            ),
                    ),
            )
            .child(
                div().flex_1().min_h_0().px_2().child(
                    Textarea::new(&self.ocr_field)
                        .h((height - px(34.)).max(px(22.)))
                        .readonly(true)
                        .bordered(false)
                        .appearance(false)
                        .aria_label("识别结果，可选择复制")
                        .text_size(px(12.)),
                ),
            )
            .child(div().h(px(14.)).flex_shrink_0())
            .child(
                div()
                    .id("ocr-resize")
                    .debug_selector(|| "ocr-resize-grip".to_string())
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .size(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::ResizeUpRightDownLeft)
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(gpui_kit::assets::IconName::MoveDiagonal2).size(px(10.)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|v, e: &MouseDownEvent, w, cx| {
                            v.ocr_resize = Some((
                                e.position + w.bounds().origin,
                                size(v.ocr_width(), v.ocr_size.height),
                            ));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            )
    }
    fn resize_ocr(&mut self, e: &MouseMoveEvent, w: &Window, cx: &mut Context<Self>) -> bool {
        if let Some((start, original)) = self.ocr_resize {
            if !e.dragging() {
                self.ocr_resize = None;
                return false;
            }
            let delta = e.position + w.bounds().origin - start;
            let next = size(
                (original.width - delta.x).clamp(px(250.), px(1200.)),
                (original.height + delta.y).clamp(px(124.), px(440.)),
            );
            if self.ocr_size != next {
                self.ocr_size = next;
                cx.notify();
            }
            return true;
        }
        false
    }
}

impl Render for EditorView {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !toast_preferences(cx).0 {
            self.toast_visible = false;
        }
        if self.message != self.toast_last {
            self.toast_last = self.message.clone();
            self.toast_generation += 1;
            let (enabled, _) = toast_preferences(cx);
            self.toast_visible = !self.message.is_empty() && enabled;
            let generation = self.toast_generation;
            let (_, delay) = toast_preferences(cx);
            if self.toast_visible {
                let timer = cx
                    .background_executor()
                    .timer(std::time::Duration::from_millis(delay as u64));
                cx.spawn(async move |this, cx| {
                    timer.await;
                    let _ = this.update(cx, |v, cx| {
                        if v.toast_generation == generation && v.message == v.toast_last {
                            v.toast_visible = false;
                            v.message.clear();
                            v.toast_last.clear();
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
        let weak = cx.entity().downgrade();
        let scale = self.scale;
        let transform = self.transform;
        let drafts: Vec<_> = self
            .draft
            .iter()
            .cloned()
            .chain(
                self.pending_annotations
                    .iter()
                    .filter(|(a, _)| !a.tool.is_typographic())
                    .map(|(a, _)| a.clone()),
            )
            .collect();
        let selected = self
            .selected
            .filter(|_| self.text_anchor.is_none())
            .and_then(|i| self.doc.objects.get(i));
        let selected_handles = selected.map_or_else(Vec::new, geometry_handles);
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
                for a in &drafts {
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
                        let (shaft, head, filled) = a.arrow_parts();
                        for (points, fill) in [(shaft, true), (head, filled)] {
                            let mut path = if fill {
                                PathBuilder::fill()
                            } else {
                                PathBuilder::stroke(px(if a.tool == Tool::Highlight {
                                    a.width * 5.
                                } else {
                                    a.width
                                } * scale))
                            };
                            if let Some(first) = points.first() {
                                path.move_to(map(*first));
                                for p in points.iter().skip(1) {
                                    path.line_to(map(*p));
                                }
                                path.line_to(map(*first));
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, rgba(u32::from_be_bytes(a.ink())));
                                }
                            }
                        }
                        points.clear();
                    }
                    let mut path = if a.tool == Tool::Arrow {
                        PathBuilder::fill()
                    } else {
                        PathBuilder::stroke(px(if a.tool == Tool::Highlight {
                            a.width * 5.
                        } else {
                            a.width
                        } * scale))
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
                                        | if a.tool == Tool::Highlight {
                                            70
                                        } else {
                                            a.color[3] as u32
                                        },
                                ),
                            );
                        }
                    }
                }
                if !selected_handles.is_empty() {
                    for &(_, p) in &selected_handles {
                        let p = map(p);
                        window.paint_quad(
                            fill(
                                Bounds::new(p - point(px(2.5), px(2.5)), size(px(5.), px(5.))),
                                rgb(design::IMAGE_HANDLE),
                            )
                            .border_widths(px(1.))
                            .border_color(rgb(design::IMAGE_FOCUS)),
                        );
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
            .cursor(if self.over_geometry_handle() {
                CursorStyle::PointingHand
            } else {
                match self.tool {
                    Tool::Select => CursorStyle::Arrow,
                    Tool::Text | Tool::Watermark => CursorStyle::IBeam,
                    _ => CursorStyle::Crosshair,
                }
            })
            .on_hover(cx.listener(|v, inside: &bool, _, cx| {
                if !inside {
                    v.pointer = None;
                    cx.notify();
                }
            }))
            .size_full()
            .child(
                div().absolute().inset_0().overflow_hidden().child(
                    img(self.texture.clone())
                        .size_full()
                        .object_fit(ObjectFit::Fill),
                ),
            )
            .child(canvas)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|v, e, w, cx| v.down(e, w, cx)),
            )
            .on_mouse_move(cx.listener(|v, e, _, cx| v.moving(e, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|v, e, _, cx| v.pointer_up(e, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|v, e, _, cx| v.pointer_up(e, cx)),
            )
            .children(
                self.pending_annotations
                    .iter()
                    .filter(|(a, _)| a.tool.is_text())
                    .map(|(a, _)| {
                        let p = self.transform.forward(a.points[0]);
                        let font = (a.width * 6. + 12.) * self.scale;
                        div()
                            .absolute()
                            .left(px(p.x * self.scale))
                            .top(px(p.y * self.scale))
                            .font_family(annotation_font_family())
                            .text_size(px(font))
                            .line_height(px(font * 1.3))
                            .text_color(rgba(u32::from_be_bytes(a.color)))
                            .child(a.text.clone())
                    }),
            )
            .children(self.cursor_indicator())
            .children(self.text_anchor.map(|p| {
                let p = self.text_top_left(p);
                let font = (self.width * 6. + 12.) * self.scale;
                let line_height = px(font * 1.3);
                let value = self.text.read(cx).value();
                let mut text_font = w.text_style().font();
                text_font.family = annotation_font_family().into();
                let content_width = f32::from(
                    w.text_system()
                        .shape_line(
                            value.clone(),
                            px(font),
                            &[TextRun {
                                len: value.len(),
                                font: text_font,
                                ..Default::default()
                            }],
                            None,
                        )
                        .width,
                );
                div()
                    .id("image-text-input")
                    .absolute()
                    .left(px(p.x * self.scale))
                    .top(px(p.y * self.scale))
                    .w(px((content_width + 6.).max(16.)))
                    .h(line_height)
                    .font_family(annotation_font_family())
                    .text_size(px(font))
                    .line_height(line_height)
                    .bg(transparent_black())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(Styled::h(
                        Input::new(&self.text)
                            .id("annotation-text-field")
                            .aria_label("图片文字")
                            .appearance(false)
                            .bordered(false)
                            .p_0()
                            .font_family(annotation_font_family())
                            .text_size(px(font))
                            .line_height(line_height)
                            .text_color(rgba(u32::from_be_bytes(
                                self.text_tool.text_ink(self.color),
                            ))),
                        line_height,
                    ))
            }))
            .when(self.text_anchor.is_some(), |d| {
                let input = self.text.clone();
                d.child(
                    gpui_kit::canvas(
                        move |_, _, _| (),
                        move |_, _, w, cx| {
                            if let Some(text_bounds) = text_frame_bounds(input.read(cx)) {
                                w.paint_quad(
                                    outline(
                                        text_bounds,
                                        rgb(design::IMAGE_FOCUS),
                                        BorderStyle::Solid,
                                    )
                                    .border_widths(px(1.)),
                                );
                            }
                            if input.read(cx).focus_handle(cx).is_focused(w)
                                && let Some((bounds, _)) = input.read(cx).cursor_layout()
                            {
                                let origin = bounds.origin;
                                let height = bounds.size.height.max(px(1.));
                                w.paint_quad(fill(
                                    Bounds::new(
                                        origin - point(px(1.), px(0.)),
                                        size(px(3.), height),
                                    ),
                                    rgb(0xffffff),
                                ));
                                w.paint_quad(fill(
                                    Bounds::new(origin, size(px(1.), height)),
                                    rgb(0x101010),
                                ));
                            }
                        },
                    )
                    .absolute()
                    .inset_0()
                    .size_full(),
                )
            })
            .when(self.ocr_scanning, |d| {
                let h = f32::from(self.view_size.height).max(1.);
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .overflow_hidden()
                        .bg(rgba(0x11192375))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .absolute()
                                .h(px(2.))
                                .w_full()
                                .bg(rgb(design::IMAGE_FOCUS))
                                .with_animation(
                                    ("ocr-scan", self.ocr_request as usize),
                                    Animation::new(std::time::Duration::from_millis(1350))
                                        .repeat()
                                        .with_max_fps(30.),
                                    move |line, t| {
                                        line.top(px(h * t))
                                            .opacity(0.45 + 0.45 * (std::f32::consts::PI * t).sin())
                                    },
                                ),
                        )
                        .child(design::image_label().px_3().py_1().child("正在识别文字…")),
                )
            })
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
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| v.key_down(e, w, cx, true)))
    }
}
fn annotation_font_family() -> &'static str {
    if cfg!(target_os = "windows") {
        "Microsoft YaHei"
    } else {
        "Arial Unicode MS"
    }
}
fn text_frame_bounds(input: &InputState) -> Option<Bounds<Pixels>> {
    let content = input.input_bounds();
    let caret = input.cursor_layout()?.0;
    Some(Bounds::new(
        point(content.left(), caret.top()),
        size(content.size.width, caret.size.height),
    ))
}

#[inline(never)]
fn capture_mode_button(display: CapturedDisplay, rect: Rect) -> AnyElement {
    Button::new("capture-mode")
        .small()
        .flex_shrink_0()
        .icon(Icon::default().path("icons/frame.svg"))
        .accessibility_label("切换截图模式")
        .tooltip("切换截图模式")
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
        })
        .into_any_element()
}

#[inline(never)]
fn arrow_style_button(current: ArrowStyle, editor: WeakEntity<EditorView>, cx: &App) -> AnyElement {
    let observer = editor.clone();
    Button::new("arrow-style-menu")
        .xsmall()
        .child(arrow_preview(current, cx.theme().foreground))
        .accessibility_label("选择箭头样式")
        .tooltip("选择箭头样式")
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu;
            for (label, style) in [
                ("燕尾", ArrowStyle::Swallow),
                ("实心三角", ArrowStyle::TriangleFilled),
                ("空心三角", ArrowStyle::TriangleHollow),
                ("实心圆点", ArrowStyle::CircleFilled),
                ("空心圆点", ArrowStyle::CircleHollow),
                ("实心菱形", ArrowStyle::DiamondFilled),
                ("空心菱形", ArrowStyle::DiamondHollow),
                ("头尾等宽", ArrowStyle::Uniform),
                ("头宽尾细", ArrowStyle::HeadWide),
                ("头细尾宽", ArrowStyle::TailWide),
            ] {
                let editor = editor.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, cx| {
                        let active = style == current;
                        div()
                            .id(label)
                            .h_flex()
                            .items_center()
                            .justify_center()
                            .w(px(86.))
                            .h(px(22.))
                            .tooltip(move |w, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(label).build(w, cx)
                            })
                            .child(arrow_preview(
                                style,
                                if active {
                                    cx.theme().primary
                                } else {
                                    cx.theme().foreground
                                },
                            ))
                    })
                    .checked(style == current)
                    .on_click(move |_, _, cx| {
                        let _ = editor.update(cx, |v, cx| {
                            v.arrow_style = style;
                            v.remember_fields(cx, 4);
                            if let Some(i) = v.selected.filter(|i| {
                                *i < v.doc.objects.len() && v.doc.objects[*i].tool == Tool::Arrow
                            }) {
                                v.doc.checkpoint();
                                v.doc.objects[i].arrow_style = style;
                                v.refresh(cx);
                            }
                            cx.notify();
                        });
                    }),
                );
            }
            menu
        })
        .on_open_change(move |open, _, cx| {
            let editor = observer.clone();
            let open = *open;
            cx.defer(move |cx| {
                let _ = editor.update(cx, |v, cx| {
                    v.popup_open = open;
                    cx.notify();
                });
            });
        })
        .into_any_element()
}

fn arrow_preview(style: ArrowStyle, ink: Hsla) -> impl IntoElement {
    let sample = Annotation {
        tool: Tool::Arrow,
        points: vec![ImagePoint::new(3., 9.), ImagePoint::new(47., 9.)],
        color: [0; 4],
        width: 2.6,
        text: String::new(),
        arrow_style: style,
        watermark: Default::default(),
    };
    let (shaft, head, filled) = sample.arrow_parts();
    canvas(
        |_, _, _| (),
        move |bounds, _, w, _| {
            for (points, fill) in [(&shaft, true), (&head, filled)] {
                let Some(first) = points.first() else {
                    continue;
                };
                let map = |p: &ImagePoint| bounds.origin + point(px(p.x), px(p.y));
                let mut path = if fill {
                    PathBuilder::fill()
                } else {
                    PathBuilder::stroke(px(1.2))
                };
                path.move_to(map(first));
                for p in points.iter().skip(1) {
                    path.line_to(map(p));
                }
                path.close();
                if let Ok(path) = path.build() {
                    w.paint_path(path, ink);
                }
            }
        },
    )
    .w(px(50.))
    .h(px(18.))
}

fn tool_icon(tool: Tool) -> Icon {
    use gpui_kit::assets::IconName as Glyph;
    Icon::new(match tool {
        Tool::Select => Glyph::MousePointer2,
        Tool::Rectangle => Glyph::Square,
        Tool::RoundedRectangle => Glyph::RectangleHorizontal,
        Tool::Diamond => Glyph::Diamond,
        Tool::Triangle => Glyph::Triangle,
        Tool::Ellipse => Glyph::Circle,
        Tool::Line => Glyph::Slash,
        Tool::Polyline => Glyph::MoveDiagonal2,
        Tool::Curve => Glyph::Spline,
        Tool::Arrow => Glyph::ArrowUpRight,
        Tool::Pen => Glyph::Pencil,
        Tool::Highlight => Glyph::Highlighter,
        Tool::Number => Glyph::ListOrdered,
        Tool::Text => Glyph::Type,
        Tool::Watermark => Glyph::Stamp,
        Tool::Mosaic => Glyph::Grid2x2,
        Tool::Blur => Glyph::Droplet,
        Tool::Eraser => Glyph::Eraser,
    })
}
fn action_icon(name: &str) -> Icon {
    use gpui_kit::assets::IconName as Glyph;
    match name {
        "undo" => Icon::new(Glyph::Undo2),
        "redo" => Icon::new(Glyph::Redo2),
        "ocr" => Icon::new(Glyph::ScanText),
        "save" => Icon::new(Glyph::Save),
        "copy" => Icon::new(Glyph::Copy),
        "pin" => Icon::new(Glyph::Pin),
        "done" => Icon::new(Glyph::Check),
        _ => Icon::default().path("icons/close.svg"),
    }
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
    if doc.font.is_none() && doc.objects.iter().any(|a| a.tool.is_typographic()) {
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
    pub fn editor(&self) -> Option<Entity<EditorView>> {
        self.editor.clone()
    }
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
        let height = editor.read(cx).host_height(cx);
        let panel_bounds = snip_core::geometry::place_toolbar(
            self.anchor,
            self.screen,
            editor.read(cx).toolbar_width(),
            editor.read(cx).toolbar_height(),
        );
        // Grow the transparent host around the controls without moving the
        // controls themselves. The popover can then flip above its trigger.
        let mut bounds = panel_bounds;
        bounds.height = height.min((self.screen.height - 12.).max(1.));
        bounds.y = panel_bounds
            .y
            .min(self.screen.bottom() - bounds.height - 6.);
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
            .relative()
            .track_focus(&self.focus)
            .key_context("AnnotationTools")
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                if let Some(editor) = &v.editor {
                    let editing_input = {
                        let view = editor.read(cx);
                        view.size_input.read(cx).focus_handle(cx).is_focused(w)
                            || view.text.read(cx).focus_handle(cx).is_focused(w)
                            || view.color_picker.read(cx).is_open()
                    };
                    if !editing_input {
                        editor.update(cx, |v, cx| v.key_down(e, w, cx, false));
                    }
                }
            }))
            .capture_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    if let Some(editor) = &v.editor {
                        editor.update(cx, |v, cx| v.cancel(cx));
                    }
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(panel_bounds.y - bounds.y))
                    .w_full()
                    .child(tools),
            )
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
                cx.observe_window_activation(w, |v: &mut FloatingToolbar, w, cx| {
                    if !w.is_window_active()
                        && let Some(editor) = &v.editor
                        && let EditorHost::Pin(id) = editor.read(cx).host
                    {
                        crate::pin::schedule_blur(id, cx);
                    }
                    cx.refresh_windows();
                })
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
    let _ = handle.update(cx, |_, w, cx| crate::window_pool::transparent_root(w, cx));
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
    fn rectangle_shortcut_returns_to_rectangle_after_ellipse(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        window_focus_for_test(window, &editor, cx);
        for (key, tool) in [
            ("2", super::Tool::Rectangle),
            ("3", super::Tool::Ellipse),
            ("2", super::Tool::Rectangle),
        ] {
            cx.simulate_keystrokes(window, key);
            cx.update(|cx| assert_eq!(editor.read(cx).tool, tool));
        }
    }

    fn window_focus_for_test(
        window: AnyWindowHandle,
        editor: &Entity<EditorView>,
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| v.focus_canvas(w, cx));
                    let _ = w.draw(cx);
                })
                .unwrap();
        });
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn drawn_rectangle_is_selected_and_wheel_changes_that_object(cx: &mut TestAppContext) {
        use gpui_kit::{MouseButton, ScrollDelta, ScrollWheelEvent, VisualTestContext};
        let (window, editor) = editor(cx);
        window_focus_for_test(window, &editor, cx);
        cx.simulate_keystrokes(window, "2");
        let mut visual = VisualTestContext::from_window(window, cx);
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        let start = point(px(470.), px(670.));
        let end = point(px(530.), px(740.));
        visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(end, Some(MouseButton::Left), Default::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
        cx.update(|cx| {
            let v = editor.read(cx);
            assert_eq!(v.doc.objects.len(), 1);
            assert_eq!(v.selected, Some(0));
            assert_eq!(v.doc.objects[0].width, 3.);
        });
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        visual.simulate_event(ScrollWheelEvent {
            position: point(px(500.), px(700.)),
            delta: ScrollDelta::Lines(point(0., 1.)),
            ..Default::default()
        });
        cx.update(|cx| assert_eq!(editor.read(cx).doc.objects[0].width, 4.));

        // The default selection allows resizing immediately while the rectangle
        // tool remains active. Select mode can then reopen the same handles.
        for (from, to, expected) in [
            (
                end,
                point(px(550.), px(750.)),
                super::Rect::new(40., 40., 160., 160.),
            ),
            (
                point(px(550.), px(750.)),
                point(px(570.), px(760.)),
                super::Rect::new(40., 40., 200., 180.),
            ),
        ] {
            visual.update(|w, cx| {
                let _ = w.draw(cx);
            });
            visual.simulate_mouse_down(from, MouseButton::Left, Default::default());
            visual.simulate_mouse_move(to, Some(MouseButton::Left), Default::default());
            visual.simulate_mouse_up(to, MouseButton::Left, Default::default());
            cx.update(|cx| {
                let v = editor.read(cx);
                assert_eq!(v.doc.objects.len(), 1, "the handle must win over drawing");
                assert_eq!(v.doc.objects[0].bounds(), expected);
                assert_eq!(v.doc.objects[0].width, 4.);
            });
            cx.simulate_keystrokes(window, "1");
            visual.update(|w, cx| {
                let _ = w.draw(cx);
            });
            let center = point(px(500.), px(700.));
            visual.simulate_mouse_down(center, MouseButton::Left, Default::default());
            visual.simulate_mouse_up(center, MouseButton::Left, Default::default());
            cx.update(|cx| assert_eq!(editor.read(cx).selected, Some(0)));
        }
        cx.update(|cx| {
            editor.update(cx, |v, _| {
                v.doc.undo();
                assert_eq!(
                    v.doc.objects[0].bounds(),
                    super::Rect::new(40., 40., 160., 160.)
                );
                v.doc.undo();
                assert_eq!(
                    v.doc.objects[0].bounds(),
                    super::Rect::new(40., 40., 120., 140.)
                );
                assert_eq!(v.doc.objects[0].width, 4.);
            })
        });
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
                        v.next_number = 9;
                    })
                })
                .unwrap()
        });
        let (window, next) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    next.update(cx, |v, cx| {
                        assert_eq!(v.tool, super::Tool::Select);
                        assert_eq!(v.next_number, 1);
                        v.choose_tool(super::Tool::Text, w, cx);
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
    fn selecting_old_arrow_does_not_overwrite_saved_arrow_width(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.choose_tool(super::Tool::Arrow, w, cx);
                        v.set_size(11., cx);
                        v.arrow_style = super::ArrowStyle::DiamondHollow;
                        v.remember(cx);
                        // Merely selecting an existing thin arrow changes the inspector, not defaults.
                        v.width = 3.;
                        v.choose_tool(super::Tool::Arrow, w, cx);
                        assert_eq!(v.width, 11.);
                        assert_eq!(v.arrow_style, super::ArrowStyle::DiamondHollow);
                    })
                })
                .unwrap()
        });
    }

    #[gpui_kit::test]
    fn f3_exports_edited_capture_instead_of_reading_clipboard(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        let (tx, rx) = async_channel::bounded(8);
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
        cx.update(|cx| {
            crate::window_pool::init(cx);
            editor.update(cx, |v, _| {
                v.host = EditorHost::Capture;
            });
            cx.set_global(super::ActiveCaptureEditor(editor.downgrade()));
            editor.update(cx, |v, _| {
                v.doc.objects.push(snip_core::annotation::Annotation {
                    tool: super::Tool::Line,
                    points: vec![ImagePoint::new(10., 30.), ImagePoint::new(100., 30.)],
                    color: [255, 0, 0, 255],
                    width: 6.,
                    text: String::new(),
                    arrow_style: Default::default(),
                    watermark: Default::default(),
                })
            });
            crate::app::read_clipboard_command(crate::app::Command::Paste, cx, || {
                panic!("visible toolbar owns F3")
            });
        });
        let crate::app::Event::Pin(image, _) = crate::app::clipboard_test_event(&rx, cx) else {
            panic!("expected current capture pin");
        };
        assert_eq!(image.pixels.dimensions(), (1240, 430));
        assert!(image.pixels.get_pixel(30, 30)[0] > 200);
        assert!(image.pixels.get_pixel(30, 30)[1] < 30);
    }

    #[gpui_kit::test]
    fn toast_timeout_and_disable_leave_toolbar_size_unchanged(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.message = "已复制".into();
                        v.preferences.toast_delay_ms = 50;
                        cx.global_mut::<super::PreferencesStore>()
                            .value
                            .toast_delay_ms = 50;
                    });
                    let _ = w.draw(cx);
                })
                .unwrap()
        });
        cx.update(|cx| assert!(editor.read(cx).toast_visible));
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        assert!(!v.toast_visible);
                        v.message = "已识别".into();
                        v.preferences.toast_enabled = false;
                        cx.global_mut::<super::PreferencesStore>()
                            .value
                            .toast_enabled = false;
                    });
                    let _ = w.draw(cx);
                    assert!(!editor.read(cx).toast_visible);
                })
                .unwrap()
        });
    }

    #[gpui_kit::test]
    fn color_picker_changes_persist_and_update_selected_annotation(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.choose_tool(super::Tool::Arrow, w, cx);
                        v.doc.add(super::Annotation {
                            tool: super::Tool::Arrow,
                            points: vec![ImagePoint::new(10., 10.), ImagePoint::new(100., 70.)],
                            color: v.color,
                            width: 9.,
                            text: String::new(),
                            arrow_style: v.arrow_style,
                            watermark: Default::default(),
                        });
                        v.selected = Some(0);
                        v.color_picker.update(cx, |picker, cx| {
                            picker.select_color(gpui_kit::rgb(0x123456).into(), w, cx)
                        });
                    })
                })
                .unwrap()
        });
        cx.update(|cx| {
            let v = editor.read(cx);
            assert_eq!(v.color, [0x12, 0x34, 0x56, 255]);
            assert_eq!(v.doc.objects[0].color, v.color);
            assert_eq!(v.preferences.style(super::Tool::Arrow).color, v.color);
        });
    }

    #[gpui_kit::test]
    fn picker_has_popup_space_arrow_options_share_row_and_text_preview_survives_commit(
        cx: &mut TestAppContext,
    ) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.choose_tool(super::Tool::Arrow, w, cx);
                        assert_eq!(v.toolbar_height(), 74.);
                        v.color_picker.update(cx, |p, cx| p.set_open(true, cx));
                        assert!(v.host_height(cx) >= 492.);
                        v.color_picker.update(cx, |p, cx| p.set_open(false, cx));
                        assert_eq!(v.host_height(cx), 74.);
                        assert_eq!(v.export_action(2), 4);
                        v.choose_tool(super::Tool::Text, w, cx);
                        v.begin_text(ImagePoint::new(20., 20.), None, w, cx);
                        v.text
                            .update(cx, |t, cx| t.set_value("continuous text", w, cx));
                        v.commit_text(cx);
                        assert_eq!(v.pending_annotations.len(), 1);
                        assert_eq!(v.pending_annotations[0].0.text, "continuous text");
                        v.choose_tool(super::Tool::Arrow, w, cx);
                        assert_eq!(
                            v.pending_annotations.len(),
                            1,
                            "text must stay visible until raster result is ready"
                        );
                    })
                })
                .unwrap()
        });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn exported_watermark_preserves_transparency_and_undo() {
        let source = Arc::new(image::RgbaImage::new(250, 100));
        let mut doc = snip_core::annotation::Document::new(source.clone());
        doc.add(snip_core::annotation::Annotation {
            tool: super::Tool::Watermark,
            points: vec![ImagePoint::new(8., 8.), ImagePoint::new(200., 60.)],
            color: [170, 40, 21, 255],
            width: 4.,
            text: "Simple Snip".into(),
            arrow_style: Default::default(),
            watermark: Default::default(),
        });
        let (mut doc, output) = super::render_document(doc, super::identity(&source)).unwrap();
        assert!(output.pixels().any(|p| p[3] > 80));
        assert!(output.pixels().all(|p| p[3] <= 107));
        assert_eq!(doc.original.as_ref(), source.as_ref());
        doc.undo();
        assert_eq!(doc.flatten(), *source);
    }

    #[gpui_kit::test]
    fn result_grip_resizes_the_ocr_box_independently(cx: &mut TestAppContext) {
        use gpui_kit::{MouseButton, VisualTestContext};
        let (window, editor) = editor(cx);
        let mut visual = VisualTestContext::from_window(window, cx);
        visual.update(|w, cx| {
            editor.update(cx, |v, cx| {
                v.host = EditorHost::Capture;
                v.ocr = Some(Arc::new(super::OcrResult::default()));
                v.ocr_text = format!("{}\ntwo\nthree\nfour\nfive", "wide text ".repeat(15)).into();
                v.ocr_size.width = px(500.);
                cx.notify();
            });
            let _ = w.draw(cx);
            w.simulate_next_frame(cx);
            let _ = w.draw(cx);
        });
        cx.update(|cx| {
            assert_eq!(editor.read(cx).ocr_width(), px(500.));
        });
        let grip = visual
            .debug_bounds("ocr-resize-grip")
            .expect("resize grip is visible");
        let start = grip.origin + point(px(6.), px(6.));
        let end = start + point(px(-80.), px(38.));
        visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        visual.simulate_mouse_move(end, Some(MouseButton::Left), Default::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
        cx.update(|cx| {
            let v = editor.read(cx);
            assert_eq!(v.ocr_size, size(px(580.), px(162.)));
            assert!(v.ocr_resize.is_none());
        });
    }

    #[gpui_kit::test]
    fn ocr_keeps_clipboard_unchanged_until_explicit_copy(cx: &mut TestAppContext) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    editor.update(cx, |v, cx| {
                        v.host = EditorHost::Capture;
                        v.ocr = Some(Arc::new(super::OcrResult::default()));
                        v.ocr_text = "Hello world".into();
                        v.focus_canvas(w, cx);
                    });
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("before".into()));
                    let _ = w.draw(cx);
                })
                .unwrap();
        });
        cx.simulate_keystrokes(
            window,
            if cfg!(target_os = "macos") {
                "cmd-c"
            } else {
                "ctrl-c"
            },
        );
        cx.update(|cx| {
            assert_eq!(
                cx.read_from_clipboard().and_then(|c| c.text()),
                Some("before".into())
            );
            editor.update(cx, |v, cx| v.copy_ocr(false, cx));
            assert_eq!(
                cx.read_from_clipboard().and_then(|c| c.text()),
                Some("Hello world".into())
            );
        });
        cx.update(|cx| {
            window
                .update(cx, |_, w, cx| {
                    let field = editor.read(cx).ocr_field.clone();
                    field.update(cx, |v, cx| v.focus(w, cx));
                    let _ = w.draw(cx);
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("before".into()));
                })
                .unwrap();
        });
        cx.simulate_keystrokes(
            window,
            if cfg!(target_os = "macos") {
                "cmd-a cmd-c"
            } else {
                "ctrl-a ctrl-c"
            },
        );
        cx.update(|cx| {
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("Hello world".into())
            );
            assert!(editor.read(cx).has_ocr_result());
        });
    }

    #[gpui_kit::test]
    fn submitted_shape_stays_in_preview_until_the_matching_raster_is_ready(
        cx: &mut TestAppContext,
    ) {
        let (window, editor) = editor(cx);
        cx.update(|cx| {
            window
                .update(cx, |_, _, cx| {
                    editor.update(cx, |v, cx| {
                        v.draft = Some(super::Annotation {
                            tool: super::Tool::Rectangle,
                            points: vec![ImagePoint::new(20., 20.), ImagePoint::new(120., 90.)],
                            width: 4.,
                            color: [255, 0, 0, 255],
                            text: String::new(),
                            arrow_style: Default::default(),
                            watermark: Default::default(),
                        });
                        v.finish_path(cx);
                        assert!(v.draft.is_none());
                        assert_eq!(v.doc.objects.len(), 1);
                        assert_eq!(v.pending_annotations.len(), 1);
                        assert_eq!(v.pending_annotations[0].0.tool, super::Tool::Rectangle);
                        // Queue a second change while the first render is in flight.
                        v.refresh(cx);
                        assert_eq!(v.pending_annotations.len(), 1);
                    })
                })
                .unwrap();
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let v = editor.read(cx);
            assert!(v.pending_annotations.is_empty());
            assert_eq!(v.rendered.get_pixel(20, 40)[0], 255);
            assert_eq!(v.rendered.get_pixel(20, 40)[1], 0);
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
            assert!(v.toolbar_width() < 1000., "{}", v.toolbar_width());
            assert!((v.toolbar_width() - v.toolbar_rows[0] - 10.).abs() < 1.);
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
                            watermark: Default::default(),
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
                v.ocr_text = "old recognition".into();
                v.clear_ocr();
                assert!(ocr.cancelled(), "switching tools must cancel stale OCR");
                assert!(
                    !session.cancelled(),
                    "drawing after OCR must still allow copy/save/done"
                );
                assert!(v.ocr_text.is_empty());
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
    fn image_copy_closes_capture_only_on_success_and_keeps_pin(cx: &mut TestAppContext) {
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
        cx.update(|cx| {
            editor.update(cx, |v, cx| {
                v.copied(Ok(()), cx);
                assert!(
                    !cx.global::<crate::app::Runtime>().overlays.is_empty(),
                    "copying from a pin must keep it open"
                );
                v.host = EditorHost::Capture;
                v.copied(Err("clipboard busy".into()), cx);
                assert_eq!(v.message, "clipboard busy");
                assert!(
                    !cx.global::<crate::app::Runtime>().overlays.is_empty(),
                    "failed writes must keep the selection"
                );
                v.copied(Ok(()), cx);
                assert!(cx.global::<crate::app::Runtime>().overlays.is_empty());
            });
        });
    }

    #[gpui_kit::test]
    fn ocr_keeps_original_pixels_and_messages_do_not_change_toolbar_height(
        cx: &mut TestAppContext,
    ) {
        let (_, editor) = editor(cx);
        cx.update(|cx| {
            editor.update(cx, |v, _| {
                let original = v.doc.original.clone();
                v.ocr = Some(Arc::new(super::OcrResult::default()));
                let height = v.toolbar_height();
                v.message = "已复制".into();
                assert_eq!(v.toolbar_height(), height);
                assert!(Arc::ptr_eq(&original, &v.doc.original));
            })
        });
    }

    #[gpui_kit::test]
    fn text_caret_and_frame_center_align_to_canvas_click(cx: &mut TestAppContext) {
        use gpui_kit::{MouseButton, VisualTestContext};
        let (window, editor) = editor(cx);
        window_focus_for_test(window, &editor, cx);
        cx.simulate_keystrokes(window, "8");
        let mut visual = VisualTestContext::from_window(window, cx);
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        let click = point(px(520.), px(705.));
        visual.simulate_mouse_down(click, MouseButton::Left, Default::default());
        visual.simulate_mouse_up(click, MouseButton::Left, Default::default());
        visual.update(|w, cx| {
            let _ = w.draw(cx);
            w.simulate_next_frame(cx);
            let _ = w.draw(cx);
        });
        cx.update(|cx| {
            let view = editor.read(cx);
            let input = view.text.read(cx);
            let frame = super::text_frame_bounds(input).unwrap();
            let caret = input.cursor_layout().unwrap().0;
            assert!(
                (frame.left() - click.x).abs() <= px(1.),
                "frame {frame:?}, click {click:?}"
            );
            assert!(
                (caret.left() - click.x).abs() <= px(1.),
                "caret {caret:?}, click {click:?}"
            );
            assert!(
                (frame.center().y - click.y).abs() <= px(1.),
                "frame {frame:?}, click {click:?}"
            );
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
            window
                .update(cx, |_, w, cx| {
                    let _ = w.draw(cx);
                })
                .unwrap();
            assert_eq!(editor.read(cx).text.read(cx).value().as_str(), "hello");
            let v = editor.read(cx);
            let input = v.text.read(cx);
            let bounds = super::text_frame_bounds(input).unwrap();
            let caret = input.cursor_layout().unwrap().0;
            assert!(
                caret.right() <= bounds.right() + px(1.),
                "caret {caret:?} outside blue frame {bounds:?}"
            );
            assert!(
                (caret.size.height - bounds.size.height).abs() <= px(1.),
                "caret {caret:?} and blue frame {bounds:?} disagree"
            );
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
                let origin = v.doc.objects[0].points[0];
                assert_eq!(origin.x, 100.);
                assert!((origin.y + (v.width * 6. + 12.) * 1.3 / 2. - 80.).abs() < 0.001);
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
                    watermark: Default::default(),
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
