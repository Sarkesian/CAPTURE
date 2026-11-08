//! CAPTURE capture engine. Driven by the `capture-worker` binary over the typed protocol in
//! `capture-protocol`; the UI never touches capture/encoding directly.

pub mod audio;
pub mod av;
pub mod clock;
pub mod detect;
pub mod devices;
pub mod gpu;
pub mod hotkey;
pub mod mic;
pub mod mux;
pub mod overlay;
pub mod pipeline;
pub mod ring;
pub mod save;
pub mod shot;
pub mod sound;
pub mod stems;
pub mod storage;
pub mod targets;
pub mod thumb;
pub mod video;
pub mod wgc;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use capture_protocol::*;

use audio::{MixParams, Role, Source, SourceSpec};
use overlay::{Kind, Notice, Overlay};
use pipeline::{Live, Pipeline, SaveKind, Settings, SourceKind};

pub type Emit = Arc<dyn Fn(Message) + Send + Sync>;

/// Why a replay could not start. `waiting` means "nothing to record yet" (not an error).
#[derive(Debug, Clone)]
struct Unresolved {
    waiting: bool,
    message: String,
}

fn waiting(m: impl Into<String>) -> Unresolved {
    Unresolved { waiting: true, message: m.into() }
}
fn failed(m: impl Into<String>) -> Unresolved {
    Unresolved { waiting: false, message: m.into() }
}

pub struct Engine {
    cfg: Option<EngineConfig>,
    replay_on: bool,
    pipeline: Option<Pipeline>,
    start_error: Option<Unresolved>,
    hotkey: Option<hotkey::HotkeyService>,
    overlay: Option<Arc<Overlay>>,
    emit: Emit,
    next_save_id: u64,
    fps_sample: (Instant, u64),
    capture_fps: f64,
    last_state: Option<EngineState>,
    last_retry: Instant,
    mic_monitor: Option<mic::Monitor>,
}

pub fn fmt_secs(s: f64) -> String {
    let s = s.round() as u64;
    match (s / 60, s % 60) {
        (0, r) => format!("{r}s"),
        (m, 0) => format!("{m} min"),
        (m, r) => format!("{m}m {r:02}s"),
    }
}

impl Engine {
    pub fn new(emit: Emit) -> Self {
        Self {
            cfg: None,
            replay_on: false,
            pipeline: None,
            start_error: None,
            hotkey: None,
            overlay: None,
            emit,
            next_save_id: 1,
            fps_sample: (Instant::now(), 0),
            capture_fps: 0.0,
            last_state: None,
            last_retry: Instant::now(),
            mic_monitor: None,
        }
    }

    /// The hotkey service needs a way to inject SaveReplay into the engine thread.
    pub fn attach_hotkey(&mut self, svc: hotkey::HotkeyService) {
        self.hotkey = Some(svc);
    }

    pub fn attach_overlay(&mut self, o: Arc<Overlay>) {
        self.overlay = Some(o);
    }

    fn send(&self, re: Option<u64>, event: Event) {
        (self.emit)(Message::new(re, event));
    }

    fn notice(&self, kind: Kind, title: String, detail: String) {
        if self.cfg.as_ref().is_some_and(|c| c.overlay) {
            if let Some(o) = &self.overlay {
                o.show(Notice { title, detail, kind });
            }
        }
    }

    pub fn handle(&mut self, re: Option<u64>, cmd: Command) -> bool {
        match cmd {
            Command::Configure(cfg) => self.configure(cfg),
            Command::SetReplay { enabled } => {
                self.replay_on = enabled;
                if enabled {
                    self.start_pipeline();
                } else {
                    self.stop_session(None);
                    self.pipeline = None;
                    self.start_error = None;
                }
            }
            Command::StartSession if re.is_none() => self.toggle_session(re), // hotkey
            Command::StartSession => self.start_session(re),
            Command::StopSession => self.stop_session(re),
            Command::SaveReplay => self.save(re, None),
            Command::SaveRange { start, end } => self.save(re, Some((start, end))),
            Command::PrepareReview => match &self.pipeline {
                Some(p) => {
                    let emit = self.emit.clone();
                    p.prepare_review(move |r| {
                        emit(Message::new(re, match r {
                            Ok(rv) => Event::ReviewReady(rv),
                            Err(message) => Event::ReviewFailed { message },
                        }))
                    });
                }
                None => self.send(re, Event::ReviewFailed { message: "Nothing is being recorded right now.".into() }),
            },
            Command::ListAudio => {
                let emit = self.emit.clone();
                std::thread::spawn(move || {
                    unsafe {
                        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                    }
                    use windows::Win32::Media::Audio::{eCapture, eRender};
                    emit(Message::new(re, Event::AudioDevices {
                        inputs: devices::list(eCapture),
                        outputs: devices::list(eRender),
                        apps: devices::audio_apps(),
                    }));
                });
            }
            Command::MicMonitor { enabled, device } => {
                self.mic_monitor = None; // stop the old one before opening the device again
                if enabled {
                    self.mic_monitor = Some(mic::Monitor::start(device));
                }
            }
            Command::MicTest { device } => {
                self.mic_monitor = None;
                let emit = self.emit.clone();
                std::thread::spawn(move || {
                    unsafe {
                        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                    }
                    mic::test(device, |phase, peak, message| {
                        emit(Message::new(re, Event::MicTest { phase: phase.into(), peak: (peak * 100.0).round() / 100.0, message }));
                    });
                });
            }
            Command::Screenshot => self.screenshot(re),
            Command::ExportStems { path } => {
                let emit = self.emit.clone();
                std::thread::spawn(move || {
                    let ev = match stems::export(Path::new(&path)) {
                        Ok(r) => Event::StemsDone {
                            path,
                            folder: Some(r.folder.to_string_lossy().into_owned()),
                            files: r.files.iter().map(|f| f.to_string_lossy().into_owned()).collect(),
                            error: None,
                        },
                        Err(e) => Event::StemsDone { path, folder: None, files: vec![], error: Some(e) },
                    };
                    emit(Message::new(re, ev));
                });
            }
            Command::ClipInfo { path } => {
                let root = self.cfg.as_ref().map(|c| PathBuf::from(&c.capture_root)).unwrap_or_default();
                let emit = self.emit.clone();
                std::thread::spawn(move || {
                    let ev = match thumb::clip_info(&root, Path::new(&path)) {
                        Ok(i) => Event::ClipInfo {
                            path,
                            thumb: Some(i.thumb.to_string_lossy().into_owned()),
                            duration_secs: Some(i.duration_secs),
                            error: None,
                        },
                        Err(e) => Event::ClipInfo { path, thumb: None, duration_secs: None, error: Some(e) },
                    };
                    emit(Message::new(re, ev));
                });
            }
            Command::ReleaseReview => {
                if let Some(p) = &self.pipeline {
                    p.shared.release_review();
                }
            }
            Command::AddMarker { label } => self.add_marker(re, label),
            Command::RenameMarker { id, label } => {
                if !self.pipeline.as_ref().is_some_and(|p| p.shared.rename_marker(id, label)) {
                    self.send(re, Event::Error { message: "That marker no longer exists.".into() });
                }
            }
            Command::RemoveMarker { id } => {
                if !self.pipeline.as_ref().is_some_and(|p| p.shared.remove_marker(id)) {
                    self.send(re, Event::Error { message: "That marker no longer exists.".into() });
                }
            }
            Command::ListTargets => {
                let windows = targets::windows();
                let displays = targets::monitors().into_iter().map(|m| m.target).collect();
                self.send(re, Event::Targets { windows, displays });
            }
            Command::GetState => {}
            Command::RecoverLeftovers => {
                let Some(root) = self.cfg.as_ref().map(|c| PathBuf::from(&c.capture_root)) else { return true };
                let dirs = self.leftover_dirs(&root);
                let emit = self.emit.clone();
                std::thread::spawn(move || {
                    let (mut clips, mut failed) = (vec![], vec![]);
                    for d in dirs {
                        match recover_session(&root, &d) {
                            Ok(p) => {
                                let _ = std::fs::remove_dir_all(&d);
                                clips.push(p.to_string_lossy().into_owned());
                            }
                            Err(e) => failed.push(format!("{}: {e}", d.display())),
                        }
                    }
                    emit(Message::new(re, Event::Recovered { clips, failed }));
                });
            }
            Command::ClearTempCache => {
                let freed = self.clear_leftovers();
                self.send(re, Event::Error { message: format!("Deleted {:.1} GB of temporary footage.", freed as f64 / 1e9) });
            }
            Command::Shutdown => {
                self.shutdown();
                return false;
            }
        }
        self.publish_state(re, true);
        true
    }

    fn configure(&mut self, cfg: EngineConfig) {
        if let Some(h) = &self.hotkey {
            h.set(bindings(&cfg));
        }
        let restart = match &self.cfg {
            Some(old) => {
                old.target != cfg.target
                    || old.fps != cfg.fps
                    || old.quality != cfg.quality
                    || old.capture_cursor != cfg.capture_cursor
                    || !old.audio.same_sources(&cfg.audio)
                    || old.capture_root != cfg.capture_root
                    || self.pipeline.as_ref().is_some_and(|p| {
                        let (a, b) = (old.for_game(&p.shared.target_exe), cfg.for_game(&p.shared.target_exe));
                        a.quality != b.quality || a.audio.game_audio != b.audio.game_audio
                    })
            }
            None => true,
        };
        // In Auto mode, editing the games/ignore lists must not interrupt a running recording
        // unless the current game was just ignored.
        let auto_lists_only = matches!((&self.cfg, &cfg), (Some(old), new)
            if matches!(old.target, CaptureTarget::Auto { .. }) && matches!(new.target, CaptureTarget::Auto { .. })
            && old.fps == new.fps && old.quality == new.quality && old.capture_cursor == new.capture_cursor
            && old.audio.same_sources(&new.audio) && old.capture_root == new.capture_root);
        let live = live_of(&cfg);
        let now_ignored = match (&cfg.target, &self.pipeline) {
            (CaptureTarget::Auto { ignore, .. }, Some(p)) => ignore.iter().any(|i| i.eq_ignore_ascii_case(&p.shared.target_exe)),
            _ => false,
        };
        self.cfg = Some(cfg);
        if let Some(p) = &self.pipeline {
            let game_cfg = self.cfg.as_ref().map(|c| c.for_game(&p.shared.target_exe));
            if let Some(c) = &game_cfg {
                p.set_live(live_of(c));
                if let Some(a) = &p.audio {
                    a.set_mix(mix_of(c));
                }
            } else {
                p.set_live(live);
            }
        }
        if !self.replay_on {
            return;
        }
        if auto_lists_only {
            if now_ignored {
                self.pipeline = None;
                self.start_error = Some(waiting("Watching for games."));
            }
            self.last_retry = Instant::now() - std::time::Duration::from_secs(10); // re-detect now
            self.tick();
        } else if restart || self.pipeline.is_none() {
            self.start_pipeline();
        }
    }

    fn start_pipeline(&mut self) {
        self.stop_session(None);
        self.pipeline = None; // finalize any previous session first
        self.start_error = None;
        let Some(cfg) = self.cfg.clone() else {
            self.start_error = Some(failed("Settings have not been loaded yet."));
            return;
        };
        match resolve(&cfg) {
            Ok(s) => self.launch(&cfg, s),
            Err(u) => self.start_error = Some(u),
        }
    }

    fn launch(&mut self, cfg: &EngineConfig, s: Settings) {
        let cfg = &cfg.for_game(&s.exe);
        self.stop_session(None);
        self.pipeline = None;
        let (name, display) = (s.game_name.clone(), matches!(s.source, SourceKind::Monitor(_)));
        match Pipeline::start(s, live_of(cfg)) {
            Ok(p) => {
                self.start_error = None;
                self.fps_sample = (Instant::now(), 0);
                self.pipeline = Some(p);
                let hk = if cfg.save_hotkey.is_empty() { "Save clip in CAPTURE".to_string() } else { cfg.save_hotkey.clone() };
                let title = if display { "Recording your display".to_string() } else { format!("Recording {name}") };
                let mic = match (cfg.audio.mic_enabled, cfg.audio.mic_muted) {
                    (true, false) => " · mic ON",
                    (true, true) => " · mic muted",
                    _ => "",
                };
                self.notice(Kind::Recording, title, format!("{hk} saves the last {}{mic}", fmt_secs(live_of(cfg).replay_secs as f64)));
            }
            Err(e) => self.start_error = Some(failed(e)),
        }
    }

    fn start_session(&mut self, re: Option<u64>) {
        let Some(p) = self.pipeline.as_ref().filter(|p| !p.target_gone()) else {
            let msg = if self.replay_on { "Nothing is being captured right now." } else { "Turn on recording first." };
            self.send(re, Event::Error { message: format!("Can't start a recording: {msg}") });
            return;
        };
        if p.start_session() {
            let game = p.shared.game_name.clone();
            self.notice(Kind::Recording, format!("Recording session · {game}"), "Press the same key again to stop and save it".into());
        }
    }

    /// Stops and saves a running session (no-op when none). Used by the command, the hotkey,
    /// and automatically when the game closes, recording is turned off, disk runs low, or the
    /// worker shuts down.
    fn stop_session(&mut self, re: Option<u64>) {
        let Some(p) = &self.pipeline else { return };
        if p.shared.session_start.lock().unwrap().is_none() {
            return;
        }
        let id = self.next_save_id;
        self.next_save_id += 1;
        let emit = self.emit.clone();
        let overlay = self.overlay.clone().filter(|_| self.cfg.as_ref().is_some_and(|c| c.overlay));
        let game = p.shared.game_name.clone();
        let requested = p.stop_session(
            move |r| {
                let (ev, n) = match r {
                    Ok((clip, _)) => {
                        let n = Notice { kind: Kind::Saved, title: format!("Session saved · {}", fmt_secs(clip.duration_secs)), detail: game };
                        (Event::SaveFinished(clip), n)
                    }
                    Err(message) => (
                        Event::SaveFailed { save_id: id, message: message.clone() },
                        Notice { kind: Kind::Failed, title: "Session not saved".into(), detail: message },
                    ),
                };
                if let Some(o) = overlay {
                    o.show(n);
                }
                emit(Message::new(None, ev));
            },
            id,
        );
        if let Some(secs) = requested {
            self.send(re, Event::SaveStarted { save_id: id, requested_secs: secs });
        }
    }

    fn screenshot(&mut self, re: Option<u64>) {
        let Some(p) = self.pipeline.as_ref().filter(|p| !p.target_gone()) else {
            self.send(re, Event::Error { message: "Nothing is being captured right now, so there is nothing to screenshot.".into() });
            return;
        };
        match shot::save(&p.gpu, &p.capture, &p.shared.root, &p.shared.game_name) {
            Ok(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.notice(Kind::Saved, "Screenshot saved".into(), name);
                self.send(re, Event::ScreenshotSaved { path: path.to_string_lossy().into_owned() });
            }
            Err(e) => {
                self.notice(Kind::Failed, "Screenshot not saved".into(), e.clone());
                self.send(re, Event::Error { message: format!("Screenshot not saved: {e}") });
            }
        }
    }

    fn toggle_session(&mut self, re: Option<u64>) {
        let running = self.pipeline.as_ref().is_some_and(|p| p.shared.session_start.lock().unwrap().is_some());
        if running {
            self.stop_session(re)
        } else {
            self.start_session(re)
        }
    }

    fn add_marker(&mut self, re: Option<u64>, label: Option<String>) {
        let mcfg = self.cfg.as_ref().map(|c| c.markers.clone()).unwrap_or_default();
        let Some(p) = self.pipeline.as_ref().filter(|p| !p.target_gone()) else {
            self.notice(Kind::Failed, "Marker not added".into(), "Nothing is being recorded right now.".into());
            self.send(re, Event::Error { message: "Nothing is being recorded right now, so there is nothing to mark.".into() });
            return;
        };
        let m = p.shared.add_marker(label, &mcfg);
        if mcfg.sound {
            sound::play_chime(mcfg.sound_volume);
        }
        let detail = match (&m.note, m.footage) {
            (Some(n), _) => n.clone(),
            (None, MarkerFootage::Protected) => format!(
                "{} · footage {}s before to {}s after is kept",
                p.shared.game_name, mcfg.protect_before_secs, mcfg.protect_after_secs
            ),
            _ => format!("{} · temporary until you save it", p.shared.game_name),
        };
        self.notice(Kind::Saved, format!("Marker added · {}", m.at), detail);
        self.send(re, Event::MarkerAdded(m));
    }

    fn save(&mut self, re: Option<u64>, range: Option<(f64, f64)>) {
        let Some(p) = &self.pipeline else {
            let msg = if self.replay_on {
                "Nothing is being recorded right now, so there is nothing to save."
            } else {
                "Instant replay is off. Turn it on to save clips."
            };
            self.notice(Kind::Failed, "Clip not saved".into(), msg.into());
            self.send(re, Event::SaveFailed { save_id: 0, message: msg.into() });
            return;
        };
        let id = self.next_save_id;
        self.next_save_id += 1;
        let emit = self.emit.clone();
        let overlay = self.overlay.clone().filter(|_| self.cfg.as_ref().is_some_and(|c| c.overlay));
        let game = p.shared.game_name.clone();
        let requested = p.request_save(
            range,
            SaveKind::Clip,
            move |r| {
                let (ev, notice) = match r {
                    Ok((clip, requested)) => {
                        let short = clip.duration_secs + 1.0 < requested;
                        let detail = if short {
                            format!("{game} · only {} was buffered", fmt_secs(clip.duration_secs))
                        } else {
                            game.clone()
                        };
                        let n = Notice { kind: Kind::Saved, title: format!("Clip saved · {}", fmt_secs(clip.duration_secs)), detail };
                        (Event::SaveFinished(clip), n)
                    }
                    Err(message) => {
                        let n = Notice { kind: Kind::Failed, title: "Clip not saved".into(), detail: message.clone() };
                        (Event::SaveFailed { save_id: id, message }, n)
                    }
                };
                if let Some(o) = overlay {
                    o.show(notice);
                }
                emit(Message::new(None, ev));
            },
            id,
        );
        self.send(re, Event::SaveStarted { save_id: id, requested_secs: requested });
    }

    /// Periodic housekeeping (every ~0.5 s):
    /// - Auto mode: follow the foreground game (start on it, switch to a different game).
    /// - Other modes: retry a target that is not running yet, or was closed and reopened.
    /// A session whose window closed stays saveable until a new one starts.
    pub fn tick(&mut self) {
        if !self.replay_on {
            return;
        }
        let Some(cfg) = self.cfg.clone() else { return };
        let closed = self.pipeline.as_ref().is_some_and(|p| p.target_gone());
        let must_stop = self.pipeline.as_ref().is_some_and(|p| p.shared.session_must_stop.swap(false, Ordering::SeqCst));
        if closed || must_stop {
            self.stop_session(None);
        }
        if let CaptureTarget::Auto { games, ignore } = &cfg.target {
            if self.last_retry.elapsed().as_secs_f64() < 1.0 {
                return;
            }
            self.last_retry = Instant::now();
            // Recording something: switch only when the user focuses a *different* game.
            // Nothing recording: take the focused game, else any open game window.
            let running = self.pipeline.is_some() && !closed;
            let found = if running { detect::foreground_game(games, ignore) } else { detect::current_game(games, ignore) };
            let Some((w, reason)) = found else {
                if self.pipeline.is_none() || closed {
                    self.start_error.get_or_insert_with(|| waiting("Watching for games."));
                }
                return;
            };
            let same = self.pipeline.as_ref().is_some_and(|p| p.shared.target_pid == w.pid && !closed);
            if !same {
                match settings_for_window(&cfg, &w, Some(reason.label())) {
                    Ok(s) => self.launch(&cfg, s),
                    Err(u) => self.start_error = Some(u),
                }
            }
            return;
        }
        if !(closed || self.pipeline.is_none()) || self.last_retry.elapsed().as_secs_f64() < 3.0 {
            return;
        }
        self.last_retry = Instant::now();
        if let Ok(s) = resolve(&cfg) {
            self.launch(&cfg, s);
        }
    }

    /// Called periodically and after every command. Emits only when something changed,
    /// unless `force`.
    pub fn publish_state(&mut self, re: Option<u64>, force: bool) {
        let st = self.state();
        if force || self.last_state.as_ref() != Some(&st) {
            self.last_state = Some(st.clone());
            self.send(re, Event::State(st));
        }
    }

    fn state(&mut self) -> EngineState {
        let hotkeys = self.hotkey.as_ref().map(|h| h.state.lock().unwrap().clone()).unwrap_or_default();
        let hotkey = self.hotkey.as_ref().map(|h| h.state_for(HotkeyAction::SaveReplay)).unwrap_or_default();
        let root = self.cfg.as_ref().map(|c| PathBuf::from(&c.capture_root));
        let free_bytes = root.as_deref().and_then(storage::free_bytes);
        let leftover_buffer_bytes = root.as_deref().map(|r| self.leftover_bytes(r)).unwrap_or(0);
        let mut st = EngineState {
            status: Status::Ready,
            attention: None,
            target_name: None,
            target_exe: None,
            detected_by: None,
            encoder: None,
            buffered_secs: 0.0,
            buffer_bytes: 0,
            capture_fps: 0.0,
            frames_captured: 0,
            tracks: vec![],
            saves_in_progress: 0,
            hotkey,
            hotkeys,
            markers: vec![],
            now_secs: 0.0,
            session_secs: None,
            mic_monitor: None,
            leftover_buffer_bytes,
            free_bytes,
        };
        if self.mic_monitor.as_ref().is_some_and(|m| m.started.elapsed().as_secs() > 300) {
            self.mic_monitor = None;
        }
        st.mic_monitor = self.mic_monitor.as_ref().map(|m| m.level());
        if !self.replay_on {
            return st;
        }
        let Some(p) = &self.pipeline else {
            let u = self.start_error.clone().unwrap_or_else(|| failed("Replay could not start."));
            st.status = if u.waiting { Status::Waiting } else { Status::NeedsAttention };
            st.attention = Some(u.message);
            return st;
        };
        let frames = p.capture.frames.load(Ordering::Relaxed);
        let dt = self.fps_sample.0.elapsed().as_secs_f64();
        if dt >= 1.0 {
            self.capture_fps = (frames - self.fps_sample.1) as f64 / dt;
            self.fps_sample = (Instant::now(), frames);
        }
        let sh = &p.shared;
        let (buffered, bytes, has_segments) = {
            let ring = sh.ring.lock().unwrap();
            let live = *sh.live.lock().unwrap();
            (ring.available_secs(sh.now()).min(live.replay_secs as f64), ring.total_bytes(), !ring.segments().is_empty())
        };
        let attention = sh.attention.lock().unwrap().clone();
        st.markers = sh.markers_now();
        st.session_secs = sh.session_start.lock().unwrap().map(|s| ((sh.now() - s) * 10.0).round() / 10.0);
        st.now_secs = (sh.now() * 10.0).round() / 10.0;
        st.target_name = Some(sh.game_name.clone());
        st.target_exe = Some(sh.target_exe.clone());
        st.detected_by = sh.detected_by.clone();
        st.encoder = Some(p.encoder.clone());
        st.buffered_secs = (buffered * 10.0).round() / 10.0;
        st.buffer_bytes = bytes;
        st.capture_fps = (self.capture_fps * 10.0).round() / 10.0;
        st.frames_captured = frames;
        st.tracks = p.audio.as_ref().map(|a| a.states()).unwrap_or_default();
        for t in &mut st.tracks {
            t.peak = (t.peak * 100.0).round() / 100.0;
        }
        st.saves_in_progress = sh.saves_in_progress.load(Ordering::Relaxed);
        if p.target_gone() {
            st.status = Status::Waiting;
            st.attention = Some(format!(
                "{} closed. Its last {} can still be saved until a new game starts.",
                sh.game_name,
                fmt_secs(buffered)
            ));
            return st;
        }
        st.status = if attention.is_some() {
            Status::NeedsAttention
        } else if frames > 0 && has_segments && sh.frames_encoded.load(Ordering::Relaxed) > 0 {
            Status::Buffering
        } else {
            Status::Starting
        };
        st.attention = attention.or_else(|| {
            (st.status == Status::Starting && sh.now() > 5.0 && frames == 0)
                .then(|| "No frames received yet. The game may be minimized, protected, or in exclusive fullscreen. Try borderless mode or Whole display.".into())
        });
        st
    }

    fn current_session(&self) -> Option<PathBuf> {
        self.pipeline.as_ref().map(|p| p.shared.session_dir.clone())
    }

    fn leftover_dirs(&self, root: &Path) -> Vec<PathBuf> {
        let current = self.current_session();
        std::fs::read_dir(storage::buffer_dir(root))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && Some(p) != current.as_ref())
            .collect()
    }

    fn leftover_bytes(&self, root: &Path) -> u64 {
        self.leftover_dirs(root).iter().map(|d| storage::dir_size(d)).sum()
    }

    /// Deletes only `Buffer\<old session>` folders — never clips, never the live session.
    fn clear_leftovers(&mut self) -> u64 {
        let Some(root) = self.cfg.as_ref().map(|c| PathBuf::from(&c.capture_root)) else { return 0 };
        let mut freed = 0;
        for d in self.leftover_dirs(&root) {
            let size = storage::dir_size(&d);
            if std::fs::remove_dir_all(&d).is_ok() {
                freed += size;
            }
        }
        freed
    }

    pub fn shutdown(&mut self) {
        self.stop_session(None);
        if let Some(p) = &self.pipeline {
            // Let in-flight saves finish (bounded) before tearing down the session.
            let until = Instant::now() + std::time::Duration::from_secs(30);
            while p.shared.saves_in_progress.load(Ordering::SeqCst) > 0 && Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        self.pipeline = None;
        self.hotkey = None;
    }
}

/// Hotkey bindings handled by the worker (every action has a working implementation).
pub fn bindings(cfg: &EngineConfig) -> hotkey::Bindings {
    vec![
        (HotkeyAction::SaveReplay, cfg.save_hotkey.clone()),
        (HotkeyAction::AddMarker, cfg.hotkeys.marker.clone()),
        (HotkeyAction::ToggleOverlay, cfg.hotkeys.overlay.clone()),
        (HotkeyAction::ToggleSession, cfg.hotkeys.session.clone()),
        (HotkeyAction::Screenshot, cfg.hotkeys.screenshot.clone()),
    ]
}

/// Save a leftover session folder (crash / power loss) as `Clips\<Game>\..._Recovered.mp4`.
/// Damaged segments are skipped; the folder is only deleted by the caller after success.
fn recover_session(root: &Path, dir: &Path) -> Result<PathBuf, String> {
    let manifest: serde_json::Value = std::fs::read(dir.join("session.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::json!({}));
    let game = manifest["game"].as_str().unwrap_or("Recovered").to_string();
    let tracks: Vec<String> = manifest["audio_tracks"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let mut segs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ts") && std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false))
        .collect();
    segs.sort();
    if segs.is_empty() {
        return Err("no footage".into());
    }
    let list: Vec<(PathBuf, f64)> = segs.into_iter().map(|p| (p, 0.0)).collect();
    let out_dir = storage::clips_dir(root, &game);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let name = storage::clip_file_name(&storage::local_stamp(), &game, "Recovered");
    let path = storage::reserve_unique_path(&out_dir, &name).map_err(|e| e.to_string())?;
    save::remux(&list, 0.0, 0.0, 1e9, &tracks, &path).map(|(c, _)| c.path)
}

fn live_of(cfg: &EngineConfig) -> Live {
    Live { replay_secs: cfg.replay_secs.clamp(5, 3600), cap_bytes: cfg.buffer_cap_bytes, min_free_bytes: cfg.min_free_bytes }
}

fn base_settings(cfg: &EngineConfig) -> Result<PathBuf, Unresolved> {
    if cfg.capture_root.trim().is_empty() {
        return Err(failed("Choose a folder for your captures first."));
    }
    Ok(PathBuf::from(&cfg.capture_root))
}

/// Audio sources for a session. `game = Some((pid, exe))` for a game/app window, `None` for
/// whole-display capture (where "System Audio" already contains everything, so voice chat is
/// kept as its own track but not added to the mix again, and Other Desktop would be identical).
pub fn audio_sources(cfg: &EngineConfig, game: Option<(u32, &str)>) -> Vec<Source> {
    let a = &cfg.audio;
    let mut v = Vec::new();
    let display = game.is_none();
    if a.game_audio {
        v.push(match game {
            Some((pid, _)) => Source { spec: SourceSpec::Process { pid }, role: Role::Game, label: "Game Audio".into(), in_mix: true },
            None => Source { spec: SourceSpec::AllExceptCapture, role: Role::Game, label: "System Audio".into(), in_mix: true },
        });
    }
    if a.mic_enabled {
        v.push(Source { spec: SourceSpec::Mic { device_id: a.mic_device.clone() }, role: Role::Mic, label: "Microphone".into(), in_mix: true });
    }
    if let (true, Some(exe)) = (a.voice_enabled, a.voice_app.clone().filter(|e| !e.trim().is_empty())) {
        let same_as_game = game.is_some_and(|(_, g)| g.eq_ignore_ascii_case(&exe));
        let in_mix = !same_as_game && !(display && a.game_audio);
        v.push(Source { spec: SourceSpec::App { exe }, role: Role::Voice, label: "Voice Chat".into(), in_mix });
    }
    if a.other_enabled && !(display && a.game_audio) {
        v.push(Source { spec: SourceSpec::AllExceptCapture, role: Role::Other, label: "Other Desktop".into(), in_mix: false });
    }
    v
}

pub fn mix_of(cfg: &EngineConfig) -> MixParams {
    let v = &cfg.audio.volumes;
    MixParams { game: v.game.clamp(0.0, 2.0), mic: v.mic.clamp(0.0, 2.0), voice: v.voice.clamp(0.0, 2.0), mic_muted: cfg.audio.mic_muted }
}

fn settings_for_window(cfg: &EngineConfig, w: &WindowTarget, detected_by: Option<&str>) -> Result<Settings, Unresolved> {
    let cfg = &cfg.for_game(&w.exe);
    let root = base_settings(cfg)?;
    let audio = audio_sources(cfg, Some((w.pid, &w.exe)));
    Ok(Settings {
        root,
        game_name: w.app_name.clone(),
        exe: w.exe.clone(),
        pid: w.pid,
        detected_by: detected_by.map(str::to_string),
        source: SourceKind::Window(w.hwnd as isize),
        fps: cfg.fps.clamp(10, 240),
        quality: cfg.quality,
        cursor: cfg.capture_cursor,
        audio,
        mix: mix_of(cfg),
    })
}

/// Turn the configured target into concrete capture settings, verifying it still exists.
fn resolve(cfg: &EngineConfig) -> Result<Settings, Unresolved> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
    let root = base_settings(cfg)?;
    match &cfg.target {
        CaptureTarget::None => Err(failed("Choose what to record: Auto, a game or app, or your whole display.")),
        CaptureTarget::Auto { games, ignore } => match detect::current_game(games, ignore) {
            Some((w, reason)) => settings_for_window(cfg, &w, Some(reason.label())),
            None => Err(waiting("Watching for games.")),
        },
        CaptureTarget::App { exe } => {
            let w = targets::windows()
                .into_iter()
                .filter(|w| w.exe.eq_ignore_ascii_case(exe))
                .max_by_key(|w| w.width as u64 * w.height as u64)
                .ok_or_else(|| waiting(format!("Waiting for {exe} to start. Recording begins automatically when it opens.")))?;
            settings_for_window(cfg, &w, None)
        }
        CaptureTarget::Window { hwnd, pid, exe, title } => {
            let h = HWND(*hwnd as *mut _);
            let mut actual = 0u32;
            unsafe { GetWindowThreadProcessId(h, Some(&mut actual)) };
            if !unsafe { IsWindow(Some(h)) }.as_bool() || actual != *pid {
                return Err(failed(format!("\"{title}\" is no longer open. Pick it again from the list.")));
            }
            let app_name = targets::app_name(&targets::process_path(*pid).unwrap_or_default(), exe, title);
            let w = WindowTarget { hwnd: *hwnd, pid: *pid, exe: exe.clone(), title: title.clone(), app_name, width: 0, height: 0 };
            settings_for_window(cfg, &w, None)
        }
        CaptureTarget::Display { index } => {
            let mons = targets::monitors();
            let m = mons.iter().find(|m| m.target.index == *index).ok_or_else(|| failed("That display is not connected."))?;
            let audio = audio_sources(cfg, None);
            Ok(Settings {
                root,
                game_name: "Desktop".into(),
                exe: String::new(),
                pid: 0,
                detected_by: None,
                source: SourceKind::Monitor(m.handle),
                fps: cfg.fps.clamp(10, 240),
                quality: cfg.quality,
                cursor: cfg.capture_cursor,
                audio,
                mix: mix_of(cfg),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn durations_read_naturally() {
        assert_eq!(super::fmt_secs(42.4), "42s");
        assert_eq!(super::fmt_secs(120.0), "2 min");
        assert_eq!(super::fmt_secs(102.0), "1m 42s");
    }
}
