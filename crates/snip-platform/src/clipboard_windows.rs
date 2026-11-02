use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use snip_core::clipboard::{MAX_BYTES, RichText};
use windows::{
    Win32::{
        Foundation::{HGLOBAL, HWND, LPARAM, RECT, WPARAM},
        Graphics::Gdi::*,
        System::{
            DataExchange::*,
            LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
            Memory::*,
        },
        UI::{Controls::RichEdit::*, WindowsAndMessaging::*},
    },
    core::w,
};

pub fn read_formats() -> Result<super::Formats> {
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    unsafe {
        OpenClipboard(None).context("剪贴板正被其他应用使用，请重试")?;
        let _open = Open;
        let read = |name| -> Result<Option<Vec<u8>>> {
            let format = RegisterClipboardFormatW(name);
            if IsClipboardFormatAvailable(format).is_err() {
                return Ok(None);
            }
            let handle = GetClipboardData(format)?;
            let global = HGLOBAL(handle.0);
            let length = GlobalSize(global);
            ensure!(length <= MAX_BYTES, "剪贴板富文本过大，请分段复制");
            let data = GlobalLock(global);
            ensure!(!data.is_null(), "无法读取剪贴板富文本");
            let bytes = std::slice::from_raw_parts(data.cast::<u8>(), length);
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(length);
            let copy = bytes[..end].to_vec();
            let _ = GlobalUnlock(global);
            Ok(Some(copy))
        };
        let read_optional = |name| {
            read(name)
                .inspect_err(|e| tracing::warn!("忽略不可读取的剪贴板格式：{e}"))
                .ok()
                .flatten()
        };
        Ok((
            read_optional(w!("HTML Format")),
            read_optional(w!("Rich Text Format")),
        ))
    }
}

/// An invisible RichEdit lives and dies on this worker. EM_FORMATRANGE does not
/// need screen capture, an activated window or an external rendering process.
pub fn render(document: &RichText) -> Result<RgbaImage> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("文字绘制服务不可用"))?;
    // RichEdit registers process-wide window classes. Keep its system DLL loaded
    // until exit, so callbacks never refer to an unloaded class procedure.
    static LIBRARY: std::sync::OnceLock<Result<usize, String>> = std::sync::OnceLock::new();
    let library = LIBRARY.get_or_init(|| unsafe {
        LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
            .map(|h| h.0 as usize)
            .map_err(|e| e.to_string())
    });
    library
        .as_ref()
        .map_err(|e| anyhow::anyhow!("文字绘制服务不可用：{e}"))?;
    struct Surface {
        window: HWND,
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
    }
    impl Drop for Surface {
        fn drop(&mut self) {
            unsafe {
                if !self.window.is_invalid() {
                    SendMessageW(
                        self.window,
                        EM_FORMATRANGE,
                        Some(WPARAM(0)),
                        Some(LPARAM(0)),
                    );
                    let _ = DestroyWindow(self.window);
                }
                if !self.dc.is_invalid() {
                    if !self.old.is_invalid() {
                        SelectObject(self.dc, self.old);
                    }
                    let _ = DeleteDC(self.dc);
                }
                if !self.bitmap.is_invalid() {
                    let _ = DeleteObject(self.bitmap.into());
                }
            }
        }
    }
    struct Input<'a>(&'a [u8]);
    unsafe extern "system" fn stream(
        cookie: usize,
        buffer: *mut u8,
        length: i32,
        written: *mut i32,
    ) -> u32 {
        unsafe {
            let input = &mut *(cookie as *mut Input<'_>);
            let count = input.0.len().min(length.max(0) as usize);
            std::ptr::copy_nonoverlapping(input.0.as_ptr(), buffer, count);
            *written = count as i32;
            input.0 = &input.0[count..];
            0
        }
    }
    let width = document.width.clamp(80, 1200);
    let mut height = 128u32;
    unsafe {
        let mut surface = Surface {
            window: HWND::default(),
            dc: HDC::default(),
            bitmap: HBITMAP::default(),
            old: HGDIOBJ::default(),
        };
        surface.window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("RICHEDIT50W"),
            w!(""),
            WS_POPUP | WINDOW_STYLE(ES_MULTILINE as u32),
            0,
            0,
            width as i32,
            height as i32,
            None,
            None,
            None,
            None,
        )?;
        let mut input = Input(&document.rtf);
        let mut stream_data = EDITSTREAM {
            dwCookie: (&mut input as *mut Input<'_>) as usize,
            dwError: 0,
            pfnCallback: Some(stream),
        };
        SendMessageW(
            surface.window,
            EM_EXLIMITTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(1_000_000)),
        );
        SendMessageW(
            surface.window,
            EM_STREAMIN,
            Some(WPARAM(SF_RTF as usize)),
            Some(LPARAM((&mut stream_data as *mut EDITSTREAM) as isize)),
        );
        let error = stream_data.dwError;
        ensure!(error == 0, "系统无法读取剪贴板富文本 ({error})");
        let length_query = GETTEXTLENGTHEX {
            flags: GTL_PRECISE | GTL_NUMCHARS,
            codepage: 1200,
        };
        let characters = SendMessageW(
            surface.window,
            EM_GETTEXTLENGTHEX,
            Some(WPARAM((&length_query as *const GETTEXTLENGTHEX) as usize)),
            Some(LPARAM(0)),
        )
        .0;
        ensure!(characters > 0, "剪贴板中没有可贴出的文字");
        surface.dc = CreateCompatibleDC(None);
        ensure!(!surface.dc.is_invalid(), "无法创建文字绘制设备");
        let dpi = GetDeviceCaps(Some(surface.dc), LOGPIXELSX).max(1);
        let twips = |pixel: u32| ((pixel as i64 * 1440) / dpi as i64) as i32;
        let format = |height: u32, draw: bool| {
            let mut range = FORMATRANGE {
                hdc: surface.dc,
                hdcTarget: surface.dc,
                rc: RECT {
                    left: twips(16),
                    top: twips(16),
                    right: twips(width - 16),
                    bottom: twips(height - 16),
                },
                rcPage: RECT {
                    left: 0,
                    top: 0,
                    right: twips(width),
                    bottom: twips(height),
                },
                chrg: CHARRANGE {
                    cpMin: 0,
                    cpMax: -1,
                },
            };
            SendMessageW(
                surface.window,
                EM_FORMATRANGE,
                Some(WPARAM(usize::from(draw))),
                Some(LPARAM((&mut range as *mut FORMATRANGE) as isize)),
            )
            .0
        };
        while format(height, false) < characters {
            ensure!(height < 10_000, "剪贴板内容过长，请分段复制");
            height = (height * 2).min(10_000);
        }
        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width as i32;
        info.bmiHeader.biHeight = -(height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;
        let mut pixels = std::ptr::null_mut();
        surface.bitmap = CreateDIBSection(
            Some(surface.dc),
            &info,
            DIB_RGB_COLORS,
            &mut pixels,
            None,
            0,
        )?;
        surface.old = SelectObject(surface.dc, surface.bitmap.into());
        let length = width as usize * height as usize * 4;
        std::ptr::write_bytes(pixels, 255, length);
        SendMessageW(
            surface.window,
            EM_FORMATRANGE,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
        let last = format(height, true);
        ensure!(
            last >= characters as isize,
            "剪贴板内容过长，请分段复制 ({last}/{characters})"
        );
        GdiFlush().ok()?;
        let mut rgba = std::slice::from_raw_parts(pixels.cast::<u8>(), length).to_vec();
        for p in rgba.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            p[3] = 255;
        }
        RgbaImage::from_raw(width, height, rgba).context("文字像素格式异常")
    }
}
