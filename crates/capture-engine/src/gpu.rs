//! One D3D11 device shared by Windows Graphics Capture and the libav hardware encoder, so
//! captured textures reach the encoder without leaving the GPU.

use crate::av::{self, AVD3D11VADeviceContext};
use ffmpeg_sys_next as ff;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub adapter: String,
    /// libav `AVHWDeviceContext` wrapping `device` (owned reference).
    pub hw_device: *mut ff::AVBufferRef,
}

unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

impl Gpu {
    pub fn create() -> Result<Self, String> {
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("Could not start the graphics device: {e}"))?;
        }
        let device: ID3D11Device = device.unwrap();
        let context: ID3D11DeviceContext = context.unwrap();
        // WGC callbacks, the encoder thread and libav all touch the immediate context.
        if let Ok(mt) = context.cast::<ID3D11Multithread>() {
            unsafe {
                let _ = mt.SetMultithreadProtected(true);
            }
        }
        raise_gpu_priority(&device);
        let adapter = unsafe {
            device
                .cast::<IDXGIDevice>()
                .and_then(|d| d.GetAdapter())
                .and_then(|a| a.GetDesc())
                .map(|d| String::from_utf16_lossy(&d.Description).trim_end_matches('\0').to_string())
                .unwrap_or_default()
        };
        let hw_device = unsafe {
            let r = ff::av_hwdevice_ctx_alloc(ff::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
            let hwctx = (*r).data as *mut ff::AVHWDeviceContext;
            let d3d = (*hwctx).hwctx as *mut AVD3D11VADeviceContext;
            // libav releases this reference when the device context is freed.
            (*d3d).device = device.clone().into_raw();
            if let Err(e) = av::check(ff::av_hwdevice_ctx_init(r), "hw device init") {
                let mut r = r;
                ff::av_buffer_unref(&mut r);
                return Err(e);
            }
            r
        };
        Ok(Self { device, context, adapter, hw_device })
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe { ff::av_buffer_unref(&mut self.hw_device) };
    }
}

/// Games keep the GPU busy; at normal priority our copies and NVENC's input conversion wait
/// behind whole game frames, which capped 4K60 encoding below 60 fps. Ask the GPU scheduler to
/// run this process's work first (as OBS does). Best effort: without the privilege Windows may
/// refuse the higher classes, and the result is logged.
fn raise_gpu_priority(device: &ID3D11Device) {
    use windows::core::s;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    use windows::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let thread = device.cast::<IDXGIDevice>().and_then(|d| d.SetGPUThreadPriority(7));
        // D3DKMT_SCHEDULINGPRIORITYCLASS: NORMAL 2, ABOVE_NORMAL 3, HIGH 4, REALTIME 5.
        type SetClass = unsafe extern "system" fn(windows::Win32::Foundation::HANDLE, i32) -> i32;
        let mut class = None;
        if let Ok(gdi) = LoadLibraryA(s!("gdi32.dll")) {
            if let Some(f) = GetProcAddress(gdi, s!("D3DKMTSetProcessSchedulingPriorityClass")) {
                let f: SetClass = std::mem::transmute(f);
                for c in [5, 4, 3] {
                    if f(GetCurrentProcess(), c) == 0 {
                        class = Some(c);
                        break;
                    }
                }
            }
        }
        eprintln!("GPU priority: thread {:?}, process class {:?}", thread.is_ok(), class);
    }
}
