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
    lead_in: Option<&Path>,
    seg0_start: f64,
    want_start: f64,
    clip_end: f64,
    track_names: &[String],
    final_path: &Path,
) -> Result<(ClipResult, f64), String> {
    write_clip(segments, lead_in, seg0_start, want_start..clip_end, track_names, final_path, false, true)
}

/// A playback window for Replay: the same stream copy, written once (no second pass moving the
/// index to the front, which a local player does not need and which doubles the disk work).
pub fn remux_preview(
    segments: &[(PathBuf, f64)],
    lead_in: Option<&Path>,
    seg0_start: f64,
    clip_end: f64,
    track_names: &[String],
    final_path: &Path,
) -> Result<(ClipResult, f64), String> {
    write_clip(segments, lead_in, seg0_start, seg0_start..clip_end, track_names, final_path, false, false)
}

pub fn remux_aligned(
    segments: &[(PathBuf, f64)], lead_in: Option<&Path>, range: std::ops::Range<f64>,
    track_names: &[String], final_path: &Path,
) -> Result<(ClipResult, f64), String> {
    let segment_start = segments.first().ok_or("No recorded footage in this range")?.1;
    write_clip(segments, lead_in, segment_start, range, track_names, final_path, true, true)
}

fn write_clip(
    segments: &[(PathBuf, f64)], lead_in: Option<&Path>, seg0_start: f64,
    range: std::ops::Range<f64>, track_names: &[String], final_path: &Path, aligned: bool, faststart: bool,
) -> Result<(ClipResult, f64), String> {
    let clip_end = range.end;
    let partial = storage::partial_path(final_path);
    let r = unsafe { remux_inner(segments, lead_in, seg0_start, range.start, clip_end, track_names, &partial, aligned, faststart) };
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
            if aligned && (v.duration_secs - (clip_end - actual_start)).abs() > 0.004 {
                let _ = std::fs::remove_file(&partial);
                return Err("This recording could not be saved at the selected duration. Adjust the trim handles and try again.".into());
            }
            if clip_end < 1e8 && v.duration_secs + 0.15 < clip_end - actual_start {
                let _ = std::fs::remove_file(&partial);
                return Err("The clip is incomplete. Original recording retained; try the available range again.".into());
            }
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

/// Presentation times and durations from one source segment, in the history timebase.
pub fn frame_timing(path: &Path, segment_start: f64) -> Result<Vec<capture_protocol::FrameTiming>, String> {
    unsafe {
        let input = open_input(path)?;
        let streams = std::slice::from_raw_parts((*input.0).streams, (*input.0).nb_streams as usize);
        let index = streams.iter().copied().position(|stream| {
            (*(*stream).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO
        }).ok_or("Segment has no video track")?;
        let stream = streams[index];
        let tb = (*stream).time_base;
        let rate = ff::av_guess_frame_rate(input.0, stream, null_mut());
        let fallback_duration = if rate.num > 0 { rate.den as f64 / rate.num as f64 } else { 0.0 };
        let seconds = tb.num as f64 / tb.den as f64;
        let mut packet = ff::av_packet_alloc();
        if packet.is_null() { return Err("Could not read frame timing".into()); }
        let mut samples = Vec::new();
        while ff::av_read_frame(input.0, packet) >= 0 {
            let p = &*packet;
            if p.stream_index as usize == index && p.pts != ff::AV_NOPTS_VALUE {
                samples.push((p.pts, p.duration, p.flags & ff::AV_PKT_FLAG_KEY != 0));
            }
            ff::av_packet_unref(packet);
        }
        ff::av_packet_free(&mut packet);
        let origin = samples.iter().find(|sample| sample.2).ok_or("Segment has no video keyframe")?.0;
        samples.sort_by_key(|sample| sample.0);
        Ok(samples.iter().enumerate().filter_map(|(i, &(pts, duration, key))| {
            if pts < origin { return None; }
            let start = segment_start + (pts - origin) as f64 * seconds;
            let duration = if duration > 0 { duration as f64 * seconds } else {
                samples.get(i + 1).map_or(fallback_duration, |next| (next.0 - pts) as f64 * seconds)
            };
            (duration > 0.0).then_some(capture_protocol::FrameTiming { start, end: start + duration, key })
        }).collect())
    }
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

/// Recovery can infer a track count from media, but must not invent lost source labels.
pub fn recovered_audio_names(path:&Path)->Option<Vec<String>> {
    unsafe {
        let ic=open_input(path).ok()?;let mut count=0;
        for i in 0..(*ic.0).nb_streams as usize {if (*(*(*(*ic.0).streams.add(i))).codecpar).codec_type==ff::AVMediaType::AVMEDIA_TYPE_AUDIO {count+=1;}}
        Some((0..count).map(|i|if i==0 {"Preview Mix".into()}else{format!("Recovered Audio {}",i+1)}).collect())
    }
}

/// Timeline time (seconds) of the last video frame in a buffer segment.
pub fn last_video_secs(path: &Path) -> Option<f64> {
    unsafe {
        let ic = open_input(path).ok()?;
        let n = (*ic.0).nb_streams as usize;
        let v = (0..n).find(|&i| (*(*(*(*ic.0).streams.add(i))).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO)?;
        let tb = (*(*(*ic.0).streams.add(v))).time_base;
        let pkt = ff::av_packet_alloc();
        let mut last = None;
        while ff::av_read_frame(ic.0, pkt) >= 0 {
            if (*pkt).stream_index as usize == v && (*pkt).pts != ff::AV_NOPTS_VALUE {
                last = Some(last.map_or((*pkt).pts, |l: i64| l.max((*pkt).pts)));
            }
            ff::av_packet_unref(pkt);
        }
        let mut p = pkt;
        ff::av_packet_free(&mut p);
        last.map(|l| l as f64 * tb.num as f64 / tb.den as f64)
    }
}

/// Pure choice of the cut point (unit-tested): the last keyframe at or before `want`, else the
/// first keyframe.
pub fn pick_origin(keys: &[i64], want: i64) -> Option<i64> {
    keys.iter().copied().filter(|&k| k <= want).max().or_else(|| keys.first().copied())
}

/// `lead_in`: the buffer segment just before `segments`. Encoders deliver video later than
/// audio, so the audio for the clip's first moments (up to the encoder's latency: ~0.1 s with
/// NVENC, ~0.5 s with the software encoder) was written into that earlier segment. Only its
/// audio at or after the clip's first frame is used; its video is never.
unsafe fn remux_inner(segments: &[(PathBuf, f64)], lead_in: Option<&Path>, seg0_start: f64, want_start: f64, clip_end: f64, names: &[String], out: &Path, aligned: bool, faststart: bool) -> Result<f64, String> {
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
    if faststart {
        av::dict_set(&mut opts, "movflags", "+faststart");
    }
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
    if aligned && (actual_start - want_start).abs() > 0.000_01 {
        return Err("The selected start frame is no longer available. Adjust the start handle and try again.".into());
    }
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
    let inputs = lead_in.map(|p| (p.to_path_buf(), true)).into_iter().chain(segments.iter().map(|(p, _)| (p.clone(), false)));
    for (path, audio_only) in inputs {
        let ic = match open_input(&path) {
            Ok(ic) => ic,
            Err(e) => return Err(format!("A required recording segment could not be read: {e}. Original footage has been retained.")),
        };
        if (*ic.0).nb_streams as usize != n {
            if audio_only {
                continue;
            }
            return Err("Buffer segments changed layout mid-clip".into());
        }
        while ff::av_read_frame(ic.0, pkt) >= 0 {
            let si = (*pkt).stream_index as usize;
            let ist = *(*ic.0).streams.add(si);
            if (*pkt).pts == ff::AV_NOPTS_VALUE || (audio_only && si == video_idx) {
                ff::av_packet_unref(pkt);
                continue;
            }
            let Some(o) = origin else {
                ff::av_packet_unref(pkt);
                continue;
            };
            let o_in = ff::av_rescale_q(o, vtb, (*ist).time_base);
            let rel = ((*pkt).pts - o_in) as f64 * (*ist).time_base.num as f64 / (*ist).time_base.den as f64;
            let past_end = if aligned { rel >= span - 0.000_001 } else { rel > span };
            if rel < 0.0 || past_end {
                ff::av_packet_unref(pkt);
                continue;
            }
            if aligned {
                let tb = (*ist).time_base;
                let remaining = ((span - rel) * tb.den as f64 / tb.num as f64).round() as i64;
                if (*pkt).duration > remaining { (*pkt).duration = remaining; }
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
