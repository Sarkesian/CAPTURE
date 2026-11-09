//! One running replay session: capture → CFR encode → segment ring, plus replay saves.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::{EncoderInfo, Marker, MarkerConfig, MarkerFootage, Quality, Review, SavedClip};

use crate::audio::{AudioPipeline, MixParams, Source};
use crate::av::Packet;
use crate::gpu::Gpu;
use crate::mux::{SegmentWriter, StreamDesc};
use crate::ring::{Ring, Segment};
use crate::video::VideoEncoder;
use crate::{clock, save, storage, wgc};

pub const SEGMENT_SECS: f64 = 2.0;
/// Extra seconds of footage kept beyond the replay length (keyframe slack + save latency).
const KEEP_SLACK_SECS: f64 = 6.0;

pub struct Settings {
    pub root: PathBuf,
    /// Where `Buffer\` lives.
    pub buffer: PathBuf,
    pub game_name: String,
    pub exe: String,
    pub pid: u32,
    pub detected_by: Option<String>,
    pub source: SourceKind,
    pub fps: u32,
    pub quality: Quality,
    pub software_encoder: bool,
    pub cursor: bool,
    pub audio: Vec<Source>,
    pub mix: MixParams,
}

pub enum SourceKind {
    Window(isize),
    Monitor(isize),
}

/// Values that can change while the session runs.
#[derive(Clone, Copy)]
pub struct Live {
    pub replay_secs: u32,
    pub cap_bytes: u64,
    pub min_free_bytes: u64,
}

pub struct Shared {
    pub ring: Mutex<Ring>,
    pub session_dir: PathBuf,
    pub root: PathBuf,
    pub game_name: String,
    pub target_exe: String,
    pub target_pid: u32,
    /// Window being captured (0 for a display).
    pub target_hwnd: isize,
    pub detected_by: Option<String>,
    pub t0: u64,
    pub live: Mutex<Live>,
    pub attention: Mutex<Option<String>>,
    pub frames_encoded: AtomicU64,
    pub frames_dropped: AtomicU64,
    pub mux_done: AtomicBool,
    pub saves_in_progress: AtomicU32,
    pub track_names: Vec<String>,
    /// Markers of this session. Lock order: `ring` before `markers`.
    pub markers: Mutex<Vec<Marker>>,
    pub next_marker: AtomicU64,
    /// Segments + file held by the current review snapshot.
    review: Mutex<Option<(Vec<u64>, PathBuf)>>,
    review_seq: AtomicU64,
    /// Start of the full-session recording in progress (timeline secs).
    pub session_start: Mutex<Option<f64>>,
    /// Timeline time video stopped, if it has (see `save_end`).
    pub ended_at: Mutex<Option<f64>>,
    /// Set when the `Pipeline` is dropped; buffer files are deleted only after that.
    pub dropped: AtomicBool,
    /// Pin id of the running session (valid while `session_start` is set).
    pub session_pin: AtomicU64,
    /// Set by the mux thread when low disk space forces the session to end.
    pub session_must_stop: AtomicBool,
}

/// Pin id reserved for full-session recording (never released by the marker budget).
/// Pin id of the next full-session recording (each session its own, so finishing one session's
/// save can never release a newer session's footage).
static NEXT_SESSION_PIN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(crate::ring::SESSION_PIN_BASE);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SaveKind {
    Clip,
    Session,
}

/// Pins may hold at most this share of the buffer cap; older marker protection is released
/// first (and the marker says so) rather than letting markers grow the buffer without bound.
const PIN_BUDGET_SHARE: f64 = 0.5;

impl Shared {
    pub fn now(&self) -> f64 {
        clock::secs_since(self.t0)
    }

    /// The latest time a save may end at: now, or when video stopped (window closed, encoder
    /// failed) — after that there is no picture to save.
    pub fn save_end(&self) -> f64 {
        let now = self.now();
        self.ended_at.lock().unwrap().map_or(now, |e| e.min(now))
    }

    pub fn add_marker(&self, label: Option<String>, cfg: &MarkerConfig) -> Marker {
        let t = self.now();
        let id = self.next_marker.fetch_add(1, Ordering::SeqCst);
        let mut note = None;
        if cfg.protect {
            let cap = self.live.lock().unwrap().cap_bytes;
            if cap == 0 {
                note = Some("Not protected: the temporary buffer limit is 0.".into());
            } else {
                self.ring.lock().unwrap().pin(id, t - cfg.protect_before_secs as f64, t + cfg.protect_after_secs as f64);
            }
        }
        let m = Marker {
            id,
            t: (t * 100.0).round() / 100.0,
            at: storage::local_stamp()[11..].replace('-', ":"),
            label: label.unwrap_or_default(),
            footage: if cfg.protect && note.is_none() { MarkerFootage::Protected } else { MarkerFootage::Buffered },
            note,
            saved_in: vec![],
        };
        self.markers.lock().unwrap().push(m.clone());
        self.persist_markers();
        m
    }

    pub fn rename_marker(&self, id: u64, label: String) -> bool {
        let found = self.markers.lock().unwrap().iter_mut().find(|m| m.id == id).map(|m| m.label = label).is_some();
        self.persist_markers();
        found
    }

    pub fn remove_marker(&self, id: u64) -> bool {
        self.ring.lock().unwrap().unpin(id);
        let mut ms = self.markers.lock().unwrap();
        let n = ms.len();
        ms.retain(|m| m.id != id);
        let removed = ms.len() != n;
        drop(ms);
        self.persist_markers();
        removed
    }

    /// Markers with their *current* footage status computed from the ring.
    pub fn markers_now(&self) -> Vec<Marker> {
        let ring = self.ring.lock().unwrap();
        let pinned = ring.pinned_ids();
        let mut ms = self.markers.lock().unwrap().clone();
        for m in &mut ms {
            m.footage = if !ring.covers(m.t) && m.t < self.now() - 1.0 {
                MarkerFootage::Expired
            } else if pinned.contains(&m.id) {
                MarkerFootage::Protected
            } else {
                MarkerFootage::Buffered
            };
        }
        ms
    }

    /// Release the oldest pins while pinned footage exceeds its share of the buffer cap.
    fn enforce_pin_budget(&self, cap: u64) {
        let budget = (cap as f64 * PIN_BUDGET_SHARE) as u64;
        let mut ring = self.ring.lock().unwrap();
        let mut released = Vec::new();
        while ring.marker_pinned_bytes() > budget {
            let Some(id) = ring.oldest_marker_pin() else { break };
            ring.unpin(id);
            released.push(id);
        }
        drop(ring);
        if !released.is_empty() {
            for m in self.markers.lock().unwrap().iter_mut().filter(|m| released.contains(&m.id)) {
                m.note = Some("Protection released: markers reached their share of the temporary buffer limit. Save it as a clip to keep it.".into());
            }
            self.persist_markers();
        }
    }

    /// Drop the current review snapshot: unprotect its segments and delete its file (if a
    /// player still has it open, it goes with the session folder later).
    pub fn release_review(&self) {
        if let Some((seqs, path)) = self.review.lock().unwrap().take() {
            self.ring.lock().unwrap().unprotect(&seqs);
            let _ = std::fs::remove_file(path);
        }
    }

    /// Crash-safe copy of this session's markers next to its buffer segments.
    fn persist_markers(&self) {
        let ms = self.markers.lock().unwrap().clone();
        let tmp = self.session_dir.join("markers.json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&ms).unwrap()).is_ok() {
            let _ = std::fs::rename(tmp, self.session_dir.join("markers.json"));
        }
    }

    fn set_attention(&self, msg: Option<String>) {
        *self.attention.lock().unwrap() = msg;
    }

    /// Delete this session's buffer files once the session has ended and no save needs them.
    pub fn cleanup_if_idle(&self) {
        // Only once the pipeline itself is gone: a session whose window closed (mux finished)
        // keeps its footage saveable until a new one replaces it.
        if !self.mux_done.load(Ordering::SeqCst) || !self.dropped.load(Ordering::SeqCst) {
            return;
        }
        let mut ring = self.ring.lock().unwrap();
        let removed = ring.retain(f64::INFINITY, f64::NEG_INFINITY, 0, u64::MAX);
        for s in removed {
            let _ = std::fs::remove_file(&s.path);
        }
        if ring.segments().iter().all(|s| s.end.is_some()) && ring.segments().iter().all(|s| s.protect == 0) {
            for s in ring.segments() {
                let _ = std::fs::remove_file(&s.path);
            }
            let _ = std::fs::remove_file(self.session_dir.join("session.json"));
            let _ = std::fs::remove_file(self.session_dir.join("markers.json"));
            if let Ok(rd) = std::fs::read_dir(&self.session_dir) {
                for e in rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("review_")) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
            let _ = std::fs::remove_dir(&self.session_dir);
        }
    }
}

pub struct Pipeline {
    pub shared: Arc<Shared>,
    pub capture: Arc<wgc::Capture>,
    pub encoder: EncoderInfo,
    pub audio: Option<AudioPipeline>,
    stop: Arc<AtomicBool>,
    video_thread: Option<std::thread::JoinHandle<()>>,
    mux_thread: Option<std::thread::JoinHandle<()>>,
    pub(crate) gpu: Arc<Gpu>,
}

impl Pipeline {
    pub fn start(s: Settings, live: Live) -> Result<Self, String> {
        let gpu = Arc::new(Gpu::create()?);
        let source = match s.source {
            SourceKind::Window(h) => wgc::Source::Window(h),
            SourceKind::Monitor(h) => wgc::Source::Monitor(h),
        };
        let capture = Arc::new(wgc::Capture::start(&gpu, source, s.cursor)?);
        let mut encoder = VideoEncoder::open(gpu.clone(), capture.width, capture.height, s.fps, s.quality, s.software_encoder)?;
        // A CPU encoder cannot sustain more than ~1080p60 (measured: 4K60 software encode reached
        // ~48 fps and dropped frames). Cap it at 30 fps above that, and say so, rather than
        // producing stuttering output.
        let load = capture.width as u64 * capture.height as u64 * s.fps as u64;
        if !encoder.info.hardware && s.fps > 30 && load > 1920 * 1080 * 60 {
            encoder = VideoEncoder::open(gpu.clone(), capture.width, capture.height, 30, s.quality, s.software_encoder)?;
            encoder.info.rejected.push(format!(
                "{} fps: too much for a CPU encoder at {}x{}; recording at 30 fps",
                s.fps, capture.width, capture.height
            ));
        }
        let info = encoder.info.clone();

        let buffer = s.buffer.clone();
        // A folder of its own even when two sessions start within the same second (a quick
        // settings change): the old session's cleanup must never touch the new one's files.
        std::fs::create_dir_all(&buffer).map_err(|e| format!("Cannot write to the capture folder: {e}"))?;
        let base = format!("{}_{}", storage::local_stamp(), std::process::id());
        let mut n = 0u32;
        let session_dir = loop {
            let d = buffer.join(if n == 0 { base.clone() } else { format!("{base}{n:02}") });
            match std::fs::create_dir(&d) {
                Ok(()) => break d,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 99 => n += 1,
                Err(e) => return Err(format!("Cannot write to the capture folder: {e}")),
            }
        };

        let stop = Arc::new(AtomicBool::new(false));
        let t0 = clock::now_100ns();
        let (tx, rx) = sync_channel::<Packet>(8192);

        let mut streams = vec![unsafe { StreamDesc::from_ctx(encoder.codec_ctx()) }];
        let audio = if s.audio.is_empty() {
            None
        } else {
            let tx = tx.clone();
            let a = AudioPipeline::start(s.audio.clone(), s.mix, t0, stop.clone(), 1, move |p| {
                let _ = tx.send(p);
            })?;
            for (p, tb) in a.params() {
                streams.push(unsafe { StreamDesc::copy_of(*p, *tb) });
            }
            Some(a)
        };
        let track_names = audio.as_ref().map(|a| a.track_names().to_vec()).unwrap_or_default();
        let manifest = serde_json::json!({
            "capture_session": 1,
            "game": s.game_name,
            "video": { "encoder": info.name, "width": info.width, "height": info.height, "fps": info.fps },
            "audio_tracks": track_names,
            "segment_container": "mpegts",
        });
        let _ = std::fs::write(session_dir.join("session.json"), serde_json::to_vec_pretty(&manifest).unwrap());

        let shared = Arc::new(Shared {
            ring: Mutex::new(Ring::default()),
            session_dir,
            root: s.root.clone(),
            game_name: s.game_name.clone(),
            target_exe: s.exe.clone(),
            target_pid: s.pid,
            target_hwnd: match s.source {
                SourceKind::Window(h) => h,
                SourceKind::Monitor(_) => 0,
            },
            detected_by: s.detected_by.clone(),
            t0,
            live: Mutex::new(live),
            attention: Mutex::new(None),
            frames_encoded: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            mux_done: AtomicBool::new(false),
            saves_in_progress: AtomicU32::new(0),
            track_names,
            markers: Mutex::new(Vec::new()),
            next_marker: AtomicU64::new(1),
            review: Mutex::new(None),
            review_seq: AtomicU64::new(1),
            session_start: Mutex::new(None),
            ended_at: Mutex::new(None),
            dropped: AtomicBool::new(false),
            session_pin: AtomicU64::new(0),
            session_must_stop: AtomicBool::new(false),
        });

        let mux_thread = {
            let shared = shared.clone();
            std::thread::Builder::new().name("mux".into()).spawn(move || run_mux(shared, rx, streams)).unwrap()
        };
        let video_thread = {
            let (shared, capture, stop) = (shared.clone(), capture.clone(), stop.clone());
            let fps = info.fps;
            std::thread::Builder::new()
                .name("video".into())
                .spawn(move || run_video(shared, capture, encoder, fps, stop, tx))
                .unwrap()
        };
        Ok(Self {
            shared,
            capture,
            encoder: info,
            audio,
            stop,
            video_thread: Some(video_thread),
            mux_thread: Some(mux_thread),
            gpu,
        })
    }

    /// True once the captured window is gone. WGC's `Closed` event is not always raised (for
    /// example when the game process is killed), so the window handle is checked directly too.
    pub fn target_gone(&self) -> bool {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::IsWindow;
        let h = self.shared.target_hwnd;
        if h != 0 && !unsafe { IsWindow(Some(HWND(h as *mut _))) }.as_bool() {
            self.capture.closed.store(true, Ordering::Relaxed);
        }
        self.capture.closed.load(Ordering::Relaxed)
    }

    pub fn set_live(&self, live: Live) {
        *self.shared.live.lock().unwrap() = live;
    }

    /// Start an asynchronous replay save ending now. `done` receives the verified clip or a
    /// plain-language failure.
    /// Start an asynchronous save. `range = None`: replay ending now with the configured
    /// length. `Some((start, end))`: exactly that timeline range (from the review timeline).
    /// `done` receives the verified clip and the requested length, or a plain-language failure.
    pub fn request_save(
        &self,
        range: Option<(f64, f64)>,
        kind: SaveKind,
        done: impl FnOnce(Result<(SavedClip, f64), String>) + Send + 'static,
        save_id: u64,
    ) -> f64 {
        let shared = self.shared.clone();
        let (t_end, secs) = match range {
            Some((a, b)) => (b.min(shared.save_end()), (b - a).max(1.0)),
            None => (shared.save_end(), shared.live.lock().unwrap().replay_secs as f64),
        };
        shared.saves_in_progress.fetch_add(1, Ordering::SeqCst);
        std::thread::Builder::new()
            .name(format!("save-{save_id}"))
            .spawn(move || {
                let r = do_save(&shared, t_end, secs, save_id, kind);
                shared.saves_in_progress.fetch_sub(1, Ordering::SeqCst);
                shared.cleanup_if_idle();
                done(r.map(|c| (c, secs)));
            })
            .unwrap();
        secs
    }

    /// Begin a full-session recording from now. Returns false if one is already running.
    pub fn start_session(&self) -> bool {
        let mut s = self.shared.session_start.lock().unwrap();
        if s.is_some() {
            return false;
        }
        let t = self.shared.now();
        *s = Some(t);
        self.shared.session_must_stop.store(false, Ordering::SeqCst);
        let pin = NEXT_SESSION_PIN.fetch_add(1, Ordering::SeqCst);
        self.shared.session_pin.store(pin, Ordering::SeqCst);
        self.shared.ring.lock().unwrap().pin(pin, t, f64::INFINITY);
        true
    }

    /// Finish the session: saves [start, now] to `<Game>\Sessions\`. The pin is released once
    /// the save has protected its own segments.
    pub fn stop_session(&self, done: impl FnOnce(Result<(SavedClip, f64), String>) + Send + 'static, save_id: u64) -> Option<f64> {
        let start = self.shared.session_start.lock().unwrap().take()?;
        let pin = self.shared.session_pin.load(Ordering::SeqCst);
        let end = self.shared.save_end();
        let shared = self.shared.clone();
        let secs = self.request_save(
            Some((start, end)),
            SaveKind::Session,
            move |r| {
                shared.ring.lock().unwrap().unpin(pin);
                done(r)
            },
            save_id,
        );
        Some(secs)
    }

    /// Build a playable snapshot of all closed buffer segments (stream copy, no re-encode) on a
    /// background thread. Its segments stay protected until `release_review` or the next review.
    pub fn prepare_review(&self, done: impl FnOnce(Result<Review, String>) + Send + 'static) {
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name("review".into())
            .spawn(move || done(build_review(&shared)))
            .unwrap();
    }
}

fn build_review(shared: &Shared) -> Result<Review, String> {
    let (seqs, segs, start, end) = {
        let mut ring = shared.ring.lock().unwrap();
        let closed: Vec<_> = ring.segments().iter().filter(|s| s.end.is_some()).cloned().collect();
        let (first, last) = match (closed.first(), closed.last()) {
            (Some(f), Some(l)) => (f.start, l.end.unwrap()),
            _ => return Err("Nothing has been recorded yet. Give it a few seconds.".into()),
        };
        let seqs: Vec<u64> = closed.iter().map(|s| s.seq).collect();
        ring.protect(&seqs);
        (seqs, closed.iter().map(|s| (s.path.clone(), s.start)).collect::<Vec<_>>(), first, last)
    };
    let n = shared.review_seq.fetch_add(1, Ordering::SeqCst);
    let path = shared.session_dir.join(format!("review_{n:03}.mp4"));
    match save::remux(&segs, start, start, end, &shared.track_names, &path) {
        Ok((clip, actual_start)) => {
            shared.release_review(); // the previous snapshot
            *shared.review.lock().unwrap() = Some((seqs, path.clone()));
            Ok(Review {
                path: path.to_string_lossy().into_owned(),
                start: actual_start,
                end: actual_start + clip.duration_secs,
                game: shared.game_name.clone(),
            })
        }
        Err(e) => {
            shared.ring.lock().unwrap().unprotect(&seqs);
            Err(format!("Could not open the recording for review: {e}"))
        }
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.video_thread.take() {
            let _ = t.join();
        }
        if let Some(a) = self.audio.as_mut() {
            a.join();
        }
        if let Some(t) = self.mux_thread.take() {
            let _ = t.join();
        }
        self.shared.dropped.store(true, Ordering::SeqCst);
        // A review snapshot nobody released (window closed meanwhile) would keep its segments
        // protected forever and the session folder would linger as "leftover" footage.
        self.shared.release_review();
        self.shared.cleanup_if_idle();
    }
}

/// When frame `n` is due, in 100 ns units: `t0 + n / fps` computed exactly. (A truncated
/// per-frame period, e.g. 166 666 for 60 fps, drifts against the audio clock by ~14 ms an hour.)
fn frame_due(t0: u64, n: i64, fps: u32) -> u64 {
    // Rounded up, so that at `frame_due(n)` frame `n` (not `n - 1`) is the newest one due.
    t0 + (n as u128 * 10_000_000).div_ceil(fps as u128) as u64
}

/// Index of the newest frame due at `now`.
fn frame_at(t0: u64, now: u64, fps: u32) -> i64 {
    (now.saturating_sub(t0) as u128 * fps as u128 / 10_000_000) as i64
}

fn run_video(shared: Arc<Shared>, capture: Arc<wgc::Capture>, mut enc: VideoEncoder, fps: u32, stop: Arc<AtomicBool>, tx: std::sync::mpsc::SyncSender<Packet>) {
    let mut n: i64 = 0;
    let mut send = |p: Packet| {
        let _ = tx.send(p);
    };
    let mut ended = false;
    while !stop.load(Ordering::Relaxed) {
        let due = frame_due(shared.t0, n, fps);
        let now = clock::now_100ns();
        if now < due {
            std::thread::sleep(Duration::from_micros((due - now) / 10));
            continue;
        }
        let behind = frame_at(shared.t0, now, fps) - n;
        if behind > 0 {
            shared.frames_dropped.fetch_add(behind as u64, Ordering::Relaxed);
            n += behind;
        }
        if capture.closed.load(Ordering::Relaxed) {
            ended = true; // reported as "Waiting" by the engine; footage stays saveable
            break;
        }
        // Encode under the frame lock, but send after releasing it: a slow disk that fills the
        // channel must not also block the capture callback and screenshots.
        let mut out = Vec::new();
        let latest = capture.latest.lock().unwrap();
        if latest.has_frame {
            let r = enc.encode(&latest, n, &mut |p| out.push(p));
            drop(latest);
            match r {
                Ok(()) => {
                    shared.frames_encoded.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    out.into_iter().for_each(&mut send);
                    shared.set_attention(Some(format!("Video encoding failed: {e}")));
                    ended = true;
                    break;
                }
            }
        } else {
            drop(latest);
        }
        out.into_iter().for_each(&mut send);
        n += 1;
    }
    enc.flush(&mut send);
    if ended {
        // No more picture: stop the whole session (audio too) so the mux closes the last
        // segment on disk and saves end where the video ended, instead of an open segment
        // growing with audio only.
        *shared.ended_at.lock().unwrap() = Some(shared.now());
        stop.store(true, Ordering::SeqCst);
    }
}

fn run_mux(shared: Arc<Shared>, rx: Receiver<Packet>, streams: Vec<StreamDesc>) {
    let mut writer: Option<SegmentWriter> = None;
    let mut seg_start = 0.0;
    let mut last_video = 0.0;
    let mut seq: u64 = 0;
    let mut disk_paused = false;
    // The buffer folder could not be written (drive unplugged, network share gone...). Cleared,
    // with its message, as soon as a new segment can be created again.
    let mut write_failed = false;
    let cant_write = |e: &dyn std::fmt::Display| {
        format!(
            "Can't write temporary footage to {} ({e}). Is the drive disconnected? Recording resumes by itself when it is back; the gap is not recorded.",
            shared.session_dir.parent().unwrap_or(&shared.session_dir).display()
        )
    };
    let mut last_free_check = Instant::now() - Duration::from_secs(10);
    loop {
        let p = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(p) => p,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let is_video = p.track == 0;
        if is_video {
            last_video = p.secs();
        }
        let rotate = is_video && p.is_key() && (writer.is_none() || p.secs() >= seg_start + SEGMENT_SECS);
        if rotate && (writer.is_some() || last_free_check.elapsed() >= Duration::from_secs(1)) {
            last_free_check = Instant::now();
            if let Some(w) = writer.take() {
                let bytes = w.finish();
                shared.ring.lock().unwrap().close_last(p.secs(), bytes);
            }
            let live = *shared.live.lock().unwrap();
            let keep = live.replay_secs as f64 + KEEP_SLACK_SECS;
            let free = storage::free_bytes(&shared.session_dir).unwrap_or(u64::MAX);
            let need = live.min_free_bytes.saturating_sub(free);
            shared.enforce_pin_budget(live.cap_bytes);
            let removed = shared.ring.lock().unwrap().retain(p.secs(), keep, live.cap_bytes, need);
            for s in &removed {
                let _ = std::fs::remove_file(&s.path);
            }
            let free = storage::free_bytes(&shared.session_dir).unwrap_or(u64::MAX);
            if free < live.min_free_bytes {
                if shared.session_start.lock().unwrap().is_some() {
                    shared.session_must_stop.store(true, Ordering::SeqCst);
                }
                if !disk_paused {
                    disk_paused = true;
                    shared.set_attention(Some(format!(
                        "Low disk space ({} GB free). Replay paused to protect saved clips. Free up space or change the capture folder.",
                        free / 1_000_000_000
                    )));
                }
                continue;
            }
            if disk_paused {
                disk_paused = false;
                shared.set_attention(None);
            }
            seq += 1;
            let path = shared.session_dir.join(format!("seg_{seq:06}.ts"));
            // A drive that came back may have lost the session folder (e.g. remounted).
            let _ = std::fs::create_dir_all(&shared.session_dir);
            match SegmentWriter::create(&path, &streams) {
                Ok(w) => {
                    writer = Some(w);
                    seg_start = p.secs();
                    shared.ring.lock().unwrap().push(Segment { seq, path, start: seg_start, end: None, bytes: 0, protect: 0 });
                    if write_failed {
                        write_failed = false;
                        shared.set_attention(None);
                    }
                }
                Err(e) => {
                    write_failed = true;
                    shared.set_attention(Some(cant_write(&e)));
                    continue;
                }
            }
        }
        if let Some(w) = writer.as_mut() {
            if let Err(e) = w.write(p) {
                // Stop writing into a dead file: close it and retry a new segment at the next
                // keyframe (about once a second) instead of failing for a whole segment.
                write_failed = true;
                shared.set_attention(Some(cant_write(&e)));
                if let Some(w) = writer.take() {
                    let bytes = w.finish();
                    shared.ring.lock().unwrap().close_last(last_video, bytes);
                }
            }
        }
    }
    if let Some(w) = writer.take() {
        let bytes = w.finish();
        shared.ring.lock().unwrap().close_last(last_video + 1.0 / 60.0, bytes);
    }
    shared.mux_done.store(true, Ordering::SeqCst);
}

/// Note which markers a saved clip contains, and write `<clip>.markers.json` next to it (only
/// when it contains markers) with times relative to the clip start, for editors and FrameCut.
fn record_markers_in_clip(shared: &Shared, clip: &std::path::Path, start: f64, end: f64) {
    let mut inside = Vec::new();
    for m in shared.markers.lock().unwrap().iter_mut().filter(|m| m.t >= start && m.t <= end) {
        m.saved_in.push(clip.to_string_lossy().into_owned());
        inside.push(serde_json::json!({ "t": ((m.t - start) * 100.0).round() / 100.0, "label": m.label, "at": m.at }));
    }
    if inside.is_empty() {
        return;
    }
    shared.persist_markers();
    let doc = serde_json::json!({ "capture_markers": 1, "clip": clip.file_name().map(|n| n.to_string_lossy().into_owned()), "markers": inside });
    let mut side = clip.as_os_str().to_owned();
    side.push(".markers.json");
    let _ = std::fs::write(std::path::PathBuf::from(side), serde_json::to_vec_pretty(&doc).unwrap());
}

fn do_save(shared: &Shared, t_end: f64, secs: f64, save_id: u64, kind: SaveKind) -> Result<SavedClip, String> {
    let sel = {
        let mut ring = shared.ring.lock().unwrap();
        let sel = ring.select(t_end, secs).ok_or("Nothing has been buffered yet. Wait a moment and try again.")?;
        ring.protect(&sel.seqs);
        sel
    };
    // Wait until the segment holding `t_end` (and late audio after it) is closed on disk.
    let deadline = Instant::now() + Duration::from_secs_f64(SEGMENT_SECS * 3.0 + 2.0);
    while Instant::now() < deadline && !shared.mux_done.load(Ordering::SeqCst) && !shared.ring.lock().unwrap().is_complete_through(t_end) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let (all, segs) = {
        let mut ring = shared.ring.lock().unwrap();
        // Only closed tail segments: an open one is still being written and would end in a
        // partial packet.
        let closed: Vec<u64> = ring.segments().iter().filter(|s| s.end.is_some()).map(|s| s.seq).collect();
        let tail: Vec<u64> = ring
            .tail_after(t_end, SEGMENT_SECS + 1.0)
            .into_iter()
            .filter(|s| !sel.seqs.contains(s) && closed.contains(s))
            .collect();
        ring.protect(&tail);
        let all: Vec<u64> = sel.seqs.iter().chain(tail.iter()).copied().collect();
        let mut segs = ring.paths_for(&all);
        segs.sort_by_key(|s| s.0);
        (all, segs.into_iter().map(|(_, p, st)| (p, st)).collect::<Vec<_>>())
    };
    let result = (|| {
        let (dir, label) = match kind {
            SaveKind::Clip => (storage::clips_dir(&shared.root, &shared.game_name), "Clip"),
            SaveKind::Session => (storage::sessions_dir(&shared.root, &shared.game_name), "Session"),
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create the folder {}: {e}", dir.display()))?;
        let name = storage::clip_file_name(&storage::local_stamp(), &shared.game_name, label);
        let path = storage::reserve_unique_path(&dir, &name).map_err(|e| format!("Cannot create the clip file: {e}"))?;
        let (clip, actual_start) = save::remux(&segs, sel.start, sel.end - secs, sel.end, &shared.track_names, &path)?;
        record_markers_in_clip(shared, &clip.path, actual_start, sel.end);
        Ok(SavedClip {
            save_id,
            path: clip.path.to_string_lossy().into_owned(),
            requested_secs: secs,
            duration_secs: clip.duration_secs,
            bytes: clip.bytes,
            video_streams: clip.video_streams,
            audio_tracks: clip.audio_tracks,
        })
    })();
    shared.ring.lock().unwrap().unprotect(&all);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_times_do_not_drift() {
        for fps in [30u32, 60, 144, 240] {
            let hour = fps as i64 * 3600;
            assert_eq!(frame_due(0, hour, fps), 36_000_000_000, "{fps} fps: exactly one hour");
            assert_eq!(frame_at(0, 36_000_000_000, fps), hour);
            assert_eq!(frame_at(0, frame_due(0, 12_345, fps), fps), 12_345);
        }
    }
}
