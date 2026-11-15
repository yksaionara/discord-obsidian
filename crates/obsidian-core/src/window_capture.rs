//! One window's frames through Windows Graphics Capture, as Discord's client
//! captures a window: what the window draws, games included, even with
//! other windows over it, and nothing of those windows.
//!
//! WebRTC's window capturer makes its Direct3D device on the system's first
//! graphics adapter, which on a machine with a virtual display driver (as
//! game streaming hosts install) isn't the GPU drawing the game, and games
//! came out black. This makes its device on the adapter driving the
//! window's screen, as duplication does for a screen.
//!
//! Like [`crate::duplication`], each frame is copied into one reused
//! staging texture and converted to I420 into one reused buffer.

use anyhow::{Context, Result, bail};
use livekit::webrtc::{
    native::yuv_helper,
    video_frame::{I420Buffer, VideoBuffer},
};
use windows::{
    Foundation::TimeSpan,
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession},
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN},
            Direct3D11::{
                D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
                ID3D11DeviceContext, ID3D11Texture2D,
            },
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, IDXGIAdapter, IDXGIAdapter1,
                IDXGIDevice, IDXGIFactory1,
            },
            Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow},
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
        },
    },
    core::Interface,
};

use crate::duplication::{Next, Unavailable};

/// Frames the pool keeps: one being read while the next is drawn.
const BUFFERS: i32 = 2;

/// A window being captured.
pub struct WindowCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    /// The size the pool's frames are made at, the window's when it opened.
    size: SizeInt32,
    staging: Option<ID3D11Texture2D>,
    /// The last picture, at the window's size, and whether there is one.
    picture: I420Buffer,
    have_picture: bool,
}

impl WindowCapture {
    /// Starts capturing the window `window`, a handle as WebRTC's window list
    /// gives. Fails with [`Unavailable`] where Windows can't capture it, so
    /// WebRTC's capturer can try.
    pub fn open(window: u64) -> Result<Self> {
        let hwnd = HWND(window as usize as *mut core::ffi::c_void);
        // SAFETY: Direct3D and WinRT calls on interfaces this function owns;
        // a window that's gone fails CreateForWindow.
        unsafe {
            let adapter = adapter_of(hwnd);
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                adapter.as_ref(),
                if adapter.is_some() {
                    D3D_DRIVER_TYPE_UNKNOWN
                } else {
                    D3D_DRIVER_TYPE_HARDWARE
                },
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
            let dxgi: IDXGIDevice = device.cast().context("no DXGI device")?;
            let winrt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
                .and_then(|inspectable| inspectable.cast())
                .map_err(|error| Unavailable(format!("no WinRT Direct3D device: {error}")))?;
            let interop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                    .map_err(|error| Unavailable(format!("no window capture here: {error}")))?;
            let item: GraphicsCaptureItem = interop
                .CreateForWindow(hwnd)
                .map_err(|error| Unavailable(format!("can't capture that window: {error}")))?;
            let size = item.Size()?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                BUFFERS,
                size,
            )?;
            let session = pool.CreateCaptureSession(&item)?;
            // Each is newer than window capture itself: Windows 11 draws no
            // yellow border without being asked, and 24H2 lets frames come
            // faster than 60 a second. Older Windows keeps its defaults.
            let _ = session.SetIsBorderRequired(false);
            let _ = session.SetIsCursorCaptureEnabled(true);
            let _ = session.SetMinUpdateInterval(TimeSpan { Duration: 10_000 });
            session.StartCapture()?;
            let (width, height) = even(size);
            Ok(Self {
                device,
                context,
                winrt_device,
                pool,
                session,
                size,
                staging: None,
                picture: I420Buffer::new(width, height),
                have_picture: false,
            })
        }
    }

    /// Waits up to `timeout_ms` for the window to draw, keeping the newest
    /// frame when several came.
    pub fn next(&mut self, timeout_ms: u32) -> Result<Next> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.into());
        loop {
            let mut newest = None;
            while let Ok(frame) = self.pool.TryGetNextFrame() {
                if let Some(older) = newest.replace(frame) {
                    let _ = older.Close();
                }
            }
            if let Some(frame) = newest {
                let taken = self.take(&frame);
                let _ = frame.Close();
                return taken.map(|taken| if taken { Next::Frame } else { Next::Timeout });
            }
            if std::time::Instant::now() >= deadline {
                return Ok(Next::Timeout);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Copies `frame`'s picture into the buffer: whether it had one. A
    /// window that changed size gets a pool at its new size, and its
    /// picture comes with the next frame.
    fn take(&mut self, frame: &windows::Graphics::Capture::Direct3D11CaptureFrame) -> Result<bool> {
        let content = frame.ContentSize()?;
        if content.Width <= 0 || content.Height <= 0 {
            return Ok(false);
        }
        if (content.Width, content.Height) != (self.size.Width, self.size.Height) {
            self.size = content;
            self.pool.Recreate(
                &self.winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                BUFFERS,
                content,
            )?;
            return Ok(false);
        }
        let (width, height) = even(content);
        // SAFETY: Direct3D calls on textures this capture owns; the mapped
        // memory is read only while mapped, within its rows.
        unsafe {
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            let texture: ID3D11Texture2D = access.GetInterface()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
                bail!("the window's format is {:?}", desc.Format);
            }
            let (width, height) = (width.min(desc.Width), height.min(desc.Height));
            if (self.picture.width(), self.picture.height()) != (width, height) {
                self.picture = I420Buffer::new(width, height);
                self.staging = None;
            }
            if self.staging.is_none() {
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
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
            // The pool's texture may be larger than what the window drew.
            let region = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: width,
                bottom: height,
                back: 1,
            };
            self.context
                .CopySubresourceRegion(staging, 0, 0, 0, 0, &texture, 0, Some(&region));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .context("could not read the window's frame")?;
            let stride = mapped.RowPitch;
            let bytes =
                std::slice::from_raw_parts(mapped.pData as *const u8, stride as usize * height as usize);
            let (y_stride, u_stride, v_stride) = self.picture.strides();
            let (y, u, v) = self.picture.data_mut();
            // libyuv's ARGB is B, G, R, A in memory, as the window's is.
            yuv_helper::argb_to_i420(
                bytes,
                stride,
                y,
                y_stride,
                u,
                u_stride,
                v,
                v_stride,
                width as i32,
                height as i32,
            );
            self.context.Unmap(staging, 0);
        }
        self.have_picture = true;
        Ok(true)
    }

    /// Whether a picture has come yet.
    pub fn has_picture(&self) -> bool {
        self.have_picture
    }

    /// The last picture, at the window's size, which the next frame
    /// replaces. Windows draws the pointer into it.
    pub fn picture(&mut self) -> &mut I420Buffer {
        &mut self.picture
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// `size` with even sides, as codecs want; at least 2 by 2.
fn even(size: SizeInt32) -> (u32, u32) {
    let side = |value: i32| ((value.max(2) as u32) & !1).max(2);
    (side(size.Width), side(size.Height))
}

/// The graphics adapter driving the screen `window` is on, or none to let
/// Direct3D pick.
fn adapter_of(window: HWND) -> Option<IDXGIAdapter> {
    // SAFETY: DXGI enumeration over interfaces this function owns.
    unsafe {
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        for adapter_index in 0.. {
            let adapter: IDXGIAdapter1 = match factory.EnumAdapters1(adapter_index) {
                Ok(adapter) => adapter,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => return None,
                Err(_) => return None,
            };
            for output_index in 0.. {
                let Ok(output) = adapter.EnumOutputs(output_index) else { break };
                if output.GetDesc().is_ok_and(|desc| desc.Monitor == monitor) {
                    return adapter.cast().ok();
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_even() {
        assert_eq!(even(SizeInt32 { Width: 1569, Height: 1079 }), (1568, 1078));
        assert_eq!(even(SizeInt32 { Width: 0, Height: 1 }), (2, 2));
    }

    /// Captures the shell's own window, which every desktop has.
    #[test]
    #[ignore = "needs a desktop"]
    fn captures_a_window() {
        use windows::Win32::UI::WindowsAndMessaging::GetShellWindow;
        // SAFETY: the shell's window, which exists on a desktop.
        let shell = unsafe { GetShellWindow() };
        let mut capture = WindowCapture::open(shell.0 as usize as u64).unwrap();
        let mut frames = 0;
        for _ in 0..50 {
            if matches!(capture.next(20).unwrap(), Next::Frame) {
                frames += 1;
            }
        }
        assert!(capture.has_picture(), "{frames} frames");
        let picture = capture.picture();
        assert!(picture.width() >= 2 && picture.height() >= 2);
    }

    /// Opens the minimized window `OBSIDIAN_TEST_WINDOW` names by handle, as
    /// a game in full screen is while the share dialog has the focus: it
    /// opens, and gives no frames until it's back.
    #[test]
    #[ignore = "needs a minimized window"]
    fn opens_a_minimized_window() {
        let window: u64 = std::env::var("OBSIDIAN_TEST_WINDOW").unwrap().parse().unwrap();
        let mut capture = WindowCapture::open(window).unwrap();
        for _ in 0..10 {
            assert!(matches!(capture.next(20).unwrap(), Next::Timeout));
        }
        assert!(!capture.has_picture());
    }
}
