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
use windows::Graphics::Capture::{Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::{IDXGIDevice,IDXGIOutput6};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::gpu::Gpu;

struct SendCtx(ID3D11DeviceContext, IDirect3DDevice);
unsafe impl Send for SendCtx {}
unsafe impl Sync for SendCtx {}
struct SendDev(ID3D11Device);
unsafe impl Send for SendDev {}
unsafe impl Sync for SendDev {}

/// Capture buffers Windows can fill while earlier ones are still being copied by a busy GPU.
/// With 2, frames were skipped whenever our copy queued behind the game's rendering.
const POOL_FRAMES: i32 = 4;

// Dropping a WinRT interface is not the same as returning the buffer to WGC.
struct FrameLease(Direct3D11CaptureFrame);
impl Drop for FrameLease {
    fn drop(&mut self) { let _ = self.0.Close(); }
}

#[derive(Clone, Copy)]
pub enum Source {
    Window(isize),
    Monitor(isize),
}

/// The most recent captured frame, already cropped and (after a size change) scaled to the
/// encoder size.
pub struct Latest {
    pub frame_qpc: u64,
    pub texture: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    pub has_frame: bool,
    last_size: SizeInt32,
    /// GPU scaler for frames whose size no longer matches the encoder (created on first need).
    scaler: Option<Scaler>,
    /// A capture failure is visible in engine status and retried on the next frame.
    pub error: Option<String>,
}

/// Where a `src_w`x`src_h` picture goes inside a `dst_w`x`dst_h` frame: as large as possible
/// with its aspect ratio kept, centred (black bars on the remaining sides).
pub fn fit_rect(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (0, 0, dst_w, dst_h);
    }
    let (w, h) = if src_w as u64 * dst_h as u64 > src_h as u64 * dst_w as u64 {
        (dst_w, ((src_h as u64 * dst_w as u64) / src_w as u64) as u32)
    } else {
        (((src_w as u64 * dst_h as u64) / src_h as u64) as u32, dst_h)
    };
    let (w, h) = (w.clamp(1, dst_w), h.clamp(1, dst_h));
    ((dst_w - w) / 2, (dst_h - h) / 2, w, h)
}

/// Window geometry can advance ahead of the queued WGC frame. Reject incomplete
/// pictures instead of clamping the new client area into an older surface.
fn frame_rect(content: SizeInt32, surface: (u32, u32), crop: Option<(i32, i32, i32, i32)>) -> Option<RECT> {
    let (x, y, w, h) = crop.unwrap_or((0, 0, content.Width, content.Height));
    let (right, bottom) = (x.checked_add(w)?, y.checked_add(h)?);
    if x < 0 || y < 0 || w < 16 || h < 16 || right > content.Width || bottom > content.Height
        || content.Width < 16 || content.Height < 16 || content.Width as u32 > surface.0 || content.Height as u32 > surface.1 {
        return None;
    }
    Some(RECT { left: x, top: y, right, bottom })
}

/// D3D11 video processor that scales a captured frame into `Latest` (same BGRA format), used
/// only while the source size differs from the encoder size (window resized, resolution
/// switched). The recording keeps one size, so the replay buffer stays continuous.
struct Scaler {
    vdev: ID3D11VideoDevice,
    vctx: ID3D11VideoContext,
    en: ID3D11VideoProcessorEnumerator,
    vp: ID3D11VideoProcessor,
    out: ID3D11VideoProcessorOutputView,
    in_size: (u32, u32),
}

impl Scaler {
    unsafe fn new(device: &ID3D11Device, context: &ID3D11DeviceContext, target: &ID3D11Texture2D, in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> windows::core::Result<Self> {
        let vdev: ID3D11VideoDevice = device.cast()?;
        let vctx: ID3D11VideoContext = context.cast()?;
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
            InputWidth: in_w,
            InputHeight: in_h,
            OutputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
            OutputWidth: out_w,
            OutputHeight: out_h,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        let en = vdev.CreateVideoProcessorEnumerator(&desc)?;
        let vp = vdev.CreateVideoProcessor(&en, 0)?;
        let od = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
        };
        let mut out = None;
        vdev.CreateVideoProcessorOutputView(target, &en, &od, Some(&mut out))?;
        let out = out.ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(-1)))?;
        vctx.VideoProcessorSetStreamFrameFormat(&vp, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
        let black = D3D11_VIDEO_COLOR { Anonymous: D3D11_VIDEO_COLOR_0 { RGBA: D3D11_VIDEO_COLOR_RGBA { R: 0.0, G: 0.0, B: 0.0, A: 1.0 } } };
        vctx.VideoProcessorSetOutputBackgroundColor(&vp, false, &black);
        Ok(Self { vdev, vctx, en, vp, out, in_size: (in_w, in_h) })
    }

    /// Scale the `rect` part of `src` into the whole output, aspect ratio kept.
    unsafe fn blit(&self, src: &ID3D11Texture2D, rect: RECT, out_w: u32, out_h: u32) -> windows::core::Result<()> {
        let id = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
        };
        let mut input = None;
        self.vdev.CreateVideoProcessorInputView(src, &self.en, &id, Some(&mut input))?;
        let (x, y, w, h) = fit_rect((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32, out_w, out_h);
        let dest = RECT { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 };
        let target = RECT { left: 0, top: 0, right: out_w as i32, bottom: out_h as i32 };
        self.vctx.VideoProcessorSetOutputTargetRect(&self.vp, true, Some(&target));
        self.vctx.VideoProcessorSetStreamSourceRect(&self.vp, 0, true, Some(&rect));
        self.vctx.VideoProcessorSetStreamDestRect(&self.vp, 0, true, Some(&dest));
        let mut streams = [D3D11_VIDEO_PROCESSOR_STREAM { Enable: true.into(), pInputSurface: std::mem::ManuallyDrop::new(input), ..Default::default() }];
        let r = self.vctx.VideoProcessorBlt(&self.vp, &self.out, 0, &streams);
        std::mem::ManuallyDrop::drop(&mut streams[0].pInputSurface); // release the input view
        r
    }
}

pub struct Capture {
    pub color: ColorState,
    source: Source,
    pub resolution_changed: Arc<AtomicBool>,
    pub native_resolution: Arc<AtomicBool>,
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

/// The game's client area inside what WGC captures for a window (WGC includes the title bar
/// and borders of windowed apps): (x, y, width, height) in captured pixels. `None` when the
/// window is minimized or the information is unavailable.
pub fn client_crop(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let h = HWND(hwnd as *mut _);
    unsafe {
        let mut frame = RECT::default();
        DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut frame as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).ok()?;
        let mut client = RECT::default();
        GetClientRect(h, &mut client).ok()?;
        let mut origin = POINT { x: 0, y: 0 };
        if !ClientToScreen(h, &mut origin).as_bool() {
            return None;
        }
        let (cw, ch) = (client.right - client.left, client.bottom - client.top);
        if cw < 16 || ch < 16 {
            return None;
        }
        Some(((origin.x - frame.left).max(0), (origin.y - frame.top).max(0), cw, ch))
    }
}

impl Capture {
    /// Also notices SDR-white-level changes while an HDR desktop is otherwise static.
    pub fn check_color(&self, gpu:&Gpu) {
        match source_color(&gpu.device, &self.source, true) {
            Ok(now) if now != self.color => { self.resolution_changed.store(true,Ordering::Relaxed); self.latest.lock().unwrap().has_frame=false; },
            Err(e) => { let mut l=self.latest.lock().unwrap();l.has_frame=false;l.error=Some(e); },
            _ => {}
        }
    }
    pub fn start(gpu: &Gpu, source: Source, cursor: bool) -> Result<Self, String> {
        Self::start_with_native(gpu,source,cursor,false)
    }
    pub fn start_native(gpu:&Gpu,source:Source,cursor:bool)->Result<Self,String> {
        Self::start_with_native(gpu,source,cursor,true)
    }
    fn start_with_native(gpu:&Gpu,source:Source,cursor:bool,native:bool)->Result<Self,String> {
        let color = if native { source_color(&gpu.device, &source, true)? } else { ColorState::sdr() };
        let format = if color.hdr { DirectXPixelFormat::R16G16B16A16Float } else { DirectXPixelFormat::B8G8R8A8UIntNormalized };
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
        // Windows: record only the game's client area (no title bar / borders).
        let window = match source {
            Source::Window(h) => Some(h),
            Source::Monitor(_) => None,
        };
        let crop0 = window.and_then(client_crop);
        let dimension=|v:i32|if native {v.max(2) as u32} else {even(v)};
        let (width, height) = match crop0 {
            Some((_, _, w, h)) => (dimension(w.min(size.Width)), dimension(h.min(size.Height))),
            None => (dimension(size.Width), dimension(size.Height)),
        };

        let winrt_device: IDirect3DDevice = unsafe {
            let dxgi: IDXGIDevice = gpu.device.cast().map_err(|e| e.to_string())?;
            CreateDirect3D11DeviceFromDXGIDevice(&dxgi).map_err(|e| e.to_string())?.cast().map_err(|e| e.to_string())?
        };

        let mut initial = make_latest_format(gpu, width, height, if color.hdr {DXGI_FORMAT_R16G16B16A16_FLOAT} else {DXGI_FORMAT_B8G8R8A8_UNORM})?;
        initial.last_size = size; // pool size includes window decorations; encoder size does not
        let latest = Arc::new(Mutex::new(initial));
        let frames = Arc::new(AtomicU64::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let resolution_changed=Arc::new(AtomicBool::new(false));
        let native_resolution=Arc::new(AtomicBool::new(native));

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            format,
            POOL_FRAMES,
            size,
        )
        .map_err(|e| e.to_string())?;
        let session = pool.CreateCaptureSession(&item).map_err(|e| e.to_string())?;
        // The Windows default can quantize a ~16 ms minimum to three refreshes on
        // a 144 Hz display (~48 arrivals/sec). Let WGC deliver source updates;
        // the configured encoder clock still selects the requested output FPS.
        let prior_interval=session.MinUpdateInterval().ok().map(|v|v.Duration);
        let legacy_pacing=cfg!(debug_assertions) && std::env::var("CAPTURE_WGC_DEFAULT_PACING").as_deref()==Ok("1");
        if !legacy_pacing {
            if let Err(e)=session.SetMinUpdateInterval(windows::Foundation::TimeSpan{Duration:0}) {
                eprintln!("WGC minimum update interval is unavailable: {e}; measured capture pacing remains visible in diagnostics");
            }
        }
        eprintln!("WGC minimum update interval (100 ns): {prior_interval:?} -> {:?}",session.MinUpdateInterval().ok().map(|v|v.Duration));
        let _ = session.SetIsBorderRequired(false); // Win11; ignored where unsupported
        let _ = session.SetIsCursorCaptureEnabled(cursor);

        {
            let (latest, frames) = (latest.clone(), frames.clone());
            let (resolution_changed,native_resolution)=(resolution_changed.clone(),native_resolution.clone());
            // The immediate context is multithread-protected (see gpu.rs), so using it from the
            // WGC callback thread is safe.
            let ctx = SendCtx(gpu.context.clone(), winrt_device.clone());
            let dev3d = SendDev(gpu.device.clone());
            // IsCurrent is the cheap per-frame invalidation check recommended by DXGI.
            // Enumerating outputs and querying HDR characteristics on every frame can
            // serialize with Present on the game; do that only after invalidation/moves.
            let factory=unsafe { windows::Win32::Graphics::Dxgi::CreateDXGIFactory1::<windows::Win32::Graphics::Dxgi::IDXGIFactory1>() }.map_err(|e|e.to_string())?;
            let callback = Mutex::new(factory);
            pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                move |pool, _| {
                    let mut callback = callback.lock().unwrap();
                    let ctx = &ctx; // capture the whole Send wrapper, not its fields
                    let (context, dev) = (&ctx.0, &ctx.1);
                    let dev3d = &dev3d.0;
                    let Ok(pool) = pool.ok() else { return Ok(()) };
                    let t_arrive = crate::clock::now_100ns();
                    let lease = FrameLease(pool.TryGetNextFrame()?);
                    let frame = &lease.0;
                    let content = frame.ContentSize()?;
                    let tex: ID3D11Texture2D = unsafe { frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()? };
                    let mut l = latest.lock().unwrap();
                    // Check before publishing: never feed a new display transfer function into
                    // the previous epoch's encoder. Display capabilities are not game metadata.
                    if resolution_changed.load(Ordering::Relaxed) {l.has_frame=false;return Ok(());}
                    if native && (source_monitor(&source)!=color.monitor || !unsafe {callback.IsCurrent()}.as_bool()) {
                        match source_color(dev3d, &source, false) {
                            Ok(now) if now.same_signal(&color) => {
                                *callback=unsafe {windows::Win32::Graphics::Dxgi::CreateDXGIFactory1()?};
                            },
                            Ok(_) => { l.has_frame=false; resolution_changed.store(true, Ordering::Relaxed); return Ok(()); },
                            Err(e) => {l.has_frame=false; l.error=Some(e); return Ok(());}
                        }
                    }
                    if crate::trace::enabled() {
                        let shown = frame.SystemRelativeTime().map(|t| t.Duration).unwrap_or(0);
                        let now = crate::clock::now_100ns();
                        crate::trace::ev("cap", t_arrive, shown, (now - t_arrive) as i64, 0);
                    }
                    if content != l.last_size {
                        // ContentSize can already describe the NEW window while its surface
                        // still has the OLD pool dimensions. Never publish that truncated frame.
                        // Return all references before Recreate, and retain the last complete
                        // picture for the CFR clock while Windows delivers the new-size frame.
                        drop(tex);
                        drop(lease);
                        drop(l); // Recreate may wait for Windows; never stall the encoder clock.
                        if content.Width < 16 || content.Height < 16 { return Ok(()) }
                        if let Err(e) = pool.Recreate(dev, format, POOL_FRAMES, content) {
                            latest.lock().unwrap().error = Some(format!("Could not resize the capture buffers: {e}"));
                            return Ok(());
                        }
                        latest.lock().unwrap().last_size = content;
                        return Ok(());
                    }
                    // Minimize/mode switches can have empty content or a surface smaller
                    // than ContentSize. Keep the complete picture rather than copying garbage.
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    unsafe { tex.GetDesc(&mut desc) };
                    let crop = window.and_then(client_crop);
                    if window.is_some() && crop.is_none() { return Ok(()) }
                    let Some(rect) = frame_rect(content, (desc.Width, desc.Height), crop) else { return Ok(()) };
                    let (cx, cy) = (rect.left as u32, rect.top as u32);
                    let (sw, sh) = ((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32);
                    // Same size as the recording (the usual case): a plain copy below. Otherwise
                    // scale it into the recording size so nothing is cut off or left half-empty.
                    let resized = if native_resolution.load(Ordering::Relaxed) {(sw,sh)!=(width,height)} else {(even(sw as i32),even(sh as i32))!=(width,height)};
                    if resized && native_resolution.load(Ordering::Relaxed) {
                        resolution_changed.store(true,Ordering::Relaxed);
                        l.has_frame=false;
                        return Ok(());
                    }
                    if resized {
                        let (tw, th) = (desc.Width, desc.Height);
                        if l.scaler.as_ref().is_none_or(|sc| sc.in_size != (tw, th)) {
                            l.scaler = None;
                            match unsafe { Scaler::new(dev3d, context, &l.texture, tw, th, width, height) } {
                                Ok(sc) => l.scaler = Some(sc),
                                Err(e) => {
                                    l.error = Some(format!("Could not scale the resized capture: {e}"));
                                    return Ok(());
                                }
                            }
                        }
                        if let Some(sc) = &l.scaler {
                            if rect.right > rect.left && rect.bottom > rect.top {
                                match unsafe { sc.blit(&tex, rect, width, height) } {
                                    Ok(()) => {
                                        l.error = None;
                                        l.has_frame = true;
                                        frames.fetch_add(1, Ordering::Relaxed);
                                    }
                                    Err(e) => l.error = Some(format!("Could not scale the resized capture: {e}")),
                                }
                            }
                        }
                        return Ok(()); // never silently replace a failed scale with cropping
                    }
                    let b = D3D11_BOX {
                        left: cx,
                        top: cy,
                        front: 0,
                        right: (rect.right as u32).min(cx + width),
                        bottom: (rect.bottom as u32).min(cy + height),
                        back: 1,
                    };
                    if b.right > b.left && b.bottom > b.top {
                        unsafe {
                            context.ClearRenderTargetView(&l.rtv, &[0.0, 0.0, 0.0, 1.0]);
                            context.CopySubresourceRegion(&l.texture, 0, 0, 0, 0, &tex, 0, Some(&b));
                        }
                        l.error = None;
                        l.has_frame = true;
                        l.frame_qpc=frame.SystemRelativeTime().map(|t|t.Duration.max(0) as u64).unwrap_or(t_arrive);
                        let generation=frames.fetch_add(1, Ordering::Relaxed)+1;
                        crate::trace::ev("src",t_arrive,l.frame_qpc as i64,generation as i64,0);
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
        Ok(Self { width, height, latest, frames, closed, session, pool, resolution_changed,native_resolution, color, source })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorState {pub hdr:bool, pub sdr_white:f32, monitor:isize, space:DXGI_COLOR_SPACE_TYPE}
impl ColorState {
    fn sdr()->Self {Self{hdr:false,sdr_white:80.0,monitor:0,space:DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709}}
    fn same_signal(&self, other:&Self)->bool {self.monitor==other.monitor && self.space==other.space}
}

fn classify_color(color:DXGI_COLOR_SPACE_TYPE)->Result<bool,String> {
    if color==DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 || color==DXGI_COLOR_SPACE_RGB_STUDIO_G22_NONE_P709 {Ok(false)}
    else if color==DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 || color==DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709 {Ok(true)}
    else {Err(format!("Unsupported display color space {color:?}. Recording was not silently converted."))}
}

fn source_color(device:&ID3D11Device,source:&Source,white:bool)->Result<ColorState,String> {
    let monitor=HMONITOR(source_monitor(source) as *mut _);
    unsafe {
        let mut adapter=device.cast::<IDXGIDevice>().and_then(|d|d.GetAdapter()).map_err(|e|format!("Cannot verify source color format: {e}"))?;
        // DXGI caches display characteristics in its factory. Refresh enumeration after
        // HDR/topology changes, matching the capture device by LUID rather than GPU index.
        let factory:windows::Win32::Graphics::Dxgi::IDXGIFactory1=adapter.GetParent().map_err(|e|e.to_string())?;
        if !factory.IsCurrent().as_bool() {
            use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1,IDXGIFactory1};
            let luid=adapter.GetDesc().map_err(|e|e.to_string())?.AdapterLuid;
            let fresh:IDXGIFactory1=CreateDXGIFactory1().map_err(|e|e.to_string())?;
            let mut n=0;
            while let Ok(candidate)=fresh.EnumAdapters1(n) {
                n+=1;
                if candidate.GetDesc1().map_err(|e|e.to_string())?.AdapterLuid==luid {
                    adapter=candidate.cast().map_err(|e|e.to_string())?;break;
                }
            }
        }
        let mut index=0;
        while let Ok(output)=adapter.EnumOutputs(index) {
            index+=1;
            let desc=output.GetDesc().map_err(|e|e.to_string())?;
            if desc.Monitor==monitor {
                let color=output.cast::<IDXGIOutput6>().and_then(|o|o.GetDesc1()).map_err(|e|format!("Cannot verify source color format: {e}"))?.ColorSpace;
                let hdr=classify_color(color)?;
                let sdr_white=if hdr && white {sdr_white_level(&String::from_utf16_lossy(&desc.DeviceName).trim_end_matches('\0'))?}else{80.0};
                return Ok(ColorState{hdr,sdr_white,monitor:monitor.0 as isize,space:color});
            }
        }
    }
    Err("Cannot verify this display's color format on the capture GPU. Multi-adapter capture is not supported by this path.".into())
}

fn source_monitor(source:&Source)->isize {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow,MONITOR_DEFAULTTONEAREST};
    match source {Source::Monitor(h)=>*h,Source::Window(h)=>unsafe{MonitorFromWindow(HWND(*h as *mut _),MONITOR_DEFAULTTONEAREST).0 as isize}}
}

// Windows reports SDR white as a multiplier of 80 nits (1000 = 80 nits).
fn sdr_white_level(display:&str)->Result<f32,String> {unsafe {
    use windows::Win32::Devices::Display::*;
    let(mut np,mut nm)=(0,0);
    GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS,&mut np,&mut nm).ok().map_err(|e|e.to_string())?;
    let(mut paths,mut modes)=(vec![DISPLAYCONFIG_PATH_INFO::default();np as usize],vec![DISPLAYCONFIG_MODE_INFO::default();nm as usize]);
    QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS,&mut np,paths.as_mut_ptr(),&mut nm,modes.as_mut_ptr(),None).ok().map_err(|e|e.to_string())?;
    for path in paths.into_iter().take(np as usize) {
        let mut name=DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        name.header=DISPLAYCONFIG_DEVICE_INFO_HEADER{r#type:DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,size:std::mem::size_of_val(&name) as u32,adapterId:path.sourceInfo.adapterId,id:path.sourceInfo.id};
        if DisplayConfigGetDeviceInfo(&mut name.header)!=0 {continue;}
        if String::from_utf16_lossy(&name.viewGdiDeviceName).trim_end_matches('\0')!=display {continue;}
        let mut level=DISPLAYCONFIG_SDR_WHITE_LEVEL::default();
        level.header=DISPLAYCONFIG_DEVICE_INFO_HEADER{r#type:DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,size:std::mem::size_of_val(&level) as u32,adapterId:path.targetInfo.adapterId,id:path.targetInfo.id};
        if DisplayConfigGetDeviceInfo(&mut level.header)==0 && level.SDRWhiteLevel>0 {return Ok(level.SDRWhiteLevel as f32*0.08);}
    }
    Err("Cannot read Windows SDR white level for the HDR display. No guessed exposure was applied; retry after the display transition.".into())
}}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

#[cfg(test)]
fn make_latest(gpu: &Gpu, width: u32, height: u32) -> Result<Latest, String> {
    make_latest_format(gpu,width,height,DXGI_FORMAT_B8G8R8A8_UNORM)
}
fn make_latest_format(gpu:&Gpu,width:u32,height:u32,format:DXGI_FORMAT)->Result<Latest,String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
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
    Ok(Latest { frame_qpc:0,texture, rtv, has_frame: false, last_size: SizeInt32 { Width: width as i32, Height: height as i32 }, scaler: None, error: None })
}

#[cfg(test)]
pub(crate) fn synthetic_latest(gpu:&Gpu,width:u32,height:u32,pixels:&[u8])->Latest {
    let mut latest=make_latest(gpu,width,height).unwrap();
    unsafe {gpu.context.UpdateSubresource(&latest.texture,0,None,pixels.as_ptr().cast(),width*4,0);}
    latest.has_frame=true;latest
}

#[cfg(test)]
pub(crate) fn synthetic_hdr(gpu:&Gpu,width:u32,height:u32,pixels:&[u16])->Latest {
    let mut latest=make_latest_format(gpu,width,height,DXGI_FORMAT_R16G16B16A16_FLOAT).unwrap();
    unsafe{gpu.context.UpdateSubresource(&latest.texture,0,None,pixels.as_ptr().cast(),width*8,0);}
    latest.has_frame=true;latest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdr_and_sdr_are_classified_without_guessing_game_metadata() {
        assert_eq!(classify_color(DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709).unwrap(),false);
        assert_eq!(classify_color(DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020).unwrap(),true);
        assert_eq!(classify_color(DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709).unwrap(),true);
        assert!(classify_color(DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P2020).is_err());
    }

    #[test]
    fn size_transitions_never_publish_a_partial_surface() {
        let size = SizeInt32 { Width: 1280, Height: 720 };
        assert!(frame_rect(size, (640, 360), None).is_none(), "new content in an old pool texture");
        assert!(frame_rect(size, (1280, 720), Some((0, 0, 1920, 1080))).is_none(), "window geometry ahead of content");
        assert!(frame_rect(size, (1280, 720), Some((10, 30, 1280, 720))).is_none(), "crop extends past content");
        assert!(frame_rect(size, (1280, 720), Some((0, 0, 0, 0))).is_none(), "minimized");
        assert!(frame_rect(size, (1920, 1080), None).is_some(), "shrinking content in a larger surface is complete");
        assert!(frame_rect(size, (1280, 720), Some((1, 30, 1278, 689))).is_some(), "normal odd client size");
    }

    #[test]
    fn resized_picture_fits_with_its_aspect_ratio() {
        assert_eq!(fit_rect(1920, 1080, 1920, 1080), (0, 0, 1920, 1080));
        assert_eq!(fit_rect(2560, 1440, 1280, 720), (0, 0, 1280, 720), "same shape: fills");
        assert_eq!(fit_rect(1280, 1024, 1920, 1080), (285, 0, 1350, 1080), "5:4 into 16:9: bars left/right");
        assert_eq!(fit_rect(3440, 1440, 1920, 1080), (0, 138, 1920, 803), "ultrawide: bars top/bottom");
    }
}
