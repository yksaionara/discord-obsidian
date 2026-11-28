//! A window's app icon, as Discord's share dialog shows beside each window:
//! the icon the window gives, else its class's, drawn to RGBA. The window id
//! is the one libwebrtc lists it by, its handle on Windows.

/// `window`'s icon as `size` by `size` straight RGBA, if it has one.
#[cfg(windows)]
pub fn icon(window: u64, size: u32) -> Option<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, SelectObject,
        },
        UI::WindowsAndMessaging::{
            DI_NORMAL, DrawIconEx, GCLP_HICON, GCLP_HICONSM, GetClassLongPtrW, HICON, ICON_BIG,
            ICON_SMALL2, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETICON,
        },
    };
    let hwnd = window as usize as HWND;
    // SAFETY: asks a window for its icon, giving up after 50 ms if it hangs;
    // a window that's gone answers nothing.
    let asked = |kind: u32| unsafe {
        let mut result: usize = 0;
        SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            kind as usize,
            0,
            SMTO_ABORTIFHUNG,
            50,
            &mut result,
        );
        result
    };
    // SAFETY: reads the window's class; zero when it has no icon.
    let class = |index| unsafe { GetClassLongPtrW(hwnd, index) };
    let icon = [
        asked(ICON_BIG),
        asked(ICON_SMALL2),
        class(GCLP_HICON),
        class(GCLP_HICONSM),
    ]
    .into_iter()
    .find(|icon| *icon != 0)? as HICON;
    let side = size as i32;
    // Drawn on black and on white: the difference is how see-through each
    // pixel is, which old icons without an alpha channel don't say.
    let draw = |background: u8| -> Option<Vec<u8>> {
        // SAFETY: a DIB section of `size` square 32-bit pixels, drawn into
        // through a memory DC, copied out, then freed.
        unsafe {
            let dc = CreateCompatibleDC(ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut core::ffi::c_void = ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let length = (size * size * 4) as usize;
            std::ptr::write_bytes(bits as *mut u8, background, length);
            let old = SelectObject(dc, bitmap);
            let drawn = DrawIconEx(dc, 0, 0, icon, side, side, 0, ptr::null_mut(), DI_NORMAL);
            let pixels = std::slice::from_raw_parts(bits as *const u8, length).to_vec();
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
            (drawn != 0).then_some(pixels)
        }
    };
    let (black, white) = (draw(0)?, draw(255)?);
    let mut rgba = vec![0u8; black.len()];
    for ((out, on_black), on_white) in rgba
        .chunks_exact_mut(4)
        .zip(black.chunks_exact(4))
        .zip(white.chunks_exact(4))
    {
        // BGRA. White shows through by 255 - alpha.
        let alpha = 255 - (on_white[1].saturating_sub(on_black[1]));
        if alpha > 0 {
            // On black, a pixel is its colour times its alpha.
            for (channel, value) in [on_black[2], on_black[1], on_black[0]]
                .into_iter()
                .enumerate()
            {
                out[channel] = ((u32::from(value) * 255 + u32::from(alpha) / 2) / u32::from(alpha))
                    .min(255) as u8;
            }
            out[3] = alpha;
        }
    }
    rgba.iter()
        .skip(3)
        .step_by(4)
        .any(|alpha| *alpha > 0)
        .then_some(rgba)
}

#[cfg(not(windows))]
pub fn icon(_window: u64, _size: u32) -> Option<Vec<u8>> {
    None
}
