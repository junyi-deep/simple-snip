use crate::{
    app::{self, Event, Runtime},
    editor::{EditorHost, EditorView},
};
use gpui_kit::{component::menu::*, prelude::*, *};
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
        Self::open_at(id, image, edit, None, cx)
    }
    pub fn open_at(
        id: u64,
        image: Arc<RgbaImage>,
        edit: bool,
        placement: Option<Bounds<Pixels>>,
        cx: &mut App,
    ) -> anyhow::Result<(AnyWindowHandle, Entity<Self>)> {
        let scale = (680. / image.width() as f32)
            .min(500. / image.height() as f32)
            .min(1.);
        let margin = cx.global::<Runtime>().config.pin.margin();
        let bounds = Bounds::centered(
            None,
            size(
                px(image.width() as f32 * scale + margin * 2.),
                px(image.height() as f32 * scale + margin * 2.),
            ),
            cx,
        );
        // A captured region is already sized in logical display coordinates.
        // Preserve its content rect, with transparent frame padding outside it.
        let (bounds, scale) = if let Some(content) = placement {
            (
                Bounds::new(
                    content.origin - point(px(margin), px(margin)),
                    size(
                        content.size.width + px(margin * 2.),
                        content.size.height + px(margin * 2.),
                    ),
                ),
                f32::from(content.size.width) / image.width() as f32,
            )
        } else {
            (bounds, scale)
        };
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
                window.on_window_should_close(cx, move |_, cx| {
                    app::send(cx, Event::ClosePin(id));
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
                            Bounds::new(
                                point(px(margin), px(margin)),
                                size(
                                    window.viewport_size().width - px(margin * 2.),
                                    window.viewport_size().height - px(margin * 2.),
                                ),
                            ),
                            window,
                            cx,
                        )
                    }))
                } else {
                    None
                };
                cx.new(|cx| {
                    cx.observe_window_activation(window, |v: &mut Self, w, cx| {
                        if w.is_window_active() && v.editor.is_none() {
                            v.focus.focus(w, cx);
                        }
                        cx.notify();
                    })
                    .detach();
                    Self {
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
                    }
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
    fn is_selected(&self, w: &Window, cx: &App) -> bool {
        !self.click_through
            && (w.is_window_active() || self.toolbar.is_some_and(|h| Some(h) == cx.active_window()))
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
    pub fn refresh_frame(&self, w: &mut Window, cx: &mut Context<Self>) {
        let margin = cx.global::<Runtime>().config.pin.margin();
        w.resize(size(
            px(self.shown.width() as f32 * self.scale + margin * 2.),
            px(self.shown.height() as f32 * self.scale + margin * 2.),
        ));
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
        let margin = cx.global::<Runtime>().config.pin.margin();
        self.editor = Some(cx.new(|cx| {
            EditorView::new(
                doc,
                transform,
                EditorHost::Pin(id),
                image,
                Bounds::new(
                    point(px(margin), px(margin)),
                    size(
                        w.viewport_size().width - px(margin * 2.),
                        w.viewport_size().height - px(margin * 2.),
                    ),
                ),
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
                            px(v.shown.width() as f32 * v.scale
                                + cx.global::<Runtime>().config.pin.margin() * 2.),
                            px(v.shown.height() as f32 * v.scale
                                + cx.global::<Runtime>().config.pin.margin() * 2.),
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = cx.global::<Runtime>().config.pin.clone();
        let margin = config.margin();
        let border = config.color().unwrap_or(0x1688ff);
        let selected = self.is_selected(window, cx);
        let pin_id = self.id;
        let content = if let Some(editor) = &self.editor {
            editor.clone().into_any_element()
        } else {
            img(self.texture.clone())
                .size_full()
                .object_fit(ObjectFit::Fill)
                .into_any_element()
        };
        let frame = div()
            .size_full()
            .relative()
            .when(config.shadow, |d| d.shadow_md())
            .child(content)
            .child(
                canvas(
                    move |_, _, _| (),
                    move |bounds, _, w, _| {
                        if selected && config.border_width > 0. {
                            let thickness = px(config.border_width);
                            w.paint_quad(
                                outline(
                                    Bounds::new(
                                        bounds.origin + point(thickness / 2., thickness / 2.),
                                        size(
                                            (bounds.size.width - thickness).max(px(0.)),
                                            (bounds.size.height - thickness).max(px(0.)),
                                        ),
                                    ),
                                    rgb(border),
                                    BorderStyle::Solid,
                                )
                                .border_widths(thickness),
                            );
                        }
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            );
        if self.editor.is_some() {
            return div()
                .id(("pin-editor", self.id))
                .key_context("PinWindow")
                .track_focus(&self.focus)
                .size_full()
                .p(px(margin))
                .bg(transparent_black())
                .child(frame)
                .on_action(cx.listener(|v, _: &app::CloseSelectedPin, _, cx| {
                    app::send(cx, Event::ClosePin(v.id));
                    cx.stop_propagation();
                }))
                .context_menu(move |menu, _, _| {
                    menu.item(PopupMenuItem::new("关闭贴图").on_click(move |_, _, cx| {
                        app::send(cx, Event::ClosePin(pin_id));
                    }))
                })
                .into_any_element();
        }
        let notice = self.notice.clone();
        div()
            .id(("pin", self.id))
            .track_focus(&self.focus)
            .key_context("PinWindow")
            .size_full()
            .relative()
            .bg(gpui_kit::transparent_black())
            .p(px(margin))
            .child(frame)
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
                    cx.activate(true);
                    w.activate_window();
                    v.focus.focus(w, cx);
                    cx.notify();
                    if e.click_count == 2 {
                        if let Ok(raw) = app::raw(w) {
                            let _ = snip_platform::show_window(raw, false);
                        }
                    } else if app::raw(w).is_ok() {
                        // Moving needs a native surface; GPUI's headless TestWindow
                        // deliberately has no implementation for this operation.
                        w.start_window_move();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|v, _, w, cx| {
                    cx.activate(true);
                    w.activate_window();
                    v.focus.focus(w, cx);
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|v, _: &app::CloseSelectedPin, _, cx| {
                app::send(cx, Event::ClosePin(v.id));
                cx.stop_propagation();
            }))
            .on_scroll_wheel(cx.listener(|v, e: &ScrollWheelEvent, w, cx| {
                let delta = f32::from(e.delta.pixel_delta(px(20.)).y);
                if e.modifiers.control || e.modifiers.platform {
                    v.opacity = (v.opacity + delta.signum() * 0.05).clamp(0.1, 1.);
                    v.style(w);
                } else {
                    v.scale = (v.scale * (1. + delta.signum() * 0.05)).clamp(0.05, 6.);
                    w.resize(size(
                        px(v.shown.width() as f32 * v.scale
                            + cx.global::<Runtime>().config.pin.margin() * 2.),
                        px(v.shown.height() as f32 * v.scale
                            + cx.global::<Runtime>().config.pin.margin() * 2.),
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
                        app::send(cx, Event::ClosePin(v.id));
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
            .context_menu(move |menu, _, _| {
                menu.item(PopupMenuItem::new("关闭贴图").on_click(move |_, _, cx| {
                    app::send(cx, Event::ClosePin(pin_id));
                }))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Arc, Event, PinView, RgbaImage, Runtime, app};
    use gpui_kit::{
        AnyWindowHandle, Entity, MouseButton, TestAppContext, VisualTestContext, point, px,
    };

    fn setup(cx: &mut TestAppContext) -> async_channel::Receiver<Event> {
        let (tx, rx) = async_channel::bounded(16);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::window_pool::init(cx);
            app::bind_keys(cx);
            cx.set_global(Runtime {
                recordings: vec![],
                config: Default::default(),
                status: String::new(),
                tx,
                pins: Default::default(),
                next_pin: 3,
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
        });
        rx
    }

    fn pin(cx: &mut TestAppContext, id: u64, edit: bool) -> (AnyWindowHandle, Entity<PinView>) {
        cx.update(|cx| {
            let (handle, view) =
                PinView::open(id, Arc::new(RgbaImage::new(240, 160)), edit, cx).unwrap();
            cx.global_mut::<Runtime>().pins.insert(
                id,
                app::PinRecord {
                    handle,
                    view: view.clone(),
                },
            );
            (handle, view)
        })
    }

    #[gpui_kit::test]
    fn capture_pin_preserves_content_rect_instead_of_centering_or_fitting(cx: &mut TestAppContext) {
        setup(cx);
        cx.update(|cx| {
            let margin = cx.global::<Runtime>().config.pin.margin();
            let selected = gpui_kit::Bounds::new(
                point(px(160.), px(100.)),
                gpui_kit::size(px(1000.), px(600.)),
            );
            let (handle, view) = PinView::open_at(
                1,
                Arc::new(RgbaImage::new(2000, 1200)),
                false,
                Some(selected),
                cx,
            )
            .unwrap();
            handle
                .update(cx, |_, w, cx| {
                    assert_eq!(
                        w.bounds().origin + point(px(margin), px(margin)),
                        selected.origin
                    );
                    assert_eq!(
                        w.viewport_size().width - px(margin * 2.),
                        selected.size.width
                    );
                    assert_eq!(
                        w.viewport_size().height - px(margin * 2.),
                        selected.size.height
                    );
                    assert_eq!(
                        view.read(cx).scale,
                        0.5,
                        "retain Retina display scale above the old fit limit"
                    );
                })
                .unwrap();
        });
    }

    #[gpui_kit::test]
    fn clicking_inactive_pin_selects_it_and_escape_closes_only_that_pin(cx: &mut TestAppContext) {
        let rx = setup(cx);
        let (first, view) = pin(cx, 1, false);
        let (second, second_view) = pin(cx, 2, false);
        let mut visual = VisualTestContext::from_window(first, cx);
        cx.update(|cx| {
            second.update(cx, |_, w, _| w.activate_window()).unwrap();
            first
                .update(cx, |_, w, cx| assert!(!view.read(cx).is_selected(w, cx)))
                .unwrap();
        });
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        visual.simulate_mouse_down(
            point(px(80.), px(80.)),
            MouseButton::Left,
            Default::default(),
        );
        visual.simulate_mouse_up(
            point(px(80.), px(80.)),
            MouseButton::Left,
            Default::default(),
        );
        visual.update(|w, cx| {
            assert!(view.read(cx).is_selected(w, cx));
            assert!(view.read(cx).focus.is_focused(w));
        });
        cx.update(|cx| {
            second
                .update(cx, |_, w, cx| {
                    assert!(!second_view.read(cx).is_selected(w, cx))
                })
                .unwrap();
        });
        let weak = view.downgrade();
        cx.simulate_keystrokes(first, "escape");
        assert!(matches!(rx.try_recv(), Ok(Event::ClosePin(1))));
        cx.update(|cx| app::close_pin(1, cx));
        drop(view);
        cx.update(|cx| {
            assert!(
                weak.upgrade().is_none(),
                "closed pin must release its image entity"
            );
            assert!(cx.global::<Runtime>().pins.contains_key(&2));
        });
    }

    #[gpui_kit::test]
    fn close_window_shortcut_works_from_pin_editor_and_toolbar(cx: &mut TestAppContext) {
        let rx = setup(cx);
        let (window, view) = pin(cx, 1, true);
        let mut visual = VisualTestContext::from_window(window, cx);
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        #[cfg(target_os = "macos")]
        let shortcut = "cmd-w";
        #[cfg(target_os = "windows")]
        let shortcut = "ctrl-w";
        cx.simulate_keystrokes(window, shortcut);
        assert!(matches!(rx.try_recv(), Ok(Event::ClosePin(1))));
        // Moving focus into the independent toolbar must retain selection and
        // route the same local action to its owning pin, never another image.
        cx.update(|cx| view.update(cx, |v, cx| v.focus_toolbar(cx)));
        let toolbar = cx.update(|cx| {
            cx.global::<Runtime>()
                .annotation_toolbar
                .as_ref()
                .unwrap()
                .0
        });
        let mut tools = VisualTestContext::from_window(toolbar, cx);
        tools.update(|w, cx| {
            let _ = w.draw(cx);
        });
        visual.update(|w, cx| assert!(view.read(cx).is_selected(w, cx)));
        cx.simulate_keystrokes(toolbar, shortcut);
        assert!(matches!(rx.try_recv(), Ok(Event::ClosePin(1))));
        cx.update(|cx| app::close_pin(1, cx));
        cx.update(|cx| {
            assert!(cx.global::<Runtime>().pins.is_empty());
        });
    }
}
