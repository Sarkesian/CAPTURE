//! `capture-hook.dll`: CAPTURE's in-game overlay hook.
//!
//! The capture worker loads this DLL into a game (never into games with anti-cheat) after
//! creating the shared memory described in `capture_protocol::hook`. On load it:
//! 1. finds DXGI's swap-chain function table by creating a throwaway Direct3D 11 swap chain,
//! 2. replaces `Present`, `Present1` and `ResizeBuffers` in that table with its own functions,
//! 3. on every frame, draws CAPTURE's current notice into the game's back buffer just before
//!    the frame is shown, with its own isolated pipeline state (the game's state is untouched).
//!
//! This is how overlays reach games in exclusive fullscreen, where no ordinary window can be
//! shown over the game. When the worker sets `detach`, the original functions are restored,
//! frames still inside the hook are waited for, and the DLL unloads itself.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod detour;

use capture_protocol::hook::{self as proto, Header};
use windows::core::{w, Interface, BOOL, HRESULT, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D9::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Performance::QueryPerformanceCounter;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// Swap-chain function table slots (IUnknown 0-2, IDXGIObject 3-6, IDXGIDeviceSubObject 7,
// IDXGISwapChain 8-17, IDXGISwapChain1 18-28).
const SLOT_PRESENT: usize = 8;
const SLOT_RESIZE_BUFFERS: usize = 13;
const SLOT_PRESENT1: usize = 22;

type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT;
type Present1Fn = unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void) -> HRESULT;
type ResizeBuffersFn = unsafe extern "system" fn(*mut c_void, u32, u32, u32, DXGI_FORMAT, u32) -> HRESULT;
// IDirect3DDevice9 slots: Present 17; IDirect3DDevice9Ex: PresentEx 121.
const SLOT_D3D9_PRESENT: usize = 17;
const SLOT_D3D9_PRESENT_EX: usize = 121;
type Present9Fn = unsafe extern "system" fn(*mut c_void, *const RECT, *const RECT, HWND, *const c_void) -> HRESULT;
type PresentEx9Fn = unsafe extern "system" fn(*mut c_void, *const RECT, *const RECT, HWND, *const c_void, u32) -> HRESULT;
// IDirect3DSwapChain9::Present is slot 3.
const SLOT_SC9_PRESENT: usize = 3;
type PresentSc9Fn = unsafe extern "system" fn(*mut c_void, *const RECT, *const RECT, HWND, *const c_void, u32) -> HRESULT;

static MODULE: AtomicUsize = AtomicUsize::new(0);
static SHARED: AtomicUsize = AtomicUsize::new(0);
/// Every function we hooked: (function, table slot it came from, our function, trampoline).
static PATCHES: Mutex<Vec<(usize, usize, usize, usize)>> = Mutex::new(Vec::new());
static ORIG_PRESENT9: AtomicUsize = AtomicUsize::new(0);
static ORIG_PRESENT_EX9: AtomicUsize = AtomicUsize::new(0);
static RENDERER9: Mutex<Option<Renderer9>> = Mutex::new(None);
static ORIG_PRESENT: AtomicUsize = AtomicUsize::new(0);
static ORIG_PRESENT1: AtomicUsize = AtomicUsize::new(0);
static ORIG_RESIZE: AtomicUsize = AtomicUsize::new(0);
/// Calls currently inside one of our functions (unloading waits for 0).
static IN_FLIGHT: AtomicI32 = AtomicI32::new(0);
/// Set when a session ends: the render thread drops our graphics resources on its next frame.
static DROP_RENDERERS: AtomicBool = AtomicBool::new(false);
static ORIG_PRESENT_SC9: AtomicUsize = AtomicUsize::new(0);
static RENDERER: Mutex<Option<Renderer>> = Mutex::new(None);
static NOTICE: Mutex<NoticeState> = Mutex::new(NoticeState { seq: 0, shown_at: None, pixels: Vec::new(), w: 0, h: 0, uploaded: false });

struct NoticeState {
    seq: u32,
    shown_at: Option<Instant>,
    pixels: Vec<u8>,
    w: u32,
    h: u32,
    /// The renderer's texture holds this notice.
    uploaded: bool,
}

fn shared() -> Option<&'static Header> {
    let p = SHARED.load(Ordering::Acquire);
    (p != 0).then(|| unsafe { &*(p as *const Header) })
}

fn qpc() -> u64 {
    let mut c = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
    }
    c as u64
}

fn report(msg: &str) {
    if let Some(h) = shared() {
        let mut bytes = [0u8; 256];
        let n = msg.len().min(255);
        bytes[..n].copy_from_slice(&msg.as_bytes()[..n]);
        for (i, chunk) in bytes.chunks(4).enumerate() {
            h.message[i].store(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]), Ordering::Relaxed);
        }
    }
}

#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllMain(module: HMODULE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(module.0 as usize, Ordering::SeqCst);
        unsafe {
            let _ = DisableThreadLibraryCalls(module);
            // Never do real work under the loader lock.
            if let Ok(h) = CreateThread(None, 0, Some(main_thread), None, THREAD_CREATION_FLAGS(0), None) {
                let _ = CloseHandle(h);
            }
        }
    }
    BOOL(1)
}

unsafe extern "system" fn main_thread(_: *mut c_void) -> u32 {
    // The DLL stays loaded for the life of the game: unloading it while any other overlay (or
    // the game) may still call into a hooked function would crash the game. Between CAPTURE
    // sessions it is dormant (draws nothing) and waits for the next session's shared memory.
    let name = windows::core::HSTRING::from(proto::mapping_name(GetCurrentProcessId()));
    let mut installed = false;
    loop {
        let (map, view) = loop {
            if let Ok(map) = OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, &name) {
                let view = MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, proto::MAPPING_SIZE);
                if !view.Value.is_null() {
                    let h = &*(view.Value as *const Header);
                    // Ours if it is this protocol, live, and unclaimed (or claimed by us).
                    let me = (MODULE.load(Ordering::SeqCst) >> 12) as u32 | 1;
                    if h.magic.load(Ordering::Acquire) == proto::MAGIC
                        && h.version.load(Ordering::Acquire) == proto::VERSION
                        && h.detach.load(Ordering::Acquire) == 0
                        && h.claimed.compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire).map_or_else(|c| c == me, |_| true)
                    {
                        break (map, view);
                    }
                    let _ = UnmapViewOfFile(view);
                }
                let _ = CloseHandle(map);
            }
            std::thread::sleep(Duration::from_millis(500));
        };
        let header = &*(view.Value as *const Header);
        SHARED.store(view.Value as usize, Ordering::Release);
        if !installed {
            match install() {
                Ok(()) => installed = true,
                Err(e) => report(&format!("hook not installed: {e}")),
            }
        }
        header.hook_state.store(if installed { proto::state::HOOKED } else { proto::state::FAILED }, Ordering::Release);

        while header.detach.load(Ordering::Acquire) == 0 {
            std::thread::sleep(Duration::from_millis(100));
        }
        // Dormant again: stop drawing, let frames inside the hook finish with the shared memory,
        // and have the render thread release our graphics resources.
        header.hook_state.store(proto::state::DETACHED, Ordering::Release);
        SHARED.store(0, Ordering::Release);
        DROP_RENDERERS.store(true, Ordering::Release);
        let until = Instant::now() + Duration::from_secs(2);
        while IN_FLIGHT.load(Ordering::SeqCst) > 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        if IN_FLIGHT.load(Ordering::SeqCst) == 0 {
            let _ = UnmapViewOfFile(view);
        }
        let _ = CloseHandle(map);
        *NOTICE.lock().unwrap() = NoticeState { seq: 0, shown_at: None, pixels: Vec::new(), w: 0, h: 0, uploaded: false };
    }
}

/// Hook the function that `slot` of `vtable` points to (the function itself, not the table:
/// see `detour`), so it calls `ours`; `orig` receives the address that runs the original.
/// Each function is hooked once however many tables point to it, and each of our functions
/// serves one target (it has one `orig`).
unsafe fn patch(vtable: *mut usize, slot: usize, ours: usize, orig: &AtomicUsize) -> bool {
    let mut list = PATCHES.lock().unwrap();
    let target = std::ptr::read_volatile(vtable.add(slot));
    if list.iter().any(|&(t, _, _, _)| t == target) {
        return true;
    }
    if list.iter().any(|&(_, _, o, _)| o == ours) {
        report(&format!("a second implementation at {target:#x} is not hooked"));
        return false;
    }
    match detour::hook(target, ours) {
        Ok(tramp) => {
            orig.store(tramp, Ordering::SeqCst);
            list.push((target, slot, ours, tramp));
            true
        }
        Err(e) => {
            report(&format!("hook at {target:#x} failed: {e}"));
            false
        }
    }
}

unsafe extern "system" fn probe_wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, w, l)
}

/// Hook every graphics API we know. Succeeds when at least one is hooked; the game presents
/// through whichever it uses.
unsafe fn install() -> windows::core::Result<()> {
    let hinst = GetModuleHandleW(None)?;
    let wc = WNDCLASSW { lpfnWndProc: Some(probe_wndproc), hInstance: hinst.into(), lpszClassName: w!("CaptureHookProbe"), ..Default::default() };
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("CaptureHookProbe"), w!(""), WS_OVERLAPPEDWINDOW, 0, 0, 64, 64, None, None, Some(hinst.into()), None)?;
    let dxgi = install_dxgi(hwnd);
    let d3d9 = install_d3d9(hwnd);
    let dinput = install_dinput();
    let _ = DestroyWindow(hwnd);
    let list: Vec<String> = PATCHES.lock().unwrap().iter().map(|(t, s, _, o)| format!("{t:x}[{s}]={o:x}")).collect();
    report(&format!("dxgi {} d3d9 {} dinput {} hooks {}", if dxgi.is_ok() { "ok" } else { "no" }, match &d3d9 { Ok(()) => "ok".to_string(), Err(e) => e.message() }, if dinput.is_ok() { "ok" } else { "no" }, list.join(" ")));
    match (dxgi, d3d9) {
        (Err(a), Err(b)) => Err(windows::core::Error::new(E_FAIL, format!("DXGI: {a}; Direct3D 9: {b}"))),
        _ => Ok(()),
    }
}

/// Create a throwaway DXGI swap chain to find DXGI's function table, then hook it.
unsafe fn install_dxgi(hwnd: HWND) -> windows::core::Result<()> {
    let desc = DXGI_SWAP_CHAIN_DESC {
        BufferDesc: DXGI_MODE_DESC { Width: 64, Height: 64, Format: DXGI_FORMAT_R8G8B8A8_UNORM, ..Default::default() },
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 1,
        OutputWindow: hwnd,
        Windowed: true.into(),
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        Flags: 0,
    };
    let mut sc = None;
    let mut r = D3D11CreateDeviceAndSwapChain(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), None, D3D11_SDK_VERSION, Some(&desc), Some(&mut sc), None, None, None);
    if r.is_err() {
        r = D3D11CreateDeviceAndSwapChain(None, D3D_DRIVER_TYPE_WARP, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), None, D3D11_SDK_VERSION, Some(&desc), Some(&mut sc), None, None, None);
    }
    r?;
    let sc: IDXGISwapChain = sc.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
    let vtable = *(sc.as_raw() as *const *mut usize);
    patch(vtable, SLOT_RESIZE_BUFFERS, resize_buffers_hook as ResizeBuffersFn as usize, &ORIG_RESIZE);
    patch(vtable, SLOT_PRESENT1, present1_hook as Present1Fn as usize, &ORIG_PRESENT1);
    if !patch(vtable, SLOT_PRESENT, present_hook as PresentFn as usize, &ORIG_PRESENT) {
        return Err(windows::core::Error::new(E_ACCESSDENIED, "the swap chain function table could not be changed"));
    }
    Ok(())
}

/// Create throwaway Direct3D 9 devices of every kind (software, hardware, pure, mixed; plain
/// and Ex) to find their function tables, then hook `Present`, `PresentEx` and the swap chain's
/// `Present`. Direct3D 9 uses a different function table per device kind, so hooking only one
/// misses games that create another kind (a pure hardware device, typically).
unsafe fn install_d3d9(hwnd: HWND) -> windows::core::Result<()> {
    let mut pp = D3DPRESENT_PARAMETERS {
        BackBufferWidth: 64,
        BackBufferHeight: 64,
        BackBufferFormat: D3DFMT_UNKNOWN,
        BackBufferCount: 1,
        SwapEffect: D3DSWAPEFFECT_DISCARD,
        hDeviceWindow: hwnd,
        Windowed: true.into(),
        ..Default::default()
    };
    let kinds = [
        D3DCREATE_SOFTWARE_VERTEXPROCESSING,
        D3DCREATE_HARDWARE_VERTEXPROCESSING,
        D3DCREATE_HARDWARE_VERTEXPROCESSING | D3DCREATE_PUREDEVICE,
        D3DCREATE_MIXED_VERTEXPROCESSING,
    ];
    let mut hooked = false;
    let mut hook_device = |dev: &IDirect3DDevice9, ex: bool| {
        let vtable = *(dev.as_raw() as *const *mut usize);
        hooked |= patch(vtable, SLOT_D3D9_PRESENT, present9_hook as Present9Fn as usize, &ORIG_PRESENT9);
        if ex {
            hooked |= patch(vtable, SLOT_D3D9_PRESENT_EX, present_ex9_hook as PresentEx9Fn as usize, &ORIG_PRESENT_EX9);
        }
        if let Ok(sc) = dev.GetSwapChain(0) {
            let t = *(sc.as_raw() as *const *mut usize);
            hooked |= patch(t, SLOT_SC9_PRESENT, present_sc9_hook as PresentSc9Fn as usize, &ORIG_PRESENT_SC9);
        }
    };
    if let Some(d3d) = Direct3DCreate9(D3D_SDK_VERSION) {
        for k in kinds {
            let mut dev = None;
            if d3d.CreateDevice(D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, hwnd, (k | D3DCREATE_DISABLE_DRIVER_MANAGEMENT) as u32, &mut pp, &mut dev).is_ok() {
                if let Some(dev) = dev {
                    hook_device(&dev, false);
                }
            }
        }
    }
    if let Ok(ex) = Direct3DCreate9Ex(D3D_SDK_VERSION) {
        for k in kinds {
            let mut dev = None;
            if ex.CreateDeviceEx(D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, hwnd, (k | D3DCREATE_DISABLE_DRIVER_MANAGEMENT) as u32, &mut pp, std::ptr::null_mut(), &mut dev).is_ok() {
                if let Some(dev) = dev {
                    if let Ok(d) = dev.cast::<IDirect3DDevice9>() {
                        hook_device(&d, true);
                    }
                }
            }
        }
    }
    if hooked {
        Ok(())
    } else {
        Err(windows::core::Error::new(E_FAIL, "no Direct3D 9 device could be created"))
    }
}

unsafe extern "system" fn present_hook(this: *mut c_void, sync: u32, flags: u32) -> HRESULT {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    before_present(this, flags);
    let orig: PresentFn = std::mem::transmute(ORIG_PRESENT.load(Ordering::SeqCst));
    let r = orig(this, sync, flags);
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    r
}

unsafe extern "system" fn present1_hook(this: *mut c_void, sync: u32, flags: u32, params: *const c_void) -> HRESULT {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    before_present(this, flags);
    let orig: Present1Fn = std::mem::transmute(ORIG_PRESENT1.load(Ordering::SeqCst));
    let r = orig(this, sync, flags, params);
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    r
}

unsafe extern "system" fn resize_buffers_hook(this: *mut c_void, count: u32, w: u32, h: u32, fmt: DXGI_FORMAT, flags: u32) -> HRESULT {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    // The game cannot resize while anyone (us) still holds a view of its back buffer.
    if let Ok(mut r) = RENDERER.lock() {
        if let Some(r) = r.as_mut() {
            r.rtv = None;
            r.backbuffer = 0;
        }
    }
    let orig: ResizeBuffersFn = std::mem::transmute(ORIG_RESIZE.load(Ordering::SeqCst));
    let res = orig(this, count, w, h, fmt, flags);
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    res
}

unsafe fn before_present(this: *mut c_void, flags: u32) {
    if flags & DXGI_PRESENT_TEST.0 != 0 {
        return;
    }
    // Never let a problem in the overlay take the game down.
    let _ = std::panic::catch_unwind(|| {
        if let Err(e) = on_present(this) {
            report(&format!("draw failed: {e}"));
        }
    });
}

unsafe extern "system" fn present9_hook(this: *mut c_void, src: *const RECT, dst: *const RECT, wnd: HWND, dirty: *const c_void) -> HRESULT {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let _ = std::panic::catch_unwind(|| {
        if let Err(e) = on_present9(this) {
            report(&format!("draw failed (Direct3D 9): {e}"));
        }
    });
    let orig: Present9Fn = std::mem::transmute(ORIG_PRESENT9.load(Ordering::SeqCst));
    let r = orig(this, src, dst, wnd, dirty);
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    r
}

unsafe extern "system" fn present_ex9_hook(this: *mut c_void, src: *const RECT, dst: *const RECT, wnd: HWND, dirty: *const c_void, flags: u32) -> HRESULT {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let _ = std::panic::catch_unwind(|| {
        if let Err(e) = on_present9(this) {
            report(&format!("draw failed (Direct3D 9Ex): {e}"));
        }
    });
    let orig: PresentEx9Fn = std::mem::transmute(ORIG_PRESENT_EX9.load(Ordering::SeqCst));
    let r = orig(this, src, dst, wnd, dirty, flags);
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    r
}

unsafe extern "system" fn present_sc9_hook(this: *mut c_void, src: *const RECT, dst: *const RECT, wnd: HWND, dirty: *const c_void, flags: u32) -> HRESULT {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let _ = std::panic::catch_unwind(|| {
        if let Some(sc) = IDirect3DSwapChain9::from_raw_borrowed(&this) {
            if let Ok(dev) = sc.GetDevice() {
                if let Err(e) = on_present9(dev.as_raw()) {
                    report(&format!("draw failed (Direct3D 9 swap chain): {e}"));
                }
            }
        }
    });
    let orig: PresentSc9Fn = std::mem::transmute(ORIG_PRESENT_SC9.load(Ordering::SeqCst));
    let r = orig(this, src, dst, wnd, dirty, flags);
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    r
}

// ---- input while the overlay is open in the game -------------------------------------------

/// The game's window and its original window procedure, once subclassed (kept for the life of
/// the game; it passes everything through while the overlay is closed).
static GAME_HWND: AtomicUsize = AtomicUsize::new(0);
static ORIG_WNDPROC: AtomicUsize = AtomicUsize::new(0);
/// Mouse position on the game's back buffer, for drawing our pointer.
static CURSOR: Mutex<Option<(f32, f32)>> = Mutex::new(None);

/// Take over the game window's input (once).
unsafe fn subclass_game_window(hwnd: HWND) {
    if hwnd.is_invalid() || GAME_HWND.load(Ordering::SeqCst) != 0 {
        return;
    }
    GAME_HWND.store(hwnd.0 as usize, Ordering::SeqCst);
    let prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, game_wndproc as WndProcFn as usize as _);
    ORIG_WNDPROC.store(prev as usize, Ordering::SeqCst);
}

type WndProcFn = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

fn overlay_is_open() -> Option<&'static Header> {
    shared().filter(|h| h.overlay_open.load(Ordering::Acquire) != 0)
}

/// Game-window client position -> overlay-window position, if over the overlay.
unsafe fn to_overlay(hwnd: HWND, cx: i32, cy: i32) -> Option<(i32, i32, (f32, f32))> {
    let (rect, (bw, bh), (ow, oh)) = (*OVERLAY_RECT.lock().ok()?)?;
    let mut rc = RECT::default();
    GetClientRect(hwnd, &mut rc).ok()?;
    let (cw, ch) = ((rc.right - rc.left).max(1) as f32, (rc.bottom - rc.top).max(1) as f32);
    let (bx, by) = (cx as f32 * bw / cw, cy as f32 * bh / ch);
    let [l, t, r, b] = rect;
    let (x, y) = ((bx - l) / (r - l) * ow as f32, (by - t) / (b - t) * oh as f32);
    Some((x.round() as i32, y.round() as i32, (bx, by)))
}

fn make_lparam(x: i32, y: i32) -> LPARAM {
    LPARAM(((y as u16 as u32) << 16 | (x as u16 as u32)) as isize)
}

unsafe extern "system" fn game_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let orig = ORIG_WNDPROC.load(Ordering::SeqCst);
    if let Some(h) = overlay_is_open() {
        let target = HWND(h.overlay_input_hwnd.load(Ordering::Relaxed) as *mut c_void);
        match msg {
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_RBUTTONDBLCLK | WM_MBUTTONDOWN | WM_MBUTTONUP => {
                let (cx, cy) = ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32);
                if let Some((x, y, at)) = to_overlay(hwnd, cx, cy) {
                    *CURSOR.lock().unwrap() = Some(at);
                    let _ = PostMessageW(Some(target), msg, wp, make_lparam(x, y));
                }
                return LRESULT(0);
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                // Wheel positions are in screen coordinates: the overlay window's own.
                let mut p = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                let _ = ScreenToClient(hwnd, &mut p);
                if let Some((x, y, _)) = to_overlay(hwnd, p.x, p.y) {
                    let mut wr = RECT::default();
                    let _ = GetWindowRect(target, &mut wr);
                    let _ = PostMessageW(Some(target), msg, wp, make_lparam(wr.left + x, wr.top + y));
                }
                return LRESULT(0);
            }
            WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR | WM_DEADCHAR => {
                let _ = PostMessageW(Some(target), msg, wp, lp);
                return LRESULT(0);
            }
            // Raw input: the game must not see it while the overlay has the player's attention.
            WM_INPUT => return DefWindowProcW(hwnd, msg, wp, lp),
            WM_SETCURSOR => {
                let _ = SetCursor(None); // our own pointer is drawn in the overlay
                return LRESULT(1);
            }
            _ => {}
        }
    }
    if orig == 0 {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    CallWindowProcW(Some(std::mem::transmute::<usize, WndProcFn>(orig)), hwnd, msg, wp, lp)
}

/// A mouse pointer (white arrow with a dark outline), BGRA, 20 x 30.
fn pointer_pixels() -> (Vec<u8>, u32, u32) {
    const ROWS: [&str; 22] = [
        "X", "XX", "X.X", "X..X", "X...X", "X....X", "X.....X", "X......X", "X.......X", "X........X", "X.........X", "X..........X", "X......XXXXX", "X...X..X", "X..XX..X", "X.X  X..X", "XX   X..X", "X     X..X", "      X..X", "       X..X", "       X..X", "        XX",
    ];
    let (w, h) = (13u32, ROWS.len() as u32);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for (y, row) in ROWS.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            let i = (y * w as usize + x) * 4;
            match c {
                'X' => px[i..i + 4].copy_from_slice(&[20, 20, 20, 255]),
                '.' => px[i..i + 4].copy_from_slice(&[255, 255, 255, 255]),
                _ => {}
            }
        }
    }
    (px, w, h)
}

// ---- DirectInput (games that read the mouse/keyboard directly, e.g. Skyrim) ---------------
//
// Such games get no window messages for the mouse, so while the overlay is open the hook reads
// what they read: mouse movement and buttons move our pointer and click the overlay, and the
// game is handed "nothing happened" so it stays still.

// IDirectInputDevice8 slots: GetCapabilities 3, GetDeviceState 9, GetDeviceData 10.
const SLOT_DI_STATE: usize = 9;
const SLOT_DI_DATA: usize = 10;
type DiStateFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT;
type DiDataFn = unsafe extern "system" fn(*mut c_void, u32, *mut u8, *mut u32, u32) -> HRESULT;
static ORIG_DI_STATE_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_DI_DATA_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_DI_STATE_A: AtomicUsize = AtomicUsize::new(0);
static ORIG_DI_DATA_A: AtomicUsize = AtomicUsize::new(0);
/// Mouse buttons held, as last seen through DirectInput (bit 0 left, 1 right, 2 middle).
static DI_BUTTONS: AtomicUsize = AtomicUsize::new(0);
/// Device kind per DirectInput device (DI8DEVTYPE_*: 0x12 mouse, 0x13 keyboard).
static DI_KINDS: Mutex<Vec<(usize, u8)>> = Mutex::new(Vec::new());

/// Create throwaway DirectInput mouse devices (wide and ANSI) and hook how devices are read.
unsafe fn install_dinput() -> windows::core::Result<()> {
    use windows::Win32::Devices::HumanInterfaceDevice::*;
    let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
    let mut hooked = false;
    let mut p: *mut c_void = std::ptr::null_mut();
    if DirectInput8Create(hinst, 0x0800, &IDirectInput8W::IID, &mut p, None).is_ok() && !p.is_null() {
        let di = IDirectInput8W::from_raw(p);
        let mut dev = None;
        if di.CreateDevice(&GUID_SysMouse, &mut dev, None).is_ok() {
            if let Some(dev) = dev {
                let t = *(dev.as_raw() as *const *mut usize);
                hooked |= patch(t, SLOT_DI_STATE, di_state_w as DiStateFn as usize, &ORIG_DI_STATE_W);
                hooked |= patch(t, SLOT_DI_DATA, di_data_w as DiDataFn as usize, &ORIG_DI_DATA_W);
            }
        }
    }
    let mut p: *mut c_void = std::ptr::null_mut();
    if DirectInput8Create(hinst, 0x0800, &IDirectInput8A::IID, &mut p, None).is_ok() && !p.is_null() {
        let di = IDirectInput8A::from_raw(p);
        let mut dev = None;
        if di.CreateDevice(&GUID_SysMouse, &mut dev, None).is_ok() {
            if let Some(dev) = dev {
                let t = *(dev.as_raw() as *const *mut usize);
                hooked |= patch(t, SLOT_DI_STATE, di_state_a as DiStateFn as usize, &ORIG_DI_STATE_A);
                hooked |= patch(t, SLOT_DI_DATA, di_data_a as DiDataFn as usize, &ORIG_DI_DATA_A);
            }
        }
    }
    if hooked {
        Ok(())
    } else {
        Err(windows::core::Error::new(E_FAIL, "DirectInput not available"))
    }
}

/// What kind of device `this` is (cached; asked once through GetCapabilities).
unsafe fn di_kind(this: *mut c_void) -> u8 {
    if let Some(&(_, k)) = DI_KINDS.lock().unwrap().iter().find(|(d, _)| *d == this as usize) {
        return k;
    }
    // DIDEVCAPS: dwSize, dwFlags, dwDevType, ... (44 bytes); GetCapabilities is slot 3.
    let mut caps = [0u32; 11];
    caps[0] = 44;
    let get_caps: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT = std::mem::transmute(*(*(this as *const *const usize)).add(3));
    let kind = if get_caps(this, caps.as_mut_ptr()).is_ok() { (caps[2] & 0xff) as u8 } else { 0 };
    DI_KINDS.lock().unwrap().push((this as usize, kind));
    kind
}

unsafe extern "system" fn di_state_w(this: *mut c_void, cb: u32, data: *mut c_void) -> HRESULT {
    di_state(this, cb, data, &ORIG_DI_STATE_W)
}
unsafe extern "system" fn di_state_a(this: *mut c_void, cb: u32, data: *mut c_void) -> HRESULT {
    di_state(this, cb, data, &ORIG_DI_STATE_A)
}
unsafe extern "system" fn di_data_w(this: *mut c_void, cb: u32, data: *mut u8, count: *mut u32, flags: u32) -> HRESULT {
    di_data(this, cb, data, count, flags, &ORIG_DI_DATA_W)
}
unsafe extern "system" fn di_data_a(this: *mut c_void, cb: u32, data: *mut u8, count: *mut u32, flags: u32) -> HRESULT {
    di_data(this, cb, data, count, flags, &ORIG_DI_DATA_A)
}

/// Immediate reads (GetDeviceState): mouse deltas and buttons drive our pointer; the game
/// gets an all-zero state.
unsafe fn di_state(this: *mut c_void, cb: u32, data: *mut c_void, orig: &AtomicUsize) -> HRESULT {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let f: DiStateFn = std::mem::transmute(orig.load(Ordering::SeqCst));
    let r = f(this, cb, data);
    if r.is_ok() && !data.is_null() && overlay_is_open().is_some() {
        let _ = std::panic::catch_unwind(|| {
            let kind = di_kind(this);
            if kind == 0x12 && cb >= 16 {
                // DIMOUSESTATE(2): lX, lY, lZ, rgbButtons[4 or 8]
                let s = data as *const i32;
                let b = (data as *const u8).add(12);
                let buttons = (*b & 0x80 != 0) as usize | ((*b.add(1) & 0x80 != 0) as usize) << 1 | ((*b.add(2) & 0x80 != 0) as usize) << 2;
                feed_mouse(*s, *s.add(1), *s.add(2), Some(buttons));
            }
            if kind == 0x12 || kind == 0x13 {
                std::ptr::write_bytes(data as *mut u8, 0, cb as usize);
            }
        });
    }
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    r
}

/// Buffered reads (GetDeviceData): the same, event by event; the game gets no events.
unsafe fn di_data(this: *mut c_void, cb: u32, data: *mut u8, count: *mut u32, flags: u32, orig: &AtomicUsize) -> HRESULT {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let f: DiDataFn = std::mem::transmute(orig.load(Ordering::SeqCst));
    let r = f(this, cb, data, count, flags);
    if r.is_ok() && !count.is_null() && overlay_is_open().is_some() {
        let _ = std::panic::catch_unwind(|| {
            let kind = di_kind(this);
            if kind == 0x12 && !data.is_null() && cb >= 8 {
                let (mut dx, mut dy, mut dz) = (0i32, 0i32, 0i32);
                let mut buttons = DI_BUTTONS.load(Ordering::Relaxed);
                for i in 0..*count as usize {
                    let e = data.add(i * cb as usize) as *const u32;
                    let (ofs, val) = (*e, *e.add(1));
                    match ofs {
                        0 => dx += val as i32,
                        4 => dy += val as i32,
                        8 => dz += val as i32,
                        12..=14 => {
                            let bit = 1usize << (ofs - 12);
                            buttons = if val & 0x80 != 0 { buttons | bit } else { buttons & !bit };
                        }
                        _ => {}
                    }
                }
                feed_mouse(dx, dy, dz, Some(buttons));
            }
            if kind == 0x12 || kind == 0x13 {
                *count = 0;
            }
        });
    }
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    r
}

/// Move our pointer by a relative mouse movement and send moves, clicks and wheel turns to the
/// overlay window.
unsafe fn feed_mouse(dx: i32, dy: i32, dz: i32, buttons: Option<usize>) {
    let Some(h) = overlay_is_open() else { return };
    let Some((rect, (bw, bh), _)) = *OVERLAY_RECT.lock().unwrap() else { return };
    let target = HWND(h.overlay_input_hwnd.load(Ordering::Relaxed) as *mut c_void);
    let mut c = CURSOR.lock().unwrap();
    let (x, y) = c.unwrap_or(((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0));
    let (x, y) = ((x + dx as f32).clamp(0.0, bw - 1.0), (y + dy as f32).clamp(0.0, bh - 1.0));
    *c = Some((x, y));
    drop(c);
    let held = DI_BUTTONS.load(Ordering::Relaxed);
    let now = buttons.unwrap_or(held);
    let mk = |b: usize| (b & 1) | (b & 2) | ((b & 4) << 2); // MK_LBUTTON 1, MK_RBUTTON 2, MK_MBUTTON 0x10
    if dx != 0 || dy != 0 {
        post_mouse_bb(target, WM_MOUSEMOVE, mk(now), x, y);
    }
    for (bit, down, up) in [(1usize, WM_LBUTTONDOWN, WM_LBUTTONUP), (2, WM_RBUTTONDOWN, WM_RBUTTONUP), (4, WM_MBUTTONDOWN, WM_MBUTTONUP)] {
        if now & bit != held & bit {
            post_mouse_bb(target, if now & bit != 0 { down } else { up }, mk(now), x, y);
        }
    }
    DI_BUTTONS.store(now, Ordering::Relaxed);
    if dz != 0 {
        if let Some((ox, oy)) = bb_to_overlay(x, y) {
            let mut wr = RECT::default();
            let _ = GetWindowRect(target, &mut wr);
            let wp = WPARAM(((dz as i16 as u16 as usize) << 16) | mk(now));
            let _ = PostMessageW(Some(target), WM_MOUSEWHEEL, wp, make_lparam(wr.left + ox, wr.top + oy));
        }
    }
}

/// Back-buffer position -> overlay-window position.
fn bb_to_overlay(bx: f32, by: f32) -> Option<(i32, i32)> {
    let (rect, _, (ow, oh)) = (*OVERLAY_RECT.lock().ok()?)?;
    let [l, t, r, b] = rect;
    Some((((bx - l) / (r - l) * ow as f32).round() as i32, ((by - t) / (b - t) * oh as f32).round() as i32))
}

unsafe fn post_mouse_bb(target: HWND, msg: u32, mk: usize, bx: f32, by: f32) {
    if let Some((x, y)) = bb_to_overlay(bx, by) {
        let _ = PostMessageW(Some(target), msg, WPARAM(mk), make_lparam(x, y));
    }
}

/// What a frame shows from CAPTURE, bottom to top.
#[derive(Clone, Copy, PartialEq)]
enum Layer {
    /// The game darkened behind the overlay.
    Dim,
    /// CAPTURE's overlay window.
    Overlay,
    Notice,
    /// Our mouse pointer over the overlay.
    Pointer,
}

/// Count the frame and report it to the worker.
fn count_frame(h: &Header, api: u32) {
    h.presents.fetch_add(1, Ordering::Relaxed);
    h.last_present_qpc.store(qpc(), Ordering::Relaxed);
    h.api.store(api, Ordering::Relaxed);
    if h.hook_state.load(Ordering::Relaxed) != proto::state::PRESENTING {
        h.hook_state.store(proto::state::PRESENTING, Ordering::Release);
    }
}

/// Pick up a newly published notice and work out how visible the current one is.
unsafe fn notice_alpha(h: &Header, n: &mut NoticeState) -> Option<f32> {
    let seq = h.notice_seq.load(Ordering::Acquire);
    if seq != n.seq && seq % 2 == 0 {
        let (w, hh) = (h.notice_w.load(Ordering::Relaxed).min(proto::NOTICE_MAX_W), h.notice_h.load(Ordering::Relaxed).min(proto::NOTICE_MAX_H));
        let len = (w * hh * 4) as usize;
        let src = std::slice::from_raw_parts((h as *const Header as *const u8).add(proto::PIXELS_OFFSET), len);
        n.pixels.clear();
        n.pixels.extend_from_slice(src);
        if h.notice_seq.load(Ordering::Acquire) == seq && w > 0 && hh > 0 {
            n.seq = seq;
            n.w = w;
            n.h = hh;
            n.shown_at = Some(Instant::now());
            n.uploaded = false;
        }
    }
    let shown_at = n.shown_at?;
    let (show, fade) = (h.notice_show_ms.load(Ordering::Relaxed) as f32, h.notice_fade_ms.load(Ordering::Relaxed).max(1) as f32);
    let t = shown_at.elapsed().as_secs_f32() * 1000.0;
    if t >= show + fade {
        n.shown_at = None;
        return None;
    }
    Some(if t <= show { 1.0 } else { 1.0 - (t - show) / fade })
}

/// The overlay window's latest picture, while the overlay is open in the game.
struct OverlayState {
    seq: u32,
    pixels: Vec<u8>,
    w: u32,
    h: u32,
    uploaded: bool,
}

static OVERLAY: Mutex<OverlayState> = Mutex::new(OverlayState { seq: 0, pixels: Vec::new(), w: 0, h: 0, uploaded: false });

/// Pick up a new overlay picture; `true` while the overlay is open and has a picture.
unsafe fn overlay_open(h: &Header, o: &mut OverlayState) -> bool {
    if h.overlay_open.load(Ordering::Acquire) == 0 {
        return false;
    }
    let seq = h.overlay_seq.load(Ordering::Acquire);
    if seq != o.seq && seq % 2 == 0 {
        let (w, hh) = (h.overlay_w.load(Ordering::Relaxed).min(proto::OVERLAY_MAX_W), h.overlay_h.load(Ordering::Relaxed).min(proto::OVERLAY_MAX_H));
        let len = (w * hh * 4) as usize;
        let src = std::slice::from_raw_parts((h as *const Header as *const u8).add(proto::OVERLAY_OFFSET), len);
        o.pixels.clear();
        o.pixels.extend_from_slice(src);
        if h.overlay_seq.load(Ordering::Acquire) == seq && w > 0 && hh > 0 {
            o.seq = seq;
            o.w = w;
            o.h = hh;
            o.uploaded = false;
        }
    }
    o.w > 0 && o.h > 0
}

/// Where each layer goes on a `bw` x `bh` back buffer: (layer, [left, top, right, bottom] in
/// pixels, opacity).
fn layout(bw: f32, bh: f32, notice: Option<(f32, u32, u32, f32)>, overlay: Option<(u32, u32)>, pointer: Option<(f32, f32)>) -> Vec<(Layer, [f32; 4], f32)> {
    let mut out = Vec::new();
    if let Some((w, h)) = overlay {
        out.push((Layer::Dim, [0.0, 0.0, bw, bh], 0.55));
        // The overlay at its real size, smaller only if the game's frame is smaller.
        let k = (bw * 0.94 / w as f32).min(bh * 0.94 / h as f32).min(1.0);
        let (ow, oh) = (w as f32 * k, h as f32 * k);
        let (x, y) = (((bw - ow) / 2.0).round(), ((bh - oh) / 2.0).round());
        out.push((Layer::Overlay, [x, y, x + ow, y + oh], 1.0));
    }
    if let (Some(_), Some((x, y))) = (overlay, pointer) {
        let k = (bh / 1080.0).clamp(1.0, 3.0) * 1.3;
        out.push((Layer::Pointer, [x, y, x + 13.0 * k, y + 22.0 * k], 1.0));
    }
    if let Some((alpha, w, h, scale_pct)) = notice {
        // Size relative to a 1080-line frame, so it looks the same at any resolution.
        let k = (bh / 1080.0).clamp(0.5, 4.0);
        let (tw, th) = (w as f32 * 100.0 / scale_pct * k, h as f32 * 100.0 / scale_pct * k);
        let margin = 24.0 * k;
        let (x0, y0) = (bw - margin - tw, margin);
        out.push((Layer::Notice, [x0, y0, x0 + tw, y0 + th], alpha));
    }
    out
}

/// Where the overlay is drawn on the current frame (for mapping the player's clicks to it).
static OVERLAY_RECT: Mutex<Option<([f32; 4], (f32, f32), (u32, u32))>> = Mutex::new(None);

fn drawn(h: &Header, seq: u32, w: u32, hh: u32, fmt: u32) {
    h.backbuffer_w.store(w, Ordering::Relaxed);
    h.backbuffer_h.store(hh, Ordering::Relaxed);
    h.backbuffer_format.store(fmt, Ordering::Relaxed);
    h.drawn_seq.store(seq, Ordering::Relaxed);
    h.drawn_frames.fetch_add(1, Ordering::Relaxed);
}

/// Release our graphics resources on the render thread once a session has ended.
fn drop_if_dormant() {
    if DROP_RENDERERS.swap(false, Ordering::AcqRel) {
        if let Ok(mut r) = RENDERER.lock() {
            *r = None;
        }
        if let Ok(mut r) = RENDERER9.lock() {
            *r = None;
        }
        if let Ok(mut o) = OVERLAY.lock() {
            *o = OverlayState { seq: 0, pixels: Vec::new(), w: 0, h: 0, uploaded: false };
        }
    }
}

unsafe fn on_present(this: *mut c_void) -> windows::core::Result<()> {
    drop_if_dormant();
    let Some(h) = shared() else { return Ok(()) };
    let Some(sc) = IDXGISwapChain::from_raw_borrowed(&this) else { return Ok(()) };
    let Ok(device) = sc.GetDevice::<ID3D11Device>() else {
        count_frame(h, proto::api::OTHER_DXGI);
        return Ok(());
    };
    count_frame(h, proto::api::D3D11);
    // Exclusive fullscreen as DXGI reports it (cheap; tells the worker the overlay must be
    // shown inside the game).
    let mut full = windows::core::BOOL(0);
    let _ = sc.GetFullscreenState(Some(&mut full), None);
    h.exclusive.store(full.as_bool() as u32, Ordering::Relaxed);
    let mut n = NOTICE.lock().unwrap();
    let mut o = OVERLAY.lock().unwrap();
    let alpha = notice_alpha(h, &mut n);
    let open = overlay_open(h, &mut o);
    if open && GAME_HWND.load(Ordering::Relaxed) == 0 {
        if let Ok(d) = sc.GetDesc() {
            subclass_game_window(d.OutputWindow);
        }
    }
    if !open {
        *CURSOR.lock().unwrap() = None;
    }
    if alpha.is_none() && !open {
        *OVERLAY_RECT.lock().unwrap() = None;
        return Ok(());
    }
    let mut guard = RENDERER.lock().unwrap();
    if guard.as_ref().is_none_or(|r| r.device.as_raw() != device.as_raw()) {
        *guard = None;
        *guard = Some(Renderer::new(&device)?);
        n.uploaded = false;
        o.uploaded = false;
    }
    let r = guard.as_mut().unwrap();
    if alpha.is_some() && !n.uploaded {
        r.set_notice(&n.pixels, n.w, n.h)?;
        n.uploaded = true;
    }
    if open && !o.uploaded {
        r.set_overlay(&o.pixels, o.w, o.h)?;
        o.uploaded = true;
    }
    let bb: ID3D11Texture2D = sc.GetBuffer(0)?;
    let mut d = D3D11_TEXTURE2D_DESC::default();
    bb.GetDesc(&mut d);
    let scale_pct = h.notice_scale_pct.load(Ordering::Relaxed).max(1) as f32;
    let pointer = if open { *CURSOR.lock().unwrap() } else { None };
    let layers = layout(d.Width as f32, d.Height as f32, alpha.map(|a| (a, n.w, n.h, scale_pct)), open.then_some((o.w, o.h)), pointer);
    remember_overlay(&layers, d.Width, d.Height, o.w, o.h);
    r.draw(&bb, &d, &layers)?;
    drawn(h, n.seq, d.Width, d.Height, d.Format.0 as u32);
    Ok(())
}

fn remember_overlay(layers: &[(Layer, [f32; 4], f32)], bw: u32, bh: u32, ow: u32, oh: u32) {
    *OVERLAY_RECT.lock().unwrap() = layers.iter().find(|l| l.0 == Layer::Overlay).map(|l| (l.1, (bw as f32, bh as f32), (ow, oh)));
}

unsafe fn on_present9(this: *mut c_void) -> windows::core::Result<()> {
    drop_if_dormant();
    let Some(h) = shared() else { return Ok(()) };
    let Some(dev) = IDirect3DDevice9::from_raw_borrowed(&this) else { return Ok(()) };
    count_frame(h, proto::api::D3D9);
    if let Ok(sc) = dev.GetSwapChain(0) {
        let mut pp = D3DPRESENT_PARAMETERS::default();
        if sc.GetPresentParameters(&mut pp).is_ok() {
            h.exclusive.store((!pp.Windowed.as_bool()) as u32, Ordering::Relaxed);
        }
    }
    let mut n = NOTICE.lock().unwrap();
    let mut o = OVERLAY.lock().unwrap();
    let alpha = notice_alpha(h, &mut n);
    let open = overlay_open(h, &mut o);
    if open && GAME_HWND.load(Ordering::Relaxed) == 0 {
        let mut cp = D3DDEVICE_CREATION_PARAMETERS::default();
        if dev.GetCreationParameters(&mut cp).is_ok() {
            subclass_game_window(cp.hFocusWindow);
        }
    }
    if !open {
        *CURSOR.lock().unwrap() = None;
    }
    if alpha.is_none() && !open {
        *OVERLAY_RECT.lock().unwrap() = None;
        return Ok(());
    }
    let mut guard = RENDERER9.lock().unwrap();
    if guard.as_ref().is_none_or(|r| r.device != this as usize) {
        *guard = Some(Renderer9 { device: this as usize, notice: None, overlay: None, dim: None, pointer: None });
        n.uploaded = false;
        o.uploaded = false;
    }
    let r = guard.as_mut().unwrap();
    if alpha.is_some() && !n.uploaded {
        r.notice = Some(Renderer9::texture(dev, &n.pixels, n.w, n.h, None)?);
        n.uploaded = true;
    }
    if open && !o.uploaded {
        let reuse = r.overlay.take().filter(|t| t.1 == (o.w, o.h));
        r.overlay = Some(Renderer9::texture(dev, &o.pixels, o.w, o.h, reuse)?);
        o.uploaded = true;
    }
    let bb = dev.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO)?;
    let mut d = D3DSURFACE_DESC::default();
    bb.GetDesc(&mut d)?;
    let scale_pct = h.notice_scale_pct.load(Ordering::Relaxed).max(1) as f32;
    let pointer = if open { *CURSOR.lock().unwrap() } else { None };
    let layers = layout(d.Width as f32, d.Height as f32, alpha.map(|a| (a, n.w, n.h, scale_pct)), open.then_some((o.w, o.h)), pointer);
    remember_overlay(&layers, d.Width, d.Height, o.w, o.h);
    r.draw(dev, &bb, &d, &layers)?;
    drawn(h, n.seq, d.Width, d.Height, d.Format.0 as u32);
    Ok(())
}

/// Draws CAPTURE's layers with a Direct3D 9 device (fixed function; the game's state is
/// captured before and applied again after).
struct Renderer9 {
    device: usize,
    notice: Option<(IDirect3DTexture9, (u32, u32))>,
    overlay: Option<(IDirect3DTexture9, (u32, u32))>,
    dim: Option<(IDirect3DTexture9, (u32, u32))>,
    pointer: Option<(IDirect3DTexture9, (u32, u32))>,
}

unsafe impl Send for Renderer9 {}

#[repr(C)]
struct Vertex9 {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    color: u32,
    u: f32,
    v: f32,
}

impl Renderer9 {
    /// A texture holding `pixels` (BGRA), reusing `reuse` when it has the right size.
    unsafe fn texture(dev: &IDirect3DDevice9, pixels: &[u8], w: u32, h: u32, reuse: Option<(IDirect3DTexture9, (u32, u32))>) -> windows::core::Result<(IDirect3DTexture9, (u32, u32))> {
        let tex = match reuse {
            Some((t, _)) => t,
            None => {
                let mut tex = None;
                dev.CreateTexture(w, h, 1, 0, D3DFMT_A8R8G8B8, D3DPOOL_MANAGED, &mut tex, std::ptr::null_mut())?;
                tex.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?
            }
        };
        let mut lr = D3DLOCKED_RECT::default();
        tex.LockRect(0, &mut lr, std::ptr::null(), 0)?;
        for y in 0..h as usize {
            std::ptr::copy_nonoverlapping(pixels.as_ptr().add(y * w as usize * 4), (lr.pBits as *mut u8).add(y * lr.Pitch as usize), w as usize * 4);
        }
        tex.UnlockRect(0)?;
        Ok((tex, (w, h)))
    }

    unsafe fn draw(&mut self, dev: &IDirect3DDevice9, bb: &IDirect3DSurface9, d: &D3DSURFACE_DESC, layers: &[(Layer, [f32; 4], f32)]) -> windows::core::Result<()> {
        if self.dim.is_none() {
            self.dim = Some(Self::texture(dev, &[0, 0, 0, 255], 1, 1, None)?);
        }
        if self.pointer.is_none() {
            let (px, w, h) = pointer_pixels();
            self.pointer = Some(Self::texture(dev, &px, w, h, None)?);
        }
        // Everything we change is captured first and applied again after, so the game's next
        // frame starts from exactly the state it left.
        let sb = dev.CreateStateBlock(D3DSBT_ALL)?;
        sb.Capture()?;
        let old_rt = dev.GetRenderTarget(0).ok();
        let _ = dev.SetRenderTarget(0, bb);
        let _ = dev.SetViewport(&D3DVIEWPORT9 { X: 0, Y: 0, Width: d.Width, Height: d.Height, MinZ: 0.0, MaxZ: 1.0 });
        for (state, value) in [
            (D3DRS_ZENABLE, 0u32),
            (D3DRS_ZWRITEENABLE, 0),
            (D3DRS_ALPHATESTENABLE, 0),
            (D3DRS_STENCILENABLE, 0),
            (D3DRS_SCISSORTESTENABLE, 0),
            (D3DRS_FOGENABLE, 0),
            (D3DRS_LIGHTING, 0),
            (D3DRS_CULLMODE, D3DCULL_NONE.0 as u32),
            (D3DRS_FILLMODE, D3DFILL_SOLID.0 as u32),
            (D3DRS_SRGBWRITEENABLE, 0),
            (D3DRS_COLORWRITEENABLE, 0xF),
            (D3DRS_ALPHABLENDENABLE, 1),
            (D3DRS_SEPARATEALPHABLENDENABLE, 0),
            (D3DRS_BLENDOP, D3DBLENDOP_ADD.0 as u32),
            (D3DRS_SRCBLEND, D3DBLEND_SRCALPHA.0 as u32),
            (D3DRS_DESTBLEND, D3DBLEND_INVSRCALPHA.0 as u32),
        ] {
            let _ = dev.SetRenderState(state, value);
        }
        let _ = dev.SetPixelShader(None);
        let _ = dev.SetVertexShader(None);
        let _ = dev.SetFVF(D3DFVF_XYZRHW | D3DFVF_DIFFUSE | D3DFVF_TEX1);
        let _ = dev.SetTextureStageState(0, D3DTSS_COLOROP, D3DTOP_SELECTARG1.0 as u32);
        let _ = dev.SetTextureStageState(0, D3DTSS_COLORARG1, D3DTA_TEXTURE);
        let _ = dev.SetTextureStageState(0, D3DTSS_ALPHAOP, D3DTOP_MODULATE.0 as u32);
        let _ = dev.SetTextureStageState(0, D3DTSS_ALPHAARG1, D3DTA_TEXTURE);
        let _ = dev.SetTextureStageState(0, D3DTSS_ALPHAARG2, D3DTA_DIFFUSE);
        let _ = dev.SetTextureStageState(1, D3DTSS_COLOROP, D3DTOP_DISABLE.0 as u32);
        let _ = dev.SetTextureStageState(1, D3DTSS_ALPHAOP, D3DTOP_DISABLE.0 as u32);
        for (st, val) in [
            (D3DSAMP_MINFILTER, D3DTEXF_LINEAR.0 as u32),
            (D3DSAMP_MAGFILTER, D3DTEXF_LINEAR.0 as u32),
            (D3DSAMP_MIPFILTER, D3DTEXF_NONE.0 as u32),
            (D3DSAMP_ADDRESSU, D3DTADDRESS_CLAMP.0 as u32),
            (D3DSAMP_ADDRESSV, D3DTADDRESS_CLAMP.0 as u32),
            (D3DSAMP_SRGBTEXTURE, 0),
        ] {
            let _ = dev.SetSamplerState(0, st, val);
        }
        let began = dev.BeginScene().is_ok();
        let mut result = Ok(());
        for &(layer, [l, t, r, b], alpha) in layers {
            let tex = match layer {
                Layer::Dim => &self.dim,
                Layer::Overlay => &self.overlay,
                Layer::Notice => &self.notice,
                Layer::Pointer => &self.pointer,
            };
            let Some((tex, _)) = tex else { continue };
            let base: Option<IDirect3DBaseTexture9> = tex.cast().ok();
            let _ = dev.SetTexture(0, base.as_ref());
            // A captured window's alpha channel means nothing: draw it opaque.
            let alpha_op = if layer == Layer::Overlay { D3DTOP_SELECTARG2 } else { D3DTOP_MODULATE };
            let _ = dev.SetTextureStageState(0, D3DTSS_ALPHAOP, alpha_op.0 as u32);
            let a = ((alpha.clamp(0.0, 1.0) * 255.0) as u32) << 24 | 0x00ff_ffff;
            let (l, t, r, b) = (l - 0.5, t - 0.5, r - 0.5, b - 0.5);
            let v = [
                Vertex9 { x: l, y: t, z: 0.0, rhw: 1.0, color: a, u: 0.0, v: 0.0 },
                Vertex9 { x: r, y: t, z: 0.0, rhw: 1.0, color: a, u: 1.0, v: 0.0 },
                Vertex9 { x: l, y: b, z: 0.0, rhw: 1.0, color: a, u: 0.0, v: 1.0 },
                Vertex9 { x: r, y: b, z: 0.0, rhw: 1.0, color: a, u: 1.0, v: 1.0 },
            ];
            if let Err(e) = dev.DrawPrimitiveUP(D3DPT_TRIANGLESTRIP, 2, v.as_ptr() as *const c_void, std::mem::size_of::<Vertex9>() as u32) {
                result = Err(e);
            }
        }
        if began {
            let _ = dev.EndScene();
        }
        if let Some(rt) = old_rt {
            let _ = dev.SetRenderTarget(0, &rt);
        }
        let _ = sb.Apply();
        result
    }
}

const SHADER: &str = r#"
cbuffer C : register(b0) { float4 rect; float4 alpha; };
Texture2D tex : register(t0);
SamplerState smp : register(s0);
struct V { float4 p : SV_Position; float2 uv : TEXCOORD0; };
V vs(uint id : SV_VertexID) {
    float2 uv = float2(id & 1, id >> 1);
    V o;
    o.p = float4(lerp(rect.x, rect.z, uv.x), lerp(rect.y, rect.w, uv.y), 0, 1);
    o.uv = uv;
    return o;
}
float4 ps(V i) : SV_Target {
    float4 c = tex.Sample(smp, i.uv);
    // alpha.y = 1: an opaque picture whose alpha channel means nothing (a captured window).
    return float4(c.rgb, lerp(c.a, 1.0, alpha.y) * alpha.x);
}
"#;

unsafe fn compile(entry: &[u8], target: &[u8]) -> windows::core::Result<ID3DBlob> {
    let mut code = None;
    let mut errors = None;
    let r = D3DCompile(SHADER.as_ptr() as *const c_void, SHADER.len(), PCSTR::null(), None, None, PCSTR(entry.as_ptr()), PCSTR(target.as_ptr()), 0, 0, &mut code, Some(&mut errors));
    r.map(|()| code.unwrap())
}

unsafe fn blob(b: &ID3DBlob) -> &[u8] {
    std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())
}

/// Draws CAPTURE's layers with one game's Direct3D 11 device.
struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext1,
    /// Our own pipeline state, swapped in and out around the draw.
    state: ID3DDeviceContextState,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    notice: Option<ID3D11ShaderResourceView>,
    /// Dynamic texture updated with each new overlay picture, and its size.
    overlay: Option<(ID3D11Texture2D, ID3D11ShaderResourceView, (u32, u32))>,
    dim: Option<ID3D11ShaderResourceView>,
    pointer: Option<ID3D11ShaderResourceView>,
    rtv: Option<ID3D11RenderTargetView>,
    /// Back buffer the view was made for.
    backbuffer: usize,
}

unsafe impl Send for Renderer {}

impl Renderer {
    unsafe fn new(device: &ID3D11Device) -> windows::core::Result<Self> {
        let device1: ID3D11Device1 = device.cast()?;
        let context: ID3D11DeviceContext1 = device.GetImmediateContext()?.cast()?;
        let fl = [device.GetFeatureLevel()];
        let mut state = None;
        device1.CreateDeviceContextState(0, &fl, D3D11_SDK_VERSION, &ID3D11Device::IID, None, Some(&mut state))?;
        let vsb = compile(b"vs\0", b"vs_4_0\0")?;
        let psb = compile(b"ps\0", b"ps_4_0\0")?;
        let (mut vs, mut ps) = (None, None);
        device.CreateVertexShader(blob(&vsb), None, Some(&mut vs))?;
        device.CreatePixelShader(blob(&psb), None, Some(&mut ps))?;
        let mut cb = None;
        let cbd = D3D11_BUFFER_DESC { ByteWidth: 32, Usage: D3D11_USAGE_DYNAMIC, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32, ..Default::default() };
        device.CreateBuffer(&cbd, None, Some(&mut cb))?;
        let mut sampler = None;
        let sd = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        device.CreateSamplerState(&sd, Some(&mut sampler))?;
        let mut bd = D3D11_BLEND_DESC::default();
        bd.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_SRC_ALPHA,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ZERO,
            DestBlendAlpha: D3D11_BLEND_ONE,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut blend = None;
        device.CreateBlendState(&bd, Some(&mut blend))?;
        let rd = D3D11_RASTERIZER_DESC { FillMode: D3D11_FILL_SOLID, CullMode: D3D11_CULL_NONE, DepthClipEnable: true.into(), ..Default::default() };
        let mut raster = None;
        device.CreateRasterizerState(&rd, Some(&mut raster))?;
        let mut r = Self {
            device: device.clone(),
            context,
            state: state.unwrap(),
            vs: vs.unwrap(),
            ps: ps.unwrap(),
            cb: cb.unwrap(),
            sampler: sampler.unwrap(),
            blend: blend.unwrap(),
            raster: raster.unwrap(),
            notice: None,
            overlay: None,
            dim: None,
            pointer: None,
            rtv: None,
            backbuffer: 0,
        };
        r.dim = Some(r.immutable(&[0, 0, 0, 255], 1, 1)?);
        let (px, w, h) = pointer_pixels();
        r.pointer = Some(r.immutable(&px, w, h)?);
        Ok(r)
    }

    unsafe fn immutable(&self, pixels: &[u8], w: u32, h: u32) -> windows::core::Result<ID3D11ShaderResourceView> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let init = D3D11_SUBRESOURCE_DATA { pSysMem: pixels.as_ptr() as *const c_void, SysMemPitch: w * 4, SysMemSlicePitch: 0 };
        let mut tex = None;
        self.device.CreateTexture2D(&desc, Some(&init), Some(&mut tex))?;
        let mut srv = None;
        self.device.CreateShaderResourceView(&tex.unwrap(), None, Some(&mut srv))?;
        srv.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))
    }

    unsafe fn set_notice(&mut self, pixels: &[u8], w: u32, h: u32) -> windows::core::Result<()> {
        self.notice = Some(self.immutable(pixels, w, h)?);
        Ok(())
    }

    unsafe fn set_overlay(&mut self, pixels: &[u8], w: u32, h: u32) -> windows::core::Result<()> {
        if self.overlay.as_ref().is_none_or(|o| o.2 != (w, h)) {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut tex = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut tex))?;
            let tex = tex.unwrap();
            let mut srv = None;
            self.device.CreateShaderResourceView(&tex, None, Some(&mut srv))?;
            self.overlay = Some((tex, srv.unwrap(), (w, h)));
        }
        let (tex, _, _) = self.overlay.as_ref().unwrap();
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        self.context.Map(tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m))?;
        for y in 0..h as usize {
            std::ptr::copy_nonoverlapping(pixels.as_ptr().add(y * w as usize * 4), (m.pData as *mut u8).add(y * m.RowPitch as usize), w as usize * 4);
        }
        self.context.Unmap(tex, 0);
        Ok(())
    }

    /// Draw `layers` onto the current back buffer.
    unsafe fn draw(&mut self, bb: &ID3D11Texture2D, d: &D3D11_TEXTURE2D_DESC, layers: &[(Layer, [f32; 4], f32)]) -> windows::core::Result<()> {
        if self.rtv.is_none() || self.backbuffer != bb.as_raw() as usize {
            let rd = D3D11_RENDER_TARGET_VIEW_DESC {
                Format: typed(d.Format),
                ViewDimension: if d.SampleDesc.Count > 1 { D3D11_RTV_DIMENSION_TEXTURE2DMS } else { D3D11_RTV_DIMENSION_TEXTURE2D },
                ..Default::default()
            };
            let mut rtv = None;
            self.device.CreateRenderTargetView(bb, Some(&rd), Some(&mut rtv))?;
            self.rtv = rtv;
            self.backbuffer = bb.as_raw() as usize;
        }
        let (bw, bh) = (d.Width as f32, d.Height as f32);
        let ndc = |x: f32, y: f32| (x / bw * 2.0 - 1.0, 1.0 - y / bh * 2.0);
        let ctx = &self.context;
        let mut prev = None;
        ctx.SwapDeviceContextState(&self.state, Some(&mut prev));
        ctx.OMSetRenderTargets(Some(&[self.rtv.clone()]), None);
        ctx.OMSetBlendState(&self.blend, None, 0xffff_ffff);
        ctx.RSSetState(&self.raster);
        ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: bw, Height: bh, MinDepth: 0.0, MaxDepth: 1.0 }]));
        ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        ctx.IASetInputLayout(None);
        ctx.VSSetShader(&self.vs, None);
        ctx.VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
        ctx.PSSetShader(&self.ps, None);
        ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
        ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
        for &(layer, [l, t, r, b], alpha) in layers {
            let srv = match layer {
                Layer::Dim => self.dim.clone(),
                Layer::Overlay => self.overlay.as_ref().map(|o| o.1.clone()),
                Layer::Notice => self.notice.clone(),
                Layer::Pointer => self.pointer.clone(),
            };
            if srv.is_none() {
                continue;
            }
            let (x0, y0) = ndc(l, t);
            let (x1, y1) = ndc(r, b);
            let opaque = if layer == Layer::Overlay { 1.0 } else { 0.0 };
            let consts: [f32; 8] = [x0, y0, x1, y1, alpha, opaque, 0.0, 0.0];
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            if ctx.Map(&self.cb, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m)).is_ok() {
                std::ptr::copy_nonoverlapping(consts.as_ptr(), m.pData as *mut f32, 8);
                ctx.Unmap(&self.cb, 0);
            }
            ctx.PSSetShaderResources(0, Some(&[srv]));
            ctx.Draw(4, 0);
        }
        if let Some(p) = prev {
            ctx.SwapDeviceContextState(&p, None);
        }
        Ok(())
    }
}

/// A render-target format for a back buffer that may be typeless.
fn typed(f: DXGI_FORMAT) -> DXGI_FORMAT {
    match f {
        DXGI_FORMAT_R8G8B8A8_TYPELESS => DXGI_FORMAT_R8G8B8A8_UNORM,
        DXGI_FORMAT_B8G8R8A8_TYPELESS => DXGI_FORMAT_B8G8R8A8_UNORM,
        DXGI_FORMAT_R10G10B10A2_TYPELESS => DXGI_FORMAT_R10G10B10A2_UNORM,
        DXGI_FORMAT_R16G16B16A16_TYPELESS => DXGI_FORMAT_R16G16B16A16_FLOAT,
        other => other,
    }
}
