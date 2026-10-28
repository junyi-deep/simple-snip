//! macOS reuses empty native windows to avoid AppKit's Touch Bar observer race.
//! Replacing the GPUI root releases images, tasks and subscriptions immediately.
#[cfg(target_os = "macos")]
use gpui_kit::prelude::*;
use gpui_kit::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Pin,
    Capture,
    Task,
    Trail,
}
#[cfg(target_os = "macos")]
#[derive(Default)]
struct Pool(std::collections::HashMap<Role, Vec<AnyWindowHandle>>);
#[cfg(target_os = "macos")]
impl Global for Pool {}

pub fn init(cx: &mut App) {
    #[cfg(target_os = "macos")]
    cx.set_global(Pool::default());
    #[cfg(target_os = "windows")]
    let _ = cx;
}

pub fn open<V: Render>(
    role: Role,
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<(AnyWindowHandle, Entity<V>)> {
    #[cfg(target_os = "macos")]
    if let Some(handle) = cx.global_mut::<Pool>().0.entry(role).or_default().pop() {
        let mut view = None;
        handle.update(cx, |_, w, cx| {
            if let Some(WindowBounds::Windowed(bounds)) = options.window_bounds {
                w.resize(bounds.size);
                if let Ok(raw) = crate::app::raw(w) {
                    let _ = snip_platform::move_window(
                        raw,
                        f32::from(bounds.origin.x),
                        f32::from(bounds.origin.y),
                        w.scale_factor(),
                    );
                }
            }
            w.replace_root(cx, |w, cx| {
                let content = build(w, cx);
                view = Some(content.clone());
                gpui_kit::base::Root::new(content, w, cx)
            });
            if matches!(role, Role::Trail | Role::Pin | Role::Task) {
                transparent_root(w, cx);
            }
            if let Ok(raw) = crate::app::raw(w) {
                let _ = snip_platform::show_window(raw, true);
            }
            if options.focus {
                w.activate_window();
            }
        })?;
        return Ok((handle, view.expect("reused window built its root")));
    }
    #[cfg(target_os = "macos")]
    let bounds = options.window_bounds;
    // On Windows, Normal + titlebar: None still creates WS_SYSMENU. A popup
    // omits the system menu/buttons while GPUI supplies the titlebar-free frame.
    #[cfg(target_os = "windows")]
    let options = WindowOptions {
        kind: WindowKind::PopUp,
        ..options
    };
    let result = gpui_kit::open_window(options, cx, build)?;
    #[cfg(target_os = "windows")]
    if role != Role::Trail {
        cx.defer(crate::effects::raise);
    }
    #[cfg(target_os = "macos")]
    if matches!(role, Role::Capture | Role::Trail | Role::Task | Role::Pin)
        && let Some(WindowBounds::Windowed(bounds)) = bounds
    {
        result.0.update(cx, |_, w, _| {
            if let Ok(raw) = crate::app::raw(w) {
                let _ = snip_platform::move_window(
                    raw,
                    f32::from(bounds.origin.x),
                    f32::from(bounds.origin.y),
                    w.scale_factor(),
                );
            }
        })?;
    }
    if matches!(role, Role::Trail | Role::Pin | Role::Task) {
        result.0.update(cx, |_, w, cx| transparent_root(w, cx))?;
    }
    Ok(result)
}

pub fn park(role: Role, w: &mut Window, cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        if let Ok(raw) = crate::app::raw(w) {
            let _ = snip_platform::show_window(raw, false);
        }
        w.blur(cx);
        w.replace_root(cx, |w, cx| {
            let empty = cx.new(|_| Empty);
            gpui_kit::base::Root::new(empty, w, cx)
        });
        // Retain only a small empty native surface, never a captured desktop.
        w.resize(size(px(1.), px(1.)));
        let handle = w.window_handle();
        let pool = cx.global_mut::<Pool>().0.entry(role).or_default();
        if !pool.contains(&handle) {
            pool.push(handle);
        }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = (role, cx);
        w.remove_window();
    }
}
#[cfg(target_os = "macos")]
struct Empty;
#[cfg(target_os = "macos")]
impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

/// Native transparency does not override Kit's own themed root background.
pub fn transparent_root(w: &mut Window, cx: &mut App) {
    // Reassert the compositor setting when reusing a native window as well.
    w.set_background_appearance(WindowBackgroundAppearance::Transparent);
    if let Some(root) = w.root::<gpui_kit::base::Root>().flatten() {
        root.update(cx, |r, cx| {
            r.style().background = Some(transparent_black().into());
            cx.notify();
        });
    }
}
