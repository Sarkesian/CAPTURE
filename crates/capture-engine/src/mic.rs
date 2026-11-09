//! Microphone level monitor (settings open, nothing stored) and the 3-second mic test.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::MicLevel;

use crate::audio::{source_thread, SourceSpec, TrackBuf};
use crate::{clock, sound};

pub struct Monitor {
    stop: Arc<AtomicBool>,
    buf: Arc<Mutex<TrackBuf>>,
    device: String,
    pub started: Instant,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Monitor {
    pub fn start(device: Option<String>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let buf = Arc::new(Mutex::new(TrackBuf::monitor()));
        let (s, b, d) = (stop.clone(), buf.clone(), device.clone());
        let join = std::thread::Builder::new()
            .name("mic-monitor".into())
            .spawn(move || source_thread(&SourceSpec::Mic { device_id: d }, &b, clock::now_100ns(), &s))
            .unwrap();
        Self { stop, buf, device: device.unwrap_or_else(|| "default".into()), started: Instant::now(), join: Some(join) }
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn take_level(&self) -> capture_protocol::Level {
        let (peak, rms) = self.buf.lock().unwrap().take_level();
        capture_protocol::Level { name: "Microphone".into(), peak, rms }
    }

    pub fn level(&self) -> MicLevel {
        let b = self.buf.lock().unwrap();
        MicLevel { device: self.device.clone(), peak: (b.live_peak() * 100.0).round() / 100.0, error: b.problem() }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub const TEST_SECS: f64 = 3.0;

/// Record `TEST_SECS` from the microphone, then play it back on the default output.
/// Playback starts only after recording stops, so there is no feedback loop.
/// `phase(name, peak, message)` reports "recording", "playing", "done" or "failed".
pub fn test(device: Option<String>, phase: impl Fn(&str, f32, Option<String>)) {
    let stop = Arc::new(AtomicBool::new(false));
    let buf = Arc::new(Mutex::new(TrackBuf::default()));
    let (s, b) = (stop.clone(), buf.clone());
    let t0 = clock::now_100ns();
    let worker = std::thread::spawn(move || source_thread(&SourceSpec::Mic { device_id: device }, &b, t0, &s));
    phase("recording", 0.0, None);
    let until = Instant::now() + Duration::from_secs_f64(TEST_SECS);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
        let p = buf.lock().unwrap().live_peak();
        phase("recording", p, None);
    }
    stop.store(true, Ordering::SeqCst);
    let _ = worker.join();
    let (samples, err, problem) = {
        let mut b = buf.lock().unwrap();
        (b.take_samples(), b.error(), b.problem())
    };
    if samples.is_empty() {
        phase("failed", 0.0, Some(err.unwrap_or_else(|| "No sound arrived from the microphone.".into())));
        return;
    }
    let peak = samples.iter().fold(0f32, |m, x| m.max(x.abs()));
    if peak < 0.002 {
        phase(
            "failed",
            peak,
            // A Windows mute / 0% level is named exactly; otherwise the generic checklist.
            Some(problem.unwrap_or_else(|| {
                "The microphone sent only silence. Check that it isn't muted (hardware switch or Windows settings) and that this is the device you speak into.".into()
            })),
        );
        return;
    }
    phase("playing", peak, None);
    let _ = sound::play_samples(&samples);
    let level = 20.0 * peak.log10();
    let msg = if peak > 0.95 { format!("Peaked at {level:.0} dB, which is close to clipping. Lower the mic level.") } else { format!("Peaked at {level:.0} dB.") };
    phase("done", peak, Some(msg));
}
