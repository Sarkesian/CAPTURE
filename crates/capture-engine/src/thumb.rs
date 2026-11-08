//! Clip thumbnails + duration for the recent-clips list.
//!
//! Decodes one video frame ~1 s into the clip, scales it to 320 px wide and writes a JPEG to
//! `<root>\Data\thumbs\` (a rebuildable cache keyed by file name, size and modification time;
//! nothing is written next to the user's clips).

use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use ffmpeg_sys_next as ff;

use crate::av::{self, check};

pub struct ClipInfo {
    pub thumb: PathBuf,
    pub duration_secs: f64,
}

pub fn thumbs_dir(root: &Path) -> PathBuf {
    root.join("Data").join("thumbs")
}

fn cache_name(clip: &Path) -> Option<String> {
    let m = std::fs::metadata(clip).ok()?;
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let stem = clip.file_stem()?.to_string_lossy();
    Some(format!("{stem}_{}_{mtime}.jpg", m.len()))
}

pub fn clip_info(root: &Path, clip: &Path) -> Result<ClipInfo, String> {
    let dir = thumbs_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(cache_name(clip).ok_or("Clip not found")?);
    let duration_secs = unsafe { make(clip, &out, !out.exists())? };
    Ok(ClipInfo { thumb: out, duration_secs })
}

struct Fmt(*mut ff::AVFormatContext);
impl Drop for Fmt {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.0) };
    }
}
struct Ctx(*mut ff::AVCodecContext);
impl Drop for Ctx {
    fn drop(&mut self) {
        unsafe { ff::avcodec_free_context(&mut self.0) };
    }
}
struct Frame(*mut ff::AVFrame);
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.0) };
    }
}
struct Pkt(*mut ff::AVPacket);
impl Drop for Pkt {
    fn drop(&mut self) {
        unsafe { ff::av_packet_free(&mut self.0) };
    }
}

/// Returns the clip duration; writes the thumbnail only when `write` is set.
unsafe fn make(clip: &Path, out: &Path, write: bool) -> Result<f64, String> {
    let mut ic = null_mut();
    check(ff::avformat_open_input(&mut ic, av::cstr(&clip.to_string_lossy()).as_ptr(), null(), null_mut()), "open clip")?;
    let ic = Fmt(ic);
    check(ff::avformat_find_stream_info(ic.0, null_mut()), "read clip")?;
    let duration = (*ic.0).duration as f64 / ff::AV_TIME_BASE as f64;
    if !write {
        return Ok(duration);
    }
    let mut dec = null();
    let vi = check(ff::av_find_best_stream(ic.0, ff::AVMediaType::AVMEDIA_TYPE_VIDEO, -1, -1, &mut dec, 0), "find video")? as usize;
    let st = *(*ic.0).streams.add(vi);
    let dctx = Ctx(ff::avcodec_alloc_context3(dec));
    ff::avcodec_parameters_to_context(dctx.0, (*st).codecpar);
    check(ff::avcodec_open2(dctx.0, dec, null_mut()), "open decoder")?;
    if duration > 2.0 {
        let ts = ff::av_rescale_q(ff::AV_TIME_BASE as i64, ff::AVRational { num: 1, den: ff::AV_TIME_BASE as i32 }, (*st).time_base);
        ff::av_seek_frame(ic.0, vi as i32, ts, ff::AVSEEK_FLAG_BACKWARD);
    }
    let pkt = Pkt(ff::av_packet_alloc());
    let frame = Frame(ff::av_frame_alloc());
    let mut got = false;
    while !got && ff::av_read_frame(ic.0, pkt.0) >= 0 {
        if (*pkt.0).stream_index as usize == vi {
            ff::avcodec_send_packet(dctx.0, pkt.0);
            got = ff::avcodec_receive_frame(dctx.0, frame.0) >= 0;
        }
        ff::av_packet_unref(pkt.0);
    }
    if !got {
        ff::avcodec_send_packet(dctx.0, null());
        got = ff::avcodec_receive_frame(dctx.0, frame.0) >= 0;
    }
    if !got {
        return Err("No video frame could be decoded".into());
    }
    let (sw, sh) = ((*frame.0).width, (*frame.0).height);
    let tw = 320;
    let th = ((sh as i64 * tw as i64 / sw.max(1) as i64) as i32) & !1;
    let sws = ff::sws_getContext(
        sw,
        sh,
        std::mem::transmute::<i32, ff::AVPixelFormat>((*frame.0).format),
        tw,
        th,
        ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P,
        ff::SwsFlags::SWS_BICUBIC as i32,
        null_mut(),
        null_mut(),
        null(),
    );
    if sws.is_null() {
        return Err("Could not scale the thumbnail".into());
    }
    let small = Frame(ff::av_frame_alloc());
    (*small.0).format = ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P as i32;
    (*small.0).width = tw;
    (*small.0).height = th;
    check(ff::av_frame_get_buffer(small.0, 0), "thumb frame")?;
    ff::sws_scale(sws, (*frame.0).data.as_ptr() as *const *const u8, (*frame.0).linesize.as_ptr(), 0, sh, (*small.0).data.as_ptr(), (*small.0).linesize.as_ptr());
    ff::sws_freeContext(sws);

    let enc = ff::avcodec_find_encoder(ff::AVCodecID::AV_CODEC_ID_MJPEG);
    if enc.is_null() {
        return Err("JPEG encoder missing".into());
    }
    let ectx = Ctx(ff::avcodec_alloc_context3(enc));
    (*ectx.0).width = tw;
    (*ectx.0).height = th;
    (*ectx.0).pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P;
    (*ectx.0).time_base = ff::AVRational { num: 1, den: 25 };
    (*ectx.0).flags |= ff::AV_CODEC_FLAG_QSCALE as i32;
    (*ectx.0).global_quality = ff::FF_QP2LAMBDA as i32 * 4;
    check(ff::avcodec_open2(ectx.0, enc, null_mut()), "open JPEG encoder")?;
    (*small.0).pts = 0;
    (*small.0).quality = (*ectx.0).global_quality;
    check(ff::avcodec_send_frame(ectx.0, small.0), "encode thumb")?;
    let jp = Pkt(ff::av_packet_alloc());
    check(ff::avcodec_receive_packet(ectx.0, jp.0), "thumb packet")?;
    let bytes = std::slice::from_raw_parts((*jp.0).data, (*jp.0).size as usize);
    let tmp = out.with_extension("jpg.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())?;
    Ok(duration)
}
