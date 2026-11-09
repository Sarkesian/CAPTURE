//! Test target for the in-game hook: a DirectX 11 "game" that clears its back buffer to a
//! slowly cycling colour and presents with vsync.
//!
//!   hook-testapp [--fullscreen] [--secs N] [--title T] [--flip]
//!
//! `--fullscreen`: true exclusive fullscreen (IDXGISwapChain::SetFullscreenState), the mode in
//! which no ordinary window can be shown over the game. `--flip`: flip-model swap chain
//! (FLIP_DISCARD) instead of the legacy blt model many older DX11 games use.

use std::time::{Duration, Instant};

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    DefWindowProcW(h, m, w, l)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let fullscreen = args.iter().any(|a| a == "--fullscreen");
    let flip = args.iter().any(|a| a == "--flip");
    let secs: f64 = args.iter().position(|a| a == "--secs").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let title = args.iter().position(|a| a == "--title").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "CAPTURE hook test".into());
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: w!("CaptureHookTest"), hCursor: LoadCursorW(None, IDC_ARROW).unwrap(), ..Default::default() };
        RegisterClassW(&wc);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let (w, h) = if fullscreen { (sw, sh) } else { (1280, 720) };
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("CaptureHookTest"),
            &HSTRING::from(title),
            if fullscreen { WS_POPUP } else { WS_OVERLAPPEDWINDOW },
            if fullscreen { 0 } else { 100 },
            if fullscreen { 0 } else { 100 },
            w,
            h,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .unwrap();
        let _ = ShowWindow(hwnd, SW_SHOW);

        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC { Width: w as u32, Height: h as u32, Format: DXGI_FORMAT_R8G8B8A8_UNORM, ..Default::default() },
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            OutputWindow: hwnd,
            Windowed: true.into(),
            SwapEffect: if flip { DXGI_SWAP_EFFECT_FLIP_DISCARD } else { DXGI_SWAP_EFFECT_DISCARD },
            Flags: DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH.0 as u32,
        };
        let (mut sc, mut dev, mut ctx) = (None, None, None);
        D3D11CreateDeviceAndSwapChain(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), None, D3D11_SDK_VERSION, Some(&desc), Some(&mut sc), Some(&mut dev), None, Some(&mut ctx)).unwrap();
        let (sc, dev, ctx): (IDXGISwapChain, ID3D11Device, ID3D11DeviceContext) = (sc.unwrap(), dev.unwrap(), ctx.unwrap());
        if fullscreen {
            if let Err(e) = sc.SetFullscreenState(true, None) {
                eprintln!("exclusive fullscreen refused: {e}");
            }
            let _ = sc.ResizeBuffers(0, 0, 0, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH);
        }
        println!("ready pid={} fullscreen={fullscreen} flip={flip}", std::process::id());
        let start = Instant::now();
        let mut rtv: Option<ID3D11RenderTargetView> = None;
        let mut msg = MSG::default();
        let mut frames = 0u64;
        while start.elapsed() < Duration::from_secs_f64(secs) {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if rtv.is_none() {
                let bb: ID3D11Texture2D = sc.GetBuffer(0).unwrap();
                let mut v = None;
                dev.CreateRenderTargetView(&bb, None, Some(&mut v)).unwrap();
                rtv = v;
            }
            let t = start.elapsed().as_secs_f32();
            let c = [0.2 + 0.2 * (t * 0.7).sin(), 0.25 + 0.2 * (t * 0.5 + 2.0).sin(), 0.4 + 0.2 * (t * 0.3 + 4.0).sin(), 1.0];
            ctx.ClearRenderTargetView(rtv.as_ref().unwrap(), &c);
            let _ = sc.Present(1, DXGI_PRESENT(0));
            frames += 1;
        }
        if fullscreen {
            let _ = sc.SetFullscreenState(false, None);
        }
        println!("done frames={frames}");
    }
}
