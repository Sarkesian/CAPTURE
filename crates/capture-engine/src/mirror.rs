//! Live picture of CAPTURE's overlay window for the in-game hook.
//!
//! Over a game in exclusive fullscreen no window can be seen, so the overlay window is shown
//! off-screen (without taking focus from the game) and this captures it with Windows Graphics
//! Capture, copying each new picture into the hook's shared memory. The hook draws it inside
//! the game's frames and forwards the player's input to the window.

use std::sync::{Arc, Mutex};

use windows::core::{IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetClassNameW, GetClientRect, GetWindowRect};

use crate::inject::GameHook;

struct SendCtx(ID3D11Device, ID3D11DeviceContext);
unsafe impl Send for SendCtx {}
unsafe impl Sync for SendCtx {}

pub struct Mirror {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    hook: Arc<Mutex<Option<GameHook>>>,
    frames: Arc<std::sync::atomic::AtomicU32>,
    started: std::time::Instant,
}

unsafe impl Send for Mirror {}

/// The child window of a WebView2 window that takes mouse and keyboard input.
pub fn input_window(hwnd: u64) -> u64 {
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> windows::core::BOOL {
        let mut buf = [0u16; 64];
        let n = GetClassNameW(h, &mut buf);
        if String::from_utf16_lossy(&buf[..n as usize]) == "Chrome_RenderWidgetHostHWND" {
            *(l.0 as *mut u64) = h.0 as u64;
            return false.into();
        }
        true.into()
    }
    let mut found = 0u64;
    unsafe {
        let _ = EnumChildWindows(Some(HWND(hwnd as *mut _)), Some(each), LPARAM(&mut found as *mut u64 as isize));
    }
    if found == 0 { hwnd } else { found }
}

/// Where the input window's client area is inside a `w` x `h` picture of window `top`:
/// (left, top, width, height). The picture covers the window's visible frame; the game's hook
/// sends mouse positions in the input window's own coordinates, so the game must be handed
/// exactly that area, or everything the player points at is off by the frame's thickness.
unsafe fn input_area(top: HWND, input: HWND, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let mut frame = RECT::default();
    if DwmGetWindowAttribute(top, DWMWA_EXTENDED_FRAME_BOUNDS, &mut frame as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_err() && GetWindowRect(top, &mut frame).is_err() {
        return (0, 0, w, h);
    }
    let (mut origin, mut client) = (POINT::default(), RECT::default());
    if !ClientToScreen(input, &mut origin).as_bool() || GetClientRect(input, &mut client).is_err() {
        return (0, 0, w, h);
    }
    let (x, y) = ((origin.x - frame.left).clamp(0, w as i32 - 1) as u32, (origin.y - frame.top).clamp(0, h as i32 - 1) as u32);
    let (cw, ch) = ((client.right.max(0) as u32).min(w - x), (client.bottom.max(0) as u32).min(h - y));
    if cw == 0 || ch == 0 { (0, 0, w, h) } else { (x, y, cw, ch) }
}

impl Mirror {
    /// Start showing window `hwnd` inside the game through `hook`.
    pub fn start(hwnd: u64, hook: Arc<Mutex<Option<GameHook>>>) -> Result<Self, String> {
        unsafe {
            let (mut dev, mut ctx) = (None, None);
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, Some(&mut ctx)).map_err(|e| e.to_string())?;
            let (dev, ctx): (ID3D11Device, ID3D11DeviceContext) = (dev.unwrap(), ctx.unwrap());
            let winrt: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dev.cast::<IDXGIDevice>().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?.cast().map_err(|e| e.to_string())?;
            let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(|e| e.to_string())?;
            let item: GraphicsCaptureItem = interop.CreateForWindow(HWND(hwnd as *mut _)).map_err(|e| format!("overlay window cannot be captured: {e}"))?;
            let size = item.Size().map_err(|e| e.to_string())?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size).map_err(|e| e.to_string())?;
            let session = pool.CreateCaptureSession(&item).map_err(|e| e.to_string())?;
            let _ = session.SetIsBorderRequired(false);
            let _ = session.SetIsCursorCaptureEnabled(false);
            let gpu = SendCtx(dev, ctx);
            let staging: Mutex<Option<(ID3D11Texture2D, u32, u32)>> = Mutex::new(None);
            let sink = hook.clone();
            let (top, input) = (hwnd, input_window(hwnd));
            {
                let (x, y, cw, ch) = input_area(HWND(top as *mut _), HWND(input as *mut _), size.Width.max(1) as u32, size.Height.max(1) as u32);
                eprintln!("in-game overlay: overlay window picture {}x{}, input area {cw}x{ch} at {x},{y}", size.Width, size.Height);
            }
            let frames = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let counted = frames.clone();
            pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |pool, _| {
                let gpu = &gpu;
                let Ok(pool) = pool.ok() else { return Ok(()) };
                let frame = pool.TryGetNextFrame()?;
                let content = frame.ContentSize()?;
                let tex: ID3D11Texture2D = frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()?;
                let mut d = D3D11_TEXTURE2D_DESC::default();
                tex.GetDesc(&mut d);
                let (w, h) = ((content.Width.max(1) as u32).min(d.Width), (content.Height.max(1) as u32).min(d.Height));
                let mut st = staging.lock().unwrap();
                if st.as_ref().is_none_or(|s| s.1 != d.Width || s.2 != d.Height) {
                    let sd = D3D11_TEXTURE2D_DESC {
                        Width: d.Width,
                        Height: d.Height,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Usage: D3D11_USAGE_STAGING,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                        ..Default::default()
                    };
                    let mut t = None;
                    gpu.0.CreateTexture2D(&sd, None, Some(&mut t))?;
                    *st = t.map(|t| (t, d.Width, d.Height));
                }
                let Some((stage, _, _)) = st.as_ref() else { return Ok(()) };
                gpu.1.CopyResource(stage, &tex);
                let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                gpu.1.Map(stage, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
                if let Some(hk) = sink.lock().unwrap().as_ref() {
                    let (x, y, w, h) = input_area(HWND(top as *mut _), HWND(input as *mut _), w, h);
                    let src = (m.pData as *const u8).add(y as usize * m.RowPitch as usize + x as usize * 4);
                    hk.overlay_frame(w, h, src, m.RowPitch as usize);
                    counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                gpu.1.Unmap(stage, 0);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
            session.StartCapture().map_err(|e| e.to_string())?;
            if let Some(h) = hook.lock().unwrap().as_ref() {
                // Input goes to the overlay window itself, which hands it to its page (posting it
                // to the browser's own input window loses hovering).
                h.set_overlay(true, hwnd, hwnd);
            }
            Ok(Self { session, pool, hook, frames, started: std::time::Instant::now() })
        }
    }

    /// Windows has given no picture of the overlay window for two seconds: there is nothing
    /// to show in the game (seen when the window is not being drawn at all).
    pub fn blind(&self) -> bool {
        self.frames.load(std::sync::atomic::Ordering::Relaxed) == 0 && self.started.elapsed().as_secs_f32() > 2.0
    }
}

impl Drop for Mirror {
    fn drop(&mut self) {
        if let Some(h) = self.hook.lock().unwrap().as_ref() {
            h.set_overlay(false, 0, 0);
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
