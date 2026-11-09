//! Turn protected ring segments into a standard MP4 and verify it before anyone calls it saved.
//!
//! - Stream copy only (no re-encode). Packets keep their absolute timeline timestamps, which
//!   are shifted so the clip starts at 0 on the first segment's keyframe.
//! - Output goes to `<name>.mp4.partial`, is probed, then atomically renamed.
//! - Track labels are written as `handler_name`; only `Preview Mix` is marked default.

use std::path::{Path, PathBuf};

use ffmpeg_sys_next as ff;
use std::ptr::{null, null_mut};

use crate::av::{self, check};
use crate::storage;

pub struct ClipResult {
    pub path: PathBuf,
    pub duration_secs: f64,
    pub bytes: u64,
    pub video_streams: u32,
    pub audio_tracks: Vec<String>,
}

struct Input(*mut ff::AVFormatContext);
impl Drop for Input {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.0) };
    }
}

unsafe fn open_input(p: &Path) -> Result<Input, String> {
    let mut ic = null_mut();
    check(ff::avformat_open_input(&mut ic, av::cstr(&p.to_string_lossy()).as_ptr(), null(), null_mut()), "open segment")?;
    let ic = Input(ic);
    check(ff::avformat_find_stream_info(ic.0, null_mut()), "read segment")?;
    Ok(ic)
}

/// `segments`: (path, timeline start secs) in order; the first starts on a keyframe at timeline
/// time `seg0_start`. The clip starts at the last keyframe at or before `want_start` (keyframes
/// are 1 s apart) and packets after `clip_end` (timeline secs) are dropped.
/// Returns the clip and the timeline time its first frame corresponds to.
pub fn remux(
    segments: &[(PathBuf, f64)],
    seg0_start: f64,
    want_start: f64,
    clip_end: f64,
    track_names: &[String],
    final_path: &Path,
) -> Result<(ClipResult, f64), String> {
    let partial = storage::partial_path(final_path);
    let r = unsafe { remux_inner(segments, seg0_start, want_start, clip_end, track_names, &partial) };
    let actual_start = match r {
        Ok(t) => t,
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
    };
    let verified = verify(&partial, track_names.len() as u32);
    let res = match verified {
        Ok(mut v) => {
            if let Err(e) = std::fs::rename(&partial, final_path) {
                let _ = std::fs::remove_file(&partial);
                return Err(format!("Could not finalize clip: {e}"));
            }
            v.path = final_path.to_path_buf();
            Ok((v, actual_start))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    };
    res
}

/// Video keyframe pts (stream tb) in a segment, in order.
unsafe fn keyframes(path: &Path, video_idx: usize) -> Vec<i64> {
    let Ok(ic) = open_input(path) else { return vec![] };
    let pkt = ff::av_packet_alloc();
    let mut out = Vec::new();
    while ff::av_read_frame(ic.0, pkt) >= 0 {
        if (*pkt).stream_index as usize == video_idx && (*pkt).flags & ff::AV_PKT_FLAG_KEY != 0 && (*pkt).pts != ff::AV_NOPTS_VALUE {
            out.push((*pkt).pts);
        }
        ff::av_packet_unref(pkt);
    }
    let mut p = pkt;
    ff::av_packet_free(&mut p);
    out
}

/// Session-timeline time (seconds) of the first video keyframe in a buffer segment. Segments
/// keep absolute timeline timestamps, so this places a recovered clip on the session timeline.
pub fn first_keyframe_secs(path: &Path) -> Option<f64> {
    unsafe {
        let ic = open_input(path).ok()?;
        let n = (*ic.0).nb_streams as usize;
        let v = (0..n).find(|&i| (*(*(*(*ic.0).streams.add(i))).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO)?;
        let tb = (*(*(*ic.0).streams.add(v))).time_base;
        drop(ic);
        let k = *keyframes(path, v).first()?;
        Some(k as f64 * tb.num as f64 / tb.den as f64)
    }
}

/// Pure choice of the cut point (unit-tested): the last keyframe at or before `want`, else the
/// first keyframe.
pub fn pick_origin(keys: &[i64], want: i64) -> Option<i64> {
    keys.iter().copied().filter(|&k| k <= want).max().or_else(|| keys.first().copied())
}

unsafe fn remux_inner(segments: &[(PathBuf, f64)], seg0_start: f64, want_start: f64, clip_end: f64, names: &[String], out: &Path) -> Result<f64, String> {
    let first = segments.first().ok_or("No footage in the buffer yet")?;
    let ic0 = open_input(&first.0)?;
    let n = (*ic0.0).nb_streams as usize;

    let cpath = av::cstr(&out.to_string_lossy());
    let mut oc = null_mut();
    check(ff::avformat_alloc_output_context2(&mut oc, null(), av::cstr("mp4").as_ptr(), cpath.as_ptr()), "mp4 ctx")?;
    struct Out(*mut ff::AVFormatContext);
    impl Drop for Out {
        fn drop(&mut self) {
            unsafe {
                if !(*self.0).pb.is_null() {
                    ff::avio_closep(&mut (*self.0).pb);
                }
                ff::avformat_free_context(self.0);
            }
        }
    }
    let oc = Out(oc);
    let mut audio_i = 0usize;
    for i in 0..n {
        let ist = *(*ic0.0).streams.add(i);
        let ost = ff::avformat_new_stream(oc.0, null());
        check(ff::avcodec_parameters_copy((*ost).codecpar, (*ist).codecpar), "codecpar")?;
        (*(*ost).codecpar).codec_tag = 0;
        (*ost).time_base = (*ist).time_base;
        let is_video = (*(*ist).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO;
        // Some encoders (Media Foundation) write no colour information into the H.264 stream.
        // Untagged HD video is read as BT.601 by some programs and BT.709 by others, so say what
        // CAPTURE encodes: BT.709, limited range (encoders that do tag, e.g. NVENC, keep theirs).
        let cp = (*ost).codecpar;
        if is_video && (*cp).color_space == ff::AVColorSpace::AVCOL_SPC_UNSPECIFIED {
            (*cp).color_space = ff::AVColorSpace::AVCOL_SPC_BT709;
            (*cp).color_primaries = ff::AVColorPrimaries::AVCOL_PRI_BT709;
            (*cp).color_trc = ff::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            (*cp).color_range = ff::AVColorRange::AVCOL_RANGE_MPEG;
        }
        let label = if is_video {
            "Video".to_string()
        } else {
            let l = names.get(audio_i).cloned().unwrap_or_else(|| format!("Audio {}", audio_i + 1));
            audio_i += 1;
            l
        };
        av::dict_set(&mut (*ost).metadata, "handler_name", &label);
        (*ost).disposition = if is_video || audio_i == 1 { ff::AV_DISPOSITION_DEFAULT } else { 0 };
    }
    // Identifies the file as CAPTURE's own (the uninstaller's "remove all data" option deletes
    // only files it can verify; see app/src-tauri/src/owned.rs).
    av::dict_set(&mut (*oc.0).metadata, "comment", "Recorded with CAPTURE");
    check(ff::avio_open(&mut (*oc.0).pb, cpath.as_ptr(), ff::AVIO_FLAG_WRITE), "create clip file")?;
    let mut opts = null_mut();
    av::dict_set(&mut opts, "movflags", "+faststart");
    let r = ff::avformat_write_header(oc.0, &mut opts);
    ff::av_dict_free(&mut opts);
    check(r, "clip header")?;

    // Origin: pts of the first video packet in the first segment (a keyframe by construction).
    let video_idx = (0..n)
        .find(|&i| (*(*(*(*ic0.0).streams.add(i))).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO)
        .ok_or("Buffer has no video stream")?;
    let vtb = (*(*(*ic0.0).streams.add(video_idx))).time_base;
    drop(ic0);
    let secs_to_tb = |s: f64| (s * vtb.den as f64 / vtb.num as f64).round() as i64;
    let keys = keyframes(&first.0, video_idx);
    let p0 = *keys.first().ok_or("Buffer contained no video frames")?;
    let want_pts = p0 + secs_to_tb((want_start - seg0_start).max(0.0));
    let origin_pts = pick_origin(&keys, want_pts).unwrap_or(p0);
    let actual_start = seg0_start + (origin_pts - p0) as f64 * vtb.num as f64 / vtb.den as f64;
    let end_pts = p0 + secs_to_tb(clip_end - seg0_start);
    let mut origin: Option<i64> = Some(origin_pts); // in video stream tb
    let mut last_dts = vec![i64::MIN; n];
    let pkt = ff::av_packet_alloc();
    struct PktGuard(*mut ff::AVPacket);
    impl Drop for PktGuard {
        fn drop(&mut self) {
            unsafe { ff::av_packet_free(&mut self.0) };
        }
    }
    let _pg = PktGuard(pkt);
    let span = (end_pts - origin_pts) as f64 * vtb.num as f64 / vtb.den as f64;
    for (path, _) in segments {
        let ic = match open_input(path) {
            Ok(ic) => ic,
            Err(_) => continue, // a damaged tail segment must not sink the whole clip
        };
        if (*ic.0).nb_streams as usize != n {
            return Err("Buffer segments changed layout mid-clip".into());
        }
        while ff::av_read_frame(ic.0, pkt) >= 0 {
            let si = (*pkt).stream_index as usize;
            let ist = *(*ic.0).streams.add(si);
            if (*pkt).pts == ff::AV_NOPTS_VALUE {
                ff::av_packet_unref(pkt);
                continue;
            }
            let Some(o) = origin else {
                ff::av_packet_unref(pkt);
                continue;
            };
            let o_in = ff::av_rescale_q(o, vtb, (*ist).time_base);
            let rel = ((*pkt).pts - o_in) as f64 * (*ist).time_base.num as f64 / (*ist).time_base.den as f64;
            if rel < 0.0 || rel > span {
                ff::av_packet_unref(pkt);
                continue;
            }
            (*pkt).pts -= o_in;
            if (*pkt).dts != ff::AV_NOPTS_VALUE {
                (*pkt).dts -= o_in;
            } else {
                (*pkt).dts = (*pkt).pts;
            }
            if (*pkt).dts <= last_dts[si] {
                ff::av_packet_unref(pkt);
                continue;
            }
            last_dts[si] = (*pkt).dts;
            let ost = *(*oc.0).streams.add(si);
            ff::av_packet_rescale_ts(pkt, (*ist).time_base, (*ost).time_base);
            (*pkt).pos = -1;
            check(ff::av_interleaved_write_frame(oc.0, pkt), "write clip")?;
        }
    }
    origin.take();
    check(ff::av_write_trailer(oc.0), "finish clip")?;
    Ok(actual_start)
}

#[cfg(test)]
mod tests {
    #[test]
    fn cut_is_the_last_keyframe_not_after_the_request() {
        let keys = [1000, 91000, 181000];
        assert_eq!(super::pick_origin(&keys, 100000), Some(91000));
        assert_eq!(super::pick_origin(&keys, 91000), Some(91000));
        assert_eq!(super::pick_origin(&keys, 500), Some(1000));
        assert_eq!(super::pick_origin(&[], 5), None);
    }
}

/// Independent re-open of the written file: stream layout, labels, duration, and that the
/// first video packet is a keyframe. Nothing is called "Saved" unless this passes.
pub fn verify(path: &Path, expected_audio: u32) -> Result<ClipResult, String> {
    unsafe {
        let mut ic = null_mut();
        check(ff::avformat_open_input(&mut ic, av::cstr(&path.to_string_lossy()).as_ptr(), null(), null_mut()), "verify open")?;
        let ic = Input(ic);
        check(ff::avformat_find_stream_info(ic.0, null_mut()), "verify streams")?;
        let mut video = 0;
        let mut audio = Vec::new();
        for i in 0..(*ic.0).nb_streams as usize {
            let st = *(*ic.0).streams.add(i);
            match (*(*st).codecpar).codec_type {
                ff::AVMediaType::AVMEDIA_TYPE_VIDEO => video += 1,
                ff::AVMediaType::AVMEDIA_TYPE_AUDIO => {
                    let e = ff::av_dict_get((*st).metadata, av::cstr("handler_name").as_ptr(), null(), 0);
                    audio.push(if e.is_null() { String::new() } else { std::ffi::CStr::from_ptr((*e).value).to_string_lossy().into_owned() });
                }
                _ => {}
            }
        }
        if video != 1 {
            return Err(format!("Clip check failed: expected 1 video stream, found {video}"));
        }
        if audio.len() as u32 != expected_audio {
            return Err(format!("Clip check failed: expected {expected_audio} audio tracks, found {}", audio.len()));
        }
        let duration = (*ic.0).duration as f64 / ff::AV_TIME_BASE as f64;
        if !(duration > 0.0) {
            return Err("Clip check failed: zero duration".into());
        }
        let pkt = ff::av_packet_alloc();
        let mut first_key = None;
        while ff::av_read_frame(ic.0, pkt) >= 0 {
            let st = *(*ic.0).streams.add((*pkt).stream_index as usize);
            if (*(*st).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO {
                first_key = Some((*pkt).flags & ff::AV_PKT_FLAG_KEY != 0);
                ff::av_packet_unref(pkt);
                break;
            }
            ff::av_packet_unref(pkt);
        }
        let mut p = pkt;
        ff::av_packet_free(&mut p);
        if first_key != Some(true) {
            return Err("Clip check failed: video does not start on a keyframe".into());
        }
        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        Ok(ClipResult { path: path.to_path_buf(), duration_secs: duration, bytes, video_streams: video, audio_tracks: audio })
    }
}
