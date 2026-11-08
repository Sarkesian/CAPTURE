//! WASAPI audio sources aligned to the session timeline, and AAC encoding of
//! `Preview Mix` + one isolated track per source.
//!
//! Alignment: every packet's QPC capture time is converted to a timeline sample index. The
//! source buffer pads silence for gaps (including the leading gap) and drops samples if the
//! device clock runs ahead, so sample N of every track always means the same instant.
//! Endpoint loopback delivers *no* packets while nothing is playing; the encoder pads stalled
//! tracks against the wall clock so silence never shifts later audio.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::TrackState;
use ffmpeg_sys_next as ff;
use std::ptr::null_mut;
use windows::core::{Interface, HSTRING};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows_core::implement;

use crate::av::{self, check, Packet, Rational};
use crate::clock;

pub const RATE: u32 = 48_000;
const CH: usize = 2;
const DRIFT_TOLERANCE: i64 = 960; // 20 ms
const STALL_PAD_AFTER: i64 = 9_600; // 200 ms behind wall clock => device is silent/stalled
const STALL_PAD_TO: i64 = 4_800; // pad up to 100 ms behind wall clock

#[derive(Debug, Clone)]
pub enum SourceSpec {
    /// Audio rendered by one process and its children (Windows build ≥ 20348).
    Process { pid: u32, label: String },
    /// Whole output device mix. `None` = default output.
    EndpointLoopback { device_id: Option<String>, label: String },
}

impl SourceSpec {
    pub fn label(&self) -> &str {
        match self {
            SourceSpec::Process { label, .. } | SourceSpec::EndpointLoopback { label, .. } => label,
        }
    }
}

#[derive(Default)]
struct TrackBuf {
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
}

impl TrackBuf {
    fn write_pos(&self) -> i64 {
        self.base + (self.data.len() / CH) as i64
    }

    /// Append `frames` interleaved stereo frames captured at timeline index `idx`.
    fn push(&mut self, idx: i64, samples: Option<&[f32]>, frames: usize) {
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
                let p = s.iter().fold(0f32, |m, x| m.max(x.abs()));
                if p > 0.0 {
                    self.signal_seen = true;
                }
                if self.peak_at.is_none_or(|t| t.elapsed() > Duration::from_millis(100)) || p > self.peak {
                    self.peak = p;
                    self.peak_at = Some(Instant::now());
                }
                self.data.extend(s.iter().copied());
            }
            None => self.data.extend(std::iter::repeat_n(0.0, (frames - skip) * CH)),
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
}

pub struct AudioPipeline {
    bufs: Vec<Arc<Mutex<TrackBuf>>>,
    labels: Vec<String>,
    threads: Vec<std::thread::JoinHandle<()>>,
    params: Vec<(*mut ff::AVCodecParameters, Rational)>,
}

unsafe impl Send for AudioPipeline {}

/// Track names in output order: `Preview Mix` then one per source.
pub fn track_names(sources: &[SourceSpec]) -> Vec<String> {
    if sources.is_empty() {
        return vec![];
    }
    std::iter::once("Preview Mix".to_string()).chain(sources.iter().map(|s| s.label().to_string())).collect()
}

impl AudioPipeline {
    /// `first_track` is the muxer track index of `Preview Mix`.
    pub fn start(
        sources: Vec<SourceSpec>,
        t0_qpc: u64,
        stop: Arc<AtomicBool>,
        first_track: usize,
        out: impl Fn(Packet) + Send + 'static,
    ) -> Result<Self, String> {
        let names = track_names(&sources);
        let bufs: Vec<_> = sources.iter().map(|_| Arc::new(Mutex::new(TrackBuf::default()))).collect();
        let mut threads = Vec::new();
        for (spec, buf) in sources.iter().cloned().zip(bufs.iter().cloned()) {
            let stop = stop.clone();
            threads.push(
                std::thread::Builder::new()
                    .name(format!("audio:{}", spec.label()))
                    .spawn(move || {
                        if let Err(e) = capture_source(&spec, &buf, t0_qpc, &stop) {
                            buf.lock().unwrap().error = Some(e);
                        }
                    })
                    .unwrap(),
            );
        }
        let mut encoders = Vec::new();
        for _ in &names {
            encoders.push(unsafe { AacEncoder::open()? });
        }
        let params = encoders.iter().map(|e| (e.params(), e.time_base())).collect();
        if !names.is_empty() {
            let bufs2 = bufs.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("audio-encode".into())
                    .spawn(move || encode_loop(bufs2, encoders, t0_qpc, stop, first_track, out))
                    .unwrap(),
            );
        }
        Ok(Self { bufs, labels: names, threads, params })
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
        let srcs: Vec<_> = self.bufs.iter().map(|b| b.lock().unwrap()).collect();
        if !srcs.is_empty() {
            let peak = srcs.iter().map(|b| live_peak(b)).fold(0f32, f32::max);
            v.push(TrackState {
                name: "Preview Mix".into(),
                mode: "mix".into(),
                source: format!("{} source(s)", srcs.len()),
                peak,
                signal_seen: srcs.iter().any(|b| b.signal_seen),
                error: None,
            });
        }
        for (b, name) in srcs.iter().zip(self.labels.iter().skip(1)) {
            v.push(TrackState {
                name: name.clone(),
                mode: b.mode.clone(),
                source: b.source.clone(),
                peak: live_peak(b),
                signal_seen: b.signal_seen,
                error: b.error.clone(),
            });
        }
        v
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

fn live_peak(b: &TrackBuf) -> f32 {
    if b.peak_at.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
        b.peak
    } else {
        0.0
    }
}

fn encode_loop(
    bufs: Vec<Arc<Mutex<TrackBuf>>>,
    mut encoders: Vec<AacEncoder>,
    t0_qpc: u64,
    stop: Arc<AtomicBool>,
    first_track: usize,
    out: impl Fn(Packet),
) {
    let frame = 1024usize;
    let mut next: i64 = 0;
    let mut src = vec![vec![0f32; frame * CH]; bufs.len()];
    let mut mix = vec![0f32; frame * CH];
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
            mix.fill(0.0);
            for (i, b) in bufs.iter().enumerate() {
                let mut b = b.lock().unwrap();
                let skip = ((next - b.base).max(0) as usize) * CH;
                let n = skip.min(b.data.len());
                b.data.drain(..n);
                b.base = b.base.max(next);
                for (j, s) in b.data.drain(..frame * CH).enumerate() {
                    src[i][j] = s;
                    mix[j] += s;
                }
                b.base += frame as i64;
            }
            let mut emit = |p: Packet| out(p);
            let _ = encoders[0].encode(&mix, next, first_track, &mut emit);
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

pub fn device_name(dev: &IMMDevice) -> String {
    unsafe {
        dev.OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "Unknown device".into())
    }
}

fn capture_source(spec: &SourceSpec, buf: &Mutex<TrackBuf>, t0_qpc: u64, stop: &AtomicBool) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let base = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    let (client, flags, mode, source): (IAudioClient, _, _, _) = unsafe {
        match spec {
            SourceSpec::Process { pid, .. } => (
                process_loopback_client(*pid).map_err(|e| format!("Could not capture this app's audio ({e})"))?,
                base | AUDCLNT_STREAMFLAGS_LOOPBACK,
                "process-loopback",
                format!("process {pid} and its child processes"),
            ),
            SourceSpec::EndpointLoopback { device_id, .. } => {
                let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
                let dev = match device_id {
                    Some(id) => en.GetDevice(&HSTRING::from(id.as_str())),
                    None => en.GetDefaultAudioEndpoint(eRender, eConsole),
                }
                .map_err(|_| "The selected output device is not connected".to_string())?;
                (
                    dev.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?,
                    base | AUDCLNT_STREAMFLAGS_LOOPBACK,
                    "endpoint-loopback",
                    format!("everything playing on {}", device_name(&dev)),
                )
            }
        }
    };
    {
        let mut b = buf.lock().unwrap();
        b.mode = mode.into();
        b.source = source;
    }
    let fmt = float_format();
    unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, &fmt, None) }.map_err(|e| e.to_string())?;
    let ev = unsafe { CreateEventW(None, false, false, None) }.map_err(|e| e.to_string())?;
    let _guard = HandleGuard(ev);
    unsafe { client.SetEventHandle(ev) }.map_err(|e| e.to_string())?;
    let cap: IAudioCaptureClient = unsafe { client.GetService() }.map_err(|e| e.to_string())?;
    unsafe { client.Start() }.map_err(|e| e.to_string())?;
    while !stop.load(Ordering::Relaxed) {
        unsafe { WaitForSingleObject(ev, 100) };
        loop {
            let n = unsafe { cap.GetNextPacketSize() }.map_err(|_| "Audio source stopped (device or app went away)".to_string())?;
            if n == 0 {
                break;
            }
            let mut data = std::ptr::null_mut();
            let (mut frames, mut bflags, mut qpc) = (0u32, 0u32, 0u64);
            unsafe { cap.GetBuffer(&mut data, &mut frames, &mut bflags, None, Some(&mut qpc)) }.map_err(|e| e.to_string())?;
            let idx = clock::qpc100ns_to_samples(qpc, t0_qpc, RATE);
            let silent = bflags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            let samples = (!silent).then(|| unsafe { std::slice::from_raw_parts(data as *const f32, frames as usize * CH) });
            buf.lock().unwrap().push(idx, samples, frames as usize);
            unsafe { cap.ReleaseBuffer(frames) }.map_err(|e| e.to_string())?;
        }
    }
    unsafe {
        let _ = client.Stop();
    }
    Ok(())
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

fn process_loopback_client(pid: u32) -> windows::core::Result<IAudioClient> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let pv = BlobPropVariant {
        vt: 65, // VT_BLOB
        r: [0; 3],
        cb: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        pad: 0,
        data: &params as *const _ as *const u8,
    };
    const _: () = assert!(std::mem::size_of::<BlobPropVariant>() == std::mem::size_of::<PROPVARIANT>());
    unsafe {
        let ev = CreateEventW(None, true, false, None)?;
        let _g = HandleGuard(ev);
        let handler: IActivateAudioInterfaceCompletionHandler = Done(ev.0 as isize).into();
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&pv as *const _ as *const PROPVARIANT),
            &handler,
        )?;
        if WaitForSingleObject(ev, 5000) != WAIT_OBJECT_0 {
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
        // small jitter within tolerance: appended as-is
        b.push(1490, Some(&s), 480);
        assert_eq!(b.write_pos(), 1960);
        // 100 ms gap: silence inserted so the packet lands at its real time
        b.push(1960 + 4800, Some(&s), 480);
        assert_eq!(b.write_pos(), 1960 + 4800 + 480);
        // device clock behind: overlapping samples dropped
        let w = b.write_pos();
        b.push(w - 2000, Some(&s), 480);
        assert_eq!(b.write_pos(), w); // all 480 overlapped -> dropped
        assert!(b.signal_seen);
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
}
