use anyhow::{Result, ensure};
use image::RgbaImage;
use windows::{
    Win32::{
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, GdiFlush, GetObjectW, HDC, SelectObject,
        },
        UI::WindowsAndMessaging::{
            CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, HICON,
            ICONINFO,
        },
    },
    core::Owned,
};

struct MemoryDc(HDC);
impl Drop for MemoryDc {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: This DC was created by CreateCompatibleDC and is owned here.
            let _ = unsafe { DeleteDC(self.0) };
        }
    }
}

pub(super) fn composite(image: &mut RgbaImage, screen_x: i32, screen_y: i32) -> Result<()> {
    let mut cursor = CURSORINFO {
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: Both Win32 calls receive writable structs of the correct size.
    unsafe { GetCursorInfo(&mut cursor)? };
    if cursor.flags != CURSOR_SHOWING || cursor.hCursor.is_invalid() {
        return Ok(());
    }
    let icon = HICON(cursor.hCursor.0);
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(icon, &mut info)? };
    // SAFETY: GetIconInfo transfers ownership of these bitmap copies.
    let (color, mask) = unsafe { (Owned::new(info.hbmColor), Owned::new(info.hbmMask)) };
    let monochrome = color.is_invalid();
    let bitmap = if monochrome { *mask } else { *color };
    let mut metrics = BITMAP::default();
    // SAFETY: The selected bitmap handle stays alive through the query.
    let read = unsafe {
        GetObjectW(
            bitmap.into(),
            size_of::<BITMAP>() as i32,
            Some((&mut metrics as *mut BITMAP).cast()),
        )
    };
    ensure!(read != 0, "无法读取鼠标指针大小");
    let width = metrics.bmWidth;
    let height = if monochrome {
        metrics.bmHeight / 2
    } else {
        metrics.bmHeight
    };
    let left = cursor.ptScreenPos.x - screen_x - info.xHotspot as i32;
    let top = cursor.ptScreenPos.y - screen_y - info.yHotspot as i32;
    draw(image, icon, left, top, width, height)
}

fn draw(
    image: &mut RgbaImage,
    icon: HICON,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
) -> Result<()> {
    let x0 = left.max(0);
    let y0 = top.max(0);
    let x1 = left.saturating_add(width).min(image.width() as i32);
    let y1 = top.saturating_add(height).min(image.height() as i32);
    if x0 >= x1 || y0 >= y1 {
        return Ok(());
    }
    let patch_width = x1 - x0;
    let patch_height = y1 - y0;
    let bitmap = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: patch_width,
            biHeight: -patch_height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: The top-down DIB owns patch_width * patch_height BGRA pixels.
    // The DC is private to this call and is deselected before the bitmap is freed.
    unsafe {
        let dc = MemoryDc(CreateCompatibleDC(None));
        ensure!(!dc.0.is_invalid(), "无法创建 GIF 指针绘制上下文");
        let mut pixels = std::ptr::null_mut();
        let dib = Owned::new(CreateDIBSection(
            Some(dc.0),
            &bitmap,
            DIB_RGB_COLORS,
            &mut pixels,
            None,
            0,
        )?);
        ensure!(!pixels.is_null(), "无法读取 GIF 指针绘制缓冲区");
        let patch = std::slice::from_raw_parts_mut(
            pixels.cast::<[u8; 4]>(),
            patch_width as usize * patch_height as usize,
        );
        let coordinates = || (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x as u32, y as u32)));
        for (bgra, (x, y)) in patch.iter_mut().zip(coordinates()) {
            let [red, green, blue, _] = image.get_pixel(x, y).0;
            *bgra = [blue, green, red, 255];
        }
        let previous = SelectObject(dc.0, (*dib).into());
        ensure!(!previous.is_invalid(), "无法选中 GIF 指针绘制缓冲区");
        let result = DrawIconEx(
            dc.0,
            left - x0,
            top - y0,
            icon,
            width,
            height,
            0,
            None,
            DI_NORMAL,
        );
        let flushed = GdiFlush().as_bool();
        SelectObject(dc.0, previous);
        result?;
        ensure!(flushed, "无法完成 GIF 指针绘制");
        let patch = std::slice::from_raw_parts(
            pixels.cast::<[u8; 4]>(),
            patch_width as usize * patch_height as usize,
        );
        for (&[blue, green, red, _], (x, y)) in patch.iter().zip(coordinates()) {
            image.get_pixel_mut(x, y).0[..3].copy_from_slice(&[red, green, blue]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{IDC_ARROW, LoadCursorW};

    #[test]
    fn native_cursor_draw_clips_to_capture_edges() {
        let mut frame = RgbaImage::from_pixel(64, 40, image::Rgba([70, 80, 90, 255]));
        let before = frame.clone();
        // SAFETY: IDC_ARROW identifies a shared system cursor and does not transfer ownership.
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap();
        let icon = HICON(cursor.0);
        draw(&mut frame, icon, -40, 2, 32, 32).unwrap();
        assert_eq!(frame, before);
        draw(&mut frame, icon, -2, 3, 32, 32).unwrap();
        assert_ne!(frame, before);
        assert_eq!(*frame.get_pixel(40, 30), *before.get_pixel(40, 30));
        assert!(frame.pixels().all(|pixel| pixel[3] == 255));
    }
}
