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
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetClassNameW};

use crate::inject::GameHook;

struct SendCtx(ID3D11Device, ID3D11DeviceContext);
unsafe impl Send for SendCtx {}
unsafe impl Sync for SendCtx {}

pub struct Mirror {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    hook: Arc<Mutex<Option<GameHook>>>,
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
                    hk.overlay_frame(w, h, m.pData as *const u8, m.RowPitch as usize);
                }
                gpu.1.Unmap(stage, 0);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
            session.StartCapture().map_err(|e| e.to_string())?;
            if let Some(h) = hook.lock().unwrap().as_ref() {
                h.set_overlay(true, hwnd, input_window(hwnd));
            }
            Ok(Self { session, pool, hook })
        }
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
