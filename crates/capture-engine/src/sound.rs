//! Short confirmation chime (marker added, etc.), played by the worker process.
//!
//! Isolation from recordings: game audio is captured with process loopback of the *game*
//! process, and whole-display "System Audio" is captured with process loopback that *excludes*
//! the worker's own process tree (see `audio::SourceSpec::AllExceptCapture`). Neither can
//! contain this sound. The only remaining path is acoustic: a microphone near speakers can hear
//! it, which no software can prevent (documented for users).

use std::time::Duration;
use std::sync::{OnceLock, mpsc::{sync_channel, SyncSender}};
use std::time::Instant;

use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

const RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug)]
pub enum Cue { Start, Stop, Screenshot, Replay, Marker, Refusal }

/// Original, self-contained soft mallet voices. Smooth attack/release, restrained
/// harmonics and matched RMS keep the family quiet and consistent across actions.
pub fn cue_samples(cue: Cue, volume: f32) -> Vec<f32> {
    let notes: &[f32] = match cue {
        Cue::Start => &[392.0, 493.88, 587.33],
        Cue::Stop => &[493.88, 392.0, 329.63],
        Cue::Screenshot => &[659.25, 523.25],
        Cue::Replay => &[440.0, 554.37, 659.25],
        Cue::Marker => &[392.0, 523.25],
        Cue::Refusal => &[349.23, 293.66],
    };
    let n = (RATE as f32 * 0.22) as usize;
    let mut mono = vec![0.0f32; n];
    for (voice, &hz) in notes.iter().enumerate() {
        let onset = voice * (RATE as usize / 50);
        for (i, sample) in mono.iter_mut().enumerate().skip(onset) {
            let t = (i - onset) as f32 / RATE as f32;
            let attack = (t / 0.014).min(1.0);
            let release = ((n - i) as f32 / (RATE as f32 * 0.07)).min(1.0);
            let env = attack * attack * (3.0 - 2.0 * attack) * (-t * 22.0).exp() * release * release;
            let phase = std::f32::consts::TAU * hz * t;
            *sample += (phase.sin() + 0.045 * (phase * 2.0).sin()) * env;
        }
    }
    let rms = (mono.iter().map(|x| x*x).sum::<f32>() / n as f32).sqrt();
    let peak = mono.iter().fold(0.0f32, |a, x| a.max(x.abs()));
    let gain = (0.032 / rms.max(0.001)).min(0.14 / peak.max(0.001)) * volume.clamp(0.0, 1.0);
    mono.into_iter().flat_map(|x| [x * gain, x * gain]).collect()
}

struct Request { cue: Option<Cue>, volume: f32, device: Option<String>, at: Instant, tick: u64 }
static PLAYER: OnceLock<SyncSender<Request>> = OnceLock::new();

fn player() -> &'static SyncSender<Request> {
    PLAYER.get_or_init(|| {
        let (tx, rx) = sync_channel::<Request>(1);
        std::thread::Builder::new().name("feedback-audio".into()).spawn(move || {
            unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
            let mut sink: Option<Sink> = None;
            while let Ok(request) = rx.recv() {
                if request.cue.is_some() && request.at.elapsed() > Duration::from_millis(100) { continue }
                if sink.as_ref().is_none_or(|s| s.device != request.device) {
                    sink = match unsafe { Sink::open(request.device.clone()) } {
                        Ok(s) => Some(s),
                        Err(e) => { eprintln!("feedback audio: {e}"); None }
                    };
                }
                if let (Some(cue), Some(s)) = (request.cue, sink.as_ref()) {
                    if let Err(e) = unsafe { s.play(&cue_samples(cue, request.volume), cue, request.tick) } {
                        eprintln!("feedback audio: {e}");
                        sink = None;
                    }
                    // Sounds never overlap or build a backlog during rapid presses.
                    while rx.try_recv().is_ok() {}
                }
            }
            drop(sink);
            unsafe { CoUninitialize(); }
        }).expect("feedback audio thread");
        tx
    })
}

pub fn warm_output(device: Option<String>) {
    let _ = player().try_send(Request { cue: None, volume: 0.0, device, at: Instant::now(), tick: crate::clock::now_100ns() });
}

pub fn play(cue: Cue, volume: f32, device: Option<String>) {
    if volume > 0.0 {
        let _ = player().try_send(Request { cue: Some(cue), volume, device, at: Instant::now(), tick: crate::clock::now_100ns() });
    }
}

struct Sink {
    client: IAudioClient,
    render: IAudioRenderClient,
    size: u32,
    event: windows::Win32::Foundation::HANDLE,
    device: Option<String>,
}

impl Sink {
    unsafe fn open(device: Option<String>) -> windows::core::Result<Self> {
        use windows::Win32::System::Threading::CreateEventW;
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let chosen = device.as_ref().and_then(|id| en.GetDevice(&windows::core::HSTRING::from(id)).ok())
            .filter(|d| d.GetState().map(|s| s == DEVICE_STATE_ACTIVE).unwrap_or(false));
        let dev = match chosen { Some(d) => d, None => en.GetDefaultAudioEndpoint(eRender, eConsole)? };
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let fmt = WAVEFORMATEX { wFormatTag: 3, nChannels: 2, nSamplesPerSec: RATE, nAvgBytesPerSec: RATE*8, nBlockAlign: 8, wBitsPerSample: 32, cbSize: 0 };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY | AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 200_000, 0, &fmt, None)?;
        let render = client.GetService()?;
        let size = client.GetBufferSize()?;
        let event = CreateEventW(None, false, false, None)?;
        if let Err(e) = client.SetEventHandle(event) {
            let _ = windows::Win32::Foundation::CloseHandle(event);
            return Err(e);
        }
        Ok(Self { client, render, size, event, device })
    }

    unsafe fn play(&self, samples: &[f32], cue: Cue, requested: u64) -> windows::core::Result<()> {
        use windows::Win32::System::Threading::WaitForSingleObject;
        let mut pos = 0;
        let mut started = false;
        let result = (|| {
            while pos < samples.len()/2 || self.client.GetCurrentPadding()? > 0 {
                let free = (self.size - self.client.GetCurrentPadding()?) as usize;
                let n = free.min(samples.len()/2 - pos);
                if n > 0 {
                    let buf = self.render.GetBuffer(n as u32)?;
                    std::ptr::copy_nonoverlapping(samples.as_ptr().add(pos*2), buf as *mut f32, n*2);
                    self.render.ReleaseBuffer(n as u32, 0)?;
                    pos += n;
                }
                if !started {
                    self.client.Start()?; started = true;
                    let now = crate::clock::now_100ns();
                    crate::trace::ev("sound_start", now, cue as i64, now.saturating_sub(requested) as i64, 0);
                }
                let _ = WaitForSingleObject(self.event, 100);
            }
            Ok(())
        })();
        let _ = self.client.Stop();
        let _ = self.client.Reset();
        crate::trace::ev("sound_end", crate::clock::now_100ns(), cue as i64, 0, 0);
        result
    }
}

impl Drop for Sink {
    fn drop(&mut self) { unsafe {
        let _ = self.client.Stop();
        let _ = windows::Win32::Foundation::CloseHandle(self.event);
    } }
}

/// The marker voice, retained for the sound preview command.
pub fn chime_samples(volume: f32) -> Vec<f32> {
    cue_samples(Cue::Marker, volume)
}

/// A soft falling interval: "not now" (the overlay cannot open over this game right now). The
/// one answer that reaches a player in exclusive fullscreen before CAPTURE can draw there.
pub fn play_refusal(volume: f32, device: Option<String>) {
    play(Cue::Refusal, volume, device);
}

/// Fire-and-forget: plays on the default output device on its own thread.
pub fn play_chime(volume: f32, device: Option<String>) {
    play(Cue::Marker, volume, device);
}

/// Play interleaved 48 kHz stereo float samples on the default output, blocking until done.
/// Streams through a 200 ms buffer, so any length works.
pub fn play_samples(samples: &[f32]) -> windows::core::Result<()> {
    play_on(samples, None)
}

/// As `play_samples`, on a chosen output (falls back to the default output if that device is
/// gone: a missing *output* for a notification sound is harmless, unlike a missing microphone).
pub fn play_on(samples: &[f32], device: Option<&str>) -> windows::core::Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let chosen = device.and_then(|id| en.GetDevice(&windows::core::HSTRING::from(id)).ok()).filter(|d| d.GetState().map(|s| s == DEVICE_STATE_ACTIVE).unwrap_or(false));
        let dev = match chosen {
            Some(d) => d,
            None => en.GetDefaultAudioEndpoint(eRender, eConsole)?,
        };
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let fmt = WAVEFORMATEX {
            wFormatTag: 3,
            nChannels: 2,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            2_000_000,
            0,
            &fmt,
            None,
        )?;
        let size = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;
        let total = samples.len() / 2;
        let mut pos = 0usize;
        let mut started = false;
        while pos < total {
            let free = (size - client.GetCurrentPadding()?) as usize;
            let n = free.min(total - pos);
            if n > 0 {
                let buf = render.GetBuffer(n as u32)?;
                std::ptr::copy_nonoverlapping(samples.as_ptr().add(pos * 2), buf as *mut f32, n * 2);
                render.ReleaseBuffer(n as u32, 0)?;
                pos += n;
            }
            if !started {
                client.Start()?;
                started = true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        while client.GetCurrentPadding()? > 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = client.Stop();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn chime_is_short_bounded_and_click_free() {
        let s = super::chime_samples(1.0);
        let secs = s.len() as f32 / 2.0 / 48_000.0;
        assert!(secs > 0.1 && secs < 0.3, "{secs}");
        assert!(s.iter().all(|x| x.abs() <= 0.5));
        assert!(s[0].abs() < 1e-3 && s[s.len() - 1].abs() < 1e-3);
        assert!(super::chime_samples(0.0).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn cue_family_is_quiet_matched_and_distinct() {
        use super::*;
        let cues = [Cue::Start, Cue::Stop, Cue::Screenshot, Cue::Replay, Cue::Marker, Cue::Refusal];
        let mut waves = Vec::new();
        for cue in cues {
            let s = cue_samples(cue, 1.0);
            let rms = (s.iter().map(|x| x*x).sum::<f32>() / s.len() as f32).sqrt();
            assert!((rms - 0.032).abs() < 0.002, "{cue:?}: rms={rms}");
            assert!(s.iter().all(|x| x.is_finite() && x.abs() <= 0.141));
            assert!(s[0].abs() < 0.0001 && s[s.len()-1].abs() < 0.0001);
            assert!(s.windows(3).all(|x| (x[2]-x[0]).abs() < 0.02), "smooth waveform");
            assert!(cue_samples(cue, 0.0).iter().all(|x| *x == 0.0));
            assert!(s.iter().zip(cue_samples(cue, 0.5)).all(|(a,b)| (a*0.5-b).abs() < 1e-6));
            if let Ok(dir) = std::env::var("CAPTURE_CUE_OUTPUT") {
                let mut data = Vec::new();
                for &value in &s { data.extend_from_slice(&((value * 32767.0) as i16).to_le_bytes()); }
                let mut wav = Vec::new();
                wav.extend_from_slice(b"RIFF"); wav.extend_from_slice(&(36+data.len() as u32).to_le_bytes()); wav.extend_from_slice(b"WAVEfmt ");
                wav.extend_from_slice(&16u32.to_le_bytes()); wav.extend_from_slice(&1u16.to_le_bytes()); wav.extend_from_slice(&2u16.to_le_bytes());
                wav.extend_from_slice(&RATE.to_le_bytes()); wav.extend_from_slice(&(RATE*4).to_le_bytes()); wav.extend_from_slice(&4u16.to_le_bytes()); wav.extend_from_slice(&16u16.to_le_bytes());
                wav.extend_from_slice(b"data"); wav.extend_from_slice(&(data.len() as u32).to_le_bytes()); wav.extend_from_slice(&data);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(std::path::Path::new(&dir).join(format!("{cue:?}.wav")), wav).unwrap();
            }
            assert!(!waves.contains(&s), "distinct action cue"); waves.push(s);
        }
    }
}
