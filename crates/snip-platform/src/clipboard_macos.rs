use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use objc2::{AnyThread, rc::autoreleasepool};
use objc2_app_kit::{
    NSAffineTransformNSAppKitAdditions, NSAttributedStringAppKitDocumentFormats,
    NSAttributedStringNSExtendedStringDrawing, NSBitmapImageFileType, NSBitmapImageRep, NSColor,
    NSDeviceRGBColorSpace, NSGraphicsContext, NSPasteboard, NSRectFill, NSStringDrawingOptions,
};
use objc2_foundation::{
    NSAffineTransform, NSAttributedString, NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString,
};
use snip_core::clipboard::{MAX_BYTES, MAX_PIXELS, RichText};

pub fn read_formats() -> Result<super::Formats> {
    autoreleasepool(|_| {
        let board = NSPasteboard::generalPasteboard();
        let read = |name: &str| -> Result<Option<Vec<u8>>> {
            let Some(data) = board.dataForType(&NSString::from_str(name)) else {
                return Ok(None);
            };
            ensure!(data.len() <= MAX_BYTES, "剪贴板富文本过大，请分段复制");
            Ok(Some(data.to_vec()))
        };
        let read_optional = |name| {
            read(name)
                .inspect_err(|e| tracing::warn!("忽略不可读取的剪贴板格式：{e}"))
                .ok()
                .flatten()
        };
        Ok((read_optional("public.html"), read_optional("public.rtf")))
    })
}
pub fn render(document: &RichText) -> Result<RgbaImage> {
    // A private bitmap graphics context is thread-local. No NSView, HTML importer,
    // AppKit main-loop rendezvous or screen-recording permission is involved.
    autoreleasepool(|_| unsafe {
        let text = NSAttributedString::initWithRTF_documentAttributes(
            NSAttributedString::alloc(),
            &NSData::with_bytes(&document.rtf),
            None,
        )
        .context("系统无法读取剪贴板富文本")?;
        let width = document.width.clamp(80, 1200);
        let scale = 96. / 72.;
        let options = NSStringDrawingOptions::UsesLineFragmentOrigin
            | NSStringDrawingOptions::UsesFontLeading;
        let bounds = text.boundingRectWithSize_options_context(
            NSSize::new((width - 32) as f64 / scale, 1_000_000.),
            options,
            None,
        );
        ensure!(
            bounds.size.height.is_finite() && bounds.size.height * scale < 9968.,
            "剪贴板内容过长，请分段复制"
        );
        let height = ((bounds.size.height * scale).ceil() as u32 + 32).max(48);
        ensure!(
            u64::from(width) * u64::from(height) <= MAX_PIXELS,
            "剪贴板内容图片过大，请分段复制"
        );
        let bitmap=NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),std::ptr::null_mut(),width as isize,height as isize,8,4,true,false,NSDeviceRGBColorSpace,width as isize*4,32)
            .context("无法创建文字位图")?;
        let graphics = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)
            .context("无法创建文字绘制上下文")?;
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                NSGraphicsContext::restoreGraphicsState_class();
            }
        }
        NSGraphicsContext::saveGraphicsState_class();
        let _restore = Restore;
        NSGraphicsContext::setCurrentContext(Some(&graphics));
        NSColor::whiteColor().setFill();
        NSRectFill(NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(width as f64, height as f64),
        ));
        let transform = NSAffineTransform::transform();
        transform.scaleBy(scale);
        transform.concat();
        text.drawWithRect_options_context(
            NSRect::new(
                NSPoint::new(16. / scale, 16. / scale),
                NSSize::new((width - 32) as f64 / scale, (height - 32) as f64 / scale),
            ),
            options,
            None,
        );
        let png = bitmap
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
            .context("文字位图编码失败")?;
        Ok(image::load_from_memory(&png.to_vec())?.into_rgba8())
    })
}
