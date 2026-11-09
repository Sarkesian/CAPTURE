//! User settings and where CAPTURE keeps its own small files.
//!
//! Portable-first: if the folder holding CAPTURE.exe is writable, settings and the WebView2
//! profile live in `<exe folder>\data\`. Only if it is not writable (e.g. Program Files) do
//! they fall back to `%LOCALAPPDATA%\CAPTURE`. Recordings never go here.

use std::path::{Path, PathBuf};

use capture_protocol::{AudioConfig, CaptureTarget, EncoderChoice, EngineConfig, GameRule, Hotkeys, MarkerConfig, Notices, Quality, Volumes};
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
    /// Microphone: off until explicitly enabled. `mic_device` None = Windows default input.
    pub mic_enabled: bool,
    pub mic_device: Option<String>,
    pub mic_muted: bool,
    pub voice_enabled: bool,
    pub voice_app: Option<String>,
    pub other_enabled: bool,
    pub volumes: Volumes,
    /// Per-game overrides by exe name.
    pub per_game: Vec<GameRule>,
    /// The welcome card has been dismissed.
    pub first_run_done: bool,
    /// Start CAPTURE (in the tray) when you sign in to Windows.
    pub start_with_windows: bool,
    pub encoder: EncoderChoice,
    pub notices: Notices,
    /// Overlay size in percent (90 / 100 / 115).
    pub overlay_scale: u32,
    /// Optional separate folder for temporary footage.
    pub buffer_root: Option<String>,
    /// Also write per-track WAV files after every save ("Create separate audio files for
    /// editors"). Off: the MP4 already contains every track; WAVs can be made later per clip.
    pub editor_audio_files: bool,
    /// Minimizing the window hides it to the tray instead of the taskbar.
    pub minimize_to_tray: bool,
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
            // A background recorder: closing the window keeps it recording in the tray. Quit
            // from the tray menu (or turn this off) to stop it.
            close_to_tray: true,
            notify_on_save: true,
            games: vec![],
            ignored: vec![],
            hotkeys: Hotkeys::default(),
            markers: MarkerConfig::default(),
            mic_enabled: false,
            mic_device: None,
            mic_muted: false,
            voice_enabled: false,
            voice_app: None,
            other_enabled: false,
            volumes: Volumes::default(),
            per_game: vec![],
            first_run_done: false,
            start_with_windows: false,
            encoder: EncoderChoice::Auto,
            notices: Notices::default(),
            overlay_scale: 100,
            buffer_root: None,
            editor_audio_files: false,
            minimize_to_tray: false,
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
            buffer_root: self.buffer_root.clone().filter(|b| !b.trim().is_empty()),
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
            audio: AudioConfig {
                game_audio: self.game_audio,
                mic_enabled: self.mic_enabled,
                mic_device: self.mic_device.clone(),
                mic_muted: self.mic_muted,
                voice_enabled: self.voice_enabled,
                voice_app: self.voice_app.clone(),
                other_enabled: self.other_enabled,
                volumes: self.volumes.clone(),
            },
            save_hotkey: self.save_hotkey.clone(),
            overlay: self.notify_on_save,
            hotkeys: self.hotkeys.clone(),
            markers: self.markers.clone(),
            per_game: self.per_game.clone(),
            encoder: self.encoder,
            notices: self.notices,
            buffer_cap_bytes: (self.buffer_cap_gb.max(1.0) * 1e9) as u64,
            min_free_bytes: (self.min_free_gb.max(1.0) * 1e9) as u64,
            editor_audio_files: self.editor_audio_files,
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
        self.overlay_scale = self.overlay_scale.clamp(80, 130);
        for v in [&mut self.volumes.game, &mut self.volumes.mic, &mut self.volumes.voice] {
            *v = v.clamp(0.0, 2.0);
        }
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
        self.per_game.retain(|r| !r.exe.trim().is_empty() && (r.replay_secs.is_some() || r.quality.is_some() || r.game_audio.is_some()));
        for r in &mut self.per_game {
            r.replay_secs = r.replay_secs.map(|s| s.clamp(10, 3600));
        }
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

/// Loads settings without ever losing them:
/// - Missing file -> settings from the newest older portable build next to this one
///   (`..\CAPTURE-<version>\data\settings.json`, copied, never moved), else defaults.
/// - Readable file -> used, and copied to `settings.json.bak` (last known good).
/// - A file where only some values are unreadable (e.g. written by a different version) ->
///   every readable value is kept and only the bad ones fall back to defaults.
/// - A file that is not JSON at all -> the last known good backup, else defaults.
///   In both damaged cases the original is kept as `settings.json.bad-<time>` first.
/// A UTF-8 BOM (Notepad/PowerShell) is accepted.
pub fn load(dir: &Path) -> AppSettings {
    let path = dir.join("settings.json");
    if !path.exists() {
        if let Some(from) = import_previous(dir) {
            eprintln!("settings: copied from {}", from.display());
        }
    }
    let Ok(bytes) = std::fs::read(&path) else { return AppSettings::default() };
    match parse(&bytes) {
        Parsed::Ok(s) => {
            let _ = std::fs::copy(&path, dir.join("settings.json.bak"));
            s.sanitized()
        }
        Parsed::Partial(s, bad) => {
            keep_damaged(dir, &path);
            eprintln!("settings.json: kept every readable value; reset {bad:?} to defaults");
            s.sanitized()
        }
        Parsed::Unreadable(e) => {
            keep_damaged(dir, &path);
            eprintln!("settings.json unreadable ({e})");
            match std::fs::read(dir.join("settings.json.bak")).map(|b| parse(&b)) {
                Ok(Parsed::Ok(s) | Parsed::Partial(s, _)) => s.sanitized(),
                _ => AppSettings::default(),
            }
        }
    }
}

enum Parsed {
    Ok(AppSettings),
    /// Usable settings plus the names of fields that could not be read.
    Partial(AppSettings, Vec<String>),
    Unreadable(String),
}

fn parse(bytes: &[u8]) -> Parsed {
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let e = match serde_json::from_slice::<AppSettings>(body) {
        Ok(s) => return Parsed::Ok(s),
        Err(e) => e,
    };
    // Salvage field by field: start from defaults and take each stored value that is valid.
    let Ok(serde_json::Value::Object(stored)) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Parsed::Unreadable(e.to_string());
    };
    let mut merged = serde_json::to_value(AppSettings::default()).unwrap();
    let mut bad = Vec::new();
    for (k, v) in stored {
        let mut trial = merged.clone();
        trial[&k] = v;
        if serde_json::from_value::<AppSettings>(trial.clone()).is_ok() {
            merged = trial;
        } else {
            bad.push(k);
        }
    }
    match serde_json::from_value::<AppSettings>(merged) {
        Ok(s) => Parsed::Partial(s, bad),
        Err(_) => Parsed::Unreadable(e.to_string()),
    }
}

/// Keep a damaged settings file for inspection, under a name that never overwrites an
/// earlier one.
fn keep_damaged(dir: &Path, path: &Path) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = std::fs::copy(path, dir.join(format!("settings.json.bad-{secs}")));
}

/// First run of a new portable version: copy the settings of the most recently used
/// `CAPTURE-*` folder beside this one, so an update keeps microphone, hotkeys, folders, games
/// and every other choice. Only reads the old folder.
fn import_previous(dir: &Path) -> Option<PathBuf> {
    let exe_dir = dir.parent()?; // <...>\CAPTURE-0.3.1  (dir = <exe folder>\data)
    let parent = exe_dir.parent()?;
    let newest = std::fs::read_dir(parent)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p != exe_dir && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_uppercase().starts_with("CAPTURE")))
        .map(|p| p.join("data").join("settings.json"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .filter(|(_, p)| std::fs::read(p).is_ok_and(|b| !matches!(parse(&b), Parsed::Unreadable(_))))
        .max_by_key(|(t, _)| *t)?
        .1;
    std::fs::copy(&newest, dir.join("settings.json")).ok()?;
    Some(newest)
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

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("capture-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn has_bad_copy(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("settings.json.bad-"))
    }

    #[test]
    fn bom_is_accepted_and_corrupt_file_falls_back_to_last_good_copy() {
        let dir = temp("bom");
        std::fs::write(dir.join("settings.json"), b"\xEF\xBB\xBF{\"replay_secs\": 300}").unwrap();
        assert_eq!(load(&dir).replay_secs, 300);
        assert!(dir.join("settings.json.bak").exists(), "last known good copy kept");
        std::fs::write(dir.join("settings.json"), b"{ not json").unwrap();
        assert_eq!(load(&dir).replay_secs, 300, "restored from the last good copy, not reset");
        assert!(has_bad_copy(&dir), "damaged file kept for inspection");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_unreadable_value_does_not_reset_the_others() {
        // e.g. a value written by a different version that this one cannot read
        let dir = temp("partial");
        let body = r#"{"replay_secs": 300, "mic_enabled": true, "mic_device": "{0.0.1}.{abc}", "save_hotkey": "Alt+F9",
                       "target": {"kind": "spaceship"}, "quality": 7, "start_with_windows": true}"#;
        std::fs::write(dir.join("settings.json"), body).unwrap();
        let s = load(&dir);
        assert_eq!((s.replay_secs, s.mic_enabled, s.mic_device.as_deref(), s.save_hotkey.as_str()), (300, true, Some("{0.0.1}.{abc}"), "Alt+F9"));
        assert!(s.start_with_windows);
        assert_eq!(s.target, TargetPref::Auto);
        assert_eq!(s.quality, Quality::High);
        assert!(has_bad_copy(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_version_folder_copies_the_previous_versions_settings() {
        let root = temp("update");
        let old = root.join("CAPTURE-0.3.0").join("data");
        let new = root.join("CAPTURE-0.3.1").join("data");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("settings.json"), r#"{"mic_enabled": true, "replay_secs": 600}"#).unwrap();
        let s = load(&new);
        assert!(s.mic_enabled && s.replay_secs == 600);
        assert!(old.join("settings.json").exists(), "the old folder is never changed");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn current_file_round_trips_unchanged() {
        let s = AppSettings { mic_enabled: true, editor_audio_files: true, minimize_to_tray: true, ..Default::default() };
        let back: AppSettings = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn microphone_is_off_unless_enabled() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!s.engine_config().audio.mic_enabled);
        assert!(!s.engine_config().audio.voice_enabled);
    }

    #[test]
    fn target_pref_maps_to_persistable_engine_target() {
        let s = AppSettings { target: TargetPref::App { exe: "game.exe".into(), label: "Game".into() }, ..Default::default() };
        assert_eq!(s.engine_config().target, CaptureTarget::App { exe: "game.exe".into() });
    }
}
