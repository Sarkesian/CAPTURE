//! WASAPI audio sources aligned to the session timeline, and AAC encoding of
//! `Preview Mix` + one isolated, unprocessed track per source.
//!
//! Sources: the game (process loopback of its process tree), a microphone (a specific capture
//! device), a voice-chat app (process loopback of that app's tree) and "All Desktop Audio"
//! (everything except CAPTURE). Each source runs on its own thread and *retries* on failure
//! (device unplugged, app not running yet, app restarted) without ever switching to a
//! different device.
//!
//! Alignment: every packet's QPC capture time is converted to a timeline sample index. The
//! source buffer pads silence for gaps (including the leading gap) and drops samples if the
//! device clock runs ahead, so sample N of every track always means the same instant.
//! Loopback delivers *no* packets while nothing plays; the encoder pads stalled tracks against
//! the wall clock so silence never shifts later audio.
//!
//! Mixing: `Preview Mix` = Σ gain × source over sources marked `in_mix` (soft-limited). A muted
//! microphone contributes silence to both its own track and the mix. Isolated tracks are
//! never gain-adjusted.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::TrackState;
use ffmpeg_sys_next as ff;
use std::ptr::null_mut;
use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{CreateEventW, OpenProcess, SetEvent, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows_core::implement;

use crate::av::{self, check, Packet, Rational};
use crate::{clock, devices};

pub const RATE: u32 = 48_000;
const CH: usize = 2;
const DRIFT_TOLERANCE: i64 = 960; // 20 ms
const STALL_PAD_AFTER: i64 = 9_600; // 200 ms behind wall clock => device is silent/stalled
const STALL_PAD_TO: i64 = 4_800; // pad up to 100 ms behind wall clock
const RETRY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub enum SourceSpec {
    /// Audio rendered by one process and its children (Windows build ≥ 20348).
    Process { pid: u32 },
    /// Audio of an app found by executable name (its root process and children). Resolved on
    /// every (re)connect, so it follows the app across restarts.
    App { exe: String },
    /// A microphone. `None` = the Windows default input.
    Mic { device_id: Option<String> },
    /// Everything every app plays *except* CAPTURE's own processes (so CAPTURE's confirmation
    /// sounds and clip playback in its window are never recorded). Process loopback in exclude
    /// mode on the UI's process tree.
    AllExceptCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Game,
    Mic,
    Voice,
    Other,
}

#[derive(Debug, Clone)]
pub struct Source {
    pub spec: SourceSpec,
    pub role: Role,
    pub label: String,
    /// Added to Preview Mix (false for sources that would double other sources).
    pub in_mix: bool,
}

/// Live-adjustable mix settings (no pipeline restart needed).
#[derive(Debug, Clone, Copy)]
pub struct MixParams {
    pub game: f32,
    pub mic: f32,
    pub voice: f32,
    pub mic_muted: bool,
}

impl Default for MixParams {
    fn default() -> Self {
        Self { game: 1.0, mic: 1.0, voice: 1.0, mic_muted: false }
    }
}

impl MixParams {
    fn gain(&self, r: Role) -> f32 {
        match r {
            Role::Game => self.game,
            Role::Mic => self.mic,
            Role::Voice => self.voice,
            Role::Other => 0.0,
        }
    }
}

#[derive(Default)]
pub(crate) struct TrackBuf {
    data: VecDeque<f32>,
    /// Timeline sample index of `data[0]`.
    base: i64,
    started: bool,
    peak: f32,
    peak_at: Option<Instant>,
    signal_seen: bool,
    error: Option<String>,
    mode: String,
    source: String,
    /// Level-only monitor: samples are measured but not stored.
    pub(crate) discard: bool,
    /// A reason the source records silence although it is connected (Windows has the
    /// microphone muted or at 0%). Not an error: capture keeps running and clears it itself.
    hint: Option<String>,
    /// When the device was (re)connected, and whether any non-zero sample arrived since.
    connected_at: Option<Instant>,
    heard_since_connect: bool,
    /// Meter window since the last `take_level` (peak, sum of squares, sample count).
    win_peak: f32,
    win_sq: f64,
    win_n: u64,
}

impl TrackBuf {
    /// A level-only buffer (mic monitor): measures peaks, stores nothing.
    pub(crate) fn monitor() -> Self {
        Self { discard: true, ..Default::default() }
    }

    fn write_pos(&self) -> i64 {
        self.base + (self.data.len() / CH) as i64
    }

    /// Append `frames` interleaved stereo frames captured at timeline index `idx`.
    fn push(&mut self, idx: i64, samples: Option<&[f32]>, frames: usize) {
        if self.discard {
            // Level only: nothing is ever drained from a monitor, so nothing may be stored
            // (silent packets included).
            if let Some(s) = samples {
                self.measure(s);
            }
            return;
        }
        if !self.started {
            self.started = true;
            self.base = idx.max(0);
            if idx > 0 {
                self.base = 0;
                self.data.extend(std::iter::repeat_n(0.0, idx as usize * CH));
            }
        }
        let mut skip = 0usize;
        let drift = idx - self.write_pos();
        if drift > DRIFT_TOLERANCE {
            self.data.extend(std::iter::repeat_n(0.0, drift as usize * CH));
        } else if drift < -DRIFT_TOLERANCE {
            skip = ((-drift) as usize).min(frames);
        }
        if idx < 0 {
            skip = skip.max(((-idx) as usize).min(frames));
        }
        match samples {
            Some(s) => {
                let s = &s[skip * CH..frames * CH];
                self.measure(s);
                self.data.extend(s.iter().copied());
            }
            None => self.data.extend(std::iter::repeat_n(0.0, (frames - skip) * CH)),
        }
    }

    fn measure(&mut self, s: &[f32]) {
        let p = s.iter().fold(0f32, |m, x| m.max(x.abs()));
        self.win_peak = self.win_peak.max(p);
        self.win_sq += s.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>();
        self.win_n += s.len() as u64;
        if p > 0.0 {
            self.signal_seen = true;
            self.heard_since_connect = true;
        }
        if self.peak_at.is_none_or(|t| t.elapsed() > Duration::from_millis(100)) || p > self.peak {
            self.peak = p;
            self.peak_at = Some(Instant::now());
        }
    }

    fn pad_to(&mut self, pos: i64) {
        if !self.started {
            self.started = true;
            self.base = 0;
        }
        let w = self.write_pos();
        if pos > w {
            self.data.extend(std::iter::repeat_n(0.0, (pos - w) as usize * CH));
        }
    }

    pub(crate) fn live_peak(&self) -> f32 {
        if self.peak_at.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
            self.peak
        } else {
            0.0
        }
    }

    /// Peak and RMS since the previous call (both 0 when no sound arrived), then reset.
    pub(crate) fn take_level(&mut self) -> (f32, f32) {
        let rms = if self.win_n > 0 { (self.win_sq / self.win_n as f64).sqrt() as f32 } else { 0.0 };
        let r = (self.win_peak, rms);
        self.win_peak = 0.0;
        self.win_sq = 0.0;
        self.win_n = 0;
        r
    }

    pub(crate) fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// What to tell the user about this source: a failure, a Windows setting that silences
    /// it, or (microphones only) a connected device that has sent nothing but digital zeros
    /// for 5 s. A real microphone always has some noise, so pure zeros mean it is muted
    /// somewhere (hardware switch, headset off, driver), never just a quiet room.
    pub(crate) fn problem(&self) -> Option<String> {
        if self.error.is_some() {
            return self.error.clone();
        }
        if self.hint.is_some() {
            return self.hint.clone();
        }
        let silent_mic = self.mode == "microphone"
            && !self.heard_since_connect
            && self.connected_at.is_some_and(|t| t.elapsed() > Duration::from_secs(5));
        silent_mic.then(|| {
            format!(
                "{} is connected but sends complete silence. Check its mute button or switch, that the headset is on, and Windows Sound settings › Input.",
                self.source
            )
        })
    }

    pub(crate) fn take_samples(&mut self) -> Vec<f32> {
        self.data.drain(..).collect()
    }
}

/// Soft limiter for the mix: transparent below 0.9, smoothly approaches ±1 above.
pub fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((a - 0.9) / 0.1).tanh())
    }
}

pub struct AudioPipeline {
    bufs: Vec<Arc<Mutex<TrackBuf>>>,
    sources: Vec<Source>,
    labels: Vec<String>,
    mix: Arc<Mutex<MixParams>>,
    threads: Vec<std::thread::JoinHandle<()>>,
    params: Vec<(*mut ff::AVCodecParameters, Rational)>,
}

unsafe impl Send for AudioPipeline {}

/// Track names in output order: `Preview Mix` then one per source.
pub fn track_names(sources: &[Source]) -> Vec<String> {
    if sources.is_empty() {
        return vec![];
    }
    std::iter::once("Preview Mix".to_string()).chain(sources.iter().map(|s| s.label.clone())).collect()
}

impl AudioPipeline {
    /// `first_track` is the muxer track index of `Preview Mix`.
    pub fn start(
        sources: Vec<Source>,
        mix: MixParams,
        t0_qpc: u64,
        stop: Arc<AtomicBool>,
        first_track: usize,
        out: impl Fn(Packet) + Send + 'static,
    ) -> Result<Self, String> {
        let names = track_names(&sources);
        let bufs: Vec<_> = sources.iter().map(|_| Arc::new(Mutex::new(TrackBuf::default()))).collect();
        let mix = Arc::new(Mutex::new(mix));
        // Encoders first: if one cannot open, no capture thread (microphone included) has been
        // started yet, so nothing is left running behind the error.
        let mut encoders = Vec::new();
        for _ in &names {
            encoders.push(unsafe { AacEncoder::open()? });
        }
        let mut threads = Vec::new();
        for (src, buf) in sources.iter().cloned().zip(bufs.iter().cloned()) {
            let stop = stop.clone();
            threads.push(
                std::thread::Builder::new()
                    .name(format!("audio:{}", src.label))
                    .spawn(move || source_thread(&src.spec, &buf, t0_qpc, &stop))
                    .unwrap(),
            );
        }
        let params = encoders.iter().map(|e| (e.params(), e.time_base())).collect();
        if !names.is_empty() {
            let (bufs2, srcs2, mix2) = (bufs.clone(), sources.clone(), mix.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("audio-encode".into())
                    .spawn(move || encode_loop(bufs2, srcs2, mix2, encoders, t0_qpc, stop, first_track, out))
                    .unwrap(),
            );
        }
        Ok(Self { bufs, sources, labels: names, mix, threads, params })
    }

    pub fn set_mix(&self, m: MixParams) {
        *self.mix.lock().unwrap() = m;
    }

    /// Codec parameters + time base per output track (owned; freed on drop).
    pub fn params(&self) -> &[(*mut ff::AVCodecParameters, Rational)] {
        &self.params
    }

    pub fn track_names(&self) -> &[String] {
        &self.labels
    }

    pub fn states(&self) -> Vec<TrackState> {
        let mut v = Vec::new();
        let mix = *self.mix.lock().unwrap();
        let srcs: Vec<_> = self.bufs.iter().map(|b| b.lock().unwrap()).collect();
        if !srcs.is_empty() {
            let mixed: Vec<usize> = (0..srcs.len()).filter(|&i| self.sources[i].in_mix).collect();
            let peak = mixed
                .iter()
                .filter(|&&i| !(self.sources[i].role == Role::Mic && mix.mic_muted))
                .map(|&i| srcs[i].live_peak() * mix.gain(self.sources[i].role))
                .fold(0f32, f32::max);
            v.push(TrackState {
                name: "Preview Mix".into(),
                mode: "mix".into(),
                source: mixed.iter().map(|&i| self.sources[i].label.as_str()).collect::<Vec<_>>().join(" + "),
                peak: peak.min(1.0),
                signal_seen: mixed.iter().any(|&i| srcs[i].signal_seen),
                error: None,
            });
        }
        for (i, (b, name)) in srcs.iter().zip(self.labels.iter().skip(1)).enumerate() {
            let muted = self.sources[i].role == Role::Mic && mix.mic_muted;
            v.push(TrackState {
                name: name.clone(),
                mode: b.mode.clone(),
                source: if muted { format!("{} (muted)", b.source) } else { b.source.clone() },
                peak: b.live_peak(),
                signal_seen: b.signal_seen,
                error: b.problem(),
            });
        }
        v
    }

    /// Meter levels since the last call: Preview Mix (as heard, with gains and mute) then each
    /// source. Muted microphone reports 0.
    pub fn take_levels(&self) -> Vec<capture_protocol::Level> {
        let mix = *self.mix.lock().unwrap();
        let mut srcs: Vec<(f32, f32)> = self.bufs.iter().map(|b| b.lock().unwrap().take_level()).collect();
        for (i, s) in srcs.iter_mut().enumerate() {
            if self.sources[i].role == Role::Mic && mix.mic_muted {
                *s = (0.0, 0.0);
            }
        }
        let mut out = Vec::new();
        if !srcs.is_empty() {
            let mixed: Vec<usize> = (0..srcs.len()).filter(|&i| self.sources[i].in_mix).collect();
            let peak = mixed.iter().map(|&i| srcs[i].0 * mix.gain(self.sources[i].role)).fold(0f32, f32::max).min(1.0);
            let rms = mixed.iter().map(|&i| (srcs[i].1 * mix.gain(self.sources[i].role)).powi(2)).sum::<f32>().sqrt().min(1.0);
            out.push(capture_protocol::Level { name: "Preview Mix".into(), peak, rms });
        }
        for (i, (p, r)) in srcs.into_iter().enumerate() {
            out.push(capture_protocol::Level { name: self.labels[i + 1].clone(), peak: p, rms: r });
        }
        out
    }

    pub fn join(&mut self) {
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.join();
        for (p, _) in &mut self.params {
            unsafe { ff::avcodec_parameters_free(p) };
        }
    }
}

/// Capture until `stop`; on any failure, report it and retry the *same* source.
/// `capture_source` result meaning "reopen now" (not an error to show).
const REOPEN: &str = "<reopen>";

pub(crate) fn source_thread(spec: &SourceSpec, buf: &Mutex<TrackBuf>, t0_qpc: u64, stop: &AtomicBool) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    while !stop.load(Ordering::Relaxed) {
        match capture_source(spec, buf, t0_qpc, stop) {
            Ok(()) => return,
            // The Windows default microphone or output device changed: switch at once, quietly.
            Err(e) if e == REOPEN => continue,
            Err(e) => buf.lock().unwrap().error = Some(e),
        }
        let until = Instant::now() + RETRY;
        while Instant::now() < until {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_loop(
    bufs: Vec<Arc<Mutex<TrackBuf>>>,
    sources: Vec<Source>,
    mix: Arc<Mutex<MixParams>>,
    mut encoders: Vec<AacEncoder>,
    t0_qpc: u64,
    stop: Arc<AtomicBool>,
    first_track: usize,
    out: impl Fn(Packet),
) {
    let frame = 1024usize;
    let mut next: i64 = 0;
    let mut src = vec![vec![0f32; frame * CH]; bufs.len()];
    let mut mixbuf = vec![0f32; frame * CH];
    loop {
        let stopping = stop.load(Ordering::Relaxed);
        let now_idx = clock::timeline_samples(t0_qpc, RATE);
        let ready = bufs
            .iter()
            .map(|b| {
                let mut b = b.lock().unwrap();
                if b.write_pos() < now_idx - STALL_PAD_AFTER || (stopping && b.write_pos() < now_idx) {
                    b.pad_to(if stopping { now_idx } else { now_idx - STALL_PAD_TO });
                }
                b.write_pos()
            })
            .min()
            .unwrap_or(0);
        while next + frame as i64 <= ready {
            let p = *mix.lock().unwrap();
            mixbuf.fill(0.0);
            for (i, b) in bufs.iter().enumerate() {
                let mut b = b.lock().unwrap();
                let skip = ((next - b.base).max(0) as usize) * CH;
                let n = skip.min(b.data.len());
                b.data.drain(..n);
                b.base = b.base.max(next);
                let silence = sources[i].role == Role::Mic && p.mic_muted;
                let g = if sources[i].in_mix { p.gain(sources[i].role) } else { 0.0 };
                for (j, s) in b.data.drain(..frame * CH).enumerate() {
                    let s = if silence { 0.0 } else { s };
                    src[i][j] = s;
                    mixbuf[j] += s * g;
                }
                b.base += frame as i64;
            }
            for x in mixbuf.iter_mut() {
                *x = soft_clip(*x);
            }
            let mut emit = |p: Packet| out(p);
            let _ = encoders[0].encode(&mixbuf, next, first_track, &mut emit);
            for (i, enc) in encoders.iter_mut().skip(1).enumerate() {
                let _ = enc.encode(&src[i], next, first_track + 1 + i, &mut emit);
            }
            next += frame as i64;
        }
        if stopping {
            for (i, enc) in encoders.iter_mut().enumerate() {
                enc.flush(first_track + i, &mut |p| out(p));
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct AacEncoder {
    ctx: *mut ff::AVCodecContext,
    frame: *mut ff::AVFrame,
    pkt: *mut ff::AVPacket,
}

// Owned exclusively by the audio-encode thread after construction.
unsafe impl Send for AacEncoder {}

impl AacEncoder {
    unsafe fn open() -> Result<Self, String> {
        let codec = ff::avcodec_find_encoder_by_name(av::cstr("aac").as_ptr());
        if codec.is_null() {
            return Err("AAC encoder missing from FFmpeg build".into());
        }
        let ctx = ff::avcodec_alloc_context3(codec);
        (*ctx).sample_rate = RATE as i32;
        (*ctx).sample_fmt = ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP;
        ff::av_channel_layout_default(&mut (*ctx).ch_layout, CH as i32);
        (*ctx).bit_rate = 192_000;
        (*ctx).time_base = Rational { num: 1, den: RATE as i32 };
        check(ff::avcodec_open2(ctx, codec, null_mut()), "open AAC encoder")?;
        let frame = ff::av_frame_alloc();
        (*frame).format = ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP as i32;
        (*frame).nb_samples = (*ctx).frame_size;
        (*frame).sample_rate = RATE as i32;
        ff::av_channel_layout_copy(&mut (*frame).ch_layout, &(*ctx).ch_layout);
        check(ff::av_frame_get_buffer(frame, 0), "audio frame")?;
        Ok(Self { ctx, frame, pkt: ff::av_packet_alloc() })
    }

    fn time_base(&self) -> Rational {
        unsafe { (*self.ctx).time_base }
    }

    fn params(&self) -> *mut ff::AVCodecParameters {
        unsafe {
            let p = ff::avcodec_parameters_alloc();
            ff::avcodec_parameters_from_context(p, self.ctx);
            p
        }
    }

    fn encode(&mut self, interleaved: &[f32], pts: i64, track: usize, out: &mut impl FnMut(Packet)) -> Result<(), String> {
        unsafe {
            check(ff::av_frame_make_writable(self.frame), "audio frame writable")?;
            let n = (*self.frame).nb_samples as usize;
            let l = std::slice::from_raw_parts_mut((*self.frame).data[0] as *mut f32, n);
            let r = std::slice::from_raw_parts_mut((*self.frame).data[1] as *mut f32, n);
            for i in 0..n {
                l[i] = interleaved[i * 2];
                r[i] = interleaved[i * 2 + 1];
            }
            (*self.frame).pts = pts;
            check(ff::avcodec_send_frame(self.ctx, self.frame), "encode audio")?;
            self.drain(track, out)
        }
    }

    fn drain(&mut self, track: usize, out: &mut impl FnMut(Packet)) -> Result<(), String> {
        unsafe {
            loop {
                let r = ff::avcodec_receive_packet(self.ctx, self.pkt);
                if r == ff::AVERROR(ff::EAGAIN) || r == ff::AVERROR_EOF {
                    return Ok(());
                }
                check(r, "audio packet")?;
                out(Packet::take(self.pkt, track, self.time_base()));
            }
        }
    }

    fn flush(&mut self, track: usize, out: &mut impl FnMut(Packet)) {
        unsafe { ff::avcodec_send_frame(self.ctx, std::ptr::null()) };
        let _ = self.drain(track, out);
    }
}

impl Drop for AacEncoder {
    fn drop(&mut self) {
        unsafe {
            ff::av_frame_free(&mut self.frame);
            ff::av_packet_free(&mut self.pkt);
            ff::avcodec_free_context(&mut self.ctx);
        }
    }
}

// ---- WASAPI capture ----------------------------------------------------------------------

/// Channel count and speaker mask of the default output device's mix, when it has more than
/// two channels (a 7.1 gaming headset or speaker set).
fn render_layout() -> Option<(usize, u32)> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).ok()?;
        let mf = client.GetMixFormat().ok()?;
        let n = (*mf).nChannels as usize;
        let mask = if (*mf).wFormatTag == 0xFFFE && (*mf).cbSize >= 22 { (*(mf as *const WAVEFORMATEXTENSIBLE)).dwChannelMask } else { 0 };
        CoTaskMemFree(Some(mf as *const _));
        let mask = if mask.count_ones() as usize == n { mask } else { default_mask(n)? };
        (n > 2).then_some((n, mask))
    }
}

/// Windows' default speaker masks for channel counts that come without one.
fn default_mask(n: usize) -> Option<u32> {
    Some(match n {
        3 => 0x7,
        4 => 0x33,
        5 => 0x37,
        6 => 0x3F,
        8 => 0x63F,
        _ => return None,
    })
}

/// 32-bit float, 48 kHz, `channels` with speaker `mask`.
fn float_format_ext(channels: u16, mask: u32) -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xFFFE, // WAVE_FORMAT_EXTENSIBLE
            nChannels: channels,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * channels as u32 * 4,
            nBlockAlign: channels * 4,
            wBitsPerSample: 32,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: 32 },
        dwChannelMask: mask,
        // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        SubFormat: windows::core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71),
    }
}

/// Surround to stereo at full level. Windows' own conversion scales the whole mix down so that
/// every channel at full scale at once could not clip, which made games on 7.1 devices ~14 dB
/// quieter in recordings than they really are. Here front left/right pass through untouched
/// (stereo content stays bit-exact), centre and surrounds are added at -3 dB (ITU-R BS.775),
/// LFE is left out as in standard downmixes, and when centre/surround channels are summed in,
/// the rare result above 0.9 is softly limited instead of clipping.
pub struct Downmix {
    pub channels: usize,
    pub mask: u32,
    /// (left, right) gain per input channel, in the stream's channel order.
    gains: Vec<(f32, f32)>,
    /// Front left/right (copied as they are) and LFE (left out): nothing to sum or limit.
    plain: Vec<bool>,
}

impl Downmix {
    pub fn new(channels: usize, mask: u32) -> Self {
        const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
        let mut gains = Vec::with_capacity(channels);
        let mut plain = Vec::with_capacity(channels);
        for bit in 0..32 {
            if gains.len() == channels {
                break;
            }
            if mask & (1 << bit) == 0 {
                continue;
            }
            plain.push(matches!(1u32 << bit, 0x1 | 0x2 | 0x8));
            gains.push(match 1u32 << bit {
                0x1 | 0x40 => (1.0, 0.0),                    // front left, front left of centre
                0x2 | 0x80 => (0.0, 1.0),                    // front right, front right of centre
                0x4 => (K, K),                               // front centre
                0x8 => (0.0, 0.0),                           // LFE
                0x10 | 0x200 | 0x1000 | 0x8000 => (K, 0.0),  // back / side / top left
                0x20 | 0x400 | 0x4000 | 0x20000 => (0.0, K), // back / side / top right
                _ => (0.5, 0.5),                             // back / top centre
            });
        }
        gains.resize(channels, (0.0, 0.0));
        plain.resize(channels, true);
        Self { channels, mask, gains, plain }
    }

    pub fn run(&self, input: &[f32], out: &mut Vec<f32>) {
        out.clear();
        out.reserve(input.len() / self.channels * CH);
        for frame in input.chunks_exact(self.channels) {
            let (mut l, mut r) = (0f32, 0f32);
            // Whether anything besides a plain left/right channel contributes to this frame.
            let mut summed = false;
            for ((&x, &(gl, gr)), &plain) in frame.iter().zip(&self.gains).zip(&self.plain) {
                summed |= !plain && x != 0.0;
                l += x * gl;
                r += x * gr;
            }
            // Stereo content (front left/right only) passes bit-exact, even near full scale;
            // only a sum with centre/surround channels can exceed it and is limited.
            if summed {
                out.push(soft_clip(l));
                out.push(soft_clip(r));
            } else {
                out.push(l);
                out.push(r);
            }
        }
    }
}

fn float_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: CH as u16,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * CH as u32 * 4,
        nBlockAlign: CH as u16 * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

/// Open the client for a source. Errors are plain language for the UI.
/// Returns the client, stream flags, mode, description, and the process to watch (if any):
/// process loopback keeps delivering silence after its target exits, so the capture loop must
/// notice the exit itself and reconnect (to the app's next instance).
type Opened = (IAudioClient, u32, &'static str, String, Option<u32>, Option<Endpoints::IAudioEndpointVolume>);

fn open(spec: &SourceSpec) -> Result<Opened, String> {
    let base = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    let loopback = base | AUDCLNT_STREAMFLAGS_LOOPBACK;
    match spec {
        SourceSpec::AllExceptCapture => Ok((
            process_loopback_client(capture_tree_root(), false).map_err(|e| format!("Could not capture computer audio ({e})"))?,
            loopback,
            "process-loopback-exclude",
            "everything you hear, except CAPTURE's own sounds".into(),
            None,
            None,
        )),
        SourceSpec::Process { pid } => Ok((
            process_loopback_client(*pid, true).map_err(|e| format!("Could not capture this app's audio ({e})"))?,
            loopback,
            "process-loopback",
            format!("process {pid} and its child processes"),
            Some(*pid),
            None,
        )),
        SourceSpec::App { exe } => {
            let pid = devices::root_process(exe).ok_or_else(|| format!("{exe} isn't running. It will be recorded as soon as it starts."))?;
            Ok((
                process_loopback_client(pid, true).map_err(|e| format!("Could not capture {exe} ({e})"))?,
                loopback,
                "process-loopback",
                format!("{exe} (process {pid}) and its child processes"),
                Some(pid),
                None,
            ))
        }
        SourceSpec::Mic { device_id } => unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
            let dev = match device_id {
                Some(id) => {
                    let d = en.GetDevice(&HSTRING::from(id.as_str())).map_err(|_| "Your chosen microphone is not installed any more. Choose another.".to_string())?;
                    if d.GetState().map(|s| s != DEVICE_STATE_ACTIVE).unwrap_or(true) {
                        return Err(format!("{} is disconnected. Reconnect it, or choose another microphone.", devices::name_of(&d)));
                    }
                    d
                }
                None => en.GetDefaultAudioEndpoint(eCapture, eConsole).map_err(|_| "No microphone is connected.".to_string())?,
            };
            let name = devices::name_of(&dev);
            let client = dev.Activate::<IAudioClient>(CLSCTX_ALL, None).map_err(|_| format!("{name} could not be opened. Is another app using it exclusively?"))?;
            let level = dev.Activate::<Endpoints::IAudioEndpointVolume>(CLSCTX_ALL, None).ok();
            Ok((client, base, "microphone", name, None, level))
        },
    }
}

fn capture_source(spec: &SourceSpec, buf: &Mutex<TrackBuf>, t0_qpc: u64, stop: &AtomicBool) -> Result<(), String> {
    let (client, flags, mode, source, watch, level) = open(spec)?;
    let level_hint = |name: &str| {
        level.as_ref().and_then(|v| unsafe {
            let (m, l) = (v.GetMute().ok()?.as_bool(), v.GetMasterVolumeLevelScalar().ok()?);
            devices::mic_level_problem(name, m, l)
        })
    };
    let mut level_checked = Instant::now();
    // "Default microphone" follows Windows: when the default input changes (a headset plugged
    // in), reopen on the new one instead of recording the old device.
    let follows_default = matches!(spec, SourceSpec::Mic { device_id: None });
    let default_at_open = if follows_default { devices::default_id(eCapture) } else { None };
    let watched = watch.and_then(|pid| unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok() }).map(HandleGuard);
    // Loopback sources are captured in the output device's own channel layout and downmixed
    // here (see `Downmix`): Windows' own conversion of a surround mix to stereo lowers the
    // level by ~14 dB on a 7.1 device. Microphones are captured as stereo directly.
    let loopback = flags & AUDCLNT_STREAMFLAGS_LOOPBACK != 0;
    // What the layout was chosen from; a change (headset plugged in, speaker setup changed in
    // Windows) reopens the source so the new device's layout is used.
    let output_at_open = if loopback { (devices::default_id(eRender), render_layout()) } else { (None, None) };
    let fmt = float_format();
    let mut client = client;
    let mut downmix = None;
    if let Some((n, mask)) = output_at_open.1 {
        let ext = float_format_ext(n as u16, mask);
        match unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, &ext as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX, None) } {
            Ok(()) => downmix = Some(Downmix::new(n, mask)),
            Err(e) => {
                // Never lose the audio over this: Windows' own stereo conversion (quieter on
                // surround devices) is still better than nothing.
                eprintln!("{n}-channel capture refused ({e}); capturing stereo");
                client = open(spec)?.0;
            }
        }
    }
    if downmix.is_none() {
        unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, &fmt, None) }.map_err(|e| e.to_string())?;
    }
    let in_ch = downmix.as_ref().map_or(CH, |d| d.channels);
    let mut output_checked = Instant::now();
    let mut stereo: Vec<f32> = Vec::new();
    let ev = unsafe { CreateEventW(None, false, false, None) }.map_err(|e| e.to_string())?;
    let _guard = HandleGuard(ev);
    unsafe { client.SetEventHandle(ev) }.map_err(|e| e.to_string())?;
    let cap: IAudioCaptureClient = unsafe { client.GetService() }.map_err(|e| e.to_string())?;
    unsafe { client.Start() }.map_err(|e| e.to_string())?;
    {
        let mut b = buf.lock().unwrap();
        b.mode = mode.into();
        b.hint = level_hint(&source);
        b.source = source.clone();
        b.error = None;
        b.connected_at = Some(Instant::now());
        b.heard_since_connect = false;
    }
    let lost = |what: &str| match spec {
        SourceSpec::Mic { .. } => format!("The microphone stopped ({what}). Reconnect it; CAPTURE will pick it up again."),
        SourceSpec::App { exe } => format!("{exe} stopped. It will be recorded again when it restarts."),
        _ => format!("Audio source stopped ({what})."),
    };
    while !stop.load(Ordering::Relaxed) {
        unsafe { WaitForSingleObject(ev, 100) };
        if level.is_some() && level_checked.elapsed() >= Duration::from_secs(1) {
            // The user may unmute it in Windows while we record: follow it live.
            level_checked = Instant::now();
            let h = level_hint(&source);
            buf.lock().unwrap().hint = h;
            if follows_default && devices::default_id(eCapture).is_some_and(|now| Some(&now) != default_at_open.as_ref()) {
                unsafe {
                    let _ = client.Stop();
                }
                return Err(REOPEN.into());
            }
        }
        if loopback && output_checked.elapsed() >= Duration::from_secs(1) {
            output_checked = Instant::now();
            let now = (devices::default_id(eRender), render_layout());
            if now != output_at_open {
                eprintln!("audio output changed ({:?} -> {:?} channels); reopening {source}", output_at_open.1.map(|l| l.0), now.1.map(|l| l.0));
                unsafe {
                    let _ = client.Stop();
                }
                return Err(REOPEN.into());
            }
        }
        if let Some(h) = &watched {
            if unsafe { WaitForSingleObject(h.0, 0) } == WAIT_OBJECT_0 {
                unsafe {
                    let _ = client.Stop();
                }
                return Err(lost("the app closed"));
            }
        }
        loop {
            let n = unsafe { cap.GetNextPacketSize() }.map_err(|e| lost(&e.message()))?;
            if n == 0 {
                break;
            }
            let mut data = std::ptr::null_mut();
            let (mut frames, mut bflags, mut qpc) = (0u32, 0u32, 0u64);
            unsafe { cap.GetBuffer(&mut data, &mut frames, &mut bflags, None, Some(&mut qpc)) }.map_err(|e| lost(&e.message()))?;
            // A packet's capture time is at most a buffer or two in the past. A timestamp the
            // device marks as wrong, or one far from now (0 or garbage from some drivers), would
            // otherwise make the track silent forever or pad gigabytes of zeros: use the clock.
            let now_idx = clock::qpc100ns_to_samples(clock::now_100ns(), t0_qpc, RATE);
            let idx = plausible_index(clock::qpc100ns_to_samples(qpc, t0_qpc, RATE), now_idx, frames, bflags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0);
            let silent = bflags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            let samples = (!silent).then(|| unsafe { std::slice::from_raw_parts(data as *const f32, frames as usize * in_ch) });
            let samples = match (samples, &downmix) {
                (Some(s), Some(d)) => {
                    d.run(s, &mut stereo);
                    Some(&stereo[..])
                }
                (s, _) => s,
            };
            buf.lock().unwrap().push(idx, samples, frames as usize);
            unsafe { cap.ReleaseBuffer(frames) }.map_err(|e| lost(&e.message()))?;
        }
    }
    unsafe {
        let _ = client.Stop();
    }
    Ok(())
}

/// The timeline index to file a packet at: its own timestamp, unless the device flagged it as
/// wrong or it is more than 2 s away from the capture clock (then: just before now).
fn plausible_index(idx: i64, now_idx: i64, frames: u32, flagged: bool) -> i64 {
    if flagged || (idx - now_idx).abs() > 2 * RATE as i64 {
        now_idx - frames as i64
    } else {
        idx
    }
}

struct HandleGuard(HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Done(isize);

impl IActivateAudioInterfaceCompletionHandler_Impl for Done_Impl {
    fn ActivateCompleted(&self, _op: windows_core::Ref<'_, IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        unsafe { SetEvent(HANDLE(self.0 as *mut _)) }
    }
}

/// x64 PROPVARIANT layout carrying a VT_BLOB.
#[repr(C)]
struct BlobPropVariant {
    vt: u16,
    r: [u16; 3],
    cb: u32,
    pad: u32,
    data: *const u8,
}

/// Root of CAPTURE's process tree for exclusion: the UI (`CAPTURE.exe`, which starts this
/// worker and its WebView2 processes, where clips are played back) when it said so, else this
/// worker. Excluding only the worker would record a clip played in CAPTURE's window.
fn capture_tree_root() -> u32 {
    std::env::var("CAPTURE_UI_PID").ok().and_then(|v| v.parse::<u32>().ok()).filter(|&p| p != 0).unwrap_or_else(std::process::id)
}

/// `include = true`: the process tree's audio only. `false`: everything except that tree.
fn process_loopback_client(pid: u32, include: bool) -> windows::core::Result<IAudioClient> {
    // On the heap so they can outlive this call if activation times out (see below).
    let params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: if include {
                    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
                } else {
                    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
                },
            },
        },
    });
    let pv = Box::new(BlobPropVariant {
        vt: 65, // VT_BLOB
        r: [0; 3],
        cb: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        pad: 0,
        data: &*params as *const _ as *const u8,
    });
    const _: () = assert!(std::mem::size_of::<BlobPropVariant>() == std::mem::size_of::<PROPVARIANT>());
    unsafe {
        let ev = CreateEventW(None, true, false, None)?;
        let _g = HandleGuard(ev);
        let handler: IActivateAudioInterfaceCompletionHandler = Done(ev.0 as isize).into();
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*pv as *const _ as *const PROPVARIANT),
            &handler,
        )?;
        if WaitForSingleObject(ev, 5000) != WAIT_OBJECT_0 {
            // The activation may still complete later and then reads `params` and signals
            // `ev`: keep both alive for good (a few bytes, only on this rare path) instead of
            // freeing memory and closing a handle that may be reused by then.
            std::mem::forget(_g);
            std::mem::forget(handler);
            std::mem::forget(op);
            Box::leak(params);
            Box::leak(pv);
            return Err(windows::core::Error::new(windows::core::HRESULT(0x8000_000Au32 as i32), "activation timed out"));
        }
        let mut hr = windows::core::HRESULT(0);
        let mut iface = None;
        op.GetActivateResult(&mut hr, &mut iface)?;
        hr.ok()?;
        iface.ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(-1)))?.cast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_gap_is_padded_and_drift_is_corrected() {
        let mut b = TrackBuf::default();
        let s = vec![0.5f32; 480 * CH];
        b.push(1000, Some(&s), 480);
        assert_eq!(b.base, 0);
        assert_eq!(b.write_pos(), 1480);
        b.push(1490, Some(&s), 480);
        assert_eq!(b.write_pos(), 1960);
        b.push(1960 + 4800, Some(&s), 480);
        assert_eq!(b.write_pos(), 1960 + 4800 + 480);
        let w = b.write_pos();
        b.push(w - 2000, Some(&s), 480);
        assert_eq!(b.write_pos(), w);
        assert!(b.signal_seen);
    }

    #[test]
    fn implausible_device_timestamps_follow_the_clock() {
        let now = 480_000; // 10 s into the session
        assert_eq!(plausible_index(now - 960, now, 480, false), now - 960, "normal latency kept");
        assert_eq!(plausible_index(-1_000_000_000, now, 480, false), now - 480, "qpc = 0");
        assert_eq!(plausible_index(i64::MAX / 4, now, 480, false), now - 480, "far future");
        assert_eq!(plausible_index(now - 960, now, 480, true), now - 480, "flagged by the device");
        // A monitor never stores anything, silent packets included.
        let mut m = TrackBuf::monitor();
        m.push(0, None, 4800);
        assert!(m.data.is_empty());
    }

    #[test]
    fn packets_before_session_start_are_trimmed() {
        let mut b = TrackBuf::default();
        let s = vec![0.1f32; 480 * CH];
        b.push(-100, Some(&s), 480);
        assert_eq!(b.base, 0);
        assert_eq!(b.write_pos(), 380);
    }

    #[test]
    fn stalled_track_is_padded_with_silence() {
        let mut b = TrackBuf::default();
        b.pad_to(4800);
        assert_eq!(b.write_pos(), 4800);
        assert!(b.data.iter().all(|&x| x == 0.0));
        assert!(!b.signal_seen);
    }

    #[test]
    fn monitor_buffers_measure_without_storing() {
        let mut b = TrackBuf { discard: true, ..Default::default() };
        b.push(0, Some(&vec![0.25f32; 4800 * CH]), 4800);
        assert!(b.data.is_empty());
        assert!((b.live_peak() - 0.25).abs() < 1e-6);
    }

    #[test]
    fn silent_microphone_is_explained_and_clears_when_sound_arrives() {
        let mut b = TrackBuf { mode: "microphone".into(), source: "Mic X".into(), ..Default::default() };
        b.connected_at = Some(Instant::now());
        assert_eq!(b.problem(), None, "no verdict during the first seconds");
        b.connected_at = Some(Instant::now() - Duration::from_secs(6));
        b.push(0, None, 4800); // AUDCLNT_BUFFERFLAGS_SILENT packets
        b.push(4800, Some(&vec![0.0f32; 480 * CH]), 480); // digital zeros
        assert!(b.problem().unwrap().contains("complete silence"));
        b.hint = devices::mic_level_problem("Mic X", true, 0.5);
        assert!(b.problem().unwrap().contains("Windows has Mic X muted"), "a named Windows mute wins");
        b.hint = None;
        b.push(5280, Some(&vec![0.001f32; 480 * CH]), 480); // room noise
        assert_eq!(b.problem(), None);
        // Loopback sources are often silent for good reasons: never flagged.
        let mut g = TrackBuf { mode: "process-loopback".into(), ..Default::default() };
        g.connected_at = Some(Instant::now() - Duration::from_secs(60));
        assert_eq!(g.problem(), None);
        assert!(devices::mic_level_problem("M", false, 0.0).unwrap().contains("0%"));
        assert_eq!(devices::mic_level_problem("M", false, 0.3), None);
    }

    #[test]
    fn meter_window_reports_peak_and_rms_then_resets() {
        let mut b = TrackBuf::default();
        let s: Vec<f32> = (0..4800).flat_map(|i| { let v = if i % 2 == 0 { 0.5 } else { -0.5 }; [v, v] }).collect();
        b.push(0, Some(&s), 4800);
        let (p, r) = b.take_level();
        assert!((p - 0.5).abs() < 1e-6 && (r - 0.5).abs() < 1e-3);
        assert_eq!(b.take_level(), (0.0, 0.0));
    }

    #[test]
    fn surround_downmix_keeps_stereo_exact_and_full_level() {
        let d = super::Downmix::new(8, 0x63F); // 7.1: FL FR FC LFE BL BR SL SR
        let mut out = Vec::new();
        // Stereo content on the front pair: unchanged.
        d.run(&[0.5, -0.25, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], &mut out);
        assert_eq!(out, vec![0.5, -0.25]);
        // Centre at -3 dB to both, LFE dropped, surrounds to their side.
        d.run(&[0.0, 0.0, 0.4, 1.0, 0.2, 0.0, 0.0, 0.2], &mut out);
        let k = std::f32::consts::FRAC_1_SQRT_2;
        assert!((out[0] - (0.4 * k + 0.2 * k)).abs() < 1e-6 && (out[1] - (0.4 * k + 0.2 * k)).abs() < 1e-6);
        // Everything at full scale: limited below 1, not clipped.
        d.run(&[1.0; 8], &mut out);
        assert!(out[0] <= 1.0 && out[0] > 0.9, "limited to full scale, not beyond");
        // Loud stereo near full scale is not touched by the limiter.
        d.run(&[0.98, -0.99, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], &mut out);
        assert_eq!(out, vec![0.98, -0.99]);
    }

    /// Every common speaker setup: each channel lands where it belongs, at the right level,
    /// and stereo content on the front pair is bit-exact.
    #[test]
    fn downmix_handles_every_common_layout() {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        // (name, mask, expected (left, right) gain per channel in stream order)
        let layouts: &[(&str, u32, &[(f32, f32)])] = &[
            ("quad", 0x33, &[(1.0, 0.0), (0.0, 1.0), (k, 0.0), (0.0, k)]),
            ("5.1 back", 0x3F, &[(1.0, 0.0), (0.0, 1.0), (k, k), (0.0, 0.0), (k, 0.0), (0.0, k)]),
            ("5.1 side", 0x60F, &[(1.0, 0.0), (0.0, 1.0), (k, k), (0.0, 0.0), (k, 0.0), (0.0, k)]),
            ("7.1", 0x63F, &[(1.0, 0.0), (0.0, 1.0), (k, k), (0.0, 0.0), (k, 0.0), (0.0, k), (k, 0.0), (0.0, k)]),
            (
                "7.1.4",
                0x2D63F,
                &[(1.0, 0.0), (0.0, 1.0), (k, k), (0.0, 0.0), (k, 0.0), (0.0, k), (k, 0.0), (0.0, k), (k, 0.0), (0.0, k), (k, 0.0), (0.0, k)],
            ),
        ];
        for &(name, mask, want) in layouts {
            let n = want.len();
            assert_eq!(mask.count_ones() as usize, n, "{name}");
            let d = super::Downmix::new(n, mask);
            let mut out = Vec::new();
            for (i, &(wl, wr)) in want.iter().enumerate() {
                let mut frame = vec![0.0f32; n];
                frame[i] = 0.5;
                d.run(&frame, &mut out);
                assert!((out[0] - 0.5 * wl).abs() < 1e-6 && (out[1] - 0.5 * wr).abs() < 1e-6, "{name} channel {i}: got {out:?}");
            }
            let mut frame = vec![0.0f32; n];
            frame[0] = 0.7;
            frame[1] = -0.3;
            d.run(&frame, &mut out);
            assert_eq!(out, vec![0.7, -0.3], "{name}: stereo exact");
        }
        assert_eq!(super::default_mask(6), Some(0x3F));
        assert_eq!(super::default_mask(8), Some(0x63F));
    }

    #[test]
    fn soft_clip_is_transparent_then_bounded() {
        assert_eq!(soft_clip(0.5), 0.5);
        assert_eq!(soft_clip(-0.9), -0.9);
        assert!(soft_clip(3.0) <= 1.0 && soft_clip(3.0) > 0.99);
        assert!(soft_clip(-3.0) >= -1.0);
    }

    #[test]
    fn other_desktop_is_never_mixed_and_track_order_is_stable() {
        let srcs = vec![
            Source { spec: SourceSpec::Process { pid: 1 }, role: Role::Game, label: "Game Audio".into(), in_mix: true },
            Source { spec: SourceSpec::Mic { device_id: None }, role: Role::Mic, label: "Microphone".into(), in_mix: true },
            Source { spec: SourceSpec::App { exe: "Discord.exe".into() }, role: Role::Voice, label: "Voice Chat".into(), in_mix: true },
            Source { spec: SourceSpec::AllExceptCapture, role: Role::Other, label: "All Desktop Audio".into(), in_mix: false },
        ];
        assert_eq!(track_names(&srcs), ["Preview Mix", "Game Audio", "Microphone", "Voice Chat", "All Desktop Audio"]);
        assert_eq!(MixParams::default().gain(Role::Other), 0.0);
    }
}
