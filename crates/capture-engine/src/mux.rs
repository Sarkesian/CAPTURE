//! Replay-ring segment files: MPEG-TS, one file per ~2 s, each starting on a video keyframe.
//!
//! MPEG-TS was chosen for the ring because a segment interrupted by a crash or power loss is
//! still readable up to the last complete packet (no trailer/moov needed). Saved clips are
//! remuxed to MP4 (see `save.rs`). Timestamps written are absolute session-timeline values,
//! so segments can be joined exactly without per-file rebasing.

use std::path::{Path, PathBuf};

use ffmpeg_sys_next as ff;
use std::ptr::{null, null_mut};

use crate::av::{self, check, Packet, Rational};

/// Owned codec parameters for one output stream.
pub struct StreamDesc {
    pub params: *mut ff::AVCodecParameters,
    pub time_base: Rational,
}

unsafe impl Send for StreamDesc {}

impl StreamDesc {
    pub unsafe fn from_ctx(ctx: *const ff::AVCodecContext) -> Self {
        let p = ff::avcodec_parameters_alloc();
        ff::avcodec_parameters_from_context(p, ctx);
        Self { params: p, time_base: (*ctx).time_base }
    }
    pub unsafe fn copy_of(p: *const ff::AVCodecParameters, tb: Rational) -> Self {
        let n = ff::avcodec_parameters_alloc();
        ff::avcodec_parameters_copy(n, p);
        Self { params: n, time_base: tb }
    }
}

impl Drop for StreamDesc {
    fn drop(&mut self) {
        unsafe { ff::avcodec_parameters_free(&mut self.params) };
    }
}

pub struct SegmentWriter {
    oc: *mut ff::AVFormatContext,
    pub path: PathBuf,
    /// Last dts written per stream (in stream time base) to drop non-monotonic packets.
    last_dts: Vec<i64>,
}

unsafe impl Send for SegmentWriter {}

impl SegmentWriter {
    pub fn create(path: &Path, streams: &[StreamDesc]) -> Result<Self, String> {
        unsafe {
            let cpath = av::cstr(&path.to_string_lossy());
            let mut oc = null_mut();
            check(ff::avformat_alloc_output_context2(&mut oc, null(), av::cstr("mpegts").as_ptr(), cpath.as_ptr()), "segment ctx")?;
            // Keep absolute timestamps: no data-dependent shifting per file.
            (*oc).avoid_negative_ts = ff::AVFMT_AVOID_NEG_TS_DISABLED;
            for s in streams {
                let st = ff::avformat_new_stream(oc, null());
                ff::avcodec_parameters_copy((*st).codecpar, s.params);
                (*(*st).codecpar).codec_tag = 0;
                (*st).time_base = s.time_base;
            }
            let fail = |oc: *mut ff::AVFormatContext, e: String| {
                ff::avformat_free_context(oc);
                Err(e)
            };
            if let Err(e) = check(ff::avio_open(&mut (*oc).pb, cpath.as_ptr(), ff::AVIO_FLAG_WRITE), "open segment") {
                return fail(oc, e);
            }
            if let Err(e) = check(ff::avformat_write_header(oc, null_mut()), "segment header") {
                ff::avio_closep(&mut (*oc).pb);
                return fail(oc, e);
            }
            Ok(Self { oc, path: path.to_path_buf(), last_dts: vec![i64::MIN; streams.len()] })
        }
    }

    pub fn write(&mut self, pkt: Packet) -> Result<(), String> {
        unsafe {
            if pkt.pts() < 0 {
                return Ok(()); // encoder priming before session start
            }
            let st = *(*self.oc).streams.add(pkt.track);
            ff::av_packet_rescale_ts(pkt.ptr, pkt.time_base, (*st).time_base);
            let dts = (*pkt.ptr).dts;
            if dts <= self.last_dts[pkt.track] {
                return Ok(());
            }
            self.last_dts[pkt.track] = dts;
            (*pkt.ptr).stream_index = pkt.track as i32;
            check(ff::av_interleaved_write_frame(self.oc, pkt.ptr), "write segment")?;
        }
        Ok(())
    }

    /// Flush and close; returns file size.
    pub fn finish(mut self) -> u64 {
        unsafe {
            ff::av_write_trailer(self.oc);
            ff::avio_closep(&mut (*self.oc).pb);
            ff::avformat_free_context(self.oc);
            self.oc = null_mut();
        }
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

impl Drop for SegmentWriter {
    fn drop(&mut self) {
        if !self.oc.is_null() {
            unsafe {
                ff::avio_closep(&mut (*self.oc).pb);
                ff::avformat_free_context(self.oc);
            }
        }
    }
}
