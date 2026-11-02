//! Clipboard payloads are copied before conversion; rendering never modifies them.
use crate::{ClipboardProvider, NativeClipboard};
use anyhow::{Result, ensure};
use image::RgbaImage;
use snip_core::clipboard::{self, RichText};
use std::sync::Arc;
#[cfg(target_os = "windows")]
#[path = "clipboard_windows.rs"]
mod native;
#[cfg(target_os = "macos")]
#[path = "clipboard_macos.rs"]
mod native;

type Formats = (Option<Vec<u8>>, Option<Vec<u8>>);

impl NativeClipboard {
    pub fn read_pin(&self) -> Result<Arc<RgbaImage>> {
        if let Ok(image) = self.read_image() {
            return Ok(image);
        }
        let (html, rtf) = native::read_formats()?;
        for (bytes, convert) in [
            (
                rtf.as_deref(),
                clipboard::from_rtf as fn(&[u8]) -> Result<RichText>,
            ),
            (
                html.as_deref(),
                clipboard::from_html as fn(&[u8]) -> Result<RichText>,
            ),
        ] {
            if let Some(bytes) = bytes {
                match convert(bytes).and_then(|d| render(&d)) {
                    Ok(image) => return Ok(Arc::new(image)),
                    Err(error) => tracing::warn!("剪贴板富文本转换未完成，尝试其他格式：{error}"),
                }
            }
        }
        let text = arboard::Clipboard::new()?
            .get_text()
            .map_err(|_| anyhow::anyhow!("剪贴板中没有可贴出的图片、文本或表格"))?;
        Ok(Arc::new(render(&clipboard::from_text(&text)?)?))
    }
}
pub(crate) fn render(document: &RichText) -> Result<RgbaImage> {
    crop(native::render(document)?)
}
fn crop(image: RgbaImage) -> Result<RgbaImage> {
    ensure!(
        u64::from(image.width()) * u64::from(image.height()) <= clipboard::MAX_PIXELS,
        "剪贴板内容图片过大，请分段复制"
    );
    let (mut left, mut top, mut right, mut bottom) = (image.width(), image.height(), 0, 0);
    for (x, y, p) in image.enumerate_pixels() {
        if p[3] > 0 && p.0[..3].iter().any(|v| *v < 250) {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    ensure!(right > left && bottom > top, "剪贴板中没有可显示的文字");
    let (left, top) = (left.saturating_sub(12), top.saturating_sub(12));
    let (right, bottom) = (
        (right + 12).min(image.width()),
        (bottom + 12).min(image.height()),
    );
    Ok(image::imageops::crop_imm(&image, left, top, right - left, bottom - top).to_image())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "windows")]
    #[test]
    fn repeated_native_renders_release_gdi_objects() {
        use windows::Win32::System::Threading::{
            GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
        };
        let doc = clipboard::from_text("Resource lifecycle 中文").unwrap();
        drop(render(&doc).unwrap());
        let before = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        for _ in 0..8 {
            drop(render(&doc).unwrap());
        }
        let after = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        assert!(
            after <= before + 16,
            "GDI handles grew from {before} to {after}"
        );
    }
    #[test]
    fn native_text_and_styled_table_render_without_clipboard_or_visible_windows() {
        let text =
            render(&clipboard::from_text("Clipboard 中文 2026\nSecond line").unwrap()).unwrap();
        assert!(text.width() > 100 && text.height() > 35);
        let table=render(&clipboard::from_html(br#"<table border='1'><tr><td style='background-color:#12ab34;color:#ff0000;font-weight:bold'>Hello</td><td>42</td></tr><tr><td>World</td><td>123</td></tr></table>"#).unwrap()).unwrap();
        if let Some(folder) = std::env::var_os("SNIP_CLIPBOARD_FIXTURES") {
            let folder = std::path::PathBuf::from(folder);
            std::fs::create_dir_all(&folder).unwrap();
            table.save(folder.join("clipboard-table.png")).unwrap();
            text.save(folder.join("clipboard-text.png")).unwrap();
        }
        assert!(table.width() > 200 && table.height() > 35);
        assert!(
            table
                .pixels()
                .filter(|p| p[1] > 130 && p[0] < 50 && p[2] < 80)
                .count()
                > (table.width() * table.height() / 9) as usize,
            "cell background must survive"
        );
        assert!(
            table
                .pixels()
                .any(|p| p[0] > 180 && p[1] < 100 && p[2] < 100),
            "red text must survive"
        );
        let rtf=clipboard::from_rtf(br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\colortbl;\red255\green0\blue0;}\f0\fs36\cf1 Rich text \u20013?\u25991?}").unwrap();
        let native_rtf = render(&rtf).unwrap();
        assert!(
            native_rtf
                .pixels()
                .any(|p| p[0] > 180 && p[1] < 100 && p[2] < 100),
            "native RTF styling must survive"
        );
        let plain = render(&clipboard::from_text("small").unwrap()).unwrap();
        assert!(
            plain.width() < 200 && plain.height() < 100,
            "trim empty render surface"
        );
    }
}
