//! IPC contract between the CAPTURE UI process and the `capture-worker` process.
//!
//! Transport: one JSON object per line. The UI writes [`Request`]s to the worker's stdin; the
//! worker writes [`Message`]s to stdout. Every line carries [`PROTOCOL_VERSION`]; a peer that
//! receives a different major version must refuse to operate rather than guess.

use serde::{Deserialize, Serialize};

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
    /// Thumbnail + duration for a saved clip (answered with `ClipInfo`).
    ClipInfo { path: String },
    ListTargets,
    GetState,
    /// Delete leftover temporary buffer footage that is not part of the running session.
    ClearTempCache,
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
    ClipInfo { path: String, thumb: Option<String>, duration_secs: Option<f64>, error: Option<String> },
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
}

impl Default for Hotkeys {
    fn default() -> Self {
        // Avoids NVIDIA's overlay defaults (Alt+Z, Alt+F9).
        Self { marker: "Alt+F8".into(), overlay: "Alt+F11".into(), session: String::new(), screenshot: String::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MarkerConfig {
    /// Short confirmation sound when a marker is added (never recorded, see docs).
    pub sound: bool,
    /// 0.0..=1.0
    pub sound_volume: f32,
    /// Keep footage around each marker from being overwritten while the session runs.
    pub protect: bool,
    pub protect_before_secs: u32,
    pub protect_after_secs: u32,
}

impl Default for MarkerConfig {
    fn default() -> Self {
        Self { sound: true, sound_volume: 0.35, protect: true, protect_before_secs: 30, protect_after_secs: 15 }
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
    /// Root folder that holds `Buffer`, `Clips`, `Sessions`, `Data`.
    pub capture_root: String,
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
    /// Show a small on-screen notice (excluded from recordings) when recording starts and
    /// when a clip is saved or fails.
    #[serde(default = "yes")]
    pub overlay: bool,
    /// Strict cap for temporary buffer footage, bytes.
    pub buffer_cap_bytes: u64,
    /// Buffering pauses when free space on the capture drive falls below this.
    pub min_free_bytes: u64,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioConfig {
    /// Capture the target app's own audio (process loopback); for Display targets this
    /// becomes the whole-device output mix ("System audio").
    pub game_audio: bool,
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
