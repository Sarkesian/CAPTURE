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
    /// Strict cap for temporary buffer footage, bytes.
    pub buffer_cap_bytes: u64,
    /// Buffering pauses when free space on the capture drive falls below this.
    pub min_free_bytes: u64,
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
    pub encoder: Option<EncoderInfo>,
    /// Seconds of footage currently available for a replay save.
    pub buffered_secs: f64,
    pub buffer_bytes: u64,
    pub capture_fps: f64,
    pub frames_captured: u64,
    pub tracks: Vec<TrackState>,
    pub saves_in_progress: u32,
    pub hotkey: HotkeyState,
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
    fn message_roundtrip() {
        let m = Message::new(Some(3), Event::SaveFailed { save_id: 2, message: "x".into() });
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Message>(&s).unwrap(), m);
        assert!(s.starts_with(r#"{"v":1,"re":3,"event":"save_failed""#));
    }
}
