//! Thin helpers over the raw libav (FFmpeg 8.1, LGPL) bindings.

use ffmpeg_sys_next as ff;
use std::ffi::{CStr, CString};

pub use ff::AVRational as Rational;

pub fn err_str(code: i32) -> String {
    let mut buf = [0i8; 256];
    unsafe { ff::av_strerror(code, buf.as_mut_ptr(), buf.len()) };
    unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

pub fn check(code: i32, what: &str) -> Result<i32, String> {
    if code < 0 {
        Err(format!("{what}: {}", err_str(code)))
    } else {
        Ok(code)
    }
}

pub fn cstr(s: &str) -> CString {
    CString::new(s).expect("no interior NUL")
}

/// Set a private (codec-specific) option; ignores "option not found" so the same option list
/// can be offered to several encoders, but reports invalid values.
pub unsafe fn opt(obj: *mut std::ffi::c_void, key: &str, val: &str) -> Result<(), String> {
    let r = ff::av_opt_set(obj, cstr(key).as_ptr(), cstr(val).as_ptr(), ff::AV_OPT_SEARCH_CHILDREN);
    if r < 0 && r != ff::AVERROR_OPTION_NOT_FOUND {
        return Err(format!("option {key}={val}: {}", err_str(r)));
    }
    Ok(())
}

/// Make `avformat_find_stream_info` on one of CAPTURE's own 2 s recording segments cheap: the
/// stream layout and codec parameters are in the first packets (every segment starts on a
/// keyframe), so the frame-rate probe (which reads and decodes ~20 frames) and a long analysis
/// window only cost time. For Replay previews; saved clips keep the full probe.
pub unsafe fn quick_probe(ic: *mut ff::AVFormatContext) {
    let _ = opt(ic as *mut std::ffi::c_void, "fpsprobesize", "0");
    let _ = opt(ic as *mut std::ffi::c_void, "analyzeduration", "300000");
}

pub unsafe fn dict_set(d: *mut *mut ff::AVDictionary, k: &str, v: &str) {
    ff::av_dict_set(d, cstr(k).as_ptr(), cstr(v).as_ptr(), 0);
}

/// An owned encoded packet tagged with its output track and encoder time base. Moved between
/// threads by value; the raw pointer is never shared.
pub struct Packet {
    pub ptr: *mut ff::AVPacket,
    pub track: usize,
    pub time_base: Rational,
}

unsafe impl Send for Packet {}

impl Packet {
    /// Takes ownership of the packet's data (the source packet is left blank).
    pub unsafe fn take(src: *mut ff::AVPacket, track: usize, time_base: Rational) -> Self {
        let ptr = ff::av_packet_alloc();
        ff::av_packet_move_ref(ptr, src);
        Self { ptr, track, time_base }
    }
    pub fn pts(&self) -> i64 {
        unsafe { (*self.ptr).pts }
    }
    pub fn is_key(&self) -> bool {
        unsafe { (*self.ptr).flags & ff::AV_PKT_FLAG_KEY != 0 }
    }
    pub fn secs(&self) -> f64 {
        self.pts() as f64 * self.time_base.num as f64 / self.time_base.den as f64
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { ff::av_packet_free(&mut self.ptr) };
    }
}

/// Mirror of `AVD3D11VADeviceContext` (libavutil/hwcontext_d3d11va.h). Stable public ABI.
#[repr(C)]
pub struct AVD3D11VADeviceContext {
    pub device: *mut std::ffi::c_void,
    pub device_context: *mut std::ffi::c_void,
    pub video_device: *mut std::ffi::c_void,
    pub video_context: *mut std::ffi::c_void,
    pub lock: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    pub unlock: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    pub lock_ctx: *mut std::ffi::c_void,
}

/// Mirror of `AVD3D11VAFramesContext`.
#[repr(C)]
pub struct AVD3D11VAFramesContext {
    pub texture: *mut std::ffi::c_void,
    pub bind_flags: u32,
    pub misc_flags: u32,
    pub texture_infos: *mut std::ffi::c_void,
}
