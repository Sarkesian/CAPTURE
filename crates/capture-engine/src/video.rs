//! H.264 encoding of the captured "latest" texture at a constant frame rate.
//!
//! Encoder choice is probed in order, and every rejection reason is kept for the UI:
//! 1. `h264_nvenc`, `h264_amf`: D3D11 textures go straight in (zero copy).
//! 2. `h264_mf` (hardware MFT, then software MFT): needs system-memory frames, so the latest
//!    texture is copied to a staging texture, mapped, and converted BGRA→NV12 with swscale.
//!
//! `h264_qsv` is not attempted: it needs QSV frames derived from D3D11, which cannot be verified
//! on the build machine (no Intel GPU). It is reported as not implemented rather than claimed.

use std::sync::Arc;

use capture_protocol::{EncoderInfo, Quality};
use ffmpeg_sys_next as ff;
use std::ptr::{null, null_mut};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use crate::av::{self, check, Packet, Rational};
use crate::gpu::Gpu;
use crate::wgc;

enum Input {
    Gpu { frames: *mut ff::AVBufferRef },
    Cpu { staging: ID3D11Texture2D, sws: *mut ff::SwsContext },
}

pub struct VideoEncoder {
    ctx: *mut ff::AVCodecContext,
    input: Input,
    gpu: Arc<Gpu>,
    pkt: *mut ff::AVPacket,
    pub info: EncoderInfo,
}

unsafe impl Send for VideoEncoder {}

struct Rate {
    cq: i32,
    maxrate: i64,
}

fn rate(q: Quality, w: u32, h: u32, fps: u32) -> Rate {
    let (bpp, cq) = match q {
        Quality::Balanced => (0.07, 24),
        Quality::High => (0.12, 21),
        Quality::Maximum => (0.20, 18),
    };
    Rate { cq, maxrate: (w as f64 * h as f64 * fps as f64 * bpp) as i64 }
}

impl VideoEncoder {
    /// `software_only`: skip GPU encoders and use Microsoft's software H.264 encoder.
    pub fn open(gpu: Arc<Gpu>, w: u32, h: u32, fps: u32, q: Quality, software_only: bool) -> Result<Self, String> {
        let mut rejected = Vec::new();
        let candidates: &[(&str, bool)] = if software_only {
            rejected.push("GPU encoders: skipped (Software encoder chosen in Settings)".into());
            &[("h264_mf", false)]
        } else {
            &[("h264_nvenc", true), ("h264_amf", true), ("h264_mf", false)]
        };
        for &(name, hardware_input) in candidates {
            match unsafe { Self::try_open(&gpu, name, hardware_input, w, h, fps, q, software_only) } {
                Ok((ctx, input, hardware)) => {
                    let info = EncoderInfo { name: name.into(), hardware, width: w, height: h, fps, rejected };
                    return Ok(Self { ctx, input, gpu, pkt: unsafe { ff::av_packet_alloc() }, info });
                }
                Err(e) => rejected.push(format!("{name}: {e}")),
            }
        }
        if !software_only {
            rejected.push("h264_qsv: not implemented in this build".into());
        }
        Err(format!("No H.264 encoder could be started. {}", rejected.join("; ")))
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn try_open(
        gpu: &Gpu,
        name: &str,
        gpu_input: bool,
        w: u32,
        h: u32,
        fps: u32,
        q: Quality,
        software_only: bool,
    ) -> Result<(*mut ff::AVCodecContext, Input, bool), String> {
        let codec = ff::avcodec_find_encoder_by_name(av::cstr(name).as_ptr());
        if codec.is_null() {
            return Err("not available in this FFmpeg build".into());
        }
        // For MF try the hardware MFT first, then Microsoft's software encoder.
        let attempts: &[bool] = if gpu_input {
            &[true]
        } else if software_only {
            &[false]
        } else {
            &[true, false]
        };
        let mut last = String::new();
        for &hw in attempts {
            let mut ctx = ff::avcodec_alloc_context3(codec);
            let r = rate(q, w, h, fps);
            (*ctx).width = w as i32;
            (*ctx).height = h as i32;
            (*ctx).time_base = Rational { num: 1, den: fps as i32 };
            (*ctx).framerate = Rational { num: fps as i32, den: 1 };
            (*ctx).gop_size = fps as i32; // keyframe every second; segments cut on keyframes
            (*ctx).max_b_frames = 0;
            (*ctx).bit_rate = r.maxrate * 6 / 10;
            (*ctx).rc_max_rate = r.maxrate;
            (*ctx).rc_buffer_size = r.maxrate as i32;
            (*ctx).color_primaries = ff::AVColorPrimaries::AVCOL_PRI_BT709;
            (*ctx).color_trc = ff::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            (*ctx).colorspace = ff::AVColorSpace::AVCOL_SPC_BT709;
            (*ctx).color_range = ff::AVColorRange::AVCOL_RANGE_MPEG;
            let pd = (*ctx).priv_data;
            let input = if gpu_input {
                let frames = ff::av_hwframe_ctx_alloc(gpu.hw_device);
                let fc = (*frames).data as *mut ff::AVHWFramesContext;
                (*fc).format = ff::AVPixelFormat::AV_PIX_FMT_D3D11;
                (*fc).sw_format = ff::AVPixelFormat::AV_PIX_FMT_BGRA;
                (*fc).width = w as i32;
                (*fc).height = h as i32;
                (*fc).initial_pool_size = 0;
                let mut f = frames;
                if let Err(e) = check(ff::av_hwframe_ctx_init(frames), "frames init") {
                    ff::av_buffer_unref(&mut f);
                    ff::avcodec_free_context(&mut ctx);
                    return Err(e);
                }
                (*ctx).pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_D3D11;
                (*ctx).hw_frames_ctx = ff::av_buffer_ref(frames);
                Input::Gpu { frames }
            } else {
                (*ctx).pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_NV12;
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: w,
                    Height: h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut staging = None;
                gpu.device.CreateTexture2D(&desc, None, Some(&mut staging)).map_err(|e| e.to_string())?;
                let sws = ff::sws_getContext(
                    w as i32,
                    h as i32,
                    ff::AVPixelFormat::AV_PIX_FMT_BGRA,
                    w as i32,
                    h as i32,
                    ff::AVPixelFormat::AV_PIX_FMT_NV12,
                    ff::SwsFlags::SWS_BILINEAR as i32,
                    null_mut(),
                    null_mut(),
                    null(),
                );
                Input::Cpu { staging: staging.unwrap(), sws }
            };
            match name {
                "h264_nvenc" => {
                    av::opt(pd, "preset", match q { Quality::Balanced => "p4", Quality::High => "p5", Quality::Maximum => "p6" })?;
                    av::opt(pd, "tune", "hq")?;
                    av::opt(pd, "rc", "vbr")?;
                    av::opt(pd, "cq", &r.cq.to_string())?;
                    av::opt(pd, "profile", "high")?;
                    av::opt(pd, "spatial-aq", "1")?;
                    (*ctx).bit_rate = 0; // pure CQ under the maxrate ceiling
                }
                "h264_amf" => {
                    av::opt(pd, "usage", "transcoding")?;
                    av::opt(pd, "quality", "balanced")?;
                    av::opt(pd, "rc", "vbr_peak")?;
                    av::opt(pd, "profile", "high")?;
                }
                "h264_mf" => {
                    av::opt(pd, "hw_encoding", if hw { "1" } else { "0" })?;
                    av::opt(pd, "rate_control", "u_vbr")?;
                    av::opt(pd, "scenario", "display_remoting")?;
                }
                _ => {}
            }
            let r = ff::avcodec_open2(ctx, codec, null_mut());
            if r >= 0 {
                return Ok((ctx, input, gpu_input || hw));
            }
            last = av::err_str(r);
            match input {
                Input::Gpu { mut frames } => ff::av_buffer_unref(&mut frames),
                Input::Cpu { sws, .. } => ff::sws_freeContext(sws),
            }
            ff::avcodec_free_context(&mut ctx);
        }
        Err(last)
    }

    pub fn time_base(&self) -> Rational {
        unsafe { (*self.ctx).time_base }
    }

    pub fn codec_ctx(&self) -> *const ff::AVCodecContext {
        self.ctx
    }

    /// Encode the current contents of `latest` as frame number `n` and hand finished packets
    /// to `out`.
    pub fn encode(&mut self, latest: &wgc::Latest, n: i64, out: &mut impl FnMut(Packet)) -> Result<(), String> {
        unsafe {
            let mut f = ff::av_frame_alloc();
            let res = (|| -> Result<(), String> {
                match &self.input {
                    Input::Gpu { frames } => {
                        check(ff::av_hwframe_get_buffer(*frames, f, 0), "get gpu frame")?;
                        let raw = (*f).data[0] as *mut std::ffi::c_void;
                        let dst = ID3D11Texture2D::from_raw_borrowed(&raw).ok_or("null texture")?;
                        let index = (*f).data[1] as usize as u32;
                        self.gpu.context.CopySubresourceRegion(dst, index, 0, 0, 0, &latest.texture, 0, None);
                    }
                    Input::Cpu { staging, sws } => {
                        let ctx = &self.gpu.context;
                        ctx.CopyResource(staging, &latest.texture);
                        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
                        ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).map_err(|e| e.to_string())?;
                        (*f).format = ff::AVPixelFormat::AV_PIX_FMT_NV12 as i32;
                        (*f).width = (*self.ctx).width;
                        (*f).height = (*self.ctx).height;
                        let r = ff::av_frame_get_buffer(f, 0);
                        if r >= 0 {
                            let src = [map.pData as *const u8, null(), null(), null()];
                            let stride = [map.RowPitch as i32, 0, 0, 0];
                            ff::sws_scale(*sws, src.as_ptr(), stride.as_ptr(), 0, (*self.ctx).height, (*f).data.as_ptr(), (*f).linesize.as_ptr());
                        }
                        ctx.Unmap(staging, 0);
                        check(r, "cpu frame")?;
                    }
                }
                (*f).pts = n;
                check(ff::avcodec_send_frame(self.ctx, f), "encode")?;
                self.drain(out)
            })();
            ff::av_frame_free(&mut f);
            res
        }
    }

    fn drain(&mut self, out: &mut impl FnMut(Packet)) -> Result<(), String> {
        unsafe {
            loop {
                let r = ff::avcodec_receive_packet(self.ctx, self.pkt);
                if r == ff::AVERROR(ff::EAGAIN) || r == ff::AVERROR_EOF {
                    return Ok(());
                }
                check(r, "encode packet")?;
                out(Packet::take(self.pkt, 0, self.time_base()));
            }
        }
    }

    pub fn flush(&mut self, out: &mut impl FnMut(Packet)) {
        unsafe { ff::avcodec_send_frame(self.ctx, null()) };
        let _ = self.drain(out);
    }
}

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        unsafe {
            ff::av_packet_free(&mut self.pkt);
            ff::avcodec_free_context(&mut self.ctx);
            match &mut self.input {
                Input::Gpu { frames } => ff::av_buffer_unref(frames),
                Input::Cpu { sws, .. } => ff::sws_freeContext(*sws),
            }
        }
    }
}
