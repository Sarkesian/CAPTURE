//! Windows Graphics Capture of a window or monitor into a fixed-size "latest frame" texture.
//!
//! WGC only delivers frames when content changes, so the encoder does not consume WGC frames
//! directly: it samples [`Latest`] on its own constant-rate clock (see `video.rs`).
//! No hooks, no injection: if Windows will not capture a target, frames simply never arrive
//! and the engine reports that honestly.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::gpu::Gpu;

struct SendCtx(ID3D11DeviceContext, IDirect3DDevice);
unsafe impl Send for SendCtx {}
unsafe impl Sync for SendCtx {}

pub enum Source {
    Window(isize),
    Monitor(isize),
}

/// The most recent captured frame, already cropped/padded to the encoder size.
pub struct Latest {
    pub texture: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    pub has_frame: bool,
    last_size: SizeInt32,
}

pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub latest: Arc<Mutex<Latest>>,
    pub frames: Arc<AtomicU64>,
    pub closed: Arc<AtomicBool>,
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
}

unsafe impl Send for Capture {}
unsafe impl Sync for Capture {}

fn even(v: i32) -> u32 {
    (v.max(2) as u32) & !1
}

impl Capture {
    pub fn start(gpu: &Gpu, source: Source, cursor: bool) -> Result<Self, String> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| format!("Windows Graphics Capture is unavailable: {e}"))?;
        let item: GraphicsCaptureItem = unsafe {
            match source {
                Source::Window(h) => interop.CreateForWindow(HWND(h as *mut _)),
                Source::Monitor(h) => interop.CreateForMonitor(HMONITOR(h as *mut _)),
            }
        }
        .map_err(|e| format!("This window or display cannot be captured ({e}). Try Display capture."))?;
        let size = item.Size().map_err(|e| e.to_string())?;
        let (width, height) = (even(size.Width), even(size.Height));

        let winrt_device: IDirect3DDevice = unsafe {
            let dxgi: IDXGIDevice = gpu.device.cast().map_err(|e| e.to_string())?;
            CreateDirect3D11DeviceFromDXGIDevice(&dxgi).map_err(|e| e.to_string())?.cast().map_err(|e| e.to_string())?
        };

        let latest = Arc::new(Mutex::new(make_latest(gpu, width, height)?));
        let frames = Arc::new(AtomicU64::new(0));
        let closed = Arc::new(AtomicBool::new(false));

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(|e| e.to_string())?;
        let session = pool.CreateCaptureSession(&item).map_err(|e| e.to_string())?;
        let _ = session.SetIsBorderRequired(false); // Win11; ignored where unsupported
        let _ = session.SetIsCursorCaptureEnabled(cursor);

        {
            let (latest, frames) = (latest.clone(), frames.clone());
            // The immediate context is multithread-protected (see gpu.rs), so using it from the
            // WGC callback thread is safe.
            let ctx = SendCtx(gpu.context.clone(), winrt_device.clone());
            pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                move |pool, _| {
                    let ctx = &ctx; // capture the whole Send wrapper, not its fields
                    let (context, dev) = (&ctx.0, &ctx.1);
                    let Ok(pool) = pool.ok() else { return Ok(()) };
                    let frame = pool.TryGetNextFrame()?;
                    let content = frame.ContentSize()?;
                    let tex: ID3D11Texture2D = unsafe { frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()? };
                    let mut l = latest.lock().unwrap();
                    if content != l.last_size {
                        // Size changed (resize, resolution switch): clear stale edges and let the
                        // pool deliver full-size frames from now on.
                        unsafe { context.ClearRenderTargetView(&l.rtv, &[0.0, 0.0, 0.0, 1.0]) };
                        l.last_size = content;
                        let _ = pool.Recreate(dev, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, content);
                    }
                    let b = D3D11_BOX {
                        left: 0,
                        top: 0,
                        front: 0,
                        right: (content.Width.max(0) as u32).min(width),
                        bottom: (content.Height.max(0) as u32).min(height),
                        back: 1,
                    };
                    if b.right > 0 && b.bottom > 0 {
                        unsafe { context.CopySubresourceRegion(&l.texture, 0, 0, 0, 0, &tex, 0, Some(&b)) };
                        l.has_frame = true;
                        frames.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(())
                },
            ))
            .map_err(|e| e.to_string())?;
        }
        {
            let closed = closed.clone();
            item.Closed(&TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                closed.store(true, Ordering::Relaxed);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        }
        session.StartCapture().map_err(|e| format!("Capture could not start: {e}"))?;
        Ok(Self { width, height, latest, frames, closed, session, pool })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn make_latest(gpu: &Gpu, width: u32, height: u32) -> Result<Latest, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut texture)) }.map_err(|e| e.to_string())?;
    let texture = texture.unwrap();
    let mut rtv = None;
    unsafe { gpu.device.CreateRenderTargetView(&texture, None, Some(&mut rtv)) }.map_err(|e| e.to_string())?;
    let rtv = rtv.unwrap();
    unsafe { gpu.context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]) };
    Ok(Latest { texture, rtv, has_frame: false, last_size: SizeInt32 { Width: width as i32, Height: height as i32 } })
}
