use crate::app::{self, Event, Runtime};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputEvent, InputState},
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
    recording::{RecordingControl, record_gif},
    stitch::Stitcher,
};
use snip_platform::{CapturedDisplay, ClipboardProvider};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone, Copy)]
pub enum EditorHost {
    Capture,
    Pin(u64),
}
/// One image canvas, embedded in the capture overlay or the existing pin window.
/// It never opens an application editor window.
pub struct EditorView {
    pub doc: Document,
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
    text_index: Option<usize>,
    _text_subscription: Subscription,
    last_wheel: Option<std::time::Instant>,
    draft: Option<Annotation>,
    selected: Option<usize>,
    last: Option<ImagePoint>,
    scale: f32,
    origin: Point<Pixels>,
    view_size: gpui_kit::Size<Pixels>,
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
        let focus = cx.focus_handle();
        focus.focus(w, cx);
        let text = cx.new(|cx| InputState::new(w, cx).placeholder("输入文字"));
        let window_handle = w.window_handle();
        let text_focus = focus.clone();
        let text_subscription = cx.subscribe(&text, move |v, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) && v.text_anchor.is_some() {
                v.commit_text(cx);
                let focus = text_focus.clone();
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, w, cx| focus.focus(w, cx));
                });
            }
        });
        Self {
            doc,
            transform,
            host,
            rendered: initial.clone(),
            texture: app::texture(&initial),
            tool: Tool::Select,
            color: [240, 60, 70, 255],
            width: 3.,
            arrow_style: ArrowStyle::default(),
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
    pub fn focus_canvas(&self, w: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(w, cx);
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
            self.doc.objects[i].width = (self.doc.objects[i].width + delta.signum()).clamp(1., 80.);
            self.width = self.doc.objects[i].width;
            self.refresh(cx);
        } else if self.tool != Tool::Select {
            self.width = (self.width + delta.signum()).clamp(1., 80.);
            cx.notify();
        }
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
        let generation = self.generation;
        let doc = self.doc.clone();
        let transform = self.transform;
        cx.spawn(async move |this, cx| {
            let result = smol::unblock(move || render_document(doc, transform)).await;
            let _ = this.update(cx, |v, cx| {
                v.rendering = false;
                if v.generation == generation {
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
        let doc = self.doc.clone();
        let transform = self.transform;
        let host = self.host;
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
                    if matches!(host, EditorHost::Capture) {
                        cx.update(app::close_overlays);
                    }
                    let _ = tx.send(Event::Pin(image)).await;
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
            Tool::Number => {
                let number = self
                    .doc
                    .objects
                    .iter()
                    .filter(|a| a.tool == Tool::Number)
                    .filter_map(|a| a.text.parse::<u32>().ok())
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
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
        if !e.dragging() {
            self.last = None;
            return;
        }
        if let Some(draft) = &mut self.draft {
            if matches!(draft.tool, Tool::Pen | Tool::Highlight) {
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
        let mut strip = div().h_flex().gap(px(1.)).flex_wrap();
        for (i, (label, tool)) in tools.into_iter().enumerate() {
            strip = strip.child(
                Button::new(("tool", i))
                    .xsmall()
                    .ghost()
                    .icon(tool_icon(tool))
                    .accessibility_label(label)
                    .tooltip(label)
                    .selected(self.tool == tool && self.ocr.is_none())
                    .on_click(cx.listener(move |v, _, _, cx| {
                        v.commit_text(cx);
                        v.tool = tool;
                        v.selected = None;
                        v.clear_ocr();
                        v.message.clear();
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
                    .ghost()
                    .icon(action_icon(name))
                    .accessibility_label(label)
                    .tooltip(label)
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
        let width = 520f32
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
                                .label("复制选中")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.copy_text(false, false);
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
                                if let (Some(ocr), Some((index, _))) = (&mut v.ocr, v.ocr_start)
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
                                    }
                                }
                                cx.notify();
                            }),
                        )),
                );
        } else if has_options {
            let mut options = div().h_flex().gap_1().items_center();
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
                            if let Some(i) = v.selected.filter(|i| *i < v.doc.objects.len()) {
                                v.doc.checkpoint();
                                v.doc.objects[i].color = color;
                                v.refresh(cx);
                            }
                            cx.notify();
                        })),
                );
            }
            for width in [2u32, 4, 8] {
                options = options.child(
                    Button::new(("width", width))
                        .xsmall()
                        .ghost()
                        .label(format!("{width}"))
                        .tooltip(format!("线宽 {width} 像素"))
                        .selected(self.width == width as f32)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.width = width as f32;
                            if let Some(i) = v.selected.filter(|i| *i < v.doc.objects.len()) {
                                v.doc.checkpoint();
                                v.doc.objects[i].width = width as f32;
                                v.refresh(cx);
                            }
                            cx.notify();
                        })),
                );
            }

            panel = panel.child(options);
            if self.option_tool() == Tool::Arrow {
                let mut styles = div().h_flex().gap_1();
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
                            .ghost()
                            .label(label)
                            .selected(self.arrow_style == style)
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.arrow_style = style;
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
        let weak = cx.entity().downgrade();
        let scale = self.scale;
        let transform = self.transform;
        let draft = self.draft.clone();
        let ocr = self.ocr.clone();
        let selection = self.ocr_start.zip(self.ocr_end);
        let hover = self.ocr_hover;
        let selected_bounds = self
            .selected
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
                    let mut points = a.points.clone();
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
                let left =
                    (p.x * self.scale).clamp(0., (f32::from(self.view_size.width) - 120.).max(0.));
                let top =
                    (p.y * self.scale).clamp(0., (f32::from(self.view_size.height) - 34.).max(0.));
                div()
                    .id("image-text-input")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(240f32.min(f32::from(self.view_size.width).max(120.))))
                    .p_1()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(rgb(0x1688ff))
                    .h_flex()
                    .gap_1()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div().flex_1().child(
                            Input::new(&self.text)
                                .id("annotation-text-field")
                                .aria_label("图片文字")
                                .xsmall(),
                        ),
                    )
                    .child(
                        Button::new("commit-image-text")
                            .xsmall()
                            .ghost()
                            .icon(action_icon("done"))
                            .accessibility_label("完成文字")
                            .on_click(cx.listener(|v, _, w, cx| {
                                v.commit_text(cx);
                                v.focus.focus(w, cx);
                            })),
                    )
            }))
            .children(toolbar)
            .on_scroll_wheel(cx.listener(|v, e: &ScrollWheelEvent, _, cx| {
                v.resize_selected(f32::from(e.delta.pixel_delta(px(20.)).y), cx);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                if v.text_anchor.is_some() {
                    if e.keystroke.key == "escape" {
                        v.text_anchor = None;
                        v.text_index = None;
                        v.focus.focus(w, cx);
                        cx.notify();
                    }
                    return;
                }

                let modifier = if cfg!(target_os = "macos") {
                    e.keystroke.modifiers.platform
                } else {
                    e.keystroke.modifiers.control
                };
                match e.keystroke.key.as_str() {
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
        Tool::Rectangle => "<rect x='4' y='5' width='16' height='14' rx='1'/>",
        Tool::Ellipse => "<ellipse cx='12' cy='12' rx='8' ry='7'/>",
        Tool::Line => "<path d='m4 20 16-16'/>",
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
    if let Err(e) = TaskView::open(display, rect, true, cx) {
        app::status(cx, e.to_string());
    }
}
pub fn open_long_capture(display: CapturedDisplay, rect: Rect, cx: &mut App) {
    if let Err(e) = TaskView::open(display, rect, false, cx) {
        app::status(cx, e.to_string());
    }
}
struct TaskView {
    display: CapturedDisplay,
    rect: Rect,
    gif: bool,
    control: RecordingControl,
    recording: bool,
    paused: bool,
    message: String,
    path: Option<PathBuf>,
    stitch: Option<Stitcher>,
    busy: bool,
    auto: bool,
    watch: bool,
    handle: AnyWindowHandle,
    empty_frames: u32,
    cancel: Cancellation,
    preview: Option<Arc<RenderImage>>,
}
impl Drop for TaskView {
    fn drop(&mut self) {
        self.control.cancel();
        self.cancel.cancel();
    }
}
impl TaskView {
    fn open(
        display: CapturedDisplay,
        rect: Rect,
        gif: bool,
        cx: &mut App,
    ) -> anyhow::Result<AnyWindowHandle> {
        let (display_bounds, _) = app::display_bounds(&display, cx);
        let d = display_bounds;
        let a = display.map.to_desktop(ImagePoint::new(rect.x, rect.y));
        let scale_x = f32::from(d.size.width) / display.map.desktop.width;
        let scale_y = f32::from(d.size.height) / display.map.desktop.height;
        let selected = Rect::new(
            f32::from(d.origin.x) + (a.x - display.map.desktop.x) * scale_x,
            f32::from(d.origin.y) + (a.y - display.map.desktop.y) * scale_y,
            rect.width / display.map.pixel_width as f32 * f32::from(d.size.width),
            rect.height / display.map.pixel_height as f32 * f32::from(d.size.height),
        );
        let candidates = [
            Rect::new(selected.x, selected.bottom() + 12., 430., 300.),
            Rect::new(selected.x, selected.y - 312., 430., 300.),
            Rect::new(selected.right() + 12., selected.y, 430., 300.),
            Rect::new(selected.x - 442., selected.y, 430., 300.),
        ];
        let screen = Rect::new(
            f32::from(d.origin.x),
            f32::from(d.origin.y),
            f32::from(d.size.width),
            f32::from(d.size.height),
        );
        let position = candidates
            .into_iter()
            .find(|r| r.intersect(screen) == *r)
            .unwrap_or(Rect::new(screen.x + 12., screen.y + 36., 430., 300.));
        let bounds = Bounds::new(
            point(px(position.x), px(position.y)),
            size(px(position.width), px(position.height)),
        );
        let (h, _) = crate::window_pool::open(
            crate::window_pool::Role::Task,
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(
                        if gif {
                            "Simple Snip · GIF 录制"
                        } else {
                            "Simple Snip · 长截图"
                        }
                        .into(),
                    ),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                window.set_window_title(if gif {
                    "Simple Snip · GIF 录制"
                } else {
                    "Simple Snip · 长截图"
                });
                app::prepare_window(window);
                window.on_window_should_close(cx, |w, cx| {
                    crate::window_pool::park(crate::window_pool::Role::Task, w, cx);
                    false
                });
                cx.new(|_| Self {
                    display,
                    rect,
                    gif,
                    control: RecordingControl::default(),
                    recording: false,
                    paused: false,
                    message: if gif {
                        "请把控制窗口移到录制区域外，3 秒后开始。"
                    } else {
                        "将本窗口移出截图区域。小幅滚动目标内容后采集下一段。"
                    }
                    .into(),
                    path: None,
                    stitch: None,
                    busy: false,
                    auto: false,
                    watch: false,
                    handle: window.window_handle(),
                    empty_frames: 0,
                    cancel: Cancellation::default(),
                    preview: None,
                })
            },
        )?;
        Ok(h)
    }
    fn start_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        if cx
            .global::<Runtime>()
            .recordings
            .iter()
            .any(|c| !c.done.load(std::sync::atomic::Ordering::Relaxed))
        {
            self.message = "已有录制任务，请先在托盘停止它".into();
            cx.notify();
            return;
        }
        self.control = RecordingControl::default();
        cx.global_mut::<Runtime>()
            .recordings
            .retain(|c| !c.done.load(std::sync::atomic::Ordering::Relaxed));
        cx.global_mut::<Runtime>()
            .recordings
            .push(self.control.clone());
        self.recording = true;
        app::refresh_tray(cx);
        self.message = "倒计时…".into();
        let config = cx.global::<Runtime>().config.clone();
        let display = self.display.id;
        let rect = self.rect;
        let control = self.control.clone();
        let weak = cx.entity().downgrade();
        let handle = self.handle;
        app::set_trails_visible(cx, false);
        let control_window = self.handle;
        let _ = control_window.update(cx, |_, window, _| {
            if let Ok(raw) = app::raw(window) {
                let _ = snip_platform::show_window(raw, false);
            }
        });
        let timer_control = self.control.clone();
        cx.spawn(async move |this, cx| {
            while !timer_control
                .done
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                smol::Timer::after(Duration::from_millis(250)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        let (tx, rx) = async_channel::bounded(1);
        cx.spawn(async move |this, cx| {
            smol::Timer::after(Duration::from_secs(config.gif.countdown as u64)).await;
            if control.stop.load(std::sync::atomic::Ordering::Relaxed) {
                control
                    .done
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                cx.update(|cx| {
                    app::set_trails_visible(cx, true);
                    app::refresh_tray(cx);
                    let _ = handle.update(cx, |_, w, _| {
                        if let Ok(raw) = app::raw(w) {
                            let _ = snip_platform::show_window(raw, true);
                        }
                    });
                });
                let _ = this.update(cx, |v, cx| {
                    v.recording = false;
                    v.message = "录制已取消".into();
                    cx.notify();
                });
                return;
            }
            let _ = this.update(cx, |v, cx| {
                v.message = "录制中（有界帧队列）".into();
                cx.notify();
            });
            let path = config.save_directory.join(format!(
                "Snip_{}.gif",
                chrono::Local::now().format("%Y%m%d_%H%M%S_%3f")
            ));
            let result = std::fs::create_dir_all(&config.save_directory);
            if let Err(e) = result {
                control
                    .done
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                let _ = tx.send(Err(e.into())).await;
            } else {
                record_gif(
                    config.gif,
                    path,
                    control,
                    move || snip_platform::capture_region(display, rect),
                    move |result| {
                        let _ = tx.send_blocking(result);
                    },
                );
            }
            if let Ok(result) = rx.recv().await {
                cx.update(|cx| {
                    app::set_trails_visible(cx, true);
                    app::refresh_tray(cx);
                    let _ = handle.update(cx, |_, window, _| {
                        if let Ok(raw) = app::raw(window) {
                            let _ = snip_platform::show_window(raw, true);
                        }
                        window.activate_window();
                    });
                });
                let _ = weak.update(cx, |v, cx| {
                    v.recording = false;
                    match result {
                        Ok(path) => {
                            v.message = format!("GIF 已保存：{}", path.display());
                            v.path = Some(path);
                        }
                        Err(e) => v.message = e.to_string(),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }
    fn sample(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let display = self.display.id;
        let rect = self.rect;
        let mut stitch = self.stitch.take();
        cx.spawn(async move |this, cx| {
            let (stitch, result, preview) = smol::unblock(move || {
                let result = (|| -> anyhow::Result<String> {
                    let image = snip_platform::capture_region(display, rect)?;
                    if let Some(s) = &mut stitch {
                        Ok(if s.append(&image)? {
                            "已拼接下一段"
                        } else {
                            "内容未变化"
                        }
                        .into())
                    } else {
                        stitch = Some(Stitcher::new(image, 256 * 1024 * 1024)?);
                        Ok("已采集首帧".into())
                    }
                })();
                let preview = stitch
                    .as_ref()
                    .map(|s| image::imageops::thumbnail(&s.image, 400, 800));
                (stitch, result, preview)
            })
            .await;
            let _ = this.update(cx, |v, cx| {
                v.busy = false;
                v.stitch = stitch;
                match result {
                    Ok(message) => {
                        if message == "内容未变化" {
                            v.empty_frames += 1;
                        } else {
                            v.empty_frames = 0;
                        }
                        if v.empty_frames >= 3 && v.auto {
                            v.auto = false;
                            v.watch = false;
                            v.message = "内容连续未变化，自动滚动已停止；可导出已有结果".into();
                        } else {
                            v.message = message;
                        }
                    }
                    Err(e) => {
                        v.message = e.to_string();
                        v.auto = false;
                        v.watch = false;
                    }
                }
                if let Some(image) = preview {
                    v.preview = Some(app::texture(&image));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn watch(&mut self, cx: &mut Context<Self>) {
        self.watch = !self.watch;
        if !self.watch {
            return;
        }
        self.auto = false;
        let cancel = self.cancel.clone();
        cx.spawn(async move |this, cx| {
            for _ in 0..300 {
                let active = this
                    .update(cx, |v, cx| {
                        if !v.watch || cancel.cancelled() {
                            return false;
                        }
                        v.sample(cx);
                        true
                    })
                    .unwrap_or(false);
                if !active {
                    break;
                }
                smol::Timer::after(Duration::from_millis(800)).await;
            }
        })
        .detach();
        cx.notify();
    }
    fn auto(&mut self, cx: &mut Context<Self>) {
        self.auto = !self.auto;
        if !self.auto {
            return;
        }
        let cancel = self.cancel.clone();
        cx.spawn(async move |this, cx| {
            for _ in 0..100 {
                let active = this
                    .update(cx, |v, cx| {
                        if !v.auto || cancel.cancelled() {
                            return false;
                        }
                        v.sample(cx);
                        true
                    })
                    .unwrap_or(false);
                if !active {
                    break;
                }
                smol::Timer::after(Duration::from_millis(1200)).await;
                let target = this
                    .update(cx, |v, _| {
                        if !v.auto || cancel.cancelled() {
                            None
                        } else {
                            Some(v.display.map.to_desktop(ImagePoint::new(
                                v.rect.x + v.rect.width / 2.,
                                v.rect.y + v.rect.height / 2.,
                            )))
                        }
                    })
                    .ok()
                    .flatten();
                let Some(target) = target else {
                    break;
                };
                if let Err(e) = snip_platform::scroll_at(target) {
                    let _ = this.update(cx, |v, cx| {
                        v.auto = false;
                        v.watch = false;
                        v.message = e.to_string();
                        cx.notify();
                    });
                    break;
                }
            }
        })
        .detach();
    }
}
impl Render for TaskView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(if self.recording {
                format!(
                    "{} · {:.1} 秒 · 通过托盘暂停、停止或取消",
                    self.message,
                    self.control
                        .elapsed_ms
                        .load(std::sync::atomic::Ordering::Relaxed) as f64
                        / 1000.
                )
            } else {
                self.message.clone()
            });
        if self.gif {
            root = root.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("start")
                            .primary()
                            .label("开始")
                            .on_click(cx.listener(|v, _, _, cx| v.start_recording(cx))),
                    )
                    .child(
                        Button::new("pause")
                            .label(if self.paused { "继续" } else { "暂停" })
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.paused = !v.paused;
                                v.control.pause(v.paused);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("stop")
                            .label("停止并保存")
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.control.finish();
                                v.message = "正在完成编码…".into();
                                cx.notify();
                            })),
                    ),
            );
            if let Some(path) = &self.path {
                let path = path.clone();
                root = root.child(
                    Button::new("preview")
                        .label("预览 GIF")
                        .on_click(move |_, _, cx| cx.open_with_system(&path)),
                );
            }
        } else {
            root = root.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("sample")
                            .primary()
                            .label(if self.busy {
                                "采集中…"
                            } else {
                                "采集下一段"
                            })
                            .on_click(cx.listener(|v, _, _, cx| v.sample(cx))),
                    )
                    .child(
                        Button::new("watch")
                            .label(if self.watch {
                                "暂停手动监测"
                            } else {
                                "监测手动滚动"
                            })
                            .on_click(cx.listener(|v, _, _, cx| v.watch(cx))),
                    )
                    .child(
                        Button::new("auto")
                            .label(if self.auto {
                                "暂停自动滚动"
                            } else {
                                "自动滚动"
                            })
                            .on_click(cx.listener(|v, _, _, cx| v.auto(cx))),
                    )
                    .child(
                        Button::new("export-long")
                            .label("编辑 / 导出已有结果")
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.auto = false;
                                v.watch = false;
                                if let Some(stitch) = &v.stitch {
                                    app::send(cx, Event::Edit(Arc::new(stitch.image.clone())));
                                }
                            })),
                    ),
            );
            if let Some(preview) = &self.preview {
                root = root.child(
                    img(preview.clone())
                        .flex_1()
                        .w_full()
                        .object_fit(ObjectFit::Contain),
                );
            }
        }
        root.child(
            Button::new("cancel-task")
                .label("取消并关闭")
                .on_click(cx.listener(|v, _, w, cx| {
                    v.control.cancel();
                    v.cancel.cancel();
                    crate::window_pool::park(crate::window_pool::Role::Task, w, cx);
                })),
        )
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
        let bounds = snip_core::geometry::place_toolbar(self.anchor, self.screen, 520., height);
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
            .child(tools)
            .on_action(cx.listener(|v, _: &app::CancelAnnotation, _, cx| {
                if let Some(editor) = &v.editor {
                    editor.update(cx, |editor, cx| editor.cancel(cx));
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
    let anchor = Rect::new(
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height),
    );
    let screen = pin_window.display(cx).map(|d| d.bounds()).unwrap_or(b);
    let screen = Rect::new(
        f32::from(screen.origin.x),
        f32::from(screen.origin.y),
        f32::from(screen.size.width),
        f32::from(screen.size.height),
    );
    let bounds =
        snip_core::geometry::place_toolbar(anchor, screen, 520., editor.read(cx).toolbar_height());
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
            #[cfg(target_os = "macos")]
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
            #[cfg(target_os = "windows")]
            {
                let _ = before;
                assert!(window.update(cx, |_, _, _| ()).is_err());
            }
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
