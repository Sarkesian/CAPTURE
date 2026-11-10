//! CAPTURE capture engine. Driven by the `capture-worker` binary over the typed protocol in
//! `capture-protocol`; the UI never touches capture/encoding directly.

pub mod audio;
pub mod av;
pub mod clock;
pub mod detect;
pub mod devices;
pub mod gpu;
pub mod hotkey;
pub mod history;
pub mod inject;
pub mod mic;
pub mod mirror;
pub mod mux;
pub mod overlay;
pub mod pipeline;
pub mod probe;
pub mod ring;
pub mod save;
pub mod shot;
pub mod sound;
pub mod stems;
pub mod storage;
pub mod targets;
pub mod trace;
pub mod video;
pub mod wgc;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
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
    history: Option<Arc<history::Catalog>>,
    history_maintenance: Instant,
    cfg: Option<EngineConfig>,
    replay_on: bool,
    pipeline: Option<Pipeline>,
    start_error: Option<Unresolved>,
    hotkey: Option<hotkey::HotkeyService>,
    overlay: Option<Arc<Overlay>>,
    /// The notice now in the notice window and when it appeared: handed over to the in-game hook
    /// if that starts drawing while the notice is still up (never shown in both places).
    window_notice: Mutex<Option<(Notice, std::time::Instant)>>,
    /// CAPTURE's overlay window shown inside the game (see `mirror`), while it is open there.
    mirror: Option<mirror::Mirror>,
    emit: Emit,
    next_save_id: u64,
    screenshots: Arc<std::sync::atomic::AtomicUsize>,
    fps_sample: (Instant, u64),
    capture_fps: f64,
    last_state: Option<EngineState>,
    last_retry: Instant,
    /// Auto mode: a different game seen in the foreground, and since when. Switching replaces
    /// the replay buffer, so it only happens once the new window has stayed for `SWITCH_AFTER`.
    switch_candidate: Option<(u32, Instant)>,
    mic_monitor: Option<mic::Monitor>,
    /// UI views currently showing meters.
    meter_clients: std::collections::HashSet<String>,
    /// Games (by process id) already told that borderless gives a better overlay.
    borderless_tip: std::collections::HashSet<u32>,
    /// The game in exclusive fullscreen CAPTURE has been drawing in, and since when; and the
    /// games already told "Overlay ready".
    ready_since: Option<(u32, Instant)>,
    ready_told: std::collections::HashSet<u32>,
    /// The Replay snapshot of the previous background recording, while one is being watched.
    past_review: Option<PathBuf>,
    /// The previous background recording's folder and game (looked up once per folder).
    past_seen: Option<(PathBuf, String)>,
}

/// How long a different game must stay in front before Auto mode switches to it: a little
/// while for a running game, longer after the recorded game closed so its replay can still be
/// saved (e.g. the launcher returning at the end of a match).
fn switch_after(previous_closed: bool) -> std::time::Duration {
    std::time::Duration::from_secs(if previous_closed { 20 } else { 4 })
}

const OVERLAY_READY: &str = "Overlay ready";

/// A hotkey as shown in notices: `"Alt+F10"`.
fn quoted(combo: &str) -> String {
    format!("\"{combo}\"")
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
    fn publish_history(&self,re:Option<u64>) {
        if let Some(h)=&self.history {
            match h.state(self.cfg.as_ref().map_or(14400,|c|c.history_secs)) {
                Ok(mut state)=>{
                    if let Some(cfg)=&self.cfg {if state.bytes+state.preview_bytes>=cfg.buffer_cap_bytes.saturating_sub(10_000_000) {state.reason=Some("Storage budget limits available history; protected or imported recordings may exceed it".into());}}
                    if !h.import_complete(){state.reason=Some("Recovering earlier recordings…".into());}
                    self.send(re,Event::HistoryReady(state));
                },
                Err(e)=>self.send(re,Event::Error{message:format!("Recording history: {e}")})
            }
        }
    }
    fn history_review(&self,re:Option<u64>,session:String,at:f64) {
        let Some(h)=self.history.clone() else {return;};let emit=self.emit.clone();
        std::thread::spawn(move || {let event=match h.preview(&session,at){Ok(v)=>Event::ReviewReady(v),Err(message)=>Event::ReviewFailed{message}};emit(Message::new(re,event));});
    }
    fn history_save(&mut self,re:Option<u64>,session:String,start:f64,end:f64) {
        let Some(h)=self.history.clone() else {return;};
        let mut lease=match h.lease(&session,start,end,"save") {Ok(l)=>l,Err(message)=>{self.send(re,Event::SaveFailed{save_id:0,message});return;}};
        let id=self.next_save_id;self.next_save_id+=1;
        self.capture_feedback(re,CaptureAction::Replay);
        self.send(re,Event::SaveStarted{save_id:id,requested_secs:end-start});
        let emit=self.emit.clone();let root=PathBuf::from(&self.cfg.as_ref().unwrap().capture_root);
        let stems=self.editor_audio_files();
        std::thread::spawn(move || {
            let until=Instant::now()+std::time::Duration::from_secs(8);
            while Instant::now()<until {
                if h.media(&session,start,end+3.0).ok().and_then(|s|s.last().map(|s|s.end)).is_some_and(|t|t>=end){break;}
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let event=match h.save_clip(&lease,&root,id) {
                Ok(c)=>{if stems{export_stems_after_save(&emit,&c);}Event::SaveFinished(c)},
                Err(message)=>{lease.failed(&message);Event::SaveFailed{save_id:id,message}}
            };
            emit(Message::new(re,event));
        });
    }
    pub fn new(emit: Emit) -> Self {
        Self {
            history: None,
            history_maintenance:Instant::now(),
            cfg: None,
            replay_on: false,
            pipeline: None,
            start_error: None,
            hotkey: None,
            overlay: None,
            window_notice: Mutex::new(None),
            mirror: None,
            emit,
            next_save_id: 1,
            screenshots: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            fps_sample: (Instant::now(), 0),
            capture_fps: 0.0,
            last_state: None,
            last_retry: Instant::now(),
            switch_candidate: None,
            mic_monitor: None,
            meter_clients: Default::default(),
            borderless_tip: Default::default(),
            ready_since: None,
            ready_told: Default::default(),
            past_review: None,
            past_seen: None,
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
        let allowed = self.cfg.as_ref().is_some_and(|c| {
            c.overlay
                && match kind {
                    Kind::Recording => c.notices.recording_started,
                    // Takes the place of the "Recording ..." notice when a game starts.
                    Kind::Saved if title == OVERLAY_READY => c.notices.recording_started,
                    Kind::Saved if title.starts_with("Marker") => c.notices.marker,
                    Kind::Working | Kind::Saved => c.notices.saved,
                    Kind::Failed => c.notices.problems,
                }
        });
        if allowed {
            let n = Notice { title, detail, kind };
            // Inside the game when CAPTURE's hook is drawing there (the only way to be seen over
            // exclusive fullscreen); otherwise the notice window.
            if let Some(p) = &self.pipeline {
                if let Some(h) = p.hook.lock().unwrap().as_ref().filter(|h| h.usable()) {
                    let (w, hh, px) = overlay::render_bitmap(&n, 200);
                    h.show(w, hh, 200, &px, overlay::SHOW_MS, 300);
                    return;
                }
            }
            if let Some(o) = &self.overlay {
                *self.window_notice.lock().unwrap() = Some((n.clone(), std::time::Instant::now()));
                o.show(n);
            }
        }
    }

    /// A notice shown in the window just before the in-game hook started drawing (typically
    /// "Recording …" at the start of a session) moves into the game for the rest of its time:
    /// over exclusive fullscreen the window cannot be seen at all.
    fn hand_over_notice(&self) {
        let mut slot = self.window_notice.lock().unwrap();
        let Some((n, at)) = slot.as_ref() else { return };
        let shown = at.elapsed().as_millis() as u32;
        if shown + 500 >= overlay::SHOW_MS {
            *slot = None;
            return;
        }
        let Some(p) = &self.pipeline else { return };
        let guard = p.hook.lock().unwrap();
        let Some(h) = guard.as_ref().filter(|h| h.usable()) else { return };
        let (w, hh, px) = overlay::render_bitmap(n, 200);
        if let Some(o) = &self.overlay {
            o.hide();
        }
        h.show(w, hh, 200, &px, overlay::SHOW_MS - shown, 300);
        *slot = None;
    }

    /// Short confirmation sound the moment a hotkey action is accepted (uses the same
    /// "confirmation sound" setting, device and volume as markers). Never recorded.
    fn ack_sound(&self, cue: sound::Cue) {
        if let Some(m) = self.cfg.as_ref().map(|c| &c.markers).filter(|m| m.sound) {
            sound::play(cue, m.sound_volume, m.sound_device.clone());
        }
    }

    fn capture_feedback(&self, re: Option<u64>, action: CaptureAction) {
        let (title, cue, kind) = match action {
            CaptureAction::Screenshot => ("Screenshot saved", sound::Cue::Screenshot, Kind::Saved),
            CaptureAction::Replay => ("Saving Clip", sound::Cue::Replay, Kind::Saved),
            CaptureAction::RecordingStart => ("Recording started", sound::Cue::Start, Kind::Recording),
            CaptureAction::RecordingStop => ("Recording saved", sound::Cue::Stop, Kind::Saved),
        };
        let overlay_notice = self.cfg.as_ref().is_some_and(|c| c.overlay && if action == CaptureAction::RecordingStart { c.notices.recording_started } else { c.notices.saved })
            && (self.overlay.is_some() || self.pipeline.as_ref().is_some_and(|p| p.hook.lock().unwrap().as_ref().is_some_and(|h| h.usable())));
        // Publish input acceptance before touching the GPU, encoder or filesystem.
        self.send(re, Event::CaptureTriggered { action, overlay_notice });
        crate::trace::ev("feedback", crate::clock::now_100ns(), action as i64, 0, 0);
        self.ack_sound(cue);
        self.notice(kind, title.into(), String::new());
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
            Command::History => self.publish_history(re),
            Command::HistoryReview { session, at } => self.history_review(re,session,at),
            Command::HistorySave { session,start,end } => self.history_save(re,session,start,end),
            Command::HistoryMarker {session,id,label,remove} => {
                if let Some(h)=&self.history {
                    let result=(||->Result<(),String>{
                        let mut ev=h.events(&session)?.into_iter().find(|e|e.id==id && e.kind=="marker").ok_or("Marker no longer exists")?;
                        if let Some(p)=self.pipeline.as_ref().filter(|p|p.shared.session_id==session) {
                            if let Some(m)=&ev.marker {if remove {p.shared.remove_marker(m.id);} else {p.shared.rename_marker(m.id,label.unwrap_or_default());}return Ok(());}
                        }
                        if remove {ev.kind="removed".into();h.unpin(&session,&id);}else {ev.label=label.unwrap_or_default();if let Some(m)=&mut ev.marker {m.label=ev.label.clone();}}
                        h.event(&session,&ev)
                    })();
                    if let Err(message)=result {self.send(re,Event::Error{message});}
                    self.publish_history(re);
                }
            }
            Command::PrepareReview => {
                let latest=self.history.as_ref().and_then(|h|h.state(self.cfg.as_ref().map_or(14400,|c|c.history_secs)).ok()).and_then(|s|s.sessions.into_iter().rev().find(|s|!s.intervals.is_empty()));
                if let Some(s)=latest {let at=(s.intervals.last().unwrap()[1]-25.0).max(s.intervals.last().unwrap()[0]);self.history_review(re,s.id,at);}
                else {self.send(re,Event::ReviewFailed{message:"Nothing has been recorded yet. Give it a few seconds.".into()});}
            }
            Command::Meters { client, enabled } => {
                if enabled {
                    self.meter_clients.insert(client);
                } else {
                    self.meter_clients.remove(&client);
                }
                return true; // no state echo needed
            }
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
            Command::OverlayOpen => {
                self.mirror = None;
                let (mode, ready_soon, facts) = self.overlay_mode();
                let what = match mode {
                    OverlayMode::InGame => "opening inside the game",
                    OverlayMode::Window => "opening as a window",
                    _ if ready_soon => "not opened, a game in exclusive fullscreen is in front and CAPTURE is not drawing in it yet",
                    _ => "not opened, a game in exclusive fullscreen is in front and CAPTURE cannot draw in it",
                };
                eprintln!("overlay: asked for; {what}; {facts}");
                match mode {
                    OverlayMode::Unavailable => self.overlay_unavailable(ready_soon),
                    OverlayMode::InGame => {
                        // Once per game: the overlay is sharper and handles better as a real
                        // window, which only a borderless game allows.
                        let pid = self.pipeline.as_ref().map_or(0, |p| p.shared.target_pid);
                        if self.borderless_tip.insert(pid) {
                            self.notice(Kind::Working, "Tip: play in borderless window".into(), "The overlay is sharper and smoother there".into());
                        }
                    }
                    _ => {}
                }
                self.send(re, Event::OverlayOpened { mode });
            }
            Command::OverlayMirror { hwnd } => {
                self.mirror = None;
                if let Some(hwnd) = hwnd {
                    let started = match &self.pipeline {
                        Some(p) if p.hook.lock().unwrap().as_ref().is_some_and(|h| h.usable()) => mirror::Mirror::start(hwnd, p.hook.clone()),
                        _ => Err("the game is not drawing through CAPTURE any more".into()),
                    };
                    match started {
                        Ok(m) => {
                            eprintln!("in-game overlay: showing the overlay in the game");
                            self.mirror = Some(m);
                        }
                        Err(e) => {
                            // Never fall back to a window here: the game is fullscreen.
                            eprintln!("in-game overlay: cannot show the overlay in the game: {e}");
                            self.overlay_unavailable(false);
                            self.send(re, Event::OverlayOpened { mode: OverlayMode::Closed });
                        }
                    }
                }
            }
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
                let emit = self.emit.clone();
                std::thread::spawn(move || {
                    let ev = match probe::duration_secs(Path::new(&path)) {
                        Ok(d) => Event::ClipInfo { path, duration_secs: Some(d), error: None },
                        Err(e) => Event::ClipInfo { path, duration_secs: None, error: Some(e) },
                    };
                    emit(Message::new(re, ev));
                });
            }
            Command::ReleaseReview { path } => {
                if let (Some(h),Some(path))=(&self.history,&path) {h.release_preview(path);}
                if self.past_review.as_ref().is_some_and(|f| path.is_none() || path.as_deref() == Some(&*f.to_string_lossy())) {
                    let _ = std::fs::remove_file(self.past_review.take().unwrap());
                }
                if let Some(p) = &self.pipeline {
                    if path.is_none() || path == p.shared.review_path() {
                        p.shared.release_review();
                    }
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
                let Some(h)=self.history.clone() else {return true;};
                let Some(cfg)=&self.cfg else {return true;};let root=PathBuf::from(&cfg.capture_root);
                let current=self.pipeline.as_ref().map(|p|p.shared.session_id.clone());
                let selections=(||->Result<Vec<history::Lease>,String>{
                    let state=h.state(cfg.history_secs)?;let mut leases=vec![];
                    for session in state.sessions.into_iter().filter(|s|Some(&s.id)!=current.as_ref()) {
                        for [a,b] in session.intervals {leases.push(h.lease(&session.id,a,b,"save")?);}
                    }
                    Ok(leases)
                })();
                match selections {
                    Err(message)=>self.send(re,Event::Error{message}),
                    Ok(leases)=>{
                        self.capture_feedback(re,CaptureAction::Replay);
                        let emit=self.emit.clone();std::thread::spawn(move||{
                            let mut clips=vec![];let mut failed=vec![];
                            for mut lease in leases {match h.save_clip(&lease,&root,0){Ok(c)=>clips.push(c.path),Err(e)=>{lease.failed(&e);failed.push(e);}}}
                            emit(Message::new(re,Event::Recovered{clips,failed}));
                        });
                    }
                }
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
        if self.cfg.as_ref().is_none_or(|old| old.capture_root != cfg.capture_root) && !cfg.capture_root.trim().is_empty() {
            // Older versions kept `<root>\Clips\<Game>`; move those into `<root>\<Game>\Clips`.
            storage::migrate_layout(Path::new(&cfg.capture_root));
        }
        // Everything except the target that needs a new pipeline.
        let restart_other = match &self.cfg {
            Some(old) => {
                old.fps != cfg.fps
                    || old.quality != cfg.quality
                    || old.encoder != cfg.encoder
                    || old.capture_cursor != cfg.capture_cursor
                    || !old.audio.same_sources(&cfg.audio)
                    || old.capture_root != cfg.capture_root
                    || old.buffer_root != cfg.buffer_root
                    || self.pipeline.as_ref().is_some_and(|p| {
                        let (a, b) = (old.for_game(&p.shared.target_exe), cfg.for_game(&p.shared.target_exe));
                        a.quality != b.quality || a.audio.game_audio != b.audio.game_audio
                    })
            }
            None => true,
        };
        let restart = restart_other || self.cfg.as_ref().is_some_and(|old| old.target != cfg.target);
        // In Auto mode, editing the games/ignore lists must not interrupt a running recording
        // unless the current game was just ignored. Any other change that needs a restart
        // (encoder, buffer folder, per-game quality...) still restarts.
        let auto_lists_only = !restart_other
            && matches!((&self.cfg, &cfg), (Some(old), new)
                if matches!(old.target, CaptureTarget::Auto { .. }) && matches!(new.target, CaptureTarget::Auto { .. }));
        let live = live_of(&cfg);
        let now_ignored = match (&cfg.target, &self.pipeline) {
            (CaptureTarget::Auto { ignore, .. }, Some(p)) => ignore.iter().any(|i| i.eq_ignore_ascii_case(&p.shared.target_exe)),
            _ => false,
        };
        if cfg.markers.sound { sound::warm_output(cfg.markers.sound_device.clone()); }
        self.cfg = Some(cfg);
        let cfg=self.cfg.as_ref().unwrap();
        let root=storage::buffer_dir_for(Path::new(&cfg.capture_root),cfg.buffer_root.as_deref());
        match history::Catalog::open(&root) {
            Ok(h)=>{if self.history.as_ref().is_none_or(|old|!Arc::ptr_eq(old,&h)){let importer=h.clone();std::thread::spawn(move || { for _ in 0..100000 {if let Err(e)=importer.import_step(){eprintln!("history recovery: {e}");importer.import_failed(&e);break;} if importer.import_complete(){break;}std::thread::sleep(std::time::Duration::from_millis(10));} });}self.history=Some(h);},
            Err(e)=>{self.start_error=Some(failed(format!("Could not open recording history: {e}")));return;}
        }
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
        self.mirror = None;
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
                // Global history survives every successful session switch.
                let save = if cfg.save_hotkey.is_empty() { "Save clip in CAPTURE".to_string() } else { quoted(&cfg.save_hotkey) };
                let title = if display { "Recording your display".to_string() } else { format!("Recording {name}") };
                let mic = match (cfg.audio.mic_enabled, cfg.audio.mic_muted) {
                    (true, false) => " · mic ON",
                    (true, true) => " · mic muted",
                    _ => "",
                };
                let detail = format!("{save} saves the last {}{mic}", fmt_secs(live_of(cfg).replay_secs as f64));
                if cfg.hotkeys.overlay.is_empty() {
                    self.notice(Kind::Recording, title, detail);
                } else if !targets::d3d_fullscreen_running() {
                    // With an overlay key, the notice at the start of a game is the reminder of
                    // that key (everything else is in the overlay). Over a game in exclusive
                    // fullscreen the overlay is not ready yet, and saying so now would be untrue
                    // and unseen: `announce_overlay_ready` says it once CAPTURE draws in the game.
                    self.notice(Kind::Saved, OVERLAY_READY.into(), format!("Press {} to open it", quoted(&cfg.hotkeys.overlay)));
                }
            }
            Err(e) => self.start_error = Some(failed(e)),
        }
    }

    fn start_session(&mut self, re: Option<u64>) {
        let Some(p) = self.pipeline.as_ref().filter(|p| !p.target_gone()) else {
            let msg = if self.replay_on { "Nothing is being captured right now." } else { "Turn on recording first." };
            self.notice(Kind::Failed, "Recording not started".into(), msg.into());
            self.send(re, Event::Error { message: format!("Can't start a recording: {msg}") });
            return;
        };
        if p.start_session() {
            self.capture_feedback(re, CaptureAction::RecordingStart);
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
        let overlay = self.overlay.clone().filter(|_| self.cfg.as_ref().is_some_and(|c| c.overlay && c.notices.problems));
        let hook = p.hook.clone();
        let stems = self.editor_audio_files();
        self.capture_feedback(re, CaptureAction::RecordingStop);
        let requested = p.stop_session(
            move |r| {
                // Input was acknowledged already; completion only updates the library.
                let ev = match r {
                    Ok((clip, _)) => {
                        if stems {
                            export_stems_after_save(&emit, &clip);
                        }
                        Event::SaveFinished(clip)
                    }
                    Err(message) => {
                        route_notice(overlay.as_ref(), &hook, Notice { kind: Kind::Failed, title: "Session not saved".into(), detail: message.clone() });
                        Event::SaveFailed { save_id: id, message }
                    }
                };
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
            self.notice(Kind::Failed, "Screenshot not taken".into(), "Nothing is being captured right now.".into());
            self.send(re, Event::Error { message: "Nothing is being captured right now, so there is nothing to screenshot.".into() });
            return;
        };
        if self.screenshots.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < 2).then_some(n + 1)).is_err() {
            self.notice(Kind::Failed, "Screenshot not taken".into(), "Two screenshots are still processing. Try again in a moment.".into());
            self.send(re, Event::Error { message: "Two screenshots are still processing. Try again in a moment.".into() });
            return;
        }
        let (gpu, capture, shared) = (p.gpu.clone(), p.capture.clone(), p.shared.clone());
        let at = shared.now();
        let emit = self.emit.clone();
        let pending = self.screenshots.clone();
        let hook = p.hook.clone();
        let overlay = self.overlay.clone().filter(|_| self.cfg.as_ref().is_some_and(|c| c.overlay && c.notices.problems));
        self.capture_feedback(re, CaptureAction::Screenshot);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| shot::save(&gpu, &capture, &shared.root, &shared.game_name)))
                .unwrap_or_else(|_| Err("Screenshot processing failed.".into()));
            match result {
                Ok((path,frame_qpc)) => {
                    let at=if frame_qpc>0 {frame_qpc.saturating_sub(shared.t0) as f64/1e7} else {at};
                    if let Err(e)=shared.history.event(&shared.session_id,&TimelineEvent{id:format!("shot:{}",path.display()),kind:"screenshot".into(),t:at,end:None,label:path.file_name().unwrap_or_default().to_string_lossy().into_owned(),path:Some(path.to_string_lossy().into_owned()),marker:None}) {emit(Message::new(re,Event::Error{message:format!("Screenshot saved, but timeline position could not be preserved: {e}")}));}
                    shared.note_saved(SavedKind::Screenshot, at, None, &path);
                    emit(Message::new(re, Event::ScreenshotSaved { path: path.to_string_lossy().into_owned() }));
                }
                Err(message) => {
                    if overlay.is_some() { route_notice(overlay.as_ref(), &hook, Notice { kind: Kind::Failed, title: "Screenshot not saved".into(), detail: message.clone() }); }
                    emit(Message::new(re, Event::Error { message: format!("Screenshot not saved: {message}") }));
                }
            }
            pending.fetch_sub(1, Ordering::SeqCst);
        });
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
            sound::play_chime(mcfg.sound_volume, mcfg.sound_device.clone());
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
        if self.pipeline.is_none() {
            if let Some((start,end))=range {
                let session=self.history.as_ref().and_then(|h|h.state(14400).ok()).and_then(|s|s.sessions.into_iter().rev().find(|s|!s.intervals.is_empty()));
                if let Some(session)=session {self.history_save(re,session.id,start,end);return;}
            }
        }
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
        let problems = self.cfg.as_ref().is_some_and(|c| c.overlay && c.notices.problems);
        let overlay = self.overlay.clone();
        let hook = p.hook.clone();
        let stems = self.editor_audio_files();
        let requested = p.request_save(
            range,
            SaveKind::Clip,
            || self.capture_feedback(re,CaptureAction::Replay),
            move |r| {
                // Input was acknowledged already; completion only updates the library.
                let ev = match r {
                    Ok((clip, _)) => {
                        if stems {
                            export_stems_after_save(&emit, &clip);
                        }
                        Event::SaveFinished(clip)
                    }
                    Err(message) => {
                        if problems {
                            route_notice(overlay.as_ref(), &hook, Notice { kind: Kind::Failed, title: "Clip not saved".into(), detail: message.clone() });
                        }
                        Event::SaveFailed { save_id: id, message }
                    }
                };
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
        self.hand_over_notice();
        if self.history_maintenance.elapsed().as_secs()>=2 {
            self.history_maintenance=Instant::now();
            if let (Some(h),Some(c))=(&self.history,&self.cfg) {
                if let Err(e)=h.refresh_files().and_then(|_|h.retain(c.history_secs,c.buffer_cap_bytes,c.min_free_bytes).map(|_|())) {eprintln!("history maintenance: {e}");}
            }
        }
        if self.pipeline.as_ref().is_some_and(|p|p.capture.resolution_changed.load(Ordering::Relaxed)) {
            let recording=self.pipeline.as_ref().is_some_and(|p|p.shared.session_start.lock().unwrap().is_some());
            self.start_pipeline();
            if recording {if let Some(p)=&self.pipeline {p.start_session();}}
        }
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
            if same {
                self.switch_candidate = None;
                return;
            }
            // Something is (or was just) being recorded: a launcher or another app flashing
            // to the front for a moment must not throw that buffer away. Switch only to a
            // window that stays (the user's own games list counts at once).
            if self.pipeline.is_some() && reason != detect::Reason::UserList {
                let since = match self.switch_candidate {
                    Some((pid, t)) if pid == w.pid => t,
                    _ => {
                        self.switch_candidate = Some((w.pid, Instant::now()));
                        return;
                    }
                };
                if since.elapsed() < switch_after(closed) {
                    return;
                }
            }
            self.switch_candidate = None;
            {
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

    /// Called ~30 times per second by the worker: sends real levels only while a visible UI
    /// view shows meters, so a hidden UI costs nothing.
    pub fn meter_tick(&mut self) {
        if self.meter_clients.is_empty() {
            return;
        }
        let tracks = self.pipeline.as_ref().and_then(|p| p.audio.as_ref()).map(|a| a.take_levels()).unwrap_or_default();
        let mic_monitor = self.mic_monitor.as_ref().map(|m| m.take_level());
        if tracks.is_empty() && mic_monitor.is_none() {
            return;
        }
        self.send(None, Event::Levels { tracks, mic_monitor });
    }

    /// Called periodically and after every command. Emits only when something changed,
    /// unless `force`.
    pub fn publish_state(&mut self, re: Option<u64>, force: bool) {
        // An overlay shown inside a game that closed or stopped drawing (minimized, alt-tabbed
        // away) is closed, so the next press of the hotkey opens it again instead of "closing"
        // something nobody can see.
        if self.mirror.is_some() && !self.pipeline.as_ref().is_some_and(|p| p.hook.lock().unwrap().as_ref().is_some_and(|h| h.usable())) {
            eprintln!("in-game overlay: closed, the game is no longer drawing");
            self.mirror = None;
            self.send(None, Event::OverlayOpened { mode: OverlayMode::Closed });
        }
        self.announce_overlay_ready();
        if self.mirror.as_ref().is_some_and(|m| m.blind()) {
            eprintln!("in-game overlay: closed, Windows gives no picture of the overlay window while this game is in front");
            self.mirror = None;
            self.overlay_unavailable(false);
            self.send(None, Event::OverlayOpened { mode: OverlayMode::Closed });
        }
        let st = self.state();
        if force || self.last_state.as_ref() != Some(&st) {
            self.last_state = Some(st.clone());
            self.send(re, Event::State(st));
        }
    }

    /// In exclusive fullscreen the overlay only opens once CAPTURE draws inside the game, a little
    /// while after the game starts. Say when that is, once per game, with the key that opens it.
    fn announce_overlay_ready(&mut self) {
        let drawing = self.pipeline.as_ref().and_then(|p| p.hook.lock().unwrap().as_ref().is_some_and(|h| h.exclusive()).then_some(p.shared.target_pid));
        let Some(pid) = drawing else { return };
        match self.ready_since {
            Some((p, since)) if p == pid => {
                if since.elapsed().as_millis() >= 1000 && self.mirror.is_none() && self.ready_told.insert(pid) {
                    let key = self.cfg.as_ref().map(|c| c.hotkeys.overlay.clone()).unwrap_or_default();
                    if !key.is_empty() {
                        self.notice(Kind::Saved, OVERLAY_READY.into(), format!("Press {} to open it", quoted(&key)));
                    }
                }
            }
            _ => self.ready_since = Some((pid, Instant::now())),
        }
    }

    /// The overlay cannot open over the game in front: say so with a notice and a sound (over
    /// exclusive fullscreen, before CAPTURE draws in the game, the sound is all that gets through).
    fn overlay_unavailable(&self, ready_soon: bool) {
        if let Some(m) = self.cfg.as_ref().map(|c| &c.markers).filter(|m| m.sound) {
            sound::play_refusal(m.sound_volume, m.sound_device.clone());
        }
        if ready_soon {
            self.notice(Kind::Failed, "Overlay not ready yet".into(), "Try again in a few seconds".into());
        } else {
            self.notice(Kind::Failed, "No overlay in exclusive fullscreen here".into(), "Set the game to borderless window to use it".into());
        }
    }

    /// Where the overlay can open right now, judged from the window in front at this moment
    /// (not from the last published state, which can be seconds old or about another window,
    /// e.g. a launcher that has just started the game); whether an unavailable overlay will be
    /// available shortly; and the facts behind it for the log.
    ///
    /// - Nothing fullscreen in front, or a borderless game: a real window. That is the best
    ///   overlay (native sharpness, the real mouse pointer), and such games stay up when another
    ///   window takes focus.
    /// - A game in exclusive fullscreen: it minimizes as soon as another window takes focus, so
    ///   the overlay is drawn inside the game by CAPTURE's hook, or not shown at all.
    fn overlay_mode(&self) -> (OverlayMode, bool, String) {
        use capture_protocol::hook::{api, state};
        let (target_pid, target_hwnd, usable, hook_exclusive, cannot_draw, hook_facts) = match &self.pipeline {
            Some(p) => {
                let g = p.hook.lock().unwrap();
                match g.as_ref() {
                    Some(h) => {
                        let hd = h.header();
                        let (st, ap) = (hd.hook_state.load(Ordering::Relaxed), hd.api.load(Ordering::Relaxed));
                        // Loaded, but it failed, or the game draws with something it cannot draw with.
                        let cannot = st == state::FAILED || (st == state::PRESENTING && ap == api::OTHER_DXGI);
                        (p.shared.target_pid, p.shared.target_hwnd, h.usable(), h.exclusive(), cannot, h.facts())
                    }
                    None => (p.shared.target_pid, p.shared.target_hwnd, false, false, false, "no hook in the captured game".to_string()),
                }
            }
            None => (0, 0, false, false, false, "nothing is being captured".to_string()),
        };
        let mut front = targets::foreground();
        if front == 0 {
            front = target_hwnd; // between two windows: judge by the captured game
        }
        let d3d = targets::d3d_fullscreen_running();
        let Some(pl) = targets::placement(front) else {
            // Nothing can be told about what is in front: only open a window when Windows does
            // not report an exclusive-fullscreen application either.
            let mode = if d3d { OverlayMode::Unavailable } else { OverlayMode::Window };
            return (mode, true, format!("no window in front; exclusive fullscreen running: {d3d}; {hook_facts}"));
        };
        let own = pl.pid == std::process::id() || targets::process_path(pl.pid).is_some_and(|p| p.to_ascii_lowercase().ends_with("\\capture.exe"));
        let game_in_front = target_pid != 0 && pl.pid == target_pid;
        let exclusive = !own && (d3d || (game_in_front && hook_exclusive));
        let mode = match (exclusive, game_in_front && usable) {
            (false, _) => OverlayMode::Window,
            (true, true) => OverlayMode::InGame,
            (true, false) => OverlayMode::Unavailable,
        };
        // Not yet: CAPTURE is still on another window (a launcher that started this game) or its
        // hook has not seen a frame yet. Not at all: the hook is in this game and cannot draw.
        let ready_soon = !(game_in_front && cannot_draw);
        (mode, ready_soon, format!("in front: {}; fills its screen: {}; captured game pid {target_pid} in front: {game_in_front}; exclusive fullscreen running: {d3d}; {hook_facts}", pl.facts, pl.fullscreen))
    }

    /// (CAPTURE's hook draws in the captured game and it is fullscreen, the game is fullscreen),
    /// for the published state. Logged when it changes, with the reasons.
    fn in_game(&mut self) -> (bool, bool) {
        let (hook, exclusive, placement) = match &self.pipeline {
            Some(p) => {
                let g = p.hook.lock().unwrap();
                (g.as_ref().is_some_and(|h| h.usable()), g.as_ref().is_some_and(|h| h.exclusive()), targets::placement(p.shared.target_hwnd))
            }
            None => (false, false, None),
        };
        let fullscreen = exclusive || placement.as_ref().is_some_and(|p| p.fullscreen);
        let now = hook && fullscreen;
        if self.last_state.as_ref().is_some_and(|s| s.in_game != now) {
            eprintln!(
                "in-game overlay: {} (hook drawing: {hook}, exclusive fullscreen: {exclusive}, {})",
                if now { "game is fullscreen with CAPTURE drawing in it" } else { "not drawing in a fullscreen game" },
                placement.map_or("no game window".to_string(), |p| p.facts)
            );
        }
        (now, fullscreen)
    }

    fn state(&mut self) -> EngineState {
        let (in_game, game_fullscreen) = self.in_game();
        let hotkeys = self.hotkey.as_ref().map(|h| h.state.lock().unwrap().clone()).unwrap_or_default();
        let hotkey = self.hotkey.as_ref().map(|h| h.state_for(HotkeyAction::SaveReplay)).unwrap_or_default();
        let root = self.cfg.as_ref().map(|c| PathBuf::from(&c.capture_root));
        let free_bytes = root.as_deref().and_then(storage::free_bytes);
        let leftover_buffer_bytes = root.as_deref().map(|r| self.leftover_bytes(r)).unwrap_or(0);
        let mut st = EngineState {
            history_buffered_secs:0.0,
            frames_encoded:0,frames_without_source:0,frames_dropped:0,frames_repeated:0,source_frames_unselected:0,
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
            in_game,
            game_fullscreen,
            saved: vec![],
            past_game: if self.pipeline.is_none() { self.past_recording().map(|(_, game)| game) } else { None },
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
            (ring.available_secs(sh.save_end()).min(live.replay_secs as f64), ring.total_bytes(), !ring.segments().is_empty())
        };
        let attention = sh.attention.lock().unwrap().clone()
            .or_else(|| p.capture.latest.lock().unwrap().error.clone());
        st.markers = sh.markers_now();
        st.session_secs = sh.session_start.lock().unwrap().map(|s| ((sh.now() - s) * 10.0).round() / 10.0);
        // Where the recording has got to: it stops when the game closes, while the footage stays
        // (the Replay timeline ends there instead of running on into nothing).
        st.now_secs = (sh.save_end() * 10.0).round() / 10.0;
        st.saved = sh.saved.lock().unwrap().clone();
        st.target_name = Some(sh.game_name.clone());
        st.target_exe = Some(sh.target_exe.clone());
        st.detected_by = sh.detected_by.clone();
        st.encoder = Some(p.encoder.clone());
        st.buffered_secs = (buffered * 10.0).round() / 10.0;
        st.buffer_bytes = bytes;
        st.history_buffered_secs=sh.ring.lock().unwrap().segments().iter().filter_map(|s|s.end.map(|e|(e-s.start).max(0.0))).sum();
        st.capture_fps = (self.capture_fps * 10.0).round() / 10.0;
        st.frames_captured = frames;
        st.frames_encoded=sh.frames_encoded.load(Ordering::Relaxed);
        st.frames_without_source=sh.frames_without_source.load(Ordering::Relaxed);
        st.frames_dropped=sh.frames_dropped.load(Ordering::Relaxed);
        st.frames_repeated=sh.frames_repeated.load(Ordering::Relaxed);
        st.source_frames_unselected=sh.source_frames_unselected.load(Ordering::Relaxed);
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
            if st.frames_dropped>0 {return Some(format!("{} recording frames missed their deadline. Resolution, FPS and quality have not been reduced. Consider lowering game GPU load or explicitly choosing other recording settings.",st.frames_dropped));}
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
        let buffer_root = self.cfg.as_ref().and_then(|c| c.buffer_root.clone());
        std::fs::read_dir(storage::buffer_dir_for(root, buffer_root.as_deref()))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| storage::is_session_dir_name(&e.file_name().to_string_lossy()))
            .map(|e| e.path())
            .filter(|p| p.is_dir() && Some(p) != current.as_ref())
            .collect()
    }

    /// The newest background recording still on disk while nothing is being recorded: its
    /// folder and game.
    fn past_recording(&mut self) -> Option<(PathBuf, String)> {
        if self.pipeline.is_some() {
            return None;
        }
        let root = PathBuf::from(&self.cfg.as_ref()?.capture_root);
        let mut dirs = self.leftover_dirs(&root);
        dirs.sort();
        let dir = dirs.pop()?;
        if self.past_seen.as_ref().is_none_or(|(d, _)| *d != dir) {
            let game = std::fs::read(dir.join("session.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|m| m["game"].as_str().map(String::from))
                .unwrap_or_else(|| "your last game".into());
            self.past_seen = Some((dir, game));
        }
        self.past_seen.clone()
    }

    /// A new game is being recorded: the recordings kept from earlier games (those that ended
    /// normally) go. What a crash left behind stays until the player saves or deletes it.
    fn leftover_bytes(&self, root: &Path) -> u64 {
        self.leftover_dirs(root).iter().map(|d| storage::dir_size(d)).sum()
    }

    /// Deletes only `Background Recording\<old session>` folders — never clips, never the live session.
    fn clear_leftovers(&mut self) -> u64 {
        // Explicit cleanup uses the same leases as automatic cleanup; legacy media is kept.
        let Some(h)=&self.history else{return 0;};
        let before=h.state(0).map(|s|s.bytes).unwrap_or(0);
        let _=h.retain(0,0,0);
        before.saturating_sub(h.state(0).map(|s|s.bytes).unwrap_or(before))
    }

    pub fn shutdown(&mut self) {
        self.stop_session(None);
        history::wait_for_saves(std::time::Duration::from_secs(14*60));
        let shots_until = Instant::now() + std::time::Duration::from_secs(30);
        while self.screenshots.load(Ordering::SeqCst) > 0 && Instant::now() < shots_until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if let Some(p) = &self.pipeline {
            // Let in-flight saves finish before tearing down the session. A full-session save
            // of a long recording can take minutes; the bound only guards against a hang.
            let until = Instant::now() + std::time::Duration::from_secs(14 * 60);
            while p.shared.saves_in_progress.load(Ordering::SeqCst) > 0 && Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        self.pipeline = None;
        self.hotkey = None;
    }
}

impl Engine {
    fn editor_audio_files(&self) -> bool {
        self.cfg.as_ref().is_some_and(|c| c.editor_audio_files)
    }
}

/// "Create separate audio files for editors": after a verified save, write the per-track WAVs
/// on a background thread (never delays the save or the next one). Only for clips that have
/// separate tracks; a clip with just Preview Mix gains nothing from a WAV copy.
fn export_stems_after_save(emit: &Emit, clip: &SavedClip) {
    if clip.audio_tracks.len() < 2 {
        return;
    }
    let (emit, path) = (emit.clone(), clip.path.clone());
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
        emit(Message::new(None, ev));
    });
}

/// Hotkey bindings handled by the worker (every action has a working implementation).
pub fn bindings(cfg: &EngineConfig) -> hotkey::Bindings {
    vec![
        (HotkeyAction::SaveReplay, cfg.save_hotkey.clone()),
        (HotkeyAction::AddMarker, cfg.hotkeys.marker.clone()),
        (HotkeyAction::ToggleOverlay, cfg.hotkeys.overlay.clone()),
        (HotkeyAction::ToggleSession, cfg.hotkeys.session.clone()),
        (HotkeyAction::Screenshot, cfg.hotkeys.screenshot.clone()),
        (HotkeyAction::ToggleMicMute, cfg.hotkeys.mic_mute.clone()),
    ]
}

/// The footage files of a kept session folder, in order, with the session time of the first
/// picture and the audio track names.
fn live_of(cfg: &EngineConfig) -> Live {
    Live { replay_secs: cfg.replay_secs.clamp(5, 3600), history_secs:cfg.history_secs.clamp(60,21600), cap_bytes: cfg.buffer_cap_bytes, min_free_bytes: cfg.min_free_bytes }
}

fn base_settings(cfg: &EngineConfig) -> Result<PathBuf, Unresolved> {
    if cfg.capture_root.trim().is_empty() {
        return Err(failed("Choose a folder for your captures first."));
    }
    Ok(PathBuf::from(&cfg.capture_root))
}

/// Audio sources for a session. `game = Some((pid, exe))` for a game/app window, `None` for
/// whole-display capture (where "System Audio" already contains everything, so voice chat is
/// kept as its own track but not added to the mix again, and All Desktop Audio would be identical).
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
        v.push(Source { spec: SourceSpec::AllExceptCapture, role: Role::Other, label: "All Desktop Audio".into(), in_mix: false });
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
        root: root.clone(),
        buffer: storage::buffer_dir_for(&root, cfg.buffer_root.as_deref()),
        game_name: w.app_name.clone(),
        exe: w.exe.clone(),
        pid: w.pid,
        detected_by: detected_by.map(str::to_string),
        source: SourceKind::Window(w.hwnd as isize),
        fps: cfg.fps.clamp(10, 240),
        quality: cfg.quality,
        software_encoder: cfg.encoder == EncoderChoice::Software,
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
                root: root.clone(),
                buffer: storage::buffer_dir_for(&root, cfg.buffer_root.as_deref()),
                game_name: "Desktop".into(),
                exe: String::new(),
                pid: 0,
                detected_by: None,
                source: SourceKind::Monitor(m.handle),
                fps: cfg.fps.clamp(10, 240),
                quality: cfg.quality,
                software_encoder: cfg.encoder == EncoderChoice::Software,
                cursor: cfg.capture_cursor,
                audio,
                mix: mix_of(cfg),
            })
        }
    }
}


/// Show `n` inside the game when CAPTURE's hook is drawing there, otherwise in the notice
/// window (one or the other, never both).
fn route_notice(overlay: Option<&Arc<Overlay>>, hook: &Arc<Mutex<Option<inject::GameHook>>>, n: Notice) {
    if let Some(h) = hook.lock().unwrap().as_ref().filter(|h| h.usable()) {
        let (w, hh, px) = overlay::render_bitmap(&n, 200);
        h.show(w, hh, 200, &px, overlay::SHOW_MS, 300);
        return;
    }
    if let Some(o) = overlay {
        o.show(n);
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
