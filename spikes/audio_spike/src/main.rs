//! Phase 0 audio spike.
//!
//! `audio_spike list`
//!     Lists active capture (mic) and render (output) endpoints with friendly name + endpoint ID.
//! `audio_spike record <secs> <out_dir> [mic=<id|default>] [loopback=<id|default>] [proc:<label>=<pid> ...]`
//!     Records every requested source concurrently as 48 kHz stereo float WAV, each stream
//!     aligned to one shared QPC timeline (leading silence is inserted so all files start at
//!     the same instant), and prints per-stream statistics as JSON lines.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use windows::core::{Interface, GUID, HSTRING, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::*;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows_core::implement;

const RATE: u32 = 48_000;
const CH: u16 = 2;

fn float_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: CH,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * CH as u32 * 4,
        nBlockAlign: CH * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

#[derive(Clone, Debug)]
enum Source {
    Mic(Option<String>),
    Loopback(Option<String>),
    Process { label: String, pid: u32 },
}

impl Source {
    fn label(&self) -> String {
        match self {
            Source::Mic(_) => "mic".into(),
            Source::Loopback(_) => "loopback".into(),
            Source::Process { label, .. } => label.clone(),
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:?}");
        std::process::exit(1);
    }
}

fn run() -> windows::core::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("play") {
        let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10.0);
        let speaker: u32 = args.get(2).and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()).unwrap_or(0x4);
        let amp: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.01);
        return play_71(secs, speaker, amp);
    }
    if args.first().map(String::as_str) == Some("native") {
        let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
        return native_levels(secs, args.get(2).and_then(|s| s.parse().ok()));
    }
    match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("record") => {
            let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).expect("secs");
            let out = std::path::PathBuf::from(args.get(2).expect("out_dir"));
            std::fs::create_dir_all(&out).expect("create out dir");
            let mut sources = Vec::new();
            for a in &args[3..] {
                if let Some(v) = a.strip_prefix("mic=") {
                    sources.push(Source::Mic((v != "default").then(|| v.to_string())));
                } else if let Some(v) = a.strip_prefix("loopback=") {
                    sources.push(Source::Loopback((v != "default").then(|| v.to_string())));
                } else if let Some(v) = a.strip_prefix("proc:") {
                    let (label, pid) = v.split_once('=').expect("proc:<label>=<pid>");
                    sources.push(Source::Process { label: label.into(), pid: pid.parse().expect("pid") });
                }
            }
            record(secs, &out, sources)
        }
        _ => {
            eprintln!("usage: audio_spike list | record <secs> <out_dir> [mic=..] [loopback=..] [proc:label=pid]");
            Ok(())
        }
    }
}

fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn device_name(dev: &IMMDevice) -> String {
    unsafe {
        dev.OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "<unknown>".into())
    }
}

fn device_id(dev: &IMMDevice) -> String {
    unsafe { dev.GetId().ok().and_then(|p| p.to_string().ok()).unwrap_or_default() }
}

fn list() -> windows::core::Result<()> {
    let en = enumerator()?;
    for (flow, kind) in [(eCapture, "input"), (eRender, "output")] {
        let default_id = unsafe { en.GetDefaultAudioEndpoint(flow, eConsole) }
            .map(|d| device_id(&d))
            .unwrap_or_default();
        let coll = unsafe { en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
        for i in 0..unsafe { coll.GetCount()? } {
            let d = unsafe { coll.Item(i)? };
            let id = device_id(&d);
            println!(
                "{{\"kind\":\"{kind}\",\"default\":{},\"name\":{:?},\"id\":{:?}}}",
                id == default_id,
                device_name(&d),
                id
            );
        }
    }
    Ok(())
}

fn endpoint(flow: EDataFlow, id: &Option<String>) -> windows::core::Result<IMMDevice> {
    let en = enumerator()?;
    unsafe {
        match id {
            Some(id) => en.GetDevice(&HSTRING::from(id.as_str())),
            None => en.GetDefaultAudioEndpoint(flow, eConsole),
        }
    }
}

// ---- process loopback activation ------------------------------------------------------

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Done(isize);

impl IActivateAudioInterfaceCompletionHandler_Impl for Done_Impl {
    fn ActivateCompleted(
        &self,
        _op: windows_core::Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        unsafe { SetEvent(HANDLE(self.0 as *mut _)) }
    }
}

/// Raw x64 PROPVARIANT layout holding a VT_BLOB, built by hand so the activation params
/// blob can be passed without depending on PROPVARIANT constructor helpers.
#[repr(C)]
struct BlobPropVariant {
    vt: u16,
    r1: u16,
    r2: u16,
    r3: u16,
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
        r1: 0,
        r2: 0,
        r3: 0,
        cb: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        pad: 0,
        data: &params as *const _ as *const u8,
    };
    assert_eq!(std::mem::size_of::<BlobPropVariant>(), std::mem::size_of::<PROPVARIANT>());
    unsafe {
        let ev = CreateEventW(None, true, false, None)?;
        let handler: IActivateAudioInterfaceCompletionHandler = Done(ev.0 as isize).into();
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&pv as *const _ as *const PROPVARIANT),
            &handler,
        )?;
        let w = WaitForSingleObject(ev, 5000);
        let _ = CloseHandle(ev);
        if w != WAIT_OBJECT_0 {
            return Err(windows::core::Error::new(windows::core::HRESULT(-1), "activation timeout"));
        }
        let mut hr = windows::core::HRESULT(0);
        let mut iface: Option<windows::core::IUnknown> = None;
        op.GetActivateResult(&mut hr, &mut iface)?;
        hr.ok()?;
        iface.unwrap().cast()
    }
}

// ---- capture ---------------------------------------------------------------------------

struct Stats {
    label: String,
    device: String,
    mode: &'static str,
    first_qpc_100ns: Option<u64>,
    frames: u64,
    packets: u64,
    silent_packets: u64,
    discontinuities: u64,
    peak: f32,
    sum_sq: f64,
    samples: Vec<f32>,
}

fn open(src: &Source) -> windows::core::Result<(IAudioClient, String, &'static str, u32)> {
    let base = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    unsafe {
        match src {
            Source::Mic(id) => {
                let d = endpoint(eCapture, id)?;
                Ok((d.Activate(CLSCTX_ALL, None)?, device_name(&d), "endpoint-capture", base))
            }
            Source::Loopback(id) => {
                let d = endpoint(eRender, id)?;
                Ok((
                    d.Activate(CLSCTX_ALL, None)?,
                    device_name(&d),
                    "endpoint-loopback (whole device mix)",
                    base | AUDCLNT_STREAMFLAGS_LOOPBACK,
                ))
            }
            Source::Process { pid, .. } => Ok((
                process_loopback_client(*pid)?,
                format!("pid {pid} (+children)"),
                "process-loopback",
                base | AUDCLNT_STREAMFLAGS_LOOPBACK,
            )),
        }
    }
}

fn capture(src: Source, stop: Arc<AtomicBool>, go: Arc<Barrier>) -> windows::core::Result<Stats> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let (client, device, mode, flags) = open(&src)?;
    let fmt = float_format();
    unsafe {
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, &fmt, None)?;
    }
    let ev = unsafe { CreateEventW(None, false, false, None)? };
    unsafe { client.SetEventHandle(ev)? };
    let cap: IAudioCaptureClient = unsafe { client.GetService()? };
    let mut st = Stats {
        label: src.label(),
        device,
        mode,
        first_qpc_100ns: None,
        frames: 0,
        packets: 0,
        silent_packets: 0,
        discontinuities: 0,
        peak: 0.0,
        sum_sq: 0.0,
        samples: Vec::with_capacity(RATE as usize * 2 * 30),
    };
    go.wait();
    unsafe { client.Start()? };
    while !stop.load(Ordering::Relaxed) {
        unsafe { WaitForSingleObject(ev, 100) };
        loop {
            let n = unsafe { cap.GetNextPacketSize()? };
            if n == 0 {
                break;
            }
            let mut data = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut bflags = 0u32;
            let mut qpc = 0u64;
            unsafe { cap.GetBuffer(&mut data, &mut frames, &mut bflags, None, Some(&mut qpc))? };
            st.packets += 1;
            if st.first_qpc_100ns.is_none() {
                st.first_qpc_100ns = Some(qpc);
            }
            if bflags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                st.discontinuities += 1;
            }
            let count = frames as usize * CH as usize;
            if bflags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                st.silent_packets += 1;
                st.samples.extend(std::iter::repeat(0.0).take(count));
            } else {
                let s = unsafe { std::slice::from_raw_parts(data as *const f32, count) };
                for &x in s {
                    st.peak = st.peak.max(x.abs());
                    st.sum_sq += (x as f64) * (x as f64);
                }
                st.samples.extend_from_slice(s);
            }
            st.frames += frames as u64;
            unsafe { cap.ReleaseBuffer(frames)? };
        }
    }
    unsafe {
        let _ = client.Stop();
        let _ = CloseHandle(ev);
    }
    Ok(st)
}

fn qpc_now_100ns() -> u64 {
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
        let _ = QueryPerformanceFrequency(&mut f);
    }
    (c as i128 * 10_000_000 / f as i128) as u64
}

fn record(secs: f64, out: &std::path::Path, sources: Vec<Source>) -> windows::core::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let go = Arc::new(Barrier::new(sources.len() + 1));
    let handles: Vec<_> = sources
        .into_iter()
        .map(|s| {
            let (stop, go) = (stop.clone(), go.clone());
            let label = s.label();
            (label, std::thread::spawn(move || capture(s, stop, go)))
        })
        .collect();
    go.wait();
    let t0 = qpc_now_100ns();
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs_f64(secs));
    stop.store(true, Ordering::Relaxed);
    let elapsed = started.elapsed().as_secs_f64();
    for (label, h) in handles {
        match h.join().expect("thread") {
            Ok(st) => write_stream(out, t0, elapsed, st),
            Err(e) => println!("{{\"label\":{label:?},\"error\":{:?}}}", e.to_string()),
        }
    }
    Ok(())
}

fn write_stream(out: &std::path::Path, t0: u64, elapsed: f64, st: Stats) {
    // Align to the shared timeline: pad leading silence for the gap between t0 and the
    // stream's first packet (or drop samples if the first packet predates t0).
    let lead_frames: i64 = st
        .first_qpc_100ns
        .map(|q| ((q as i64 - t0 as i64) * RATE as i64) / 10_000_000)
        .unwrap_or(0);
    let path = out.join(format!("{}.wav", st.label));
    let spec = hound::WavSpec { channels: CH, sample_rate: RATE, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(&path, spec).expect("wav");
    let skip = if lead_frames < 0 { (-lead_frames) as usize * CH as usize } else { 0 };
    for _ in 0..(lead_frames.max(0) as usize * CH as usize) {
        w.write_sample(0.0f32).unwrap();
    }
    for &s in st.samples.iter().skip(skip) {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
    let n = (st.samples.len().max(1)) as f64;
    println!(
        "{{\"label\":{:?},\"mode\":{:?},\"device\":{:?},\"wav\":{:?},\"lead_ms\":{:.2},\"captured_s\":{:.3},\"wall_s\":{:.3},\"packets\":{},\"silent_packets\":{},\"discontinuities\":{},\"peak\":{:.4},\"rms\":{:.5}}}",
        st.label,
        st.mode,
        st.device,
        path.display().to_string(),
        lead_frames as f64 * 1000.0 / RATE as f64,
        st.frames as f64 / RATE as f64,
        elapsed,
        st.packets,
        st.silent_packets,
        st.discontinuities,
        st.peak,
        (st.sum_sq / n).sqrt()
    );
}

#[allow(dead_code)]
fn _unused(_: GUID, _: PCWSTR) {}

/// Loopback of the default output device in its own mix format (no conversion): peak per channel.
fn native_levels(secs: f64, pid: Option<u32>) -> windows::core::Result<()> {
    unsafe {
        let d = endpoint(eRender, &None)?;
        let dev_client: IAudioClient = d.Activate(CLSCTX_ALL, None)?;
        let mf = dev_client.GetMixFormat()?;
        // With a pid: the same format from process loopback (the app's audio only).
        let client: IAudioClient = match pid { Some(p) => process_loopback_client(p)?, None => dev_client };
        let (ch, bits, tag, rate) = ((*mf).nChannels as usize, (*mf).wBitsPerSample, (*mf).wFormatTag, (*mf).nSamplesPerSec);
        let mask = if tag == 0xFFFE { (*(mf as *const WAVEFORMATEXTENSIBLE)).dwChannelMask } else { 0 };
        eprintln!("device {} mix format: tag {tag:#x} {ch} ch {bits} bit {rate} Hz mask {mask:#x}", device_name(&d));
        let flags = if pid.is_some() { AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM } else { AUDCLNT_STREAMFLAGS_LOOPBACK };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, mf, None)?;
        let cap: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let mut peak = vec![0f32; ch];
        let end = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
            loop {
                let n = cap.GetNextPacketSize()?;
                if n == 0 { break; }
                let (mut data, mut frames, mut fl) = (std::ptr::null_mut(), 0u32, 0u32);
                cap.GetBuffer(&mut data, &mut frames, &mut fl, None, None)?;
                if bits == 32 && fl & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 == 0 {
                    let s = std::slice::from_raw_parts(data as *const f32, frames as usize * ch);
                    for (i, x) in s.iter().enumerate() { peak[i % ch] = peak[i % ch].max(x.abs()); }
                }
                cap.ReleaseBuffer(frames)?;
            }
        }
        let _ = client.Stop();
        let db: Vec<String> = peak.iter().map(|p| format!("{:.1}", 20.0 * p.max(1e-9).log10())).collect();
        println!("{{\"channels\":{ch},\"mask\":{mask},\"peaks\":[{}]}}", peak.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(","));
        eprintln!("peak dBFS per channel: {}", db.join(" | "));
    }
    Ok(())
}

/// Render a 1 kHz tone on one speaker of a 7.1 stream (float, mask 0x63F) to the default output
/// device. Windows converts it if the device has another layout.
fn play_71(secs: f64, speaker: u32, amp: f32) -> windows::core::Result<()> {
    const MASK: u32 = 0x63F;
    let ch = 8usize;
    // `speaker` is a mask: the tone plays on every 7.1 channel it names.
    let on: Vec<bool> = (0..32).filter(|b| MASK & (1 << b) != 0).map(|b| speaker & (1 << b) != 0).collect();
    unsafe {
        let d = endpoint(eRender, &None)?;
        let client: IAudioClient = d.Activate(CLSCTX_ALL, None)?;
        let fmt = WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX { wFormatTag: 0xFFFE, nChannels: ch as u16, nSamplesPerSec: RATE, nAvgBytesPerSec: RATE * ch as u32 * 4, nBlockAlign: ch as u16 * 4, wBitsPerSample: 32, cbSize: 22 },
            Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: 32 },
            dwChannelMask: MASK,
            SubFormat: GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71),
        };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, 2_000_000, 0, &fmt as *const _ as *const WAVEFORMATEX, None)?;
        let size = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;
        let mut phase = 0u64;
        client.Start()?;
        let end = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < end {
            let free = size - client.GetCurrentPadding()?;
            if free > 0 {
                let buf = render.GetBuffer(free)? as *mut f32;
                let s = std::slice::from_raw_parts_mut(buf, free as usize * ch);
                for f in 0..free as usize {
                    let v = amp * (2.0 * std::f64::consts::PI * 1000.0 * phase as f64 / RATE as f64).sin() as f32;
                    phase += 1;
                    for c in 0..ch { s[f * ch + c] = if on[c] { v } else { 0.0 }; }
                }
                render.ReleaseBuffer(free, 0)?;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = client.Stop();
    }
    Ok(())
}
