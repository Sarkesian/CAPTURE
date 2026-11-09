//! IPC contract between the CAPTURE UI process and the `capture-worker` process.
//!
//! Transport: one JSON object per line. The UI writes [`Request`]s to the worker's stdin; the
//! worker writes [`Message`]s to stdout. Every line carries [`PROTOCOL_VERSION`]; a peer that
//! receives a different major version must refuse to operate rather than guess.

use serde::{Deserialize, Serialize};

pub mod hook;

pub const PROTOCOL_VERSION: u32 = 1;

/// Envelope for UI → worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub v: u32,
    pub id: u64,
    #[serde(flatten)]
    pub cmd: Command,
}

/// Envelope for worker → UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub v: u32,
    /// Echoes the request id this message answers, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub re: Option<u64>,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", content = "args", rename_all = "snake_case")]
pub enum Command {
    /// Replace the full engine configuration. Restarts the pipeline if the replay is running
    /// and a capture-affecting field changed.
    Configure(EngineConfig),
    SetReplay { enabled: bool },
    SaveReplay,
    /// Bookmark the current moment of the running session.
    AddMarker { #[serde(default)] label: Option<String> },
    RenameMarker { id: u64, label: String },
    RemoveMarker { id: u64 },
    /// Make the buffered footage playable for the review timeline (stream copy into a
    /// temporary MP4). Footage used by the review is protected until released.
    PrepareReview,
    ReleaseReview,
    /// Save a clip of an exact timeline range (seconds), e.g. from the review timeline.
    SaveRange { start: f64, end: f64 },
    /// Full-session recording, independent of the replay length (uses the same capture).
    StartSession,
    StopSession,
    /// Live level meters: a UI view (`client`) that shows meters subscribes while visible.
    /// While any client is subscribed the worker sends `Levels` ~30 times per second.
    Meters { client: String, enabled: bool },
    /// Microphones, outputs and apps that currently play audio (answered with `AudioDevices`).
    ListAudio,
    /// Live level of a microphone while settings are open (nothing is recorded).
    MicMonitor { enabled: bool, #[serde(default)] device: Option<String> },
    /// Record ~3 s from the microphone, then play it back on the default output.
    MicTest { #[serde(default)] device: Option<String> },
    /// Save the current captured frame as PNG in <Game>\Screenshots\.
    Screenshot,
    /// "Editor audio files": synchronized WAV per isolated track next to the clip.
    ExportStems { path: String },
    /// Duration of a saved clip (answered with `ClipInfo`).
    ClipInfo { path: String },
    ListTargets,
    GetState,
    /// Delete leftover temporary buffer footage that is not part of the running session.
    ClearTempCache,
    /// Save leftover buffer footage (from a crash or power loss) as clips, then remove it.
    RecoverLeftovers,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum Event {
    Hello { worker_version: String },
    State(EngineState),
    Targets { windows: Vec<WindowTarget>, displays: Vec<DisplayTarget> },
    SaveStarted { save_id: u64, requested_secs: f64 },
    SaveFinished(SavedClip),
    SaveFailed { save_id: u64, message: String },
    /// A command could not be executed. `message` is plain language for the user.
    Error { message: String },
    /// A global hotkey whose action belongs to the UI (e.g. opening the overlay).
    HotkeyPressed { action: HotkeyAction },
    MarkerAdded(Marker),
    ReviewReady(Review),
    ReviewFailed { message: String },
    ClipInfo { path: String, duration_secs: Option<f64>, error: Option<String> },
    AudioDevices { inputs: Vec<AudioDevice>, outputs: Vec<AudioDevice>, apps: Vec<AudioApp> },
    /// Real audio levels since the previous `Levels` message (peak and RMS, linear 0..1).
    Levels { tracks: Vec<Level>, mic_monitor: Option<Level> },
    StemsDone { path: String, folder: Option<String>, files: Vec<String>, error: Option<String> },
    ScreenshotSaved { path: String },
    /// Result of RecoverLeftovers: saved clip paths and any folders that could not be recovered.
    Recovered { clips: Vec<String>, failed: Vec<String> },
    /// Mic test progress: phase "recording" | "playing" | "done" | "failed".
    MicTest { phase: String, peak: f32, message: Option<String> },
}

/// A playable snapshot of the buffer. `start`/`end` are session-timeline seconds of its first
/// and last frame (marker `t` values map to `t - start` in the file).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Review {
    pub path: String,
    pub start: f64,
    pub end: f64,
    pub game: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    SaveReplay,
    AddMarker,
    ToggleOverlay,
    ToggleSession,
    Screenshot,
    /// Handled by the UI process (it owns settings): flips microphone mute.
    ToggleMicMute,
}

/// Additional global hotkeys (the save hotkey stays in `EngineConfig::save_hotkey`).
/// Empty string = not set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Hotkeys {
    pub marker: String,
    pub overlay: String,
    pub session: String,
    pub screenshot: String,
    pub mic_mute: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        // Avoids NVIDIA's overlay defaults (Alt+Z, Alt+F9).
        Self { marker: "Alt+F8".into(), overlay: "Alt+F11".into(), session: String::new(), screenshot: "Alt+F12".into(), mic_mute: String::new() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Notices {
    pub recording_started: bool,
    pub saved: bool,
    pub marker: bool,
    /// Failures: clip not saved, mic problems, low disk, etc.
    pub problems: bool,
}

impl Default for Notices {
    fn default() -> Self {
        Self { recording_started: true, saved: true, marker: true, problems: true }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EncoderChoice {
    #[default]
    Auto,
    /// Microsoft's software H.264 encoder: works on any PC, uses the CPU.
    Software,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct GameRule {
    pub exe: String,
    pub replay_secs: Option<u32>,
    pub quality: Option<Quality>,
    pub game_audio: Option<bool>,
}

impl EngineConfig {
    /// This configuration with the rule for `exe` (if any) applied.
    pub fn for_game(&self, exe: &str) -> EngineConfig {
        let mut c = self.clone();
        if let Some(r) = self.per_game.iter().find(|r| r.exe.eq_ignore_ascii_case(exe)) {
            if let Some(s) = r.replay_secs {
                c.replay_secs = s;
            }
            if let Some(q) = r.quality {
                c.quality = q;
            }
            if let Some(a) = r.game_audio {
                c.audio.game_audio = a;
            }
        }
        c
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MarkerConfig {
    /// Short confirmation sound when a marker is added (never recorded, see docs).
    pub sound: bool,
    /// 0.0..=1.0
    pub sound_volume: f32,
    /// Output device for the chime (endpoint ID); `None` = Windows default output.
    pub sound_device: Option<String>,
    /// Keep footage around each marker from being overwritten while the session runs.
    pub protect: bool,
    pub protect_before_secs: u32,
    pub protect_after_secs: u32,
}

impl Default for MarkerConfig {
    fn default() -> Self {
        Self { sound: true, sound_volume: 0.35, sound_device: None, protect: true, protect_before_secs: 30, protect_after_secs: 15 }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkerFootage {
    /// Footage exists in the temporary buffer and will be overwritten eventually.
    Buffered,
    /// Footage around the marker is held back from overwriting (still temporary).
    Protected,
    /// The footage is no longer in the buffer.
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Marker {
    pub id: u64,
    /// Seconds on the session timeline.
    pub t: f64,
    /// Local wall-clock time, "HH:MM:SS".
    pub at: String,
    pub label: String,
    pub footage: MarkerFootage,
    /// Why protection is not active although enabled (e.g. storage limit), if so.
    pub note: Option<String>,
    /// Saved clips that contain this moment.
    pub saved_in: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineConfig {
    /// Root folder that holds `Buffer` and one folder per game (`<Game>\Clips|Sessions|Screenshots`).
    pub capture_root: String,
    /// Optional separate folder for the temporary buffer (e.g. a faster drive). Its `Buffer`
    /// subfolder is used; `None` = `<capture_root>\Buffer`.
    #[serde(default)]
    pub buffer_root: Option<String>,
    pub replay_secs: u32,
    pub target: CaptureTarget,
    pub quality: Quality,
    pub fps: u32,
    pub capture_cursor: bool,
    pub audio: AudioConfig,
    /// e.g. "Alt+F10". Empty disables the hotkey.
    pub save_hotkey: String,
    #[serde(default)]
    pub hotkeys: Hotkeys,
    #[serde(default)]
    pub markers: MarkerConfig,
    /// Per-game overrides (matched by executable name); unset fields use the defaults above.
    #[serde(default)]
    pub per_game: Vec<GameRule>,
    /// Which on-screen notices to show (all also require `overlay`).
    #[serde(default)]
    pub notices: Notices,
    /// Encoder selection: automatic (GPU first) or software only (troubleshooting).
    #[serde(default)]
    pub encoder: EncoderChoice,
    /// Show a small on-screen notice (excluded from recordings) when recording starts and
    /// when a clip is saved or fails.
    #[serde(default = "yes")]
    pub overlay: bool,
    /// Strict cap for temporary buffer footage, bytes.
    pub buffer_cap_bytes: u64,
    /// Buffering pauses when free space on the capture drive falls below this.
    pub min_free_bytes: u64,
    /// After every saved clip/session with more than one audio track, also write the
    /// synchronized per-track WAV files ("editor audio files"). Off by default: the MP4
    /// already holds every track, and the WAVs roughly double the audio's disk use.
    #[serde(default)]
    pub editor_audio_files: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioConfig {
    /// Capture the target app's own audio (process loopback); for Display targets this
    /// becomes everything you hear ("System Audio").
    pub game_audio: bool,
    /// Microphone is OFF unless explicitly enabled.
    #[serde(default)]
    pub mic_enabled: bool,
    /// Endpoint ID of the chosen microphone; `None` = Windows default input. A chosen device
    /// that disconnects is never silently replaced by another one.
    #[serde(default)]
    pub mic_device: Option<String>,
    /// Muted: the Microphone track and the mix receive silence (nothing from the mic is kept).
    #[serde(default)]
    pub mic_muted: bool,
    /// Separate voice-chat app (e.g. Discord.exe), captured from that app only.
    #[serde(default)]
    pub voice_enabled: bool,
    #[serde(default)]
    pub voice_app: Option<String>,
    /// "All Desktop Audio" (formerly "Other Desktop"): everything you hear except CAPTURE. Overlaps game and voice chat, so it
    /// is kept as its own track and NOT added to Preview Mix (no doubled sound).
    #[serde(default)]
    pub other_enabled: bool,
    /// Preview Mix levels (0..=2). Isolated tracks are always recorded unprocessed.
    #[serde(default)]
    pub volumes: Volumes,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Volumes {
    pub game: f32,
    pub mic: f32,
    pub voice: f32,
}

impl Default for Volumes {
    fn default() -> Self {
        Self { game: 1.0, mic: 1.0, voice: 1.0 }
    }
}

impl AudioConfig {
    /// Same sources (a pipeline restart is needed only when this differs).
    pub fn same_sources(&self, o: &AudioConfig) -> bool {
        self.game_audio == o.game_audio
            && self.mic_enabled == o.mic_enabled
            && self.mic_device == o.mic_device
            && self.voice_enabled == o.voice_enabled
            && self.voice_app == o.voice_app
            && self.other_enabled == o.other_enabled
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    /// Virtual devices (e.g. "Steam Streaming Microphone") are labelled so they are never
    /// mistaken for a real microphone.
    pub is_virtual: bool,
    /// Windows' own mute switch for this device (Sound settings). A muted microphone records
    /// silence, so the UI says so instead of showing a flat meter.
    #[serde(default)]
    pub muted: bool,
    /// Windows' level for this device, 0..1 (None if unknown).
    #[serde(default)]
    pub volume: Option<f32>,
}

/// An app that currently has an audio session (i.e. can be picked as voice chat).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioApp {
    pub exe: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Level {
    pub name: String,
    pub peak: f32,
    pub rms: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MicLevel {
    pub device: String,
    pub peak: f32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureTarget {
    None,
    /// Detect games automatically: the foreground window is recorded when it belongs to a
    /// game library folder, is listed in `games`, or covers its whole monitor. `ignore` wins.
    Auto {
        #[serde(default)]
        games: Vec<String>,
        #[serde(default)]
        ignore: Vec<String>,
    },
    /// A specific live window (not persisted: handles change between launches).
    Window { hwnd: u64, pid: u32, exe: String, title: String },
    /// Whatever main window the named executable currently has. Persistable. While the app
    /// is not running the engine reports "waiting" and starts automatically when it appears.
    App { exe: String },
    Display { index: u32 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Balanced,
    High,
    Maximum,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowTarget {
    pub hwnd: u64,
    pub pid: u32,
    pub exe: String,
    pub title: String,
    /// Human-readable app name (file description / product name), used for folder names.
    pub app_name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisplayTarget {
    pub index: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Replay is off.
    Ready,
    /// Pipeline is starting; no frames confirmed yet.
    Starting,
    /// Replay is on and waiting for a game / the chosen app to appear. Not an error.
    Waiting,
    /// Frames confirmed and segments being written.
    Buffering,
    /// Buffering paused for a reason the user must act on (see `attention`).
    NeedsAttention,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineState {
    pub status: Status,
    /// Plain-language reason when status is NeedsAttention.
    pub attention: Option<String>,
    pub target_name: Option<String>,
    /// Executable of the current target, and why it was chosen in Auto mode
    /// ("game library folder", "your games list", "fullscreen").
    #[serde(default)]
    pub target_exe: Option<String>,
    #[serde(default)]
    pub detected_by: Option<String>,
    pub encoder: Option<EncoderInfo>,
    /// Seconds of footage currently available for a replay save.
    pub buffered_secs: f64,
    pub buffer_bytes: u64,
    pub capture_fps: f64,
    pub frames_captured: u64,
    pub tracks: Vec<TrackState>,
    pub saves_in_progress: u32,
    /// The save hotkey (kept for compatibility); `hotkeys` lists every action.
    pub hotkey: HotkeyState,
    #[serde(default)]
    pub hotkeys: Vec<HotkeyState>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// Session timeline position "now", seconds (for placing markers on a timeline).
    #[serde(default)]
    pub now_secs: f64,
    /// Full-session recording in progress: seconds recorded so far.
    #[serde(default)]
    pub session_secs: Option<f64>,
    /// Live microphone level while the mic monitor is on (settings open, not recording).
    #[serde(default)]
    pub mic_monitor: Option<MicLevel>,
    pub leftover_buffer_bytes: u64,
    pub free_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncoderInfo {
    pub name: String,
    pub hardware: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Encoders tried before this one, with the reason each was rejected.
    pub rejected: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackState {
    pub name: String,
    /// "process-loopback", "endpoint-loopback", "mix"
    pub mode: String,
    pub source: String,
    /// Peak level of the last ~100 ms, 0..1, from real samples.
    pub peak: f32,
    /// True once non-silent samples have been observed this session.
    pub signal_seen: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HotkeyState {
    #[serde(default)]
    pub action: Option<HotkeyAction>,
    pub combo: String,
    pub registered: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedClip {
    pub save_id: u64,
    pub path: String,
    pub requested_secs: f64,
    pub duration_secs: f64,
    pub bytes: u64,
    pub video_streams: u32,
    pub audio_tracks: Vec<String>,
}

impl Request {
    pub fn new(id: u64, cmd: Command) -> Self {
        Self { v: PROTOCOL_VERSION, id, cmd }
    }

    /// Lenient parse: `"args": {}` and a missing `args` mean the same thing, so clients need
    /// not know which commands carry fields.
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        let mut v: serde_json::Value = serde_json::from_str(line)?;
        let first = serde_json::from_value::<Request>(v.clone());
        if first.is_ok() {
            return first;
        }
        if let Some(o) = v.as_object_mut() {
            match o.get("args") {
                Some(a) if a.as_object().is_some_and(|m| m.is_empty()) => {
                    o.remove("args");
                }
                None => {
                    o.insert("args".into(), serde_json::json!({}));
                }
                _ => return first,
            }
        }
        serde_json::from_value::<Request>(v).or(first)
    }
}

impl Message {
    pub fn new(re: Option<u64>, event: Event) -> Self {
        Self { v: PROTOCOL_VERSION, re, event }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_is_tagged_and_versioned() {
        let cfg: EngineConfig = serde_json::from_str(r#"{"capture_root":"x","replay_secs":60,"target":{"kind":"none"},"quality":"high","fps":60,"capture_cursor":true,
        "audio":{"game_audio":true},"save_hotkey":"","buffer_cap_bytes":1,"min_free_bytes":1,
        "per_game":[{"exe":"Game.exe","replay_secs":300,"quality":"maximum"}]}"#).unwrap();
    assert_eq!(cfg.for_game("game.EXE").replay_secs, 300);
    assert_eq!(cfg.for_game("game.exe").quality, Quality::Maximum);
    assert_eq!(cfg.for_game("other.exe").replay_secs, 60);
    let r = Request::new(7, Command::SetReplay { enabled: true });
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, r#"{"v":1,"id":7,"cmd":"set_replay","args":{"enabled":true}}"#);
        assert_eq!(serde_json::from_str::<Request>(&s).unwrap(), r);
        let s = serde_json::to_string(&Request::new(8, Command::SaveReplay)).unwrap();
        assert_eq!(s, r#"{"v":1,"id":8,"cmd":"save_replay"}"#);
    }

    #[test]
    fn args_may_be_omitted_or_empty_for_any_command() {
        let a = Request::parse(r#"{"v":1,"id":1,"cmd":"start_session","args":{}}"#).unwrap();
        assert_eq!(a.cmd, Command::StartSession);
        let b = Request::parse(r#"{"v":1,"id":2,"cmd":"add_marker"}"#).unwrap();
        assert_eq!(b.cmd, Command::AddMarker { label: None });
        let c = Request::parse(r#"{"v":1,"id":3,"cmd":"add_marker","args":{"label":"boss"}}"#).unwrap();
        assert_eq!(c.cmd, Command::AddMarker { label: Some("boss".into()) });
        assert!(Request::parse(r#"{"v":1,"id":4,"cmd":"save_range"}"#).is_err());
    }

    #[test]
    fn message_roundtrip() {
        let m = Message::new(Some(3), Event::SaveFailed { save_id: 2, message: "x".into() });
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Message>(&s).unwrap(), m);
        assert!(s.starts_with(r#"{"v":1,"re":3,"event":"save_failed""#));
    }
}
