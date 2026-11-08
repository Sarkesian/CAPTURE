//! CAPTURE capture engine. Driven by the `capture-worker` binary over the typed protocol in
//! `capture-protocol`; the UI never touches capture/encoding directly.

pub mod audio;
pub mod av;
pub mod clock;
pub mod gpu;
pub mod hotkey;
pub mod mux;
pub mod pipeline;
pub mod ring;
pub mod save;
pub mod storage;
pub mod targets;
pub mod video;
pub mod wgc;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use capture_protocol::*;

use audio::SourceSpec;
use pipeline::{Live, Pipeline, Settings, SourceKind};

pub type Emit = Arc<dyn Fn(Message) + Send + Sync>;

pub struct Engine {
    cfg: Option<EngineConfig>,
    replay_on: bool,
    pipeline: Option<Pipeline>,
    start_error: Option<String>,
    hotkey: Option<hotkey::HotkeyService>,
    emit: Emit,
    next_save_id: u64,
    fps_sample: (Instant, u64),
    capture_fps: f64,
    last_state: Option<EngineState>,
    last_retry: Instant,
}

impl Engine {
    pub fn new(emit: Emit) -> Self {
        Self {
            cfg: None,
            replay_on: false,
            pipeline: None,
            start_error: None,
            hotkey: None,
            emit,
            next_save_id: 1,
            fps_sample: (Instant::now(), 0),
            capture_fps: 0.0,
            last_state: None,
            last_retry: Instant::now(),
        }
    }

    /// The hotkey service needs a way to inject SaveReplay into the engine thread.
    pub fn attach_hotkey(&mut self, svc: hotkey::HotkeyService) {
        self.hotkey = Some(svc);
    }

    fn send(&self, re: Option<u64>, event: Event) {
        (self.emit)(Message::new(re, event));
    }

    pub fn handle(&mut self, re: Option<u64>, cmd: Command) -> bool {
        match cmd {
            Command::Configure(cfg) => self.configure(cfg),
            Command::SetReplay { enabled } => {
                self.replay_on = enabled;
                if enabled {
                    self.start_pipeline();
                } else {
                    self.pipeline = None;
                    self.start_error = None;
                }
            }
            Command::SaveReplay => self.save(re),
            Command::ListTargets => {
                let windows = targets::windows();
                let displays = targets::monitors().into_iter().map(|m| m.target).collect();
                self.send(re, Event::Targets { windows, displays });
            }
            Command::GetState => {}
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
            h.set(&cfg.save_hotkey);
        }
        let restart = match &self.cfg {
            Some(old) => {
                old.target != cfg.target
                    || old.fps != cfg.fps
                    || old.quality != cfg.quality
                    || old.capture_cursor != cfg.capture_cursor
                    || old.audio != cfg.audio
                    || old.capture_root != cfg.capture_root
            }
            None => true,
        };
        let live = live_of(&cfg);
        self.cfg = Some(cfg);
        if let Some(p) = &self.pipeline {
            p.set_live(live);
        }
        if self.replay_on && (restart || self.pipeline.is_none()) {
            self.start_pipeline();
        }
    }

    fn start_pipeline(&mut self) {
        self.pipeline = None; // finalize any previous session first
        self.start_error = None;
        let Some(cfg) = self.cfg.clone() else {
            self.start_error = Some("Settings have not been loaded yet.".into());
            return;
        };
        match resolve(&cfg).and_then(|s| Pipeline::start(s, live_of(&cfg))) {
            Ok(p) => {
                self.fps_sample = (Instant::now(), 0);
                self.pipeline = Some(p);
            }
            Err(e) => self.start_error = Some(e),
        }
    }

    fn save(&mut self, re: Option<u64>) {
        let Some(p) = &self.pipeline else {
            let msg = if self.replay_on {
                "Replay is not recording right now, so there is nothing to save."
            } else {
                "Instant replay is off. Turn it on to save clips."
            };
            self.send(re, Event::SaveFailed { save_id: 0, message: msg.into() });
            return;
        };
        let id = self.next_save_id;
        self.next_save_id += 1;
        let emit = self.emit.clone();
        let requested = p.request_save(
            move |r| {
                let ev = match r {
                    Ok((clip, _)) => Event::SaveFinished(clip),
                    Err(message) => Event::SaveFailed { save_id: id, message },
                };
                emit(Message::new(None, ev));
            },
            id,
        );
        self.send(re, Event::SaveStarted { save_id: id, requested_secs: requested });
    }

    /// Periodic housekeeping: retry a replay that is waiting for its target, and move to a
    /// relaunched game after the captured window closed (the old session stays saveable until
    /// the new one starts).
    pub fn tick(&mut self) {
        if !self.replay_on || self.cfg.is_none() {
            return;
        }
        let closed = self.pipeline.as_ref().is_some_and(|p| p.capture.closed.load(Ordering::Relaxed));
        let waiting = self.pipeline.is_none();
        if !(closed || waiting) || self.last_retry.elapsed().as_secs_f64() < 3.0 {
            return;
        }
        self.last_retry = Instant::now();
        let cfg = self.cfg.clone().unwrap();
        if let Ok(s) = resolve(&cfg) {
            self.pipeline = None;
            match Pipeline::start(s, live_of(&cfg)) {
                Ok(p) => {
                    self.start_error = None;
                    self.fps_sample = (Instant::now(), 0);
                    self.pipeline = Some(p);
                }
                Err(e) => self.start_error = Some(e),
            }
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
        let hotkey = self.hotkey.as_ref().map(|h| h.state.lock().unwrap().clone()).unwrap_or_default();
        let root = self.cfg.as_ref().map(|c| PathBuf::from(&c.capture_root));
        let free_bytes = root.as_deref().and_then(storage::free_bytes);
        let leftover_buffer_bytes = root.as_deref().map(|r| self.leftover_bytes(r)).unwrap_or(0);
        let mut st = EngineState {
            status: Status::Ready,
            attention: None,
            target_name: None,
            encoder: None,
            buffered_secs: 0.0,
            buffer_bytes: 0,
            capture_fps: 0.0,
            frames_captured: 0,
            tracks: vec![],
            saves_in_progress: 0,
            hotkey,
            leftover_buffer_bytes,
            free_bytes,
        };
        if !self.replay_on {
            return st;
        }
        let Some(p) = &self.pipeline else {
            st.status = Status::NeedsAttention;
            st.attention = Some(self.start_error.clone().unwrap_or_else(|| "Replay could not start.".into()));
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
        st.target_name = Some(sh.game_name.clone());
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
        st.status = if attention.is_some() {
            Status::NeedsAttention
        } else if frames > 0 && has_segments && sh.frames_encoded.load(Ordering::Relaxed) > 0 {
            Status::Buffering
        } else {
            Status::Starting
        };
        st.attention = attention.or_else(|| {
            (st.status == Status::Starting && sh.now() > 5.0 && frames == 0)
                .then(|| "No frames received yet. The target may be minimized, protected, or using exclusive fullscreen. Try borderless mode or Display capture.".into())
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

fn live_of(cfg: &EngineConfig) -> Live {
    Live { replay_secs: cfg.replay_secs.clamp(5, 3600), cap_bytes: cfg.buffer_cap_bytes, min_free_bytes: cfg.min_free_bytes }
}

/// Turn the configured target into concrete capture settings, verifying it still exists.
fn resolve(cfg: &EngineConfig) -> Result<Settings, String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
    let root = PathBuf::from(&cfg.capture_root);
    if cfg.capture_root.trim().is_empty() {
        return Err("Choose a folder for your captures first.".into());
    }
    let (source, game_name, audio) = match &cfg.target {
        CaptureTarget::None => return Err("Choose a window or display to capture.".into()),
        CaptureTarget::App { exe } => {
            let w = targets::windows()
                .into_iter()
                .filter(|w| w.exe.eq_ignore_ascii_case(exe))
                .max_by_key(|w| w.width as u64 * w.height as u64)
                .ok_or_else(|| format!("Waiting for {exe} to start. Replay begins automatically when it opens."))?;
            let audio = if cfg.audio.game_audio {
                vec![SourceSpec::Process { pid: w.pid, label: "Game Audio".into() }]
            } else {
                vec![]
            };
            (SourceKind::Window(w.hwnd as isize), w.app_name, audio)
        }
        CaptureTarget::Window { hwnd, pid, exe, title } => {
            let h = HWND(*hwnd as *mut _);
            let mut actual = 0u32;
            unsafe { GetWindowThreadProcessId(h, Some(&mut actual)) };
            if !unsafe { IsWindow(Some(h)) }.as_bool() || actual != *pid {
                return Err(format!("\"{title}\" is no longer open. Pick it again from the list."));
            }
            let name = targets::app_name(&targets::process_path(*pid).unwrap_or_default(), exe, title);
            let audio = if cfg.audio.game_audio {
                vec![SourceSpec::Process { pid: *pid, label: "Game Audio".into() }]
            } else {
                vec![]
            };
            (SourceKind::Window(*hwnd as isize), name, audio)
        }
        CaptureTarget::Display { index } => {
            let mons = targets::monitors();
            let m = mons.iter().find(|m| m.target.index == *index).ok_or("That display is not connected.")?;
            let audio = if cfg.audio.game_audio {
                vec![SourceSpec::EndpointLoopback { device_id: None, label: "System Audio".into() }]
            } else {
                vec![]
            };
            (SourceKind::Monitor(m.handle), "Desktop".to_string(), audio)
        }
    };
    Ok(Settings {
        root,
        game_name,
        source,
        fps: cfg.fps.clamp(10, 240),
        quality: cfg.quality,
        cursor: cfg.capture_cursor,
        audio,
    })
}
