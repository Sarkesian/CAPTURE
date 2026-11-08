//! "Editor audio files": one synchronized 48 kHz / 24-bit WAV per isolated track of a saved
//! clip, in `<clip>_Audio\`, plus `manifest.json` and a short README. The video is untouched.
//!
//! Alignment: every WAV starts at the clip's time zero (the audio stream's start offset is
//! padded with silence) and is padded to the clip's full duration, so stems line up with the
//! video and with each other when dropped on a timeline at 0.

use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use ffmpeg_sys_next as ff;

use crate::av::{self, check};

pub struct StemResult {
    pub folder: PathBuf,
    pub files: Vec<PathBuf>,
}

fn file_name_for(label: &str) -> String {
    let s: String = label.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("{}.wav", if s.is_empty() { "Audio".into() } else { s })
}

struct Out {
    oc: *mut ff::AVFormatContext,
    enc: *mut ff::AVCodecContext,
    dec: *mut ff::AVCodecContext,
    swr: *mut ff::SwrContext,
    in_index: usize,
    in_tb: ff::AVRational,
    /// Samples written so far (output timeline, 48 kHz).
    written: i64,
    lead: i64,
    path: PathBuf,
}

impl Drop for Out {
    fn drop(&mut self) {
        unsafe {
            ff::swr_free(&mut self.swr);
            ff::avcodec_free_context(&mut self.enc);
            ff::avcodec_free_context(&mut self.dec);
            if !self.oc.is_null() {
                if !(*self.oc).pb.is_null() {
                    ff::avio_closep(&mut (*self.oc).pb);
                }
                ff::avformat_free_context(self.oc);
            }
        }
    }
}

pub fn export(clip: &Path) -> Result<StemResult, String> {
    let stem = clip.file_stem().ok_or("Not a clip file")?.to_string_lossy().into_owned();
    let folder = clip.with_file_name(format!("{stem}_Audio"));
    std::fs::create_dir_all(&folder).map_err(|e| format!("Cannot create {}: {e}", folder.display()))?;
    let r = unsafe { export_inner(clip, &folder) };
    if r.is_err() {
        let _ = std::fs::remove_dir_all(&folder);
    }
    r
}

unsafe fn export_inner(clip: &Path, folder: &Path) -> Result<StemResult, String> {
    let mut ic = null_mut();
    check(ff::avformat_open_input(&mut ic, av::cstr(&clip.to_string_lossy()).as_ptr(), null(), null_mut()), "open clip")?;
    struct In(*mut ff::AVFormatContext);
    impl Drop for In {
        fn drop(&mut self) {
            unsafe { ff::avformat_close_input(&mut self.0) };
        }
    }
    let ic = In(ic);
    check(ff::avformat_find_stream_info(ic.0, null_mut()), "read clip")?;
    let duration = (*ic.0).duration as f64 / ff::AV_TIME_BASE as f64;
    let total = (duration * 48_000.0).round() as i64;

    let mut outs: Vec<Out> = Vec::new();
    let mut manifest_tracks = Vec::new();
    for i in 0..(*ic.0).nb_streams as usize {
        let st = *(*ic.0).streams.add(i);
        let par = (*st).codecpar;
        if (*par).codec_type != ff::AVMediaType::AVMEDIA_TYPE_AUDIO {
            continue;
        }
        let e = ff::av_dict_get((*st).metadata, av::cstr("handler_name").as_ptr(), null(), 0);
        let label = if e.is_null() { format!("Audio {}", i) } else { std::ffi::CStr::from_ptr((*e).value).to_string_lossy().into_owned() };
        if label == "Preview Mix" {
            continue; // a mix of the others: exporting it too would double the sound in an editor
        }
        let dc = ff::avcodec_find_decoder((*par).codec_id);
        let dec = ff::avcodec_alloc_context3(dc);
        ff::avcodec_parameters_to_context(dec, par);
        check(ff::avcodec_open2(dec, dc, null_mut()), "open audio decoder")?;
        let ec = ff::avcodec_find_encoder(ff::AVCodecID::AV_CODEC_ID_PCM_S24LE);
        let enc = ff::avcodec_alloc_context3(ec);
        (*enc).sample_rate = 48_000;
        (*enc).sample_fmt = ff::AVSampleFormat::AV_SAMPLE_FMT_S32;
        ff::av_channel_layout_default(&mut (*enc).ch_layout, 2);
        (*enc).time_base = ff::AVRational { num: 1, den: 48_000 };
        check(ff::avcodec_open2(enc, ec, null_mut()), "open WAV encoder")?;
        let mut swr = null_mut();
        check(
            ff::swr_alloc_set_opts2(&mut swr, &(*enc).ch_layout, ff::AVSampleFormat::AV_SAMPLE_FMT_S32, 48_000, &(*dec).ch_layout, (*dec).sample_fmt, (*dec).sample_rate, 0, null_mut()),
            "resampler",
        )?;
        check(ff::swr_init(swr), "resampler init")?;
        let path = folder.join(file_name_for(&label));
        let cpath = av::cstr(&path.to_string_lossy());
        let mut oc = null_mut();
        check(ff::avformat_alloc_output_context2(&mut oc, null(), av::cstr("wav").as_ptr(), cpath.as_ptr()), "wav ctx")?;
        let ost = ff::avformat_new_stream(oc, null());
        ff::avcodec_parameters_from_context((*ost).codecpar, enc);
        (*ost).time_base = (*enc).time_base;
        check(ff::avio_open(&mut (*oc).pb, cpath.as_ptr(), ff::AVIO_FLAG_WRITE), "create wav")?;
        check(ff::avformat_write_header(oc, null_mut()), "wav header")?;
        let start = if (*st).start_time == ff::AV_NOPTS_VALUE { 0 } else { (*st).start_time };
        let lead = ff::av_rescale_q(start, (*st).time_base, ff::AVRational { num: 1, den: 48_000 }).max(0);
        manifest_tracks.push(serde_json::json!({ "file": file_name_for(&label), "track": label, "stream_index": i }));
        outs.push(Out { oc, enc, dec, swr, in_index: i, in_tb: (*st).time_base, written: 0, lead, path });
    }
    if outs.is_empty() {
        return Err("This clip has no separate audio tracks to export.".into());
    }
    for o in &mut outs {
        write_silence(o, o.lead)?;
    }
    let pkt = ff::av_packet_alloc();
    let frame = ff::av_frame_alloc();
    while ff::av_read_frame(ic.0, pkt) >= 0 {
        if let Some(o) = outs.iter_mut().find(|o| o.in_index == (*pkt).stream_index as usize) {
            if ff::avcodec_send_packet(o.dec, pkt) >= 0 {
                while ff::avcodec_receive_frame(o.dec, frame) >= 0 {
                    convert_and_write(o, frame)?;
                }
            }
        }
        ff::av_packet_unref(pkt);
    }
    for o in &mut outs {
        ff::avcodec_send_packet(o.dec, null());
        while ff::avcodec_receive_frame(o.dec, frame) >= 0 {
            convert_and_write(o, frame)?;
        }
        let pad = total - o.written;
        if pad > 0 {
            write_silence(o, pad)?;
        }
        ff::avcodec_send_frame(o.enc, null());
        drain(o)?;
        check(ff::av_write_trailer(o.oc), "finish wav")?;
    }
    let mut f = frame;
    ff::av_frame_free(&mut f);
    let mut p = pkt;
    ff::av_packet_free(&mut p);

    let manifest = serde_json::json!({
        "capture_stems": 1,
        "clip": clip.file_name().map(|n| n.to_string_lossy().into_owned()),
        "sample_rate": 48000, "bits": 24, "channels": 2,
        "duration_secs": duration,
        "alignment": "every file starts at the clip's time 0 and lasts the clip's full duration",
        "tracks": manifest_tracks,
    });
    std::fs::write(folder.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(
        folder.join("README.txt"),
        "Editor audio files made by CAPTURE.\r\n\r\nPut the video and these WAV files on your timeline, all starting at 0:00. They are already in sync.\r\nUse these instead of the video's own audio (mute or remove the video's audio track) so nothing plays twice.\r\n",
    )
    .map_err(|e| e.to_string())?;
    Ok(StemResult { folder: folder.to_path_buf(), files: outs.iter().map(|o| o.path.clone()).collect() })
}

unsafe fn convert_and_write(o: &mut Out, input: *mut ff::AVFrame) -> Result<(), String> {
    let out = ff::av_frame_alloc();
    (*out).format = ff::AVSampleFormat::AV_SAMPLE_FMT_S32 as i32;
    (*out).sample_rate = 48_000;
    ff::av_channel_layout_copy(&mut (*out).ch_layout, &(*o.enc).ch_layout);
    (*out).nb_samples = (*input).nb_samples + 64;
    let r = (|| -> Result<(), String> {
        check(ff::av_frame_get_buffer(out, 0), "pcm frame")?;
        let n = check(
            ff::swr_convert(o.swr, (*out).data.as_mut_ptr(), (*out).nb_samples, (*input).data.as_ptr() as *mut *const u8, (*input).nb_samples),
            "convert",
        )?;
        (*out).nb_samples = n;
        if n > 0 {
            (*out).pts = o.written;
            o.written += n as i64;
            check(ff::avcodec_send_frame(o.enc, out), "encode pcm")?;
            drain(o)?;
        }
        Ok(())
    })();
    let mut f = out;
    ff::av_frame_free(&mut f);
    r
}

unsafe fn write_silence(o: &mut Out, samples: i64) -> Result<(), String> {
    let mut left = samples;
    while left > 0 {
        let n = left.min(4800) as i32;
        let f = ff::av_frame_alloc();
        (*f).format = ff::AVSampleFormat::AV_SAMPLE_FMT_S32 as i32;
        (*f).sample_rate = 48_000;
        ff::av_channel_layout_copy(&mut (*f).ch_layout, &(*o.enc).ch_layout);
        (*f).nb_samples = n;
        let r = check(ff::av_frame_get_buffer(f, 0), "silence");
        if r.is_ok() {
            ff::av_samples_set_silence((*f).data.as_mut_ptr(), 0, n, 2, ff::AVSampleFormat::AV_SAMPLE_FMT_S32);
            (*f).pts = o.written;
            o.written += n as i64;
            ff::avcodec_send_frame(o.enc, f);
        }
        let mut ff_ = f;
        ff::av_frame_free(&mut ff_);
        r?;
        drain(o)?;
        left -= n as i64;
    }
    Ok(())
}

unsafe fn drain(o: &mut Out) -> Result<(), String> {
    let pkt = ff::av_packet_alloc();
    let r = loop {
        let r = ff::avcodec_receive_packet(o.enc, pkt);
        if r == ff::AVERROR(ff::EAGAIN) || r == ff::AVERROR_EOF {
            break Ok(());
        }
        if let Err(e) = check(r, "pcm packet") {
            break Err(e);
        }
        let st = *(*o.oc).streams;
        ff::av_packet_rescale_ts(pkt, (*o.enc).time_base, (*st).time_base);
        (*pkt).stream_index = 0;
        if let Err(e) = check(ff::av_interleaved_write_frame(o.oc, pkt), "write wav") {
            break Err(e);
        }
    };
    let mut p = pkt;
    ff::av_packet_free(&mut p);
    r
}

#[cfg(test)]
mod tests {
    #[test]
    fn stem_names_are_plain() {
        assert_eq!(super::file_name_for("Game Audio"), "GameAudio.wav");
        assert_eq!(super::file_name_for("Voice Chat"), "VoiceChat.wav");
        assert_eq!(super::file_name_for("Other Desktop"), "OtherDesktop.wav");
    }
}
