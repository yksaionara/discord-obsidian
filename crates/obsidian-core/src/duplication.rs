//! A screen's frames through DXGI Desktop Duplication, as each one is
//! presented. Windows' own capture (WGC) gives at most about 60 new frames a
//! second, and the stream then repeated them up to its rate; duplication
//! gives every frame the screen shows, up to its refresh rate, and none
//! while nothing changes.
//!
//! Each frame is copied from the GPU into one reused staging texture, and
//! converted from there to I420 into one reused buffer. The pointer, which
//! duplication leaves out of the picture, is drawn onto that buffer while a
//! frame is sent and taken off again after, so a frame that only moves the
//! pointer is the last picture again with the pointer in its new place.

use anyhow::{Context, Result, bail};
use livekit::webrtc::{native::yuv_helper, video_frame::I420Buffer};
use windows::{
    Win32::{
        Foundation::{HMODULE, POINT},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::{
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_NOT_FOUND,
                DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
                DXGI_OUTDUPL_POINTER_SHAPE_INFO, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput,
                IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
            },
        },
    },
    core::Interface,
};

/// The screen the window `window` fills, as the monitor handle
/// [`Duplication::open`] takes: a fullscreen or borderless game's. None for
/// a window smaller than its screen, a minimised one, or one that's gone.
pub fn filled_screen(window: u64) -> Option<u64> {
    use windows::Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow},
        UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindowVisible},
    };
    // SAFETY: reads where a window and its screen are; a window that's gone
    // fails each call.
    unsafe {
        let hwnd = HWND(window as usize as *mut core::ffi::c_void);
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok()?;
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let screen = info.rcMonitor;
        (rect.left <= screen.left
            && rect.top <= screen.top
            && rect.right >= screen.right
            && rect.bottom >= screen.bottom)
            .then_some(monitor.0 as usize as u64)
    }
}

/// Whether `window` is a game's as a game shows: minimized, as one in full
/// screen is while another app has the focus, or filling its screen; and
/// still there.
pub fn minimized_or_filling(window: u64) -> bool {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{IsIconic, IsWindow},
    };
    // SAFETY: reads the window's state; a window that's gone fails IsWindow.
    unsafe {
        let hwnd = HWND(window as usize as *mut core::ffi::c_void);
        IsWindow(Some(hwnd)).as_bool() && (IsIconic(hwnd).as_bool() || filled_screen(window).is_some())
    }
}

/// Whether the window `window` is minimized.
pub fn minimized(window: u64) -> bool {
    use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::IsIconic};
    // SAFETY: only reads the window's state; a window that's gone isn't.
    unsafe { IsIconic(HWND(window as usize as *mut core::ffi::c_void)).as_bool() }
}

/// Whether the window `window` is still there.
pub fn window_exists(window: u64) -> bool {
    use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::IsWindow};
    // SAFETY: only asks whether the handle names a window.
    unsafe { IsWindow(Some(HWND(window as usize as *mut core::ffi::c_void))).as_bool() }
}

/// The screen the window `window` fills while it's the one in front, as
/// [`filled_screen`] gives it: only then does duplicating the screen show
/// the window and nothing else. A borderless game alt-tabbed away from
/// still fills its screen, behind the windows now in front.
pub fn fronted_screen(window: u64) -> Option<u64> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow},
    };
    // SAFETY: reads which window is in front; a window that's gone gives a
    // null root, which matches no other.
    let in_front = unsafe {
        let hwnd = HWND(window as usize as *mut core::ffi::c_void);
        let front = GetForegroundWindow();
        !front.is_invalid() && GetAncestor(front, GA_ROOT) == GetAncestor(hwnd, GA_ROOT)
    };
    if in_front { filled_screen(window) } else { None }
}

/// What [`Duplication::next`] found.
pub enum Next {
    /// A new picture, or the last one with the pointer moved.
    Frame,
    /// Nothing changed in the time given.
    Timeout,
    /// The desktop changed mode or went to a secure screen: open again.
    Lost,
}

/// Desktop duplication of one screen.
pub struct Duplication {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    output: IDXGIOutput1,
    duplication: IDXGIOutputDuplication,
    staging: Option<ID3D11Texture2D>,
    /// The screen's size in pixels.
    pub width: u32,
    pub height: u32,
    /// The last picture, at the screen's size, without the pointer.
    picture: I420Buffer,
    /// Whether `picture` holds a picture yet.
    have_picture: bool,
    pointer: Pointer,
    /// What the pointer covers in `picture` while it's drawn on.
    under: Vec<u8>,
}

#[derive(Default)]
struct Pointer {
    visible: bool,
    position: POINT,
    /// What drawing the pointer does to each of its pixels, row by row,
    /// `width` to a row.
    shape: Vec<Paint>,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
enum Paint {
    Clear,
    /// Blended in by its alpha, out of 255.
    Over { y: u8, u: u8, v: u8, alpha: u8 },
    /// Inverts what's under it, as a text cursor does.
    Invert,
}

/// Desktop duplication can't work with the screen; the caller shares it
/// another way.
#[derive(Debug)]
pub struct Unavailable(pub String);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Unavailable {}

impl Duplication {
    /// Duplicates the screen `id` names: a monitor handle, as WebRTC's
    /// screen list gives, or else a screen's place in DXGI's list. Without
    /// one, or when none matches, the primary screen.
    pub fn open(id: Option<u64>) -> Result<Self> {
        // SAFETY: DXGI and Direct3D calls on interfaces this function owns.
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().context("no DXGI factory")?;
            let mut outputs: Vec<(IDXGIAdapter1, IDXGIOutput)> = vec![];
            for adapter_index in 0.. {
                let adapter = match factory.EnumAdapters1(adapter_index) {
                    Ok(adapter) => adapter,
                    Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(error) => return Err(error).context("listing graphics adapters"),
                };
                for output_index in 0.. {
                    match adapter.EnumOutputs(output_index) {
                        Ok(output) => outputs.push((adapter.clone(), output)),
                        Err(_) => break,
                    }
                }
            }
            let described: Vec<_> = outputs
                .iter()
                .filter_map(|(adapter, output)| {
                    let desc = output.GetDesc().ok()?;
                    desc.AttachedToDesktop.as_bool().then_some((adapter, output, desc))
                })
                .collect();
            let chosen = id
                .and_then(|id| {
                    described
                        .iter()
                        .find(|(_, _, desc)| desc.Monitor.0 as usize as u64 == id)
                        .or_else(|| described.get(id as usize))
                })
                .or_else(|| {
                    described.iter().find(|(_, _, desc)| {
                        desc.DesktopCoordinates.left == 0 && desc.DesktopCoordinates.top == 0
                    })
                })
                .or_else(|| described.first());
            let Some((adapter, output, _)) = chosen else {
                bail!(Unavailable("no screen to duplicate".into()));
            };
            let output: IDXGIOutput1 = output
                .cast()
                .map_err(|_| Unavailable("this Windows has no desktop duplication".into()))?;
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                *adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|error| Unavailable(format!("no Direct3D device: {error}")))?;
            let device: ID3D11Device = device.context("no Direct3D device")?;
            let context = context.context("no Direct3D context")?;
            // Each frame's copy waits on the GPU, which a game keeps busy:
            // ahead of the game's work, as OBS puts its own, the copy
            // doesn't hold the stream back. It's a small copy, and Windows
            // may refuse; the copy then waits its turn.
            if let Ok(dxgi) = device.cast::<windows::Win32::Graphics::Dxgi::IDXGIDevice>()
                && let Err(error) = dxgi.SetGPUThreadPriority(7)
            {
                tracing::debug!(%error, "the screen's copies keep the GPU's usual priority");
            }
            let duplication = output
                .DuplicateOutput(&device)
                .map_err(|error| Unavailable(format!("could not duplicate the screen: {error}")))?;
            let mode = duplication.GetDesc().ModeDesc;
            let (width, height) = (mode.Width.max(2), mode.Height.max(2));
            Ok(Self {
                device,
                context,
                output,
                duplication,
                staging: None,
                width,
                height,
                picture: I420Buffer::new(width, height),
                have_picture: false,
                pointer: Pointer::default(),
                under: Vec::new(),
            })
        }
    }

    /// Opens the same screen again after [`Next::Lost`].
    pub fn reopen(&mut self) -> Result<()> {
        // SAFETY: as in `open`.
        unsafe {
            let duplication = self
                .output
                .DuplicateOutput(&self.device)
                .context("could not duplicate the screen again")?;
            let mode = duplication.GetDesc().ModeDesc;
            let (width, height) = (mode.Width.max(2), mode.Height.max(2));
            if (width, height) != (self.width, self.height) {
                self.width = width;
                self.height = height;
                self.picture = I420Buffer::new(width, height);
                self.staging = None;
                self.have_picture = false;
            }
            self.duplication = duplication;
        }
        Ok(())
    }

    /// Waits up to `timeout_ms` for the screen to change.
    pub fn next(&mut self, timeout_ms: u32) -> Result<Next> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: the out pointers live through the call.
        let acquired = unsafe {
            self.duplication
                .AcquireNextFrame(timeout_ms, &mut info, &mut resource)
        };
        match acquired {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(Next::Timeout),
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => return Ok(Next::Lost),
            Err(error) => return Err(error).context("could not take the screen's frame"),
        }
        let result = self.take(&info, resource);
        // SAFETY: a frame was acquired above.
        let released = unsafe { self.duplication.ReleaseFrame() };
        if let Err(error) = released
            && error.code() == DXGI_ERROR_ACCESS_LOST
        {
            return Ok(Next::Lost);
        }
        let changed = result?;
        Ok(if changed { Next::Frame } else { Next::Timeout })
    }

    /// Takes the acquired frame's picture and pointer: whether either
    /// changed.
    fn take(&mut self, info: &DXGI_OUTDUPL_FRAME_INFO, resource: Option<IDXGIResource>) -> Result<bool> {
        let mut changed = false;
        if info.LastMouseUpdateTime != 0 {
            let position = info.PointerPosition;
            let visible = position.Visible.as_bool();
            if visible != self.pointer.visible
                || (visible
                    && (position.Position.x, position.Position.y)
                        != (self.pointer.position.x, self.pointer.position.y))
            {
                changed = true;
            }
            self.pointer.visible = visible;
            if visible {
                self.pointer.position = position.Position;
            }
            if info.PointerShapeBufferSize > 0 {
                self.read_pointer_shape(info.PointerShapeBufferSize);
                changed = true;
            }
        }
        if info.LastPresentTime != 0 && info.AccumulatedFrames > 0 {
            let resource = resource.context("a frame without its picture")?;
            self.copy_picture(&resource)?;
            changed = true;
        }
        Ok(changed && self.have_picture)
    }

    fn copy_picture(&mut self, resource: &IDXGIResource) -> Result<()> {
        // SAFETY: Direct3D calls on textures this duplication owns; the
        // mapped memory is read only while mapped, within its rows.
        unsafe {
            let texture: ID3D11Texture2D = resource.cast().context("the frame isn't a texture")?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
                bail!(Unavailable(format!("the screen's format is {:?}", desc.Format)));
            }
            let (width, height) = (desc.Width.max(2), desc.Height.max(2));
            if (width, height) != (self.width, self.height) {
                self.width = width;
                self.height = height;
                self.picture = I420Buffer::new(width, height);
                self.staging = None;
            }
            if self.staging.is_none() {
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut staging = None;
                self.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                    .context("no staging texture")?;
                self.staging = staging;
            }
            let staging = self.staging.as_ref().expect("made above");
            self.context.CopyResource(staging, &texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .context("could not read the screen's frame")?;
            let stride = mapped.RowPitch;
            let bytes = std::slice::from_raw_parts(
                mapped.pData as *const u8,
                stride as usize * desc.Height as usize,
            );
            let (y_stride, u_stride, v_stride) = self.picture.strides();
            let (y, u, v) = self.picture.data_mut();
            // libyuv's ARGB is B, G, R, A in memory, as the screen's is.
            yuv_helper::argb_to_i420(
                bytes,
                stride,
                y,
                y_stride,
                u,
                u_stride,
                v,
                v_stride,
                (desc.Width as i32).min(width as i32),
                (desc.Height as i32).min(height as i32),
            );
            self.context.Unmap(staging, 0);
            self.have_picture = true;
        }
        Ok(())
    }

    fn read_pointer_shape(&mut self, size: u32) {
        let mut buffer = vec![0u8; size as usize];
        let mut required = 0u32;
        let mut info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
        // SAFETY: the buffer is `size` bytes, as the call is told.
        let read = unsafe {
            self.duplication.GetFramePointerShape(
                size,
                buffer.as_mut_ptr().cast(),
                &mut required,
                &mut info,
            )
        };
        if read.is_err() {
            return;
        }
        let pointer = &mut self.pointer;
        // The hot spot is where the position points; the picture starts above
        // and left of it by that much. Duplication gives the picture's corner
        // already, so it's kept as is.
        pointer.shape.clear();
        let (width, pitch) = (info.Width as usize, info.Pitch as usize);
        match info.Type {
            // Monochrome: an AND mask above an XOR mask, a bit a pixel.
            1 => {
                let height = info.Height as usize / 2;
                for row in 0..height {
                    for column in 0..width {
                        let bit = 0x80 >> (column % 8);
                        let and = buffer.get(row * pitch + column / 8).is_some_and(|byte| byte & bit != 0);
                        let xor = buffer
                            .get((row + height) * pitch + column / 8)
                            .is_some_and(|byte| byte & bit != 0);
                        pointer.shape.push(match (and, xor) {
                            (false, false) => solid(0, 0, 0, 255),
                            (false, true) => solid(255, 255, 255, 255),
                            (true, false) => Paint::Clear,
                            (true, true) => Paint::Invert,
                        });
                    }
                }
                pointer.width = info.Width;
                pointer.height = height as u32;
            }
            // Colour, with alpha.
            2 => {
                for row in 0..info.Height as usize {
                    for column in 0..width {
                        let at = row * pitch + column * 4;
                        let Some(&[blue, green, red, alpha]) = buffer.get(at..at + 4).map(|pixel| <&[u8; 4]>::try_from(pixel).expect("four")) else {
                            pointer.shape.push(Paint::Clear);
                            continue;
                        };
                        pointer.shape.push(if alpha == 0 {
                            Paint::Clear
                        } else {
                            solid(red, green, blue, alpha)
                        });
                    }
                }
                pointer.width = info.Width;
                pointer.height = info.Height;
            }
            // Masked colour: alpha 0 replaces, 0xFF XORs.
            4 => {
                for row in 0..info.Height as usize {
                    for column in 0..width {
                        let at = row * pitch + column * 4;
                        let Some(&[blue, green, red, mask]) = buffer.get(at..at + 4).map(|pixel| <&[u8; 4]>::try_from(pixel).expect("four")) else {
                            pointer.shape.push(Paint::Clear);
                            continue;
                        };
                        pointer.shape.push(match (mask, red | green | blue) {
                            (0, _) => solid(red, green, blue, 255),
                            (_, 0) => Paint::Clear,
                            _ => Paint::Invert,
                        });
                    }
                }
                pointer.width = info.Width;
                pointer.height = info.Height;
            }
            _ => {
                pointer.width = 0;
                pointer.height = 0;
            }
        }
    }

    /// Whether a picture has come yet.
    pub fn has_picture(&self) -> bool {
        self.have_picture
    }

    /// Calls `show` with the last picture, at the screen's size, with the
    /// pointer drawn on. What the pointer covers is kept aside meanwhile and
    /// put back after, so the picture stays the screen's, for the pointer's
    /// next place.
    pub fn with_picture<T>(&mut self, show: impl FnOnce(&mut I420Buffer) -> T) -> T {
        self.draw_pointer();
        let shown = show(&mut self.picture);
        self.lift_pointer();
        shown
    }

    /// The part of the picture the pointer covers: its columns and rows, or
    /// none while it's hidden, has no shape or is off the screen.
    fn pointer_area(&self) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
        let pointer = &self.pointer;
        if !pointer.visible || pointer.width == 0 {
            return None;
        }
        let span = |at: i32, length: u32, limit: u32| {
            let start = i64::from(at).clamp(0, i64::from(limit));
            let end = (i64::from(at) + i64::from(length)).clamp(0, i64::from(limit));
            (start < end).then_some(start as usize..end as usize)
        };
        Some((
            span(pointer.position.x, pointer.width, self.width)?,
            span(pointer.position.y, pointer.height, self.height)?,
        ))
    }

    /// Draws the pointer onto the picture, keeping what it covers in
    /// `under`: the luma's rows, then each chroma row's U and V.
    fn draw_pointer(&mut self) {
        self.under.clear();
        let Some((columns, rows)) = self.pointer_area() else {
            return;
        };
        let pointer = &self.pointer;
        let (y_stride, u_stride, v_stride) = self.picture.strides();
        let (y_stride, u_stride, v_stride) = (y_stride as usize, u_stride as usize, v_stride as usize);
        let (y_plane, u_plane, v_plane) = self.picture.data_mut();
        let chroma_columns = columns.start / 2..columns.end.div_ceil(2);
        for row in rows.clone() {
            self.under.extend_from_slice(&y_plane[row * y_stride..][columns.clone()]);
        }
        for row in rows.start / 2..rows.end.div_ceil(2) {
            self.under.extend_from_slice(&u_plane[row * u_stride..][chroma_columns.clone()]);
            self.under.extend_from_slice(&v_plane[row * v_stride..][chroma_columns.clone()]);
        }
        for screen_y in rows {
            let row = (screen_y as i64 - i64::from(pointer.position.y)) as usize;
            for screen_x in columns.clone() {
                let column = (screen_x as i64 - i64::from(pointer.position.x)) as usize;
                let paint = pointer.shape[row * pointer.width as usize + column];
                let at = screen_y * y_stride + screen_x;
                let chroma = (screen_x % 2 == 0 && screen_y % 2 == 0).then(|| {
                    (
                        (screen_y / 2) * u_stride + screen_x / 2,
                        (screen_y / 2) * v_stride + screen_x / 2,
                    )
                });
                match paint {
                    Paint::Clear => {}
                    Paint::Invert => {
                        y_plane[at] = 255 - y_plane[at];
                    }
                    Paint::Over { y, u, v, alpha } => {
                        y_plane[at] = blend(y_plane[at], y, alpha);
                        if let Some((u_at, v_at)) = chroma {
                            u_plane[u_at] = blend(u_plane[u_at], u, alpha);
                            v_plane[v_at] = blend(v_plane[v_at], v, alpha);
                        }
                    }
                }
            }
        }
    }

    /// Puts back what [`Self::draw_pointer`] drew over.
    fn lift_pointer(&mut self) {
        if self.under.is_empty() {
            return;
        }
        let Some((columns, rows)) = self.pointer_area() else {
            return;
        };
        let (y_stride, u_stride, v_stride) = self.picture.strides();
        let (y_stride, u_stride, v_stride) = (y_stride as usize, u_stride as usize, v_stride as usize);
        let (y_plane, u_plane, v_plane) = self.picture.data_mut();
        let chroma_columns = columns.start / 2..columns.end.div_ceil(2);
        let mut under = self.under.as_slice();
        let mut put = |into: &mut [u8], columns: &std::ops::Range<usize>| {
            let (kept, rest) = under.split_at(columns.len());
            into[columns.clone()].copy_from_slice(kept);
            under = rest;
        };
        for row in rows.clone() {
            put(&mut y_plane[row * y_stride..], &columns);
        }
        for row in rows.start / 2..rows.end.div_ceil(2) {
            put(&mut u_plane[row * u_stride..], &chroma_columns);
            put(&mut v_plane[row * v_stride..], &chroma_columns);
        }
    }
}

/// A pointer pixel of this colour, in BT.601's limited range as libyuv's
/// ARGB to I420 makes the rest of the picture.
fn solid(red: u8, green: u8, blue: u8, alpha: u8) -> Paint {
    let (r, g, b) = (red as i32, green as i32, blue as i32);
    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
    Paint::Over {
        y: y.clamp(0, 255) as u8,
        u: u.clamp(0, 255) as u8,
        v: v.clamp(0, 255) as u8,
        alpha,
    }
}

fn blend(under: u8, over: u8, alpha: u8) -> u8 {
    ((over as u32 * alpha as u32 + under as u32 * (255 - alpha as u32) + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a desktop: `cargo test -p obsidian-core duplicates -- --ignored`.
    #[test]
    #[ignore = "needs a screen"]
    fn duplicates_the_primary_screen() {
        let mut duplication = Duplication::open(None).expect("opens");
        let started = std::time::Instant::now();
        let mut frames = 0;
        while started.elapsed() < std::time::Duration::from_secs(2) {
            if let Next::Frame = duplication.next(50).expect("a frame") {
                frames += 1;
            }
        }
        assert!(duplication.has_picture());
        // The pointer comes off again: the picture is the screen's.
        let luma = |picture: &mut I420Buffer| picture.data().0.to_vec();
        let screen = luma(&mut duplication.picture);
        let shown = duplication.with_picture(luma);
        assert_eq!(shown.len(), screen.len());
        assert!(luma(&mut duplication.picture) == screen);
        eprintln!("{}x{}: {frames} frames in 2 s", duplication.width, duplication.height);
    }
}

#[cfg(test)]
mod tests_filled {
    use super::filled_screen;
    use windows::Win32::UI::WindowsAndMessaging::{GetDesktopWindow, GetShellWindow};

    #[test]
    fn the_desktop_fills_a_screen_and_nothing_fills_none() {
        // SAFETY: the shell's own windows, which always exist.
        let (desktop, shell) = unsafe { (GetDesktopWindow(), GetShellWindow()) };
        let window = if shell.is_invalid() { desktop } else { shell };
        assert!(filled_screen(window.0 as usize as u64).is_some());
        assert_eq!(filled_screen(0), None);
        assert_eq!(super::fronted_screen(0), None, "nothing is in front");
    }
}
