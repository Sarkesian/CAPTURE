//! Small pictures of recorded footage for the timeline (hover previews, library cards).
//!
//! One frame is decoded straight from the recording segment that holds the asked-for moment:
//! either the keyframe at or before it (cheap: one frame) or that very frame (decodes forward
//! from the keyframe, at most one keyframe interval). Nothing is written to disk; the caller
//! keeps a bounded in-memory cache of the JPEGs.

use std::path::Path;
use std::ptr::{null, null_mut};

use ffmpeg_sys_next as ff;

use crate::av::check;

struct Input(*mut ff::AVFormatContext);
impl Drop for Input {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.0) };
    }
}
struct Codec(*mut ff::AVCodecContext);
impl Drop for Codec {
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
struct Packets(Vec<*mut ff::AVPacket>);
impl Packets {
    fn clear(&mut self) {
        for p in self.0.drain(..) {
            let mut p = p;
            unsafe { ff::av_packet_free(&mut p) };
        }
    }
}
impl Drop for Packets {
    fn drop(&mut self) {
        self.clear();
    }
}

/// The frame of `segment` at timeline time `target` as a JPEG at most `max_width` wide, with
/// the timeline time of the frame actually shown. `segment_start` is the timeline time of the
/// segment's first keyframe. `exact = false` gives the keyframe at or before `target`.
pub fn frame_jpeg(segment: &Path, segment_start: f64, target: f64, exact: bool, max_width: i32) -> Result<(f64, Vec<u8>), String> {
    unsafe { frame_jpeg_inner(segment, segment_start, target, exact, max_width) }
}

unsafe fn frame_jpeg_inner(segment: &Path, segment_start: f64, target: f64, exact: bool, max_width: i32) -> Result<(f64, Vec<u8>), String> {
    let mut ic = null_mut();
    check(ff::avformat_open_input(&mut ic, crate::av::cstr(&segment.to_string_lossy()).as_ptr(), null(), null_mut()), "open recording")?;
    let ic = Input(ic);
    check(ff::avformat_find_stream_info(ic.0, null_mut()), "read recording")?;
    let streams = std::slice::from_raw_parts((*ic.0).streams, (*ic.0).nb_streams as usize);
    let index = streams
        .iter()
        .position(|s| (*(**s).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO)
        .ok_or("This recording has no picture")?;
    let stream = streams[index];
    let tb = (*stream).time_base;
    let secs = tb.num as f64 / tb.den as f64;

    // Gather the packets from the last keyframe at or before the target up to the target frame.
    let mut wanted = Packets(Vec::new());
    let mut origin: Option<i64> = None;
    let mut pkt = ff::av_packet_alloc();
    let time_of = |pts: i64, origin: i64| segment_start + (pts - origin) as f64 * secs;
    while ff::av_read_frame(ic.0, pkt) >= 0 {
        if (*pkt).stream_index as usize == index && (*pkt).pts != ff::AV_NOPTS_VALUE {
            let key = (*pkt).flags & ff::AV_PKT_FLAG_KEY != 0;
            if origin.is_none() && key {
                origin = Some((*pkt).pts);
            }
            if let Some(o) = origin {
                let t = time_of((*pkt).pts, o);
                if key && (t <= target + 0.000_5 || wanted.0.is_empty()) {
                    wanted.clear();
                    wanted.0.push(ff::av_packet_clone(pkt));
                } else if key {
                    ff::av_packet_unref(pkt);
                    break; // the next keyframe is after the target
                } else if exact && !wanted.0.is_empty() && t <= target + 0.25 {
                    wanted.0.push(ff::av_packet_clone(pkt));
                } else if t > target + 0.25 {
                    ff::av_packet_unref(pkt);
                    break;
                }
            }
        }
        ff::av_packet_unref(pkt);
    }
    ff::av_packet_free(&mut pkt);
    let origin = origin.ok_or("This recording has no keyframe here")?;
    if wanted.0.is_empty() {
        return Err("No picture at this position".into());
    }

    let par = (*stream).codecpar;
    let decoder = ff::avcodec_find_decoder((*par).codec_id);
    if decoder.is_null() {
        return Err("No decoder for this recording".into());
    }
    let dec = Codec(ff::avcodec_alloc_context3(decoder));
    check(ff::avcodec_parameters_to_context(dec.0, par), "decoder setup")?;
    // A preview does not need the in-loop filter; skipping it roughly halves the 4K decode cost.
    (*dec.0).skip_loop_filter = ff::AVDiscard::AVDISCARD_ALL;
    (*dec.0).flags2 |= ff::AV_CODEC_FLAG2_FAST as i32;
    (*dec.0).thread_count = 0;
    (*dec.0).thread_type = if wanted.0.len() > 1 { ff::FF_THREAD_FRAME as i32 | ff::FF_THREAD_SLICE as i32 } else { ff::FF_THREAD_SLICE as i32 };
    check(ff::avcodec_open2(dec.0, decoder, null_mut()), "open decoder")?;

    // Decode in order, keeping the last frame at or before the target (the first one otherwise).
    let frame = Frame(ff::av_frame_alloc());
    let best = Frame(ff::av_frame_alloc());
    let mut best_t: Option<f64> = None;
    let mut done = false;
    let take = |frame: &Frame, best_t: &mut Option<f64>, done: &mut bool| {
        while !*done && ff::avcodec_receive_frame(dec.0, frame.0) >= 0 {
            let pts = if (*frame.0).pts != ff::AV_NOPTS_VALUE { (*frame.0).pts } else { (*frame.0).best_effort_timestamp };
            let t = time_of(pts, origin);
            if best_t.is_none() || t <= target + 0.000_5 {
                ff::av_frame_unref(best.0);
                ff::av_frame_move_ref(best.0, frame.0);
                *best_t = Some(t);
            } else {
                *done = true;
            }
            ff::av_frame_unref(frame.0);
        }
    };
    for &p in &wanted.0 {
        if done {
            break;
        }
        if ff::avcodec_send_packet(dec.0, p) >= 0 {
            take(&frame, &mut best_t, &mut done);
        }
    }
    if !done {
        ff::avcodec_send_packet(dec.0, null());
        take(&frame, &mut best_t, &mut done);
    }
    let shown = best_t.ok_or("This moment could not be decoded")?;
    let jpeg = encode_jpeg(best.0, max_width)?;
    Ok((shown, jpeg))
}

unsafe fn encode_jpeg(src: *mut ff::AVFrame, max_width: i32) -> Result<Vec<u8>, String> {
    let (w, h) = ((*src).width, (*src).height);
    if w <= 0 || h <= 0 {
        return Err("Empty picture".into());
    }
    let out_w = (w.min(max_width.max(16)) / 2 * 2).max(2);
    let out_h = (((h as i64 * out_w as i64 + w as i64 / 2) / w as i64) as i32 / 2 * 2).max(2);
    let src_format: ff::AVPixelFormat = std::mem::transmute((*src).format);
    let sws = ff::sws_getContext(w, h, src_format, out_w, out_h, ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P, ff::SwsFlags::SWS_BILINEAR as i32, null_mut(), null_mut(), null());
    if sws.is_null() {
        return Err("Could not scale the preview picture".into());
    }
    // CAPTURE records BT.709 limited range (also what an untagged stream of its own means);
    // a stream tagged otherwise is converted as tagged. JPEG is BT.601 full range.
    let matrix = match (*src).colorspace {
        ff::AVColorSpace::AVCOL_SPC_BT470BG | ff::AVColorSpace::AVCOL_SPC_SMPTE170M => ff::SWS_CS_ITU601,
        _ => ff::SWS_CS_ITU709,
    };
    let full_range = ((*src).color_range == ff::AVColorRange::AVCOL_RANGE_JPEG) as i32;
    ff::sws_setColorspaceDetails(sws, ff::sws_getCoefficients(matrix), full_range, ff::sws_getCoefficients(ff::SWS_CS_DEFAULT), 1, 0, 1 << 16, 1 << 16);
    let out = Frame(ff::av_frame_alloc());
    (*out.0).format = ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P as i32;
    (*out.0).width = out_w;
    (*out.0).height = out_h;
    let scaled = check(ff::av_frame_get_buffer(out.0, 0), "preview picture").map(|_| {
        ff::sws_scale(sws, (*src).data.as_ptr() as *const *const u8, (*src).linesize.as_ptr(), 0, h, (*out.0).data.as_ptr(), (*out.0).linesize.as_ptr());
    });
    ff::sws_freeContext(sws);
    scaled?;

    let codec = ff::avcodec_find_encoder(ff::AVCodecID::AV_CODEC_ID_MJPEG);
    if codec.is_null() {
        return Err("JPEG encoder missing".into());
    }
    let enc = Codec(ff::avcodec_alloc_context3(codec));
    (*enc.0).width = out_w;
    (*enc.0).height = out_h;
    (*enc.0).pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P;
    (*enc.0).color_range = ff::AVColorRange::AVCOL_RANGE_JPEG;
    (*enc.0).time_base = ff::AVRational { num: 1, den: 25 };
    (*enc.0).flags |= ff::AV_CODEC_FLAG_QSCALE as i32;
    (*enc.0).global_quality = ff::FF_QP2LAMBDA as i32 * 5;
    check(ff::avcodec_open2(enc.0, codec, null_mut()), "open JPEG encoder")?;
    (*out.0).quality = (*enc.0).global_quality;
    (*out.0).pts = 0;
    check(ff::avcodec_send_frame(enc.0, out.0), "encode preview")?;
    let mut pkt = ff::av_packet_alloc();
    let bytes = check(ff::avcodec_receive_packet(enc.0, pkt), "preview picture").map(|_| std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize).to_vec());
    ff::av_packet_free(&mut pkt);
    bytes
}

/// `data:image/jpeg;base64,...` (no dependency for 20 lines of base64).
pub fn data_url(jpeg: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(24 + jpeg.len().div_ceil(3) * 4);
    out.push_str("data:image/jpeg;base64,");
    for c in jpeg.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(super::data_url(b"Man"), "data:image/jpeg;base64,TWFu");
        assert_eq!(super::data_url(b"Ma"), "data:image/jpeg;base64,TWE=");
        assert_eq!(super::data_url(b"M"), "data:image/jpeg;base64,TQ==");
    }
}
