//! Short confirmation chime (marker added, etc.), played by the worker process.
//!
//! Isolation from recordings: game audio is captured with process loopback of the *game*
//! process, and whole-display "System Audio" is captured with process loopback that *excludes*
//! the worker's own process tree (see `audio::SourceSpec::AllExceptCapture`). Neither can
//! contain this sound. The only remaining path is acoustic: a microphone near speakers can hear
//! it, which no software can prevent (documented for users).

use std::time::Duration;

use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

const RATE: u32 = 48_000;

/// Two soft rising blips, ~170 ms total, with smooth envelopes (no clicks).
pub fn chime_samples(volume: f32) -> Vec<f32> {
    let v = volume.clamp(0.0, 1.0) * 0.5;
    let mut out = Vec::new();
    for (freq, ms) in [(880.0f32, 70u32), (1318.5f32, 90u32)] {
        let n = RATE * ms / 1000;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let x = i as f32 / n as f32;
            let env = (std::f32::consts::PI * x).sin().powi(2);
            let s = (2.0 * std::f32::consts::PI * freq * t).sin() * env * v;
            out.push(s);
            out.push(s);
        }
        out.extend(std::iter::repeat_n(0.0, (RATE / 100) as usize * 2)); // 10 ms gap
    }
    out
}

/// Fire-and-forget: plays on the default output device on its own thread.
pub fn play_chime(volume: f32) {
    if volume <= 0.0 {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = play(&chime_samples(volume)) {
            eprintln!("chime: {e}");
        }
    });
}

fn play(samples: &[f32]) -> windows::core::Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
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
        let frames = (samples.len() / 2) as u32;
        let dur_100ns = (frames as i64 * 10_000_000 / RATE as i64) + 1_000_000;
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            dur_100ns,
            0,
            &fmt,
            None,
        )?;
        let cap = client.GetBufferSize()?.min(frames);
        let render: IAudioRenderClient = client.GetService()?;
        let buf = render.GetBuffer(cap)?;
        std::ptr::copy_nonoverlapping(samples.as_ptr(), buf as *mut f32, cap as usize * 2);
        render.ReleaseBuffer(cap, 0)?;
        client.Start()?;
        std::thread::sleep(Duration::from_millis((cap as u64 * 1000 / RATE as u64) + 60));
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
}
