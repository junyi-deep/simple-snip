#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
compile_error!("Only macos-arm64 and win-amd64 are supported");
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
pub use windows::*;

use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use raw_window_handle::RawWindowHandle;
use snip_core::geometry::{DisplayMap, Point, Rect};
use std::{borrow::Cow, sync::Arc};

#[derive(Debug)]
pub struct ScreenPermissionDenied;
impl std::fmt::Display for ScreenPermissionDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("当前运行的应用尚未获得屏幕录制权限")
    }
}
impl std::error::Error for ScreenPermissionDenied {}

#[derive(Clone)]
pub struct CapturedDisplay {
    pub id: u32,
    pub map: DisplayMap,
    pub image: Arc<RgbaImage>,
    pub windows: Vec<Rect>,
}
pub trait ScreenCapture {
    fn capture_desktop(&self) -> Result<Vec<CapturedDisplay>>;
}
pub struct NativeCapture;
impl ScreenCapture for NativeCapture {
    fn capture_desktop(&self) -> Result<Vec<CapturedDisplay>> {
        verify_screen_permission()?;
        let windows = window_rectangles();
        xcap::Monitor::all()?
            .into_iter()
            .map(|m| {
                let image = Arc::new(m.capture_image()?);
                let desktop = monitor_rect(&m)?;
                Ok(CapturedDisplay {
                    id: m.id()?,
                    map: DisplayMap {
                        desktop,
                        pixel_width: image.width(),
                        pixel_height: image.height(),
                    },
                    image,
                    windows: windows
                        .iter()
                        .map(|r| r.intersect(desktop))
                        .filter(|r| r.width > 1. && r.height > 1.)
                        .collect(),
                })
            })
            .collect()
    }
}
fn monitor_rect(m: &xcap::Monitor) -> Result<Rect> {
    Ok(Rect::new(
        m.x()? as f32,
        m.y()? as f32,
        m.width()? as f32,
        m.height()? as f32,
    ))
}
pub fn window_rectangles() -> Vec<Rect> {
    xcap::Window::all()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w.pid().ok() != Some(std::process::id()) && !w.is_minimized().unwrap_or(true))
        .filter_map(|w| {
            Some(Rect::new(
                w.x().ok()? as f32,
                w.y().ok()? as f32,
                w.width().ok()? as f32,
                w.height().ok()? as f32,
            ))
        })
        .filter(|r| r.width > 10. && r.height > 10.)
        .collect()
}
pub fn capture_region(display: u32, rect: Rect) -> Result<RgbaImage> {
    verify_screen_permission()?;
    let monitor = xcap::Monitor::all()?
        .into_iter()
        .find(|m| m.id().ok() == Some(display))
        .context("目标显示器已断开")?;
    let scale = capture_unit_scale(&monitor)?;
    let pixel_width = (monitor.width()? as f32 * scale).round() as u32;
    let pixel_height = (monitor.height()? as f32 * scale).round() as u32;
    let (x, y, w, h) = rect.pixel_bounds(pixel_width, pixel_height);
    ensure!(w > 0 && h > 0, "截图区域为空");
    let left = (x as f32 / scale).floor() as u32;
    let top = (y as f32 / scale).floor() as u32;
    let right = ((x + w) as f32 / scale).ceil() as u32;
    let bottom = ((y + h) as f32 / scale).ceil() as u32;
    let region = monitor.capture_region(left, top, right - left, bottom - top)?;
    let local_x = x.saturating_sub((left as f32 * scale).round() as u32);
    let local_y = y.saturating_sub((top as f32 * scale).round() as u32);
    ensure!(
        local_x + w <= region.width() && local_y + h <= region.height(),
        "显示器缩放在采集期间发生变化，请重新选择区域"
    );
    Ok(image::imageops::crop_imm(&region, local_x, local_y, w, h).to_image())
}

pub trait ClipboardProvider {
    fn read_image(&self) -> Result<Arc<RgbaImage>>;
    fn write_image(&self, image: &RgbaImage) -> Result<()>;
}
pub struct NativeClipboard;
impl ClipboardProvider for NativeClipboard {
    fn read_image(&self) -> Result<Arc<RgbaImage>> {
        let mut c = arboard::Clipboard::new()?;
        match c.get_image() {
            Ok(i) => {
                ensure!(
                    i.width as u64 * i.height as u64 <= 100_000_000,
                    "剪贴板图片过大"
                );
                Ok(Arc::new(
                    RgbaImage::from_raw(i.width as u32, i.height as u32, i.bytes.into_owned())
                        .context("剪贴板像素格式异常")?,
                ))
            }
            Err(e) => {
                if let Ok(text) = c.get_text() {
                    let path = text.trim().trim_start_matches("file://");
                    if std::path::Path::new(path).is_file() {
                        let reader = image::ImageReader::open(path)?;
                        let mut reader = reader.with_guessed_format()?;
                        reader.limits(image::Limits::default());
                        return Ok(Arc::new(reader.decode()?.into_rgba8()));
                    }
                }
                Err(anyhow::anyhow!(
                    "剪贴板中没有可读取的图片。请先复制图片：{e}"
                ))
            }
        }
    }
    fn write_image(&self, image: &RgbaImage) -> Result<()> {
        arboard::Clipboard::new()?.set_image(arboard::ImageData {
            width: image.width() as usize,
            height: image.height() as usize,
            bytes: Cow::Borrowed(image.as_raw()),
        })?;
        Ok(())
    }
}
pub fn write_text(text: String) -> Result<()> {
    arboard::Clipboard::new()?.set_text(text)?;
    Ok(())
}
#[derive(Clone, Copy)]
pub struct WindowStyle {
    pub topmost: bool,
    pub click_through: bool,
    pub opacity: f32,
    pub no_activate: bool,
    /// Full-display overlays must bypass macOS titled-window frame constraints.
    pub desktop_overlay: bool,
}
impl Default for WindowStyle {
    fn default() -> Self {
        Self {
            topmost: true,
            click_through: false,
            opacity: 1.,
            no_activate: false,
            desktop_overlay: false,
        }
    }
}
/// Called only on the GPUI/UI thread while its borrowed native handle is alive.
pub fn style_window(handle: RawWindowHandle, style: WindowStyle) -> Result<()> {
    native_style(handle, style)
}
pub fn show_window(handle: RawWindowHandle, show: bool) -> Result<()> {
    native_show(handle, show)
}
pub fn cursor_position() -> Result<Point> {
    native_cursor()
}
