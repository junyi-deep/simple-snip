use crate::WindowStyle;
use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use objc2::{AnyThread, MainThreadMarker, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_core_graphics::{
    CGEvent, CGEventTapLocation, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess,
    CGScrollEventUnit,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSRange, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};
use raw_window_handle::RawWindowHandle;
use snip_core::{
    geometry::{Point, Rect},
    ocr::{Cancellation, OcrEngine, OcrLine, OcrResult, OcrWord},
};

pub fn screen_permission() -> bool {
    CGPreflightScreenCaptureAccess()
}
/// A stale legacy preflight must not be the only authority. ScreenCaptureKit
/// checks the running app's current identity and returns an OS authorization error.
/// Call off the GUI thread; the system may ask the user to authorize this app.
pub fn verify_screen_permission() -> Result<()> {
    if screen_permission() {
        tracing::debug!("screen capture permission: legacy preflight granted");
        return Ok(());
    }
    tracing::debug!("screen capture permission: verifying through ScreenCaptureKit");
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let callback = block2::RcBlock::new(
        move |content: *mut objc2_screen_capture_kit::SCShareableContent,
              error: *mut objc2_foundation::NSError| {
            let result = if let Some(error) = unsafe { error.as_ref() } {
                Err((error.code(), error.localizedDescription().to_string()))
            } else if content.is_null() {
                Err((0, "系统没有返回可捕获内容".into()))
            } else {
                Ok(())
            };
            let _ = tx.try_send(result);
        },
    );
    unsafe {
        objc2_screen_capture_kit::SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(false, true, &callback);
    }
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(Ok(())) => {
            tracing::debug!("screen capture permission: ScreenCaptureKit granted");
            Ok(())
        }
        Ok(Err((code, _)))
            if code == objc2_screen_capture_kit::SCStreamErrorCode::UserDeclined.0 =>
        {
            Err(crate::ScreenPermissionDenied.into())
        }
        Ok(Err((code, message))) => anyhow::bail!("系统屏幕采集服务失败 ({code}): {message}"),
        Err(_) => anyhow::bail!("等待系统屏幕录制授权超时，请完成系统对话框后重试"),
    }
}
pub fn request_screen_permission() -> bool {
    CGRequestScreenCaptureAccess()
}
pub fn menu_bar_mode() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}
fn window(handle: RawWindowHandle) -> Result<Retained<NSWindow>> {
    ensure!(
        MainThreadMarker::new().is_some(),
        "窗口操作必须在主线程执行"
    );
    let RawWindowHandle::AppKit(handle) = handle else {
        anyhow::bail!("非 AppKit 窗口")
    };
    // SAFETY: GPUI owns this NSView and keeps it alive for this call; NSView's
    // window is retained for the duration of the immediate UI-thread operation.
    unsafe {
        let view = handle.ns_view.as_ptr() as *mut AnyObject;
        let window: *mut NSWindow = msg_send![view, window];
        Retained::retain(window).context("NSView 尚未关联窗口")
    }
}
pub fn native_style(handle: RawWindowHandle, style: WindowStyle) -> Result<()> {
    let w = window(handle)?;
    // GPUI's titlebar: None still creates a titled full-size-content window.
    // AppKit constrains it below the menu bar. Image surfaces require an actual
    // borderless window; preserve a popup's nonactivating-panel behavior.
    let mask = w.styleMask() & NSWindowStyleMask::NonactivatingPanel;
    // Only screen-wide overlays override the normal GPUI window style.
    if style.desktop_overlay && w.styleMask() != mask {
        let mtm = MainThreadMarker::new().context("窗口操作必须在主线程")?;
        let window = dispatch2::MainThreadBound::new(w.clone(), mtm);
        dispatch2::DispatchQueue::main().exec_async(move || {
            let mtm = MainThreadMarker::new().expect("main queue runs on the main thread");
            let w = window.get(mtm);
            if w.styleMask() != mask {
                let frame = w.frame();
                w.setStyleMask(mask);
                w.setFrame_display(frame, true);
            }
        });
    }
    w.setLevel(if style.topmost { 25 } else { 0 });
    w.setIgnoresMouseEvents(style.click_through);
    w.setAlphaValue(style.opacity.clamp(0.1, 1.) as f64);
    w.setHasShadow(false);
    w.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    if style.desktop_overlay {
        tracing::debug!(
            opaque = w.isOpaque(),
            background_alpha = w.backgroundColor().alphaComponent(),
            click_through = w.ignoresMouseEvents(),
            "desktop overlay composition"
        );
    }
    Ok(())
}
pub fn native_show(handle: RawWindowHandle, show: bool) -> Result<()> {
    let w = window(handle)?;
    if show {
        w.orderFrontRegardless();
    } else {
        w.orderOut(None);
    }
    Ok(())
}
/// An owned top-level NSWindow id, used only to exclude our capture controls.
pub fn exclude_capture_controls(handle: RawWindowHandle) -> Result<u32> {
    Ok(u32::try_from(window(handle)?.windowNumber())?)
}

/// ScreenCaptureKit filters remove the toolbar even when placement must fall
/// inside a full-screen selection. Objective-C objects stay in their callbacks;
/// only owned RGBA bytes cross the channel back to the capture worker.
pub fn capture_region_excluding(display_id: u32, rect: Rect, excluded: u32) -> Result<RgbaImage> {
    use objc2_screen_capture_kit::{
        SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
    };
    let monitor = xcap::Monitor::all()?
        .into_iter()
        .find(|m| m.id().ok() == Some(display_id))
        .context("目标显示器已断开")?;
    let scale = crate::capture_unit_scale(&monitor)? as f64;
    let (x, y, width, height) = rect.pixel_bounds(
        (monitor.width()? as f64 * scale).round() as u32,
        (monitor.height()? as f64 * scale).round() as u32,
    );
    ensure!(
        width > 0 && height > 0 && width as u64 * height as u64 * 4 <= 128 * 1024 * 1024,
        "采集区域为空或超过单帧内存上限"
    );
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let callback = block2::RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut objc2_foundation::NSError| {
            let result = (|| -> Result<()> {
                if let Some(error) = unsafe { error.as_ref() } {
                    if error.code() == -3801 {
                        return Err(crate::ScreenPermissionDenied.into());
                    }
                    anyhow::bail!(
                        "屏幕采集服务失败 ({}): {}",
                        error.code(),
                        error.localizedDescription()
                    );
                }
                let content = unsafe { content.as_ref() }.context("系统未返回可捕获内容")?;
                let display = unsafe { content.displays() }
                    .iter()
                    .find(|d| unsafe { d.displayID() == display_id })
                    .context("目标显示器已断开")?;
                let windows: Vec<_> = unsafe { content.windows() }
                    .iter()
                    .filter(|w| unsafe { w.windowID() == excluded })
                    .collect();
                let filter = unsafe {
                    SCContentFilter::initWithDisplay_excludingWindows(
                        SCContentFilter::alloc(),
                        &display,
                        &NSArray::from_retained_slice(&windows),
                    )
                };
                let config = unsafe { SCStreamConfiguration::new() };
                unsafe {
                    config.setWidth(width as usize);
                    config.setHeight(height as usize);
                    config.setSourceRect(objc2_core_foundation::CGRect::new(
                        objc2_core_foundation::CGPoint::new(x as f64 / scale, y as f64 / scale),
                        objc2_core_foundation::CGSize::new(
                            width as f64 / scale,
                            height as f64 / scale,
                        ),
                    ));
                    config.setShowsCursor(false);
                    config.setScalesToFit(false);
                }
                let tx_image = tx.clone();
                let image_callback = block2::RcBlock::new(
                    move |image: *mut objc2_core_graphics::CGImage,
                          error: *mut objc2_foundation::NSError| {
                        let result = (|| -> Result<RgbaImage> {
                            if let Some(error) = unsafe { error.as_ref() } {
                                if error.code() == -3801 {
                                    return Err(crate::ScreenPermissionDenied.into());
                                }
                                anyhow::bail!(
                                    "屏幕采集失败 ({}): {}",
                                    error.code(),
                                    error.localizedDescription()
                                );
                            }
                            let image = unsafe { image.as_ref() }.context("系统未返回屏幕图像")?;
                            rgba_from_cgimage(image, width, height)
                        })();
                        let _ = tx_image.try_send(result);
                    },
                );
                unsafe {
                    SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                        &filter,
                        &config,
                        Some(&image_callback),
                    );
                }
                Ok(())
            })();
            if let Err(error) = result {
                let _ = tx.try_send(Err(error));
            }
        },
    );
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(false, true, &callback);
    }
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .context("等待屏幕采集超时")?
}

fn rgba_from_cgimage(
    image: &objc2_core_graphics::CGImage,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
    };
    ensure!(
        objc2_core_graphics::CGImage::width(Some(image)) == width as usize
            && objc2_core_graphics::CGImage::height(Some(image)) == height as usize,
        "系统返回的采集尺寸与所选区域不一致，请重新选择区域"
    );
    let mut pixels = RgbaImage::new(width, height);
    let space = CGColorSpace::new_device_rgb().context("无法创建屏幕色彩空间")?;
    // SAFETY: pixels owns a width*height*4 allocation until after context drops;
    // the requested 8-bit RGBA context uses exactly width*4 bytes per row.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width as usize,
            height as usize,
            8,
            width as usize * 4,
            Some(&space),
            CGImageByteOrderInfo::Order32Big.0 | CGImageAlphaInfo::PremultipliedLast.0,
        )
    }
    .context("无法创建屏幕像素缓冲区")?;
    CGContext::draw_image(
        Some(&context),
        objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0., 0.),
            objc2_core_foundation::CGSize::new(width as f64, height as f64),
        ),
        Some(image),
    );
    drop(context);
    Ok(pixels)
}
pub fn window_visible(handle: RawWindowHandle) -> Result<bool> {
    Ok(window(handle)?.isVisible())
}
pub fn native_cursor() -> Result<Point> {
    let e = CGEvent::new(None).context("无法读取鼠标位置")?;
    let p = CGEvent::location(Some(&e));
    Ok(Point::new(p.x as f32, p.y as f32))
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
}
pub fn accessibility_permission() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}
pub fn scroll_at(point: Point) -> Result<()> {
    ensure!(
        accessibility_permission(),
        "自动滚动需要辅助功能权限。请在系统设置 → 隐私与安全性 → 辅助功能中允许 Simple Snip；也可继续手动滚动采集。"
    );
    // The OS may reject synthetic input without Accessibility. Never bypass it.
    let event = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 1, -350, 0, 0)
        .context("无法创建滚动事件")?;
    CGEvent::set_location(
        Some(&event),
        objc2_core_foundation::CGPoint::new(point.x as f64, point.y as f64),
    );
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    Ok(())
}
pub struct NativeOcr;
impl OcrEngine for NativeOcr {
    fn recognize(&self, image: &RgbaImage, cancel: &Cancellation) -> Result<OcrResult> {
        objc2::rc::autoreleasepool(|_| {
            ensure!(!cancel.cancelled(), "识别已取消");
            let mut png = std::io::Cursor::new(Vec::new());
            image.write_to(&mut png, image::ImageFormat::Png)?;
            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            let supported = unsafe { request.supportedRecognitionLanguagesAndReturnError() }
                .map_err(|e| anyhow::anyhow!("Vision 语言检测失败: {e}"))?;
            let langs: Vec<_> = ["zh-Hans", "zh-Hant", "en-US"]
                .into_iter()
                .filter(|s| supported.iter().any(|l| l.to_string() == *s))
                .map(NSString::from_str)
                .collect();
            ensure!(!langs.is_empty(), "Vision 没有可用的中英文识别语言");
            request.setRecognitionLanguages(&NSArray::from_retained_slice(&langs));
            let data = NSData::with_bytes(png.get_ref());
            let handler = VNImageRequestHandler::initWithData_options(
                VNImageRequestHandler::alloc(),
                &data,
                &NSDictionary::new(),
            );
            let requests = NSArray::from_slice(&[&*request as &VNRequest]);
            handler
                .performRequests_error(&requests)
                .map_err(|e| anyhow::anyhow!("Vision OCR 失败: {e}"))?;
            ensure!(!cancel.cancelled(), "识别已取消");
            let mut lines = vec![];
            let convert = |r: objc2_core_foundation::CGRect| {
                Rect::new(
                    r.origin.x as f32 * image.width() as f32,
                    (1. - r.origin.y - r.size.height) as f32 * image.height() as f32,
                    r.size.width as f32 * image.width() as f32,
                    r.size.height as f32 * image.height() as f32,
                )
            };
            if let Some(results) = request.results() {
                for observation in results.iter() {
                    let candidates = observation.topCandidates(1);
                    let Some(text) = candidates.firstObject() else {
                        continue;
                    };
                    let full = text.string().to_string();
                    let bounds = convert(unsafe { observation.boundingBox() });
                    let mut words = vec![];
                    let mut offset = 0;
                    // Use actual Vision boxes for each Unicode scalar (UTF-16 ranges),
                    // including CJK. Never invent equal-width OCR hit targets.
                    for ch in full.chars() {
                        let len = ch.len_utf16();
                        if !ch.is_whitespace()
                            && let Ok(boxed) =
                                unsafe { text.boundingBoxForRange_error(NSRange::new(offset, len)) }
                        {
                            words.push(OcrWord {
                                text: ch.to_string(),
                                bounds: convert(unsafe { boxed.boundingBox() }),
                                confidence: Some(text.confidence()),
                            });
                        }
                        offset += len;
                    }
                    if words.is_empty() {
                        words.push(OcrWord {
                            text: full.clone(),
                            bounds,
                            confidence: Some(text.confidence()),
                        });
                    }
                    lines.push(OcrLine {
                        text: full,
                        bounds,
                        words,
                        confidence: Some(text.confidence()),
                        language: None,
                    });
                }
            }
            lines.sort_by(|a, b| {
                a.bounds
                    .y
                    .total_cmp(&b.bounds.y)
                    .then(a.bounds.x.total_cmp(&b.bounds.x))
            });
            Ok(OcrResult { lines })
        })
    }
}

pub fn annotation_font() -> Result<Vec<u8>> {
    Ok(
        std::fs::read("/System/Library/Fonts/Supplemental/Arial Unicode.ttf")
            .or_else(|_| std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf"))?,
    )
}

pub fn inspect_window(handle: RawWindowHandle) -> Result<(bool, bool, f32)> {
    let w = window(handle)?;
    Ok((w.level() > 0, w.ignoresMouseEvents(), w.alphaValue() as f32))
}

/// Native frame in top-left desktop points, for geometry regression probes.
pub fn inspect_window_rect(handle: RawWindowHandle) -> Result<Rect> {
    let mtm = MainThreadMarker::new().context("窗口检查必须在主线程")?;
    let primary = objc2_app_kit::NSScreen::screens(mtm)
        .firstObject()
        .context("没有可用显示器")?;
    let frame = window(handle)?.frame();
    Ok(Rect::new(
        frame.origin.x as f32,
        (primary.frame().size.height - frame.origin.y - frame.size.height) as f32,
        frame.size.width as f32,
        frame.size.height as f32,
    ))
}

/// Give AppKit a stable Touch Bar provider instead of relying on discovery down
/// GPUI's changing responder tree. Public NSResponder API; no method swizzling.
/// Workaround candidate for gpui-kit#3192, covered by repeated native UI probes.
pub fn prepare_window(handle: RawWindowHandle) -> Result<()> {
    let mtm = MainThreadMarker::new().context("窗口操作必须在主线程")?;
    let w = window(handle)?;
    tracing::debug!(frame = ?w.frame(), content = ?w.contentView().map(|v| v.frame()), "native window geometry");
    if w.touchBar().is_none() {
        w.setTouchBar(Some(&objc2_app_kit::NSTouchBar::new(mtm)));
    }
    if let Some(view) = w.contentView()
        && view.touchBar().is_none()
    {
        view.setTouchBar(Some(&objc2_app_kit::NSTouchBar::new(mtm)));
    }
    Ok(())
}

pub fn capture_unit_scale(monitor: &xcap::Monitor) -> Result<f32> {
    Ok(monitor.scale_factor()?)
}

pub fn move_window(handle: RawWindowHandle, x: f32, y: f32, _scale: f32) -> Result<()> {
    let mtm = MainThreadMarker::new().context("窗口移动必须在主线程")?;
    let primary = objc2_app_kit::NSScreen::screens(mtm)
        .firstObject()
        .context("没有可用显示器")?;
    let point = objc2_foundation::NSPoint::new(x as f64, primary.frame().size.height - y as f64);
    // GPUI holds its App RefCell while calling platform operations. A synchronous
    // AppKit move calls back into that same App and loses the move notification.
    // Match GPUI's own asynchronous resize: retain the window, then move it on
    // the next main-queue turn, outside the App borrow.
    let window = dispatch2::MainThreadBound::new(window(handle)?, mtm);
    dispatch2::DispatchQueue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("main queue runs on the main thread");
        window.get(mtm).setFrameTopLeftPoint(point);
    });
    Ok(())
}

pub fn screen_permission_settings_url() -> Option<&'static str> {
    Some("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
}

#[cfg(test)]
mod capture_tests {
    #[test]
    fn core_graphics_rgba_conversion_preserves_channels_and_row_order() {
        use objc2_core_graphics::{
            CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGImageAlphaInfo,
            CGImageByteOrderInfo,
        };
        let mut source = image::RgbaImage::from_fn(5, 3, |x, y| {
            image::Rgba([(x * 40) as u8, (y * 70) as u8, 140, 255])
        });
        let space = CGColorSpace::new_device_rgb().unwrap();
        // SAFETY: the owned test buffer outlives the context and image snapshot.
        let context = unsafe {
            CGBitmapContextCreate(
                source.as_mut_ptr().cast(),
                5,
                3,
                8,
                20,
                Some(&space),
                CGImageByteOrderInfo::Order32Big.0 | CGImageAlphaInfo::PremultipliedLast.0,
            )
        }
        .unwrap();
        let image = CGBitmapContextCreateImage(Some(&context)).unwrap();
        let actual = super::rgba_from_cgimage(&image, 5, 3).unwrap();
        assert_eq!(actual, source);
    }
}
