//! One running recording: capture → CFR encode → segments on disk, indexed by the history
//! catalog (which also owns retention, saving and playback).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::{EncoderInfo, Marker, MarkerConfig, MarkerFootage, Quality};

use crate::audio::{AudioPipeline, MixParams, Source};
use crate::av::Packet;
use crate::gpu::Gpu;
use crate::mux::{SegmentWriter, StreamDesc};
use crate::video::{self, VideoEncoder};
use crate::{clock, storage, wgc};

pub const SEGMENT_SECS: f64 = 2.0;


pub struct Settings {
    pub root: PathBuf,
    /// Where `Background Recording\` lives.
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
    pub history_secs: u32,
    pub cap_bytes: u64,
    pub min_free_bytes: u64,
}

pub struct Shared {
    pub fps: u32,
    pub frames_repeated: AtomicU64,
    pub frames_without_source: AtomicU64,
    pub source_frames_unselected: AtomicU64,
    pub history: Arc<crate::history::Catalog>,
    pub session_id: String,
    /// The segment being written (sequence number, file, media time of its first frame).
    open_segment: Mutex<Option<(u64, PathBuf, f64)>>,
    /// Media time of this recording's first frame on disk, once there is one.
    pub first_frame: Mutex<Option<f64>>,
    /// Footage written by this recording so far (closed segments): bytes and seconds.
    pub written_bytes: AtomicU64,
    pub written_ms: AtomicU64,
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
    /// Markers of this recording.
    pub markers: Mutex<Vec<Marker>>,
    pub next_marker: AtomicU64,
    /// Clips and screenshots saved from this recording (the latest 200), for the Replay timeline.
    pub saved: Mutex<Vec<capture_protocol::SavedMark>>,
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

/// Pin id of the next full recording (each its own, so finishing one recording's save can
/// never release a newer one's footage).
static NEXT_SESSION_PIN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Shared {
    fn open_segment(&self, seq: u64, path: PathBuf, start: f64) {
        if let Err(e) = self.history.segment(&self.session_id, seq, &path, start, None, 0) {
            self.set_attention(Some(format!("History index: {e}")));
        }
        self.first_frame.lock().unwrap().get_or_insert(start);
        *self.open_segment.lock().unwrap() = Some((seq, path, start));
    }

    fn close_segment(&self, end: f64, bytes: u64) {
        let Some((seq, path, start)) = self.open_segment.lock().unwrap().take() else { return };
        let end = end.max(start);
        if let Err(e) = self.history.segment(&self.session_id, seq, &path, start, Some(end), bytes) {
            self.set_attention(Some(format!("History index: {e}")));
        }
        self.written_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.written_ms.fetch_add(((end - start) * 1000.0) as u64, Ordering::Relaxed);
    }

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
        self.journal_marker(&m);
        if cfg.protect && m.footage == MarkerFootage::Protected {
            let _ = self.history.pin(&self.session_id, &format!("marker:{id}"), t - cfg.protect_before_secs as f64, t + cfg.protect_after_secs as f64);
        }
        self.persist_markers();
        m
    }

    pub fn rename_marker(&self, id: u64, label: String) -> bool {
        let updated = self.markers.lock().unwrap().iter_mut().find(|m| m.id == id).map(|m| {m.label=label;m.clone()});
        if let Some(m)=&updated {self.journal_marker(m);}
        self.persist_markers();
        updated.is_some()
    }

    pub fn remove_marker(&self, id: u64) -> bool {
        self.history.unpin(&self.session_id,&format!("marker:{id}"));
        let _ = self.history.event(&self.session_id,&capture_protocol::TimelineEvent {id:format!("marker:{id}"),kind:"removed".into(),t:0.0,end:None,label:String::new(),path:None,marker:None});
        let mut ms = self.markers.lock().unwrap();
        let n = ms.len();
        ms.retain(|m| m.id != id);
        let removed = ms.len() != n;
        drop(ms);
        self.persist_markers();
        removed
    }

    /// Markers with their *current* footage status, from the catalog: footage that retention
    /// has removed is `Expired`; protection the marker budget released says so.
    pub fn markers_now(&self) -> Vec<Marker> {
        let mut ms = self.markers.lock().unwrap().clone();
        if ms.is_empty() {
            return ms;
        }
        let (oldest, pinned) = self.history.marker_facts(&self.session_id);
        let now = self.now();
        for m in &mut ms {
            let protected = pinned.iter().any(|p| p == &format!("marker:{}", m.id));
            if m.footage == MarkerFootage::Protected && !protected && m.note.is_none() {
                m.note = Some("Protection released: markers reached their share of the temporary buffer limit. Save it as a clip to keep it.".into());
            }
            m.footage = if oldest.is_none_or(|first| m.t < first) && m.t < now - 3.0 {
                MarkerFootage::Expired
            } else if protected {
                MarkerFootage::Protected
            } else {
                MarkerFootage::Buffered
            };
        }
        ms
    }

    /// Remember something saved from this recording (its timeline event is the catalog's job).
    pub fn note_saved(&self, kind: capture_protocol::SavedKind, t: f64, end: Option<f64>, path: &std::path::Path) {
        let mut list = self.saved.lock().unwrap();
        if list.len() >= 200 {
            list.remove(0);
        }
        list.push(capture_protocol::SavedMark { kind, t, end, name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() });
    }

    /// A clip of `[start, end]` was saved from this recording: note it, and which markers it holds.
    pub fn clip_saved(&self, clip: &std::path::Path, start: f64, end: f64) {
        self.note_saved(capture_protocol::SavedKind::Clip, start, Some(end), clip);
        let mut changed = false;
        for m in self.markers.lock().unwrap().iter_mut().filter(|m| m.t >= start && m.t <= end) {
            m.saved_in.push(clip.to_string_lossy().into_owned());
            self.journal_marker(m);
            changed = true;
        }
        if changed {
            self.persist_markers();
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

    fn journal_marker(&self,m:&Marker) {
        if let Err(e)=self.history.event(&self.session_id,&capture_protocol::TimelineEvent {id:format!("marker:{}",m.id),kind:"marker".into(),t:m.t,end:None,label:m.label.clone(),path:None,marker:Some(m.clone())}) {self.set_attention(Some(format!("Could not preserve marker: {e}")));}
    }

    fn set_attention(&self, msg: Option<String>) {
        *self.attention.lock().unwrap() = msg;
    }

    /// Delete this session's buffer files once the session has ended and no save needs them.
    pub fn cleanup_if_idle(&self) {
        if self.mux_done.load(Ordering::SeqCst) && self.dropped.load(Ordering::SeqCst) {
            self.history.end(&self.session_id);
            let _=std::fs::write(self.session_dir.join(crate::storage::ENDED_FILE),b"{}");
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
    /// CAPTURE's in-game hook in the captured game, once loaded (window targets only; never
    /// for games with anti-cheat). Dropped with the session, which unloads it from the game.
    pub hook: Arc<Mutex<Option<crate::inject::GameHook>>>,
}

impl Pipeline {
    pub fn start(s: Settings, live: Live) -> Result<Self, String> {
        let gpu = Arc::new(Gpu::create()?);
        let source = match s.source {
            SourceKind::Window(h) => wgc::Source::Window(h),
            SourceKind::Monitor(h) => wgc::Source::Monitor(h),
        };
        let started_unix_ms = crate::history::millis();
        // Compatibility path: an HDR display never blocks recording. Windows converts the HDR
        // picture to 8-bit SDR for this capture; HDR is not preserved, and the session says so.
        let hdr_display = wgc::display_is_hdr(&gpu, &source);
        eprintln!("capture: display HDR {:?}; recording 8-bit SDR{}", hdr_display, if hdr_display==Some(true) {" (Windows converts the HDR picture to SDR; HDR is not preserved)"} else {""});
        let capture = Arc::new(wgc::Capture::start_native(&gpu, source, s.cursor)?);
        capture.native_resolution.store(true,Ordering::Relaxed);
        let encoder = VideoEncoder::open(gpu.clone(), capture.width, capture.height, s.fps, s.quality, s.software_encoder)?;
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
            "started_unix_ms": started_unix_ms,
            "game": s.game_name,
            "exe": s.exe,
            "video": { "encoder": info.name, "width": info.width, "height": info.height, "fps": info.fps, "quality":s.quality,"capture_format":if hdr_display==Some(true) {"BGRA8 SDR (converted by Windows from an HDR display; HDR not preserved)"} else {"BGRA8 SDR"},"source_display_hdr":hdr_display,"encoded_chroma":"4:2:0","color":"BT.709 limited","rgb_bit_exact":false },
            "audio_tracks": track_names,
            "segment_container": "mpegts",
        });
        let _ = std::fs::write(session_dir.join("session.json"), serde_json::to_vec_pretty(&manifest).unwrap());
        let history=crate::history::Catalog::open(&buffer)?;
        let session_id=history.register(&session_dir,&s.game_name,&manifest,true)?;

        let shared = Arc::new(Shared {
            fps:info.fps,
            frames_repeated:AtomicU64::new(0),
            frames_without_source:AtomicU64::new(0),
            source_frames_unselected:AtomicU64::new(0),
            history,
            session_id,
            open_segment: Mutex::new(None),
            first_frame: Mutex::new(None),
            written_bytes: AtomicU64::new(0),
            written_ms: AtomicU64::new(0),
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
            saved: Mutex::new(Vec::new()),
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
        // Load the in-game hook off this thread (it can take a moment), unless the session has
        // already ended by the time it is in.
        let hook = Arc::new(Mutex::new(None));
        if let (SourceKind::Window(_), pid) = (&s.source, s.pid) {
            if pid != 0 && in_game_overlay_enabled() {
                let (slot, stop) = (hook.clone(), stop.clone());
                let _ = std::thread::Builder::new().name("hook-attach".into()).spawn(move || match crate::inject::GameHook::attach(pid) {
                    Ok(h) if !stop.load(Ordering::SeqCst) => {
                        eprintln!("in-game overlay: loaded into process {pid}");
                        *slot.lock().unwrap() = Some(h);
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("in-game overlay: not used for process {pid}: {e}"),
                });
            }
        }
        Ok(Self {
            shared,
            capture,
            encoder: info,
            audio,
            stop,
            video_thread: Some(video_thread),
            mux_thread: Some(mux_thread),
            gpu,
            hook,
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

    /// Begin a full recording from now. Returns false if one is already running.
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
        // Everything from here on is kept until the recording has been saved.
        let _ = self.shared.history.pin(&self.shared.session_id, &format!("session:{pin}"), t, 1e99);
        true
    }

    /// End the full recording: its media range `[start, end]` and the pin holding its footage
    /// (released by the caller once the save has its own hold on it).
    pub fn end_session(&self) -> Option<(f64, f64, String)> {
        let start = self.shared.session_start.lock().unwrap().take()?;
        let pin = self.shared.session_pin.load(Ordering::SeqCst);
        Some((start, self.shared.save_end(), format!("session:{pin}")))
    }
}

fn in_game_overlay_enabled() -> bool {
    std::env::var("CAPTURE_IN_GAME_OVERLAY").map_or(true, |v| v != "0")
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        *self.hook.lock().unwrap() = None; // unloads the hook from the game
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
        self.shared.history.end(&self.shared.session_id);
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

fn observe_generation(shared:&Shared,previous:&mut Option<u64>,current:u64) {
    if let Some(last)=*previous {
        if current==last {shared.frames_repeated.fetch_add(1,Ordering::Relaxed);}
        else {shared.source_frames_unselected.fetch_add(current.saturating_sub(last+1),Ordering::Relaxed);}
    }
    *previous=Some(current);
}

/// Pictures taken but not yet encoded (GPU input). Enough to ride out an encoder call that waits
/// on a GPU busy with the game; if the encoder falls further behind than this, ticks are dropped.
const ENCODE_QUEUE: usize = 6;

fn run_video(shared: Arc<Shared>, capture: Arc<wgc::Capture>, enc: VideoEncoder, fps: u32, stop: Arc<AtomicBool>, tx: std::sync::mpsc::SyncSender<Packet>) {
    let grabber = enc.grabber();
    // GPU input: the clock thread only takes pictures (a queued GPU copy) and an encoder thread
    // submits them. An encoder call can block for longer than a frame while the GPU is busy
    // with the game; on the clock thread, and under the frame lock, that dropped ticks and
    // starved the capture callback (frozen runs of repeated pictures in the recording).
    let (mut sync_enc, queue, enc_thread) = if grabber.pipelined() {
        let (qtx, qrx) = sync_channel::<video::Grab>(ENCODE_QUEUE);
        let (shared, tx) = (shared.clone(), tx.clone());
        let t = std::thread::Builder::new()
            .name("video-encode".into())
            .spawn(move || encode_loop(shared, enc, qrx, tx))
            .unwrap();
        (None, Some(qtx), Some(t))
    } else {
        (Some(enc), None, None)
    };
    let mut send = |p: Packet| {
        shared.frames_encoded.fetch_add(1,Ordering::Relaxed);
        let _ = tx.send(p);
    };
    let mut n: i64 = 0;
    let mut previous_generation=None;
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
            crate::trace::ev("drop", now, n, behind, (now - due) as i64);
            shared.frames_dropped.fetch_add(behind as u64, Ordering::Relaxed);
            n += behind;
        }
        if capture.closed.load(Ordering::Relaxed) || capture.resolution_changed.load(Ordering::Relaxed) {
            ended = true; // reported as "Waiting" by the engine; footage stays saveable
            break;
        }
        // Only the GPU copy happens under the frame lock (microseconds); encoding happens after
        // it is released, so the capture callback and screenshots never wait for the encoder.
        let t_lock = clock::now_100ns();
        let latest = capture.latest.lock().unwrap();
        let generation=capture.frames.load(Ordering::Relaxed);
        let t_locked = clock::now_100ns();
        let grab = latest.has_frame.then(|| grabber.grab(&latest, n));
        drop(latest);
        crate::trace::ev("enc", t_lock, n, (t_locked - t_lock) as i64, (clock::now_100ns() - t_locked) as i64);
        let result = match grab {
            None => {shared.frames_without_source.fetch_add(1,Ordering::Relaxed);Ok(())},
            Some(Err(e)) => Err(e),
            Some(Ok(g)) => match (&queue, sync_enc.as_mut()) {
                (Some(q), _) => match q.try_send(g) {
                    Ok(()) => {observe_generation(&shared,&mut previous_generation,generation);Ok(())},
                    Err(TrySendError::Full(_)) => {
                        // The encoder is more than ENCODE_QUEUE frames behind: lose this one.
                        crate::trace::ev("drop", now, n, 1, -1);
                        shared.frames_dropped.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    }
                    // The encoder thread stopped after an error (already reported).
                    Err(TrySendError::Disconnected(_)) => {
                        ended = true;
                        break;
                    }
                },
                (None, Some(enc)) => {
                    let mut out = Vec::new();
                    let r = enc.encode(g, &mut |p| out.push(p));
                    out.into_iter().for_each(&mut send);
                    if r.is_ok() {
                        observe_generation(&shared,&mut previous_generation,generation);
                    }
                    r
                }
                (None, None) => unreachable!(),
            },
        };
        if let Err(e) = result {
            shared.set_attention(Some(format!("Video encoding failed: {e}")));
            ended = true;
            break;
        }
        n += 1;
    }
    drop(queue);
    if let Some(t) = enc_thread {
        let _ = t.join(); // encodes what is queued, then flushes
    }
    if let Some(enc) = sync_enc.as_mut() {
        enc.flush(&mut send);
    }
    crate::trace::flush();
    if ended {
        // No more picture: stop the whole session (audio too) so the mux closes the last
        // segment on disk and saves end where the video ended, instead of an open segment
        // growing with audio only.
        *shared.ended_at.lock().unwrap() = Some(shared.now());
        stop.store(true, Ordering::SeqCst);
    }
}

fn encode_loop(shared: Arc<Shared>, mut enc: VideoEncoder, queue: Receiver<video::Grab>, tx: std::sync::mpsc::SyncSender<Packet>) {
    let mut send = |p: Packet| {
        shared.frames_encoded.fetch_add(1,Ordering::Relaxed);
        let _ = tx.send(p);
    };
    for g in queue.iter() {
        match enc.encode(g, &mut send) {
            Ok(()) => {}
            Err(e) => {
                shared.set_attention(Some(format!("Video encoding failed: {e}")));
                break; // dropping `queue` tells the clock thread to end the session
            }
        }
    }
    enc.flush(&mut send);
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
    // Audio that arrives while no segment is open (before the first video keyframe of the
    // session, or during a pause). Encoders deliver video later than audio (~0.5 s for the
    // software encoder), so without this the first moments of a session had no sound.
    let mut pending: std::collections::VecDeque<Packet> = std::collections::VecDeque::new();
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
                shared.close_segment(p.secs(), bytes);
            }
            let live = *shared.live.lock().unwrap();
            // Retention runs here too, so a busy engine thread can never let the disk fill up.
            if let Err(e) = shared.history.retain(live.history_secs, live.cap_bytes, live.min_free_bytes) {
                shared.set_attention(Some(format!("History cleanup: {e}")));
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
                Ok(mut w) => {
                    seg_start = p.secs();
                    // The keyframe first (a segment must start with it), then the waiting
                    // audio that belongs at or after it.
                    let r = w.write(p);
                    for q in std::mem::take(&mut pending).into_iter().filter(|q| q.secs() >= seg_start) {
                        let _ = w.write(q);
                    }
                    writer = Some(w);
                    shared.open_segment(seq, path, seg_start);
                    if write_failed && r.is_ok() {
                        write_failed = false;
                        shared.set_attention(None);
                    }
                    if let Err(e) = r {
                        write_failed = true;
                        shared.set_attention(Some(cant_write(&e)));
                        if let Some(w) = writer.take() {
                            let bytes = w.finish();
                            shared.close_segment(last_video, bytes);
                        }
                    }
                    continue;
                }
                Err(e) => {
                    write_failed = true;
                    shared.set_attention(Some(cant_write(&e)));
                    continue;
                }
            }
        }
        let Some(w) = writer.as_mut() else {
            if !is_video {
                pending.push_back(p);
                if pending.len() > 1000 {
                    pending.pop_front(); // only the last few seconds can belong to a new segment
                }
            }
            continue;
        };
        {
            if let Err(e) = w.write(p) {
                // Stop writing into a dead file: close it and retry a new segment at the next
                // keyframe (about once a second) instead of failing for a whole segment.
                write_failed = true;
                shared.set_attention(Some(cant_write(&e)));
                if let Some(w) = writer.take() {
                    let bytes = w.finish();
                    shared.close_segment(last_video, bytes);
                }
            }
        }
    }
    if let Some(w) = writer.take() {
        let bytes = w.finish();
        shared.close_segment(last_video + 1.0 / shared.fps as f64, bytes);
    }
    shared.mux_done.store(true, Ordering::SeqCst);
    shared.history.end(&shared.session_id);
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
