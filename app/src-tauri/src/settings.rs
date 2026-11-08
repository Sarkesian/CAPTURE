//! User settings and where CAPTURE keeps its own small files.
//!
//! Portable-first: if the folder holding CAPTURE.exe is writable, settings and the WebView2
//! profile live in `<exe folder>\data\`. Only if it is not writable (e.g. Program Files) do
//! they fall back to `%LOCALAPPDATA%\CAPTURE`. Recordings never go here.

use std::path::{Path, PathBuf};

use capture_protocol::{AudioConfig, CaptureTarget, EngineConfig, Hotkeys, MarkerConfig, Quality};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetPref {
    None,
    /// Detect games automatically (default).
    Auto,
    /// Remembered by executable name; `label` is what the user saw when choosing it.
    App { exe: String, label: String },
    Display { index: u32, label: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppSettings {
    pub version: u32,
    pub capture_root: String,
    pub replay_enabled: bool,
    pub replay_secs: u32,
    pub target: TargetPref,
    pub quality: Quality,
    pub fps: u32,
    pub capture_cursor: bool,
    pub game_audio: bool,
    pub save_hotkey: String,
    pub buffer_cap_gb: f64,
    pub min_free_gb: f64,
    pub close_to_tray: bool,
    /// Small on-screen notices (recording started, clip saved). Excluded from recordings.
    pub notify_on_save: bool,
    /// Auto mode: apps the user marked as games / as not games (exe names).
    pub games: Vec<String>,
    pub ignored: Vec<String>,
    /// Marker / overlay / session / screenshot hotkeys.
    pub hotkeys: Hotkeys,
    pub markers: MarkerConfig,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            capture_root: default_root(),
            replay_enabled: false,
            replay_secs: 120,
            target: TargetPref::Auto,
            quality: Quality::High,
            fps: 60,
            capture_cursor: true,
            game_audio: true,
            save_hotkey: "Alt+F10".into(),
            buffer_cap_gb: 20.0,
            min_free_gb: 10.0,
            close_to_tray: false,
            notify_on_save: true,
            games: vec![],
            ignored: vec![],
            hotkeys: Hotkeys::default(),
            markers: MarkerConfig::default(),
        }
    }
}

/// `D:\Captures` when a D: drive exists (the user's data drive), else `Videos\Captures`.
pub fn default_root() -> String {
    if Path::new("D:\\").exists() {
        return "D:\\Captures".into();
    }
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into());
    format!("{home}\\Videos\\Captures")
}

impl AppSettings {
    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            capture_root: self.capture_root.clone(),
            replay_secs: self.replay_secs,
            target: match &self.target {
                TargetPref::None => CaptureTarget::None,
                TargetPref::Auto => CaptureTarget::Auto { games: self.games.clone(), ignore: self.ignored.clone() },
                TargetPref::App { exe, .. } => CaptureTarget::App { exe: exe.clone() },
                TargetPref::Display { index, .. } => CaptureTarget::Display { index: *index },
            },
            quality: self.quality,
            fps: self.fps,
            capture_cursor: self.capture_cursor,
            audio: AudioConfig { game_audio: self.game_audio },
            save_hotkey: self.save_hotkey.clone(),
            overlay: self.notify_on_save,
            hotkeys: self.hotkeys.clone(),
            markers: self.markers.clone(),
            buffer_cap_bytes: (self.buffer_cap_gb.max(1.0) * 1e9) as u64,
            min_free_bytes: (self.min_free_gb.max(1.0) * 1e9) as u64,
        }
    }

    pub fn sanitized(mut self) -> Self {
        self.replay_secs = self.replay_secs.clamp(10, 3600);
        self.fps = match self.fps {
            30 | 60 | 120 => self.fps,
            _ => 60,
        };
        self.buffer_cap_gb = self.buffer_cap_gb.clamp(1.0, 2000.0);
        self.min_free_gb = self.min_free_gb.clamp(1.0, 2000.0);
        self.markers.sound_volume = self.markers.sound_volume.clamp(0.0, 1.0);
        self.markers.protect_before_secs = self.markers.protect_before_secs.min(600);
        self.markers.protect_after_secs = self.markers.protect_after_secs.min(600);
        // "Nothing chosen" from older builds becomes Auto: there is always something sensible to do.
        if self.target == TargetPref::None {
            self.target = TargetPref::Auto;
        }
        let dedup = |v: &mut Vec<String>| {
            let mut seen = std::collections::HashSet::new();
            v.retain(|x| !x.trim().is_empty() && seen.insert(x.to_ascii_lowercase()));
        };
        dedup(&mut self.games);
        dedup(&mut self.ignored);
        self
    }
}

pub fn data_dir() -> PathBuf {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    if let Some(dir) = exe_dir.map(|d| d.join("data")) {
        if std::fs::create_dir_all(&dir).is_ok() && writable(&dir) {
            return dir;
        }
    }
    let local = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(local).join("CAPTURE");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".write-test");
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

/// Missing file -> defaults. Unreadable file -> kept as `settings.json.bad` (never silently
/// overwritten) and defaults are used. A UTF-8 BOM (Notepad/PowerShell) is accepted.
pub fn load(dir: &Path) -> AppSettings {
    let path = dir.join("settings.json");
    let Ok(bytes) = std::fs::read(&path) else { return AppSettings::default() };
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    match serde_json::from_slice::<AppSettings>(body) {
        Ok(s) => s.sanitized(),
        Err(e) => {
            eprintln!("settings.json unreadable ({e}); keeping it as settings.json.bad");
            let _ = std::fs::rename(&path, dir.join("settings.json.bad"));
            AppSettings::default()
        }
    }
}

/// Write-then-rename so a crash mid-write never leaves corrupt settings.
pub fn save(dir: &Path, s: &AppSettings) -> std::io::Result<()> {
    let tmp = dir.join("settings.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap())?;
    std::fs::rename(tmp, dir.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_missing_fields_fall_back_to_defaults() {
        let s: AppSettings = serde_json::from_str(r#"{"replay_secs": 5, "fps": 47, "future_field": 1}"#).unwrap();
        let s = s.sanitized();
        assert_eq!(s.replay_secs, 10);
        assert_eq!(s.fps, 60);
        assert_eq!(s.save_hotkey, "Alt+F10");
    }

    #[test]
    fn bom_is_accepted_and_corrupt_file_is_preserved() {
        let dir = std::env::temp_dir().join(format!("capture-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), b"\xEF\xBB\xBF{\"replay_secs\": 300}").unwrap();
        assert_eq!(load(&dir).replay_secs, 300);
        std::fs::write(dir.join("settings.json"), b"{ not json").unwrap();
        assert_eq!(load(&dir), AppSettings::default());
        assert!(dir.join("settings.json.bad").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn target_pref_maps_to_persistable_engine_target() {
        let s = AppSettings { target: TargetPref::App { exe: "game.exe".into(), label: "Game".into() }, ..Default::default() };
        assert_eq!(s.engine_config().target, CaptureTarget::App { exe: "game.exe".into() });
    }
}
