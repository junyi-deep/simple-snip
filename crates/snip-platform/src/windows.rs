use crate::WindowStyle;
use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use raw_window_handle::RawWindowHandle;
use snip_core::{
    geometry::{Point, Rect},
    ocr::{Cancellation, OcrEngine, OcrLine, OcrResult, OcrWord},
};
use windows::{
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine as WinOcr,
    Storage::Streams::DataWriter,
    Win32::{
        Foundation::{COLORREF, GetLastError, HWND, POINT, SetLastError, WIN32_ERROR},
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        UI::WindowsAndMessaging::*,
    },
};

pub fn screen_permission() -> bool {
    true
}
pub fn request_screen_permission() -> bool {
    true
}
fn hwnd(handle: RawWindowHandle) -> Result<HWND> {
    let RawWindowHandle::Win32(h) = handle else {
        anyhow::bail!("非 Win32 窗口")
    };
    Ok(HWND(h.hwnd.get() as *mut _))
}
pub fn native_style(handle: RawWindowHandle, style: WindowStyle) -> Result<()> {
    let hwnd = hwnd(handle)?;
    // SAFETY: The handle is borrowed from the live GPUI window on its UI thread.
    unsafe {
        let mut bits = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        bits |= WS_EX_TOOLWINDOW.0 | WS_EX_LAYERED.0;
        if style.click_through {
            bits |= WS_EX_TRANSPARENT.0;
        } else {
            bits &= !WS_EX_TRANSPARENT.0;
        }
        if style.no_activate {
            bits |= WS_EX_NOACTIVATE.0;
        } else {
            bits &= !WS_EX_NOACTIVATE.0;
        }
        SetLastError(WIN32_ERROR(0));
        let old = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, bits as isize);
        if old == 0 && GetLastError().0 != 0 {
            anyhow::bail!("修改贴图窗口样式失败: {:?}", GetLastError());
        }
        SetLayeredWindowAttributes(
            hwnd,
            COLORREF(0),
            (style.opacity.clamp(0.1, 1.) * 255.) as u8,
            LWA_ALPHA,
        )?;
        SetWindowPos(
            hwnd,
            Some(if style.topmost {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )?;
    }
    Ok(())
}
pub fn native_show(handle: RawWindowHandle, show: bool) -> Result<()> {
    unsafe {
        let _ = ShowWindow(
            hwnd(handle)?,
            if show { SW_SHOWNOACTIVATE } else { SW_HIDE },
        );
    }
    Ok(())
}
pub fn window_visible(handle: RawWindowHandle) -> Result<bool> {
    Ok(unsafe { IsWindowVisible(hwnd(handle)?).as_bool() })
}
pub fn native_cursor() -> Result<Point> {
    let mut p = POINT::default();
    unsafe {
        GetCursorPos(&mut p)?;
    }
    Ok(Point::new(p.x as f32, p.y as f32))
}
pub fn scroll_at(point: Point) -> Result<()> {
    let p = POINT {
        x: point.x as i32,
        y: point.y as i32,
    };
    unsafe {
        let target = WindowFromPoint(p);
        ensure!(!target.0.is_null(), "未找到滚动目标窗口");
        let coordinates = ((p.y as u32 & 0xffff) << 16) | (p.x as u32 & 0xffff);
        PostMessageW(
            Some(target),
            WM_MOUSEWHEEL,
            windows::Win32::Foundation::WPARAM((-360i16 as u16 as usize) << 16),
            windows::Win32::Foundation::LPARAM(coordinates as isize),
        )?;
    }
    Ok(())
}
pub fn annotation_font() -> Result<Vec<u8>> {
    let dir = std::env::var_os("WINDIR").context("缺少 WINDIR")?;
    Ok(std::fs::read(
        std::path::PathBuf::from(dir).join("Fonts/msyh.ttc"),
    )?)
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
pub struct NativeOcr;
impl OcrEngine for NativeOcr {
    fn recognize(&self, image: &RgbaImage, cancel: &Cancellation) -> Result<OcrResult> {
        ensure!(!cancel.cancelled(), "识别已取消");
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        let _apartment = Apartment;
        let max = WinOcr::MaxImageDimension()?;
        ensure!(
            image.width() <= max && image.height() <= max,
            "图片超过系统 OCR 的 {max}px 尺寸上限，请先裁剪图片"
        );
        let engine = WinOcr::TryCreateFromUserProfileLanguages().context(
            "Windows OCR 不可用。请在系统语言设置中安装中文 / 英文语言包及 OCR 功能后重试",
        )?;
        let writer = DataWriter::new()?;
        writer.WriteBytes(image.as_raw())?;
        let buffer = writer.DetachBuffer()?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Rgba8,
            image.width() as i32,
            image.height() as i32,
        )?;
        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        ensure!(!cancel.cancelled(), "识别已取消");
        let mut lines = vec![];
        for line in result.Lines()? {
            let mut words = vec![];
            let mut bounds: Option<Rect> = None;
            for word in line.Words()? {
                let b = word.BoundingRect()?;
                let r = Rect::new(b.X, b.Y, b.Width, b.Height);
                bounds = Some(if let Some(a) = bounds {
                    let x = a.x.min(r.x);
                    let y = a.y.min(r.y);
                    Rect::new(
                        x,
                        y,
                        a.right().max(r.right()) - x,
                        a.bottom().max(r.bottom()) - y,
                    )
                } else {
                    r
                });
                words.push(OcrWord {
                    text: word.Text()?.to_string(),
                    bounds: r,
                    confidence: None,
                });
            }
            lines.push(OcrLine {
                text: line.Text()?.to_string(),
                bounds: bounds.unwrap_or_default(),
                words,
                confidence: None,
                language: Some(engine.RecognizerLanguage()?.LanguageTag()?.to_string()),
            });
        }
        Ok(OcrResult { lines })
    }
}

pub fn inspect_window(handle: RawWindowHandle) -> Result<(bool, bool, f32)> {
    let hwnd = hwnd(handle)?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let mut alpha = 255;
        let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS::default();
        GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags))?;
        Ok((
            style & WS_EX_TOPMOST.0 != 0,
            style & WS_EX_TRANSPARENT.0 != 0,
            alpha as f32 / 255.,
        ))
    }
}

pub fn accessibility_permission() -> bool {
    true
}

pub fn prepare_window(_handle: RawWindowHandle) -> Result<()> {
    Ok(())
}

pub fn capture_unit_scale(_monitor: &xcap::Monitor) -> Result<f32> {
    Ok(1.)
}

pub fn move_window(handle: RawWindowHandle, x: f32, y: f32, scale: f32) -> Result<()> {
    unsafe {
        SetWindowPos(
            hwnd(handle)?,
            None,
            (x * scale).round() as i32,
            (y * scale).round() as i32,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
        )?;
    }
    Ok(())
}

/// Windows desktop capture does not use macOS TCC authorization.
pub fn verify_screen_permission() -> Result<()> {
    Ok(())
}

pub fn screen_permission_settings_url() -> Option<&'static str> {
    None
}
