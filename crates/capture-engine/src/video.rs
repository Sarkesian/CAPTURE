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
    hdr: Option<Arc<crate::hdr::Converter>>,
    converted: Option<ID3D11Texture2D>,
}

unsafe impl Send for VideoEncoder {}

enum GrabKind {
    /// Own reference to the encoder's D3D11 frames context.
    Gpu(*mut ff::AVBufferRef),
    Cpu(ID3D11Texture2D),
}

/// Takes pictures from [`wgc::Latest`] for a [`VideoEncoder`]. Taking one only queues a GPU copy,
/// so the frame lock (which the capture callback also needs) is held for microseconds, never
/// for the duration of an encode.
pub struct Grabber {
    hdr:Option<Arc<crate::hdr::Converter>>,
    converted:Option<ID3D11Texture2D>,
    kind: GrabKind,
    gpu: Arc<Gpu>,
}

unsafe impl Send for Grabber {}

/// One picture, copied out of `Latest`, waiting to be encoded as frame `pts`.
pub struct Grab {
    frame: *mut ff::AVFrame,
    pub pts: i64,
}

unsafe impl Send for Grab {}

impl Drop for Grab {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.frame) };
    }
}

impl Grabber {
    /// Pictures can be encoded on another thread while the next ones are taken (GPU input).
    /// CPU input shares one staging texture, so each picture must be encoded before the next.
    pub fn pipelined(&self) -> bool {
        matches!(self.kind, GrabKind::Gpu(_))
    }

    /// Copy the current picture for frame `n`. Call with the frame lock held; returns at once.
    pub fn grab(&self, latest: &wgc::Latest, n: i64) -> Result<Grab, String> {
        unsafe {
            let g = Grab { frame: ff::av_frame_alloc(), pts: n };
            if g.frame.is_null() {
                return Err("out of memory".into());
            }
            let ctx = &self.gpu.context;
            match &self.kind {
                GrabKind::Gpu(frames) => {
                    check(ff::av_hwframe_get_buffer(*frames, g.frame, 0), "get gpu frame")?;
                    let raw = (*g.frame).data[0] as *mut std::ffi::c_void;
                    let dst = ID3D11Texture2D::from_raw_borrowed(&raw).ok_or("null texture")?;
                    let index = (*g.frame).data[1] as usize as u32;
                    if let Some(hdr)=&self.hdr {hdr.convert(&self.gpu,&latest.texture,dst,index)?;} else {ctx.CopySubresourceRegion(dst, index, 0, 0, 0, &latest.texture, 0, None);}
                }
                GrabKind::Cpu(staging) => {
                    if let (Some(converter),Some(dst))=(&self.hdr,&self.converted) {
                        converter.convert(&self.gpu,&latest.texture,dst,0)?;
                        ctx.CopyResource(staging,dst);
                    } else { ctx.CopyResource(staging, &latest.texture); }
                },
            }
            // Submit now: the GPU (shared with a busy game) gets the copy as early as possible,
            // so it is done by the time the encoder needs it.
            ctx.Flush();
            Ok(g)
        }
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        if let GrabKind::Gpu(frames) = &mut self.kind {
            unsafe { ff::av_buffer_unref(frames) };
        }
    }
}

struct Rate {
    cq: i32,
    maxrate: i64,
}

fn rate(q: Quality, w: u32, h: u32, fps: u32) -> Rate {
    let (bpp, cq) = match q {
        Quality::Balanced => (0.07, 24),
        Quality::High => (0.12, 21),
        Quality::Maximum => (0.20, 18),
        Quality::Archival => (0.0, 0),
    };
    Rate { cq, maxrate: (w as f64 * h as f64 * fps as f64 * bpp) as i64 }
}

/// NVENC preset (1..=7) and CQ for a quality at a given pixel rate.
///
/// Keep the selected quality at every pixel rate. Overload is measured by the pipeline;
/// it must never silently switch to a faster preset or a different CQ.
fn nvenc_preset(q: Quality, w: u32, h: u32, fps: u32, cq: i32) -> (u32, i32) {
    let preset=match q {Quality::Balanced=>4,Quality::High=>5,Quality::Maximum=>6,Quality::Archival=>1};
    let _=(w,h,fps); (preset,cq)
}

impl VideoEncoder {
    /// `software_only`: skip GPU encoders and use Microsoft's software H.264 encoder.
    pub fn open(gpu: Arc<Gpu>, w: u32, h: u32, fps: u32, q: Quality, software_only: bool) -> Result<Self, String> {
        let mut rejected = Vec::new();
        let candidates: &[(&str, bool)] = if q == Quality::Archival {
            if software_only {return Err("Archival 4:4:4 requires a supported NVIDIA hardware encoder; choose a different quality explicitly for software encoding".into());}
            &[("h264_nvenc",true)]
        } else if software_only {
            rejected.push("GPU encoders: skipped (Software encoder chosen in Settings)".into());
            &[("h264_mf", false)]
        } else {
            &[("h264_nvenc", true), ("h264_amf", true), ("h264_mf", false)]
        };
        for &(name, hardware_input) in candidates {
            match unsafe { Self::try_open(&gpu, name, hardware_input, w, h, fps, q, software_only, false) } {
                Ok((ctx, input, hardware)) => {
                    let info = EncoderInfo { name: name.into(), hardware, width: w, height: h, fps, rejected };
                    return Ok(Self { ctx, input, gpu, pkt: unsafe { ff::av_packet_alloc() }, info, hdr:None, converted:None });
                }
                Err(e) => rejected.push(format!("{name}: {e}")),
            }
        }
        if q==Quality::Archival {return Err(format!("Archival 4:4:4 encoding could not start. No lower-quality fallback was used. {}",rejected.join("; ")));}
        if !software_only {
            rejected.push("h264_qsv: not implemented in this build".into());
        }
        Err(format!("No H.264 encoder could be started. {}", rejected.join("; ")))
    }

    pub fn open_tonemapped(gpu:Arc<Gpu>,w:u32,h:u32,fps:u32,q:Quality,software_only:bool,white:f32)->Result<Self,String> {
        if q==Quality::Archival {return Err("Archival 4:4:4 is an SDR-only advanced option. Choose High/Maximum with Compatible Recording for tone-mapped HDR, or Original HDR Recording for Main10 preservation.".into());}
        let converter=Arc::new(crate::hdr::Converter::new_sdr(&gpu,w,h,white)?);
        let mut enc=Self::open(gpu.clone(),w,h,fps,q,software_only)?;
        if matches!(enc.input,Input::Cpu{..}) {enc.converted=Some(crate::hdr::sdr_texture(&gpu,w,h)?);}
        enc.hdr=Some(converter);
        enc.info.name.push_str(" (HDR to SDR)");
        Ok(enc)
    }

    pub fn open_hdr(gpu:Arc<Gpu>,w:u32,h:u32,fps:u32,q:Quality,software_only:bool)->Result<Self,String> {
        if software_only || q==Quality::Archival {return Err("HDR Main10 requires NVIDIA hardware and a validated 4:2:0 quality mode. HDR archival 4:4:4 is not validated; no fallback was made.".into());}
        let converter=Arc::new(crate::hdr::Converter::new(&gpu,w,h)?);
        let(ctx,input,hardware)=unsafe{Self::try_open(&gpu,"hevc_nvenc",true,w,h,fps,q,false,true)}
            .map_err(|e|format!("Original HDR could not start NVIDIA HEVC Main10 encoding: {e}. No SDR conversion or lower-quality fallback was used."))?;
        Ok(Self{ctx,input,gpu,pkt:unsafe{ff::av_packet_alloc()},hdr:Some(converter),converted:None,info:EncoderInfo{name:"hevc_nvenc Main10 HDR (PQ)".into(),hardware,width:w,height:h,fps,rejected:vec![]}})
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
        hdr: bool,
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
            if hdr {(*ctx).color_primaries=ff::AVColorPrimaries::AVCOL_PRI_BT2020;(*ctx).color_trc=ff::AVColorTransferCharacteristic::AVCOL_TRC_SMPTE2084;(*ctx).colorspace=ff::AVColorSpace::AVCOL_SPC_BT2020_NCL;}
            let pd = (*ctx).priv_data;
            let input = if gpu_input {
                let frames = ff::av_hwframe_ctx_alloc(gpu.hw_device);
                let fc = (*frames).data as *mut ff::AVHWFramesContext;
                (*fc).format = ff::AVPixelFormat::AV_PIX_FMT_D3D11;
                (*fc).sw_format = if hdr {ff::AVPixelFormat::AV_PIX_FMT_P010LE}else{ff::AVPixelFormat::AV_PIX_FMT_BGRA};
                let d3d=(*fc).hwctx as *mut crate::av::AVD3D11VAFramesContext;(*d3d).bind_flags=(D3D11_BIND_RENDER_TARGET.0|D3D11_BIND_SHADER_RESOURCE.0) as u32;
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
                if sws.is_null() {
                    ff::avcodec_free_context(&mut ctx);
                    return Err("Could not set up colour conversion for the software encoder".into());
                }
                // BT.709 limited range, as the stream is tagged (swscale defaults to BT.601).
                ff::sws_setColorspaceDetails(
                    sws,
                    ff::sws_getCoefficients(ff::SWS_CS_DEFAULT),
                    1,
                    ff::sws_getCoefficients(ff::SWS_CS_ITU709),
                    0,
                    0,
                    1 << 16,
                    1 << 16,
                );
                Input::Cpu { staging: staging.unwrap(), sws }
            };
            match name {
                "h264_nvenc" | "hevc_nvenc" => {
                    let (preset, cq) = nvenc_preset(q, w, h, fps, r.cq);
                    av::opt(pd, "preset", &format!("p{preset}"))?;
                    av::opt(pd, "tune", "hq")?;
                    av::opt(pd, "rc", "vbr")?;
                    av::opt(pd, "cq", &cq.to_string())?;
                    av::opt(pd, "profile", if hdr {"main10"}else{"high"})?;
                    av::opt(pd, "spatial-aq", "1")?;
                    if q==Quality::Archival {
                        av::opt(pd,"tune","lossless")?; av::opt(pd,"rc","constqp")?;
                        av::opt(pd,"qp","0")?;av::opt(pd,"profile","high444p")?;
                        av::opt(pd,"rgb_mode","yuv444")?;av::opt(pd,"spatial-aq","0")?;
                        (*ctx).rc_max_rate=0;(*ctx).rc_buffer_size=0;
                    }
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

    /// What the clock thread needs to take pictures while this encoder runs on its own thread.
    pub fn grabber(&self) -> Grabber {
        let kind = match &self.input {
            Input::Gpu { frames } => GrabKind::Gpu(unsafe { ff::av_buffer_ref(*frames) }),
            Input::Cpu { staging, .. } => GrabKind::Cpu(staging.clone()),
        };
        Grabber { kind, gpu: self.gpu.clone(),hdr:self.hdr.clone(),converted:self.converted.clone() }
    }

    /// Encode a picture taken by [`Grabber::grab`] and hand finished packets to `out`.
    ///
    /// GPU input: the copy was queued (and flushed) when the picture was taken, typically a frame
    /// or more earlier, so NVENC/AMF rarely has to wait for it here. CPU input: the picture is in
    /// the staging texture; it is mapped (waiting for the copy) and converted now.
    pub fn encode(&mut self, g: Grab, out: &mut impl FnMut(Packet)) -> Result<(), String> {
        unsafe {
            let n = g.pts;
            let res = (|| -> Result<(), String> {
                let f = match &self.input {
                    Input::Gpu { .. } => {
                        if g.frame.is_null() {
                            return Err("picture taken without a GPU frame".into());
                        }
                        g.frame
                    }
                    Input::Cpu { staging, sws } => {
                        let ctx = &self.gpu.context;
                        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
                        ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).map_err(|e| e.to_string())?;
                        let f = g.frame;
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
                        f
                    }
                };
                (*f).pts = n;
                (*f).color_primaries=(*self.ctx).color_primaries;
                (*f).color_trc=(*self.ctx).color_trc;
                (*f).colorspace=(*self.ctx).colorspace;
                (*f).color_range=(*self.ctx).color_range;
                let t_send = crate::clock::now_100ns();
                check(ff::avcodec_send_frame(self.ctx, f), "encode")?;
                let t_sent = crate::clock::now_100ns();
                let r = self.drain(out);
                crate::trace::ev("encd", t_send, n, (t_sent - t_send) as i64, (crate::clock::now_100ns() - t_sent) as i64);
                r
            })();
            drop(g);
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

#[cfg(test)]
mod tests {
    use super::nvenc_preset;
    use capture_protocol::Quality;

    #[test]
    #[ignore="Explicit synthetic FP16 GPU HDR proof; no screen capture"]
    fn hdr_main10_gpu_color_roundtrip() {
        use super::*;
        let gpu=Arc::new(Gpu::create().unwrap());
        let mut encoder=VideoEncoder::open_hdr(gpu.clone(),256,144,60,Quality::Maximum,false).unwrap();
        let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-output").join(format!("hdr-proof-{}",crate::history::millis()));std::fs::create_dir_all(&root).unwrap();
        let bands=[[0.0,0.0,0.0],[1.0,1.0,1.0],[12.5,12.5,12.5],[4.0,0.1,0.1],[0.1,4.0,0.1],[0.1,0.1,4.0],[0.01,0.01,0.01],[2.5375,2.5375,2.5375]];
        let mut pixels=vec![0u16;256*144*4];for y in 0..144 {for x in 0..256 {let rgb=bands[x/32];let i=(y*256+x)*4;for c in 0..3{pixels[i+c]=crate::hdr::test_half(rgb[c] as f32);}pixels[i+3]=crate::hdr::test_half(1.0);}}
        let latest=crate::wgc::synthetic_hdr(&gpu,256,144,&pixels);
        let segment=root.join("hdr.ts");let streams=vec![unsafe{crate::mux::StreamDesc::from_ctx(encoder.codec_ctx())}];let mut writer=crate::mux::SegmentWriter::create(&segment,&streams).unwrap();
        for n in 0..60 {let grab=encoder.grabber().grab(&latest,n).unwrap();encoder.encode(grab,&mut |p|writer.write(p).unwrap()).unwrap();}encoder.flush(&mut |p|writer.write(p).unwrap());writer.finish();
        let clip=root.join("hdr.mp4");crate::save::remux(&[(segment,0.0)],None,0.0,0.0,1.0,&[],&clip).unwrap();
        let ffroot=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/ffmpeg/bin");
        let probe=std::process::Command::new(ffroot.join("ffprobe.exe")).args(["-v","error","-count_frames","-show_streams","-of","json"]).arg(&clip).output().unwrap();assert!(probe.status.success());
        let v:serde_json::Value=serde_json::from_slice(&probe.stdout).unwrap();let s=&v["streams"][0];println!("HDR stream: {s}");
        assert_eq!(s["pix_fmt"],"yuv420p10le");assert_eq!(s["color_primaries"],"bt2020");assert_eq!(s["color_transfer"],"smpte2084");assert_eq!(s["color_space"],"bt2020nc");assert_eq!(s["nb_read_frames"],"60");assert_eq!(s["profile"],"Main 10");
        let raw=std::process::Command::new(ffroot.join("ffmpeg.exe")).args(["-v","error","-i"]).arg(&clip).args(["-pix_fmt","yuv420p10le","-f","rawvideo","pipe:1"]).output().unwrap();assert!(raw.status.success());assert_eq!(raw.stdout.len(),256*144*3*60);
        let read=|offset:usize|u16::from_le_bytes([raw.stdout[offset*2],raw.stdout[offset*2+1]]) as f64;
        let mut max_error=0.0f64; for (n,rgb) in bands.into_iter().enumerate(){let rgb=crate::hdr::rgb2020(rgb).map(|v|crate::hdr::pq(v*80.0));let y=0.2627*rgb[0]+0.6780*rgb[1]+0.0593*rgb[2];let expected=[64.0+876.0*y,512.0+896.0*(rgb[2]-y)/(2.0*(1.0-0.0593)),512.0+896.0*(rgb[0]-y)/(2.0*(1.0-0.2627))];let x=n*32+16;let actual=[read(72*256+x),read(256*144+36*128+x/2),read(256*144+128*72+36*128+x/2)];println!("band {n}: expected {expected:?}, actual {actual:?}");for i in 0..3{max_error=max_error.max((actual[i]-expected[i]).abs());}}
        println!("HDR proof MP4 {}",clip.display());assert!(max_error<8.0,"HDR color conversion mismatch: {max_error}");
    }

    #[test]
    #[ignore = "Explicit NVIDIA D3D11 hardware proof using synthetic textures only"]
    fn archival_d3d11_encoder_preserves_native_frames_and_444() {
        use super::*;
        let gpu=Arc::new(Gpu::create().unwrap());
        let mut encoder=VideoEncoder::open(gpu.clone(),256,144,60,Quality::Archival,false).unwrap();
        assert!(encoder.info.hardware);assert_eq!(encoder.info.fps,60);
        let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-output").join(format!("d3d11-fidelity-{}",crate::history::millis()));std::fs::create_dir_all(&root).unwrap();
        let segment=root.join("synthetic.ts");
        let streams=vec![unsafe{crate::mux::StreamDesc::from_ctx(encoder.codec_ctx())}];
        let mut writer=crate::mux::SegmentWriter::create(&segment,&streams).unwrap();
        let mut reference=vec![];
        for n in 0..60 {
            let mut pixels=vec![0u8;256*144*4];for y in 0..144 {for x in 0..256 {let i=(y*256+x)*4;pixels[i]=(x+n) as u8;pixels[i+1]=(y+n) as u8;pixels[i+2]=(x/2+y/2+n) as u8;pixels[i+3]=255;}}
            let latest=crate::wgc::synthetic_latest(&gpu,256,144,&pixels);reference.extend_from_slice(&pixels);
            let frame=encoder.grabber().grab(&latest,n as i64).unwrap();
            encoder.encode(frame,&mut |p|writer.write(p).unwrap()).unwrap();
        }
        encoder.flush(&mut |p|writer.write(p).unwrap());writer.finish();
        let clip=root.join("archival.mp4");crate::save::remux(&[(segment,0.0)],None,0.0,0.0,1.0,&[],&clip).unwrap();
        let ffroot=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/ffmpeg/bin");
        let probe=std::process::Command::new(ffroot.join("ffprobe.exe")).args(["-v","error","-count_frames","-show_streams","-of","json"]).arg(&clip).output().unwrap();assert!(probe.status.success());
        let v:serde_json::Value=serde_json::from_slice(&probe.stdout).unwrap();assert_eq!(v["streams"][0]["pix_fmt"],"yuv444p");assert_eq!(v["streams"][0]["nb_read_frames"],"60");assert_eq!(v["streams"][0]["width"],256);assert_eq!(v["streams"][0]["height"],144);
        let decoded=std::process::Command::new(ffroot.join("ffmpeg.exe")).args(["-v","error","-i"]).arg(&clip).args(["-pix_fmt","bgra","-f","rawvideo","pipe:1"]).output().unwrap();assert!(decoded.status.success());assert_eq!(decoded.stdout.len(),reference.len());
        let max=reference.iter().zip(decoded.stdout.iter()).map(|(a,b)|a.abs_diff(*b)).max().unwrap();
        println!("D3D11 synthetic archival proof: 60/60 frames, 256x144@60, yuv444p; max RGB channel error {max}; output {} bytes",std::fs::metadata(&clip).unwrap().len());
        assert!(max<=3,"RGB conversion unexpectedly changes color by {max} levels");
    }

    #[test]
    fn preset_and_quality_are_never_silently_reduced() {
        for (w,h,fps) in [(1920,1080,60),(3840,2160,60),(3840,2160,120)] {
            assert_eq!(nvenc_preset(Quality::High,w,h,fps,21),(5,21));
            assert_eq!(nvenc_preset(Quality::Maximum,w,h,fps,18),(6,18));
        }
    }
}
