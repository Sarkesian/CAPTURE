//! Phase 0 spike: prove that one Rust process can link the LGPL FFmpeg 8.1 shared libraries,
//! create a D3D11 hardware frames pool, hand GPU textures straight to a hardware encoder
//! (no CPU readback), and mux the result to MP4.
//!
//! `libav_spike <out.mp4> [encoder ...]` tries each encoder in order (default: h264_nvenc,
//! h264_amf, h264_qsv, h264_mf) and reports which one opened and how fast it encoded.

use ffmpeg_sys_next as ff;
use std::ffi::{CStr, CString};
use std::ptr::{null, null_mut};
use std::time::Instant;

const W: i32 = 1920;
const H: i32 = 1080;
const FPS: i32 = 60;
const FRAMES: i64 = 300;

fn err(code: i32) -> String {
    let mut buf = [0i8; 256];
    unsafe { ff::av_strerror(code, buf.as_mut_ptr(), buf.len()) };
    unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

macro_rules! check {
    ($e:expr, $what:expr) => {{
        let r = $e;
        if r < 0 {
            return Err(format!("{}: {}", $what, err(r)));
        }
        r
    }};
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().unwrap_or_else(|| "libav_spike.mp4".into());
    let list: Vec<String> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        ["h264_nvenc", "h264_amf", "h264_qsv", "h264_mf"].map(String::from).to_vec()
    };
    unsafe { ff::av_log_set_level(ff::AV_LOG_ERROR) };
    for enc in &list {
        match unsafe { encode(enc, &out) } {
            Ok(msg) => {
                println!("{{\"encoder\":{enc:?},\"ok\":true,{msg}}}");
                return;
            }
            Err(e) => println!("{{\"encoder\":{enc:?},\"ok\":false,\"error\":{e:?}}}"),
        }
    }
    std::process::exit(1);
}

unsafe fn encode(name: &str, out: &str) -> Result<String, String> {
    // GPU device + frame pool of D3D11 BGRA textures (what WGC hands us).
    let mut dev: *mut ff::AVBufferRef = null_mut();
    check!(
        ff::av_hwdevice_ctx_create(&mut dev, ff::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA, null(), null_mut(), 0),
        "d3d11 device"
    );
    let frames_ref = ff::av_hwframe_ctx_alloc(dev);
    let fc = (*frames_ref).data as *mut ff::AVHWFramesContext;
    (*fc).format = ff::AVPixelFormat::AV_PIX_FMT_D3D11;
    (*fc).sw_format = ff::AVPixelFormat::AV_PIX_FMT_BGRA;
    (*fc).width = W;
    (*fc).height = H;
    (*fc).initial_pool_size = 8;
    check!(ff::av_hwframe_ctx_init(frames_ref), "hw frames init");

    let cname = CString::new(name).unwrap();
    let codec = ff::avcodec_find_encoder_by_name(cname.as_ptr());
    if codec.is_null() {
        return Err("encoder not compiled in".into());
    }
    let ctx = ff::avcodec_alloc_context3(codec);
    (*ctx).width = W;
    (*ctx).height = H;
    (*ctx).time_base = ff::AVRational { num: 1, den: FPS };
    (*ctx).framerate = ff::AVRational { num: FPS, den: 1 };
    (*ctx).gop_size = FPS * 2;
    (*ctx).max_b_frames = 0;
    (*ctx).pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_D3D11;
    (*ctx).hw_frames_ctx = ff::av_buffer_ref(frames_ref);
    (*ctx).color_primaries = ff::AVColorPrimaries::AVCOL_PRI_BT709;
    (*ctx).color_trc = ff::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
    (*ctx).colorspace = ff::AVColorSpace::AVCOL_SPC_BT709;
    (*ctx).color_range = ff::AVColorRange::AVCOL_RANGE_MPEG;
    (*ctx).flags |= ff::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
    check!(ff::avcodec_open2(ctx, codec, null_mut()), "avcodec_open2");

    let cout = CString::new(out).unwrap();
    let mut oc: *mut ff::AVFormatContext = null_mut();
    check!(ff::avformat_alloc_output_context2(&mut oc, null(), null(), cout.as_ptr()), "output ctx");
    let st = ff::avformat_new_stream(oc, null());
    check!(ff::avcodec_parameters_from_context((*st).codecpar, ctx), "codecpar");
    (*st).time_base = (*ctx).time_base;
    check!(ff::avio_open(&mut (*oc).pb, cout.as_ptr(), ff::AVIO_FLAG_WRITE), "avio_open");
    check!(ff::avformat_write_header(oc, null_mut()), "write_header");

    let pkt = ff::av_packet_alloc();
    let t = Instant::now();
    let mut packets = 0u64;
    let mut drain = |flush: bool| -> Result<(), String> {
        if flush {
            ff::avcodec_send_frame(ctx, null());
        }
        loop {
            let r = ff::avcodec_receive_packet(ctx, pkt);
            if r == ff::AVERROR(ff::EAGAIN) || r == ff::AVERROR_EOF {
                return Ok(());
            }
            check!(r, "receive_packet");
            ff::av_packet_rescale_ts(pkt, (*ctx).time_base, (*st).time_base);
            (*pkt).stream_index = 0;
            check!(ff::av_interleaved_write_frame(oc, pkt), "write_frame");
            packets += 1;
        }
    };
    for i in 0..FRAMES {
        let f = ff::av_frame_alloc();
        check!(ff::av_hwframe_get_buffer(frames_ref, f, 0), "hwframe_get_buffer");
        (*f).pts = i;
        check!(ff::avcodec_send_frame(ctx, f), "send_frame");
        let mut f = f;
        ff::av_frame_free(&mut f);
        drain(false)?;
    }
    drain(true)?;
    let secs = t.elapsed().as_secs_f64();
    check!(ff::av_write_trailer(oc), "trailer");
    ff::avio_closep(&mut (*oc).pb);
    ff::avformat_free_context(oc);
    let mut ctx = ctx;
    ff::avcodec_free_context(&mut ctx);
    let mut p = pkt;
    ff::av_packet_free(&mut p);
    let mut fr = frames_ref;
    ff::av_buffer_unref(&mut fr);
    ff::av_buffer_unref(&mut dev);
    Ok(format!(
        "\"frames\":{FRAMES},\"packets\":{packets},\"encode_fps\":{:.0},\"input\":\"d3d11 texture (zero-copy)\",\"out\":{out:?}",
        FRAMES as f64 / secs
    ))
}
