//! User settings: one versioned `settings.json` in `Data\Settings` (see `paths.rs`).
//! Recordings never go here.
//!
//! Schema versions: 1 = 0.3–0.4 builds, 2 = 0.5 (new data layout, first-run setup). Before a
//! file written by another schema version is rewritten, a copy is kept in `Settings\Backups\`,
//! so neither an update nor going back to an older build can lose settings.

use std::path::{Path, PathBuf};

use capture_protocol::{AudioConfig, CaptureTarget, EncoderChoice, EngineConfig, GameRule, Hotkeys, MarkerConfig, Notices, Quality, RecordingOutput, Volumes};
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
    pub history_secs: u32,
    pub target: TargetPref,
    pub quality: Quality,
    pub recording_output: RecordingOutput,
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

/// Current settings schema version.
pub const SCHEMA_VERSION: u32 = 4;

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            capture_root: default_root(),
            replay_enabled: false,
            replay_secs: 120,
            history_secs: 4 * 3600,
            target: TargetPref::Auto,
            quality: Quality::High,
            recording_output: RecordingOutput::Compatible,
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

/// `<Videos>\Captures`, where `<Videos>` is the user's Windows Videos folder (which follows the
/// user if they moved it to another drive). First-run setup shows it and offers to change it.
pub fn default_root() -> String {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Videos, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let videos = unsafe {
        SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None).ok().map(|p| {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as *const _));
            s
        })
    };
    let videos = videos.filter(|v| !v.is_empty()).unwrap_or_else(|| {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into());
        format!("{home}\\Videos")
    });
    format!("{}\\Captures", videos.trim_end_matches('\\'))
}

impl AppSettings {
    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            capture_root: self.capture_root.clone(),
            buffer_root: self.buffer_root.clone().filter(|b| !b.trim().is_empty()),
            replay_secs: self.replay_secs,
            history_secs: self.history_secs,
            target: match &self.target {
                TargetPref::None => CaptureTarget::None,
                TargetPref::Auto => CaptureTarget::Auto { games: self.games.clone(), ignore: self.ignored.clone() },
                TargetPref::App { exe, .. } => CaptureTarget::App { exe: exe.clone() },
                TargetPref::Display { index, .. } => CaptureTarget::Display { index: *index },
            },
            quality: self.quality,
            recording_output: self.recording_output,
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
        self.history_secs = self.history_secs.clamp(60, 6 * 3600);
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

/// Loads settings without ever losing them. `dir` is the `Data\Settings` folder; `app_dir`
/// (the folder holding CAPTURE.exe) is used to find an earlier portable copy beside it.
/// - Missing file -> settings from the newest other portable copy next to this one
///   (`..\CAPTURE-<version>\Data\Settings\settings.json`, or `..\CAPTURE-<version>\data\settings.json`
///   from 0.3–0.4), copied, never moved; else defaults.
/// - Readable file -> used, and copied to `settings.json.bak` (last known good).
/// - A file where only some values are unreadable (e.g. written by a different version) ->
///   every readable value is kept and only the bad ones fall back to defaults.
/// - A file that is not JSON at all -> the last known good backup, else defaults.
///   In both damaged cases the original is kept as `settings.json.bad-<time>` first.
/// - A file from another schema version -> a copy goes to `Backups\` before it is upgraded.
/// A UTF-8 BOM (Notepad/PowerShell) is accepted.
pub fn load(dir: &Path, app_dir: Option<&Path>) -> AppSettings {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("settings.json");
    if !path.exists() {
        if let Some(from) = app_dir.and_then(|a| import_previous(a, dir)) {
            eprintln!("settings: copied from {}", from.display());
        }
    }
    let Ok(bytes) = std::fs::read(&path) else { return AppSettings::default() };
    let s = match parse(&bytes) {
        Parsed::Ok(s) => {
            let _ = std::fs::copy(&path, dir.join("settings.json.bak"));
            s
        }
        Parsed::Partial(s, bad) => {
            keep_damaged(dir, &path);
            eprintln!("settings.json: kept every readable value; reset {bad:?} to defaults");
            s
        }
        Parsed::Unreadable(e) => {
            keep_damaged(dir, &path);
            eprintln!("settings.json unreadable ({e})");
            match std::fs::read(dir.join("settings.json.bak")).map(|b| parse(&b)) {
                Ok(Parsed::Ok(s) | Parsed::Partial(s, _)) => {
                    // The damaged file is kept as `.bad-<time>`; put the recovered settings back
                    // so the next start does not report (and copy) the same damage again.
                    let _ = save(dir, &s);
                    s
                }
                _ => AppSettings::default(),
            }
        }
    };
    migrate(dir, &path, s).sanitized()
}

/// Upgrade a file written by another schema version, after keeping a copy of it in `Backups\`.
fn migrate(dir: &Path, path: &Path, mut s: AppSettings) -> AppSettings {
    if s.version == SCHEMA_VERSION {
        return s;
    }
    let backups = dir.join("Backups");
    let _ = std::fs::create_dir_all(&backups);
    let note = if s.version > SCHEMA_VERSION { "-from-newer-build" } else { "" };
    let name = format!("settings-v{}{note}-{}.json", s.version, unix_secs());
    if std::fs::copy(path, backups.join(&name)).is_ok() {
        eprintln!("settings: schema v{} -> v{SCHEMA_VERSION}; previous file kept as Backups\\{name}", s.version);
    }
    // v1 -> v2: no value changes. People who already dismissed the 0.3/0.4 welcome card are not
    // asked the first-run questions again (`first_run_done` carries over).
    s.version = SCHEMA_VERSION;
    let _ = save(dir, &s);
    s
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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
    let _ = std::fs::copy(path, dir.join(format!("settings.json.bad-{}", unix_secs())));
}

/// First run of a new portable copy: copy the settings of the most recently used `CAPTURE*`
/// folder beside this one, so a new version keeps microphone, hotkeys, folders, games and every
/// other choice. Only reads the old folder. Understands the 0.5 layout
/// (`Data\Settings\settings.json`) and the 0.3–0.4 one (`data\settings.json`).
fn import_previous(app_dir: &Path, dir: &Path) -> Option<PathBuf> {
    let newest = newest_previous_settings(app_dir)?;
    std::fs::copy(&newest, dir.join("settings.json")).ok()?;
    Some(newest)
}

/// The app folder of the earlier copy whose settings a new copy imports (see `import_previous`).
pub fn previous_copy(app_dir: &Path) -> Option<PathBuf> {
    let f = newest_previous_settings(app_dir)?;
    let settings_dir = f.parent()?;
    // `<copy>\Data\Settings\settings.json` (0.5+) or `<copy>\data\settings.json` (0.3–0.4)
    let data = if settings_dir.file_name()?.eq_ignore_ascii_case("Settings") { settings_dir.parent()? } else { settings_dir };
    data.parent().map(Path::to_path_buf)
}

fn newest_previous_settings(app_dir: &Path) -> Option<PathBuf> {
    let parent = app_dir.parent()?;
    let newest = std::fs::read_dir(parent)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p != app_dir && p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_uppercase().starts_with("CAPTURE")))
        .flat_map(|p| [p.join("Data").join("Settings").join("settings.json"), p.join("data").join("settings.json")])
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .filter(|(_, p)| std::fs::read(p).is_ok_and(|b| !matches!(parse(&b), Parsed::Unreadable(_))))
        .max_by_key(|(t, _)| *t)?
        .1;
    Some(newest)
}

/// Write-then-rename so a crash mid-write never leaves corrupt settings.
pub fn save(dir: &Path, s: &AppSettings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("settings.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap())?;
    std::fs::rename(tmp, dir.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_migration_never_expands_storage_or_hotkey_length() {
        let old:AppSettings=serde_json::from_str(r#"{"version":2,"replay_secs":120,"buffer_cap_gb":7.5,"min_free_gb":4.0}"#).unwrap();
        let next=old.sanitized();assert_eq!(next.replay_secs,120);assert_eq!(next.buffer_cap_gb,7.5);assert_eq!(next.min_free_gb,4.0);assert_eq!(next.history_secs,14400);
        let cfg=next.engine_config();assert_eq!(cfg.history_secs,14400);assert_eq!(cfg.replay_secs,120);assert_eq!(cfg.buffer_cap_bytes,7_500_000_000);
    }

    #[test]
    fn recording_format_migration_and_hdr_preference_roundtrip() {
        let old:AppSettings=serde_json::from_str(r#"{"version":3,"quality":"archival","fps":60,"buffer_cap_gb":7.5}"#).unwrap();
        assert_eq!(old.recording_output,RecordingOutput::Compatible);
        assert_eq!(old.quality,Quality::Archival);
        assert_eq!(old.fps,60);
        assert_eq!(old.buffer_cap_gb,7.5);
        let mut next=old;next.recording_output=RecordingOutput::OriginalHdr;
        let restored:AppSettings=serde_json::from_slice(&serde_json::to_vec(&next).unwrap()).unwrap();
        assert_eq!(restored.engine_config().recording_output,RecordingOutput::OriginalHdr);
    }

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
        assert_eq!(load(&dir, None).replay_secs, 300);
        assert!(dir.join("settings.json.bak").exists(), "last known good copy kept");
        std::fs::write(dir.join("settings.json"), b"{ not json").unwrap();
        assert_eq!(load(&dir, None).replay_secs, 300, "restored from the last good copy, not reset");
        assert!(has_bad_copy(&dir), "damaged file kept for inspection");
        let main = std::fs::read(dir.join("settings.json")).unwrap();
        assert!(matches!(parse(&main), Parsed::Ok(s) if s.replay_secs == 300), "recovered settings written back");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_unreadable_value_does_not_reset_the_others() {
        // e.g. a value written by a different version that this one cannot read
        let dir = temp("partial");
        let body = r#"{"replay_secs": 300, "mic_enabled": true, "mic_device": "{0.0.1}.{abc}", "save_hotkey": "Alt+F9",
                       "target": {"kind": "spaceship"}, "quality": 7, "start_with_windows": true}"#;
        std::fs::write(dir.join("settings.json"), body).unwrap();
        let s = load(&dir, None);
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
        // 0.4 layout beside a 0.5 copy
        let old = root.join("CAPTURE-0.4.1").join("data");
        let app = root.join("CAPTURE-0.5.0");
        let new = app.join("Data").join("Settings");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("settings.json"), r#"{"version": 1, "mic_enabled": true, "replay_secs": 600}"#).unwrap();
        assert!(crate::paths::same_path(&previous_copy(&app).unwrap(), &root.join("CAPTURE-0.4.1")));
        let s = load(&new, Some(&app));
        assert!(s.mic_enabled && s.replay_secs == 600);
        assert!(old.join("settings.json").exists(), "the old folder is never changed");
        // 0.5 layout beside a newer copy
        let app2 = root.join("CAPTURE-0.5.1");
        std::thread::sleep(std::time::Duration::from_millis(50)); // newest by modification time
        std::fs::write(new.join("settings.json"), r#"{"version": 2, "mic_enabled": true, "replay_secs": 600}"#).unwrap();
        assert!(crate::paths::same_path(&previous_copy(&app2).unwrap(), &app));
        let s2 = load(&app2.join("Data").join("Settings"), Some(&app2));
        assert!(s2.mic_enabled && s2.replay_secs == 600);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_older_schema_is_backed_up_before_it_is_upgraded() {
        let dir = temp("schema");
        std::fs::write(dir.join("settings.json"), r#"{"version": 1, "replay_secs": 300, "first_run_done": true}"#).unwrap();
        let s = load(&dir, None);
        assert_eq!((s.version, s.replay_secs, s.first_run_done), (SCHEMA_VERSION, 300, true));
        let backups: Vec<_> = std::fs::read_dir(dir.join("Backups")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(backups.iter().any(|n| n.starts_with("settings-v1-")), "{backups:?}");
        let on_disk: AppSettings = serde_json::from_slice(&std::fs::read(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(on_disk.version, SCHEMA_VERSION);
        // A file from a newer build is kept too before this build rewrites it.
        std::fs::write(dir.join("settings.json"), r#"{"version": 9, "replay_secs": 400}"#).unwrap();
        assert_eq!(load(&dir, None).replay_secs, 400);
        assert!(std::fs::read_dir(dir.join("Backups")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("settings-v9-from-newer-build")));
        std::fs::remove_dir_all(&dir).unwrap();
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
