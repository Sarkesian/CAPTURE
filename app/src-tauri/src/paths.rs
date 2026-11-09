//! Every location CAPTURE uses for its own files, resolved once at start-up from the folder that
//! holds CAPTURE.exe. Nothing depends on the current working directory, so a shortcut, the
//! installer, the Run entry and a command line all behave the same.
//!
//! One layout for the portable folder and the installed copy (the installer deploys exactly
//! the portable payload):
//!
//! ```text
//! <app>\CAPTURE.exe
//! <app>\manifest.json            version + SHA-256 of every program file (updates verify it)
//! <app>\Runtime\                 capture-worker.exe + FFmpeg DLLs (read-only at run time)
//! <app>\Resources\               guide and licences
//! <app>\Data\Settings\           settings.json, settings.json.bak, Backups\
//! <app>\Data\Cache\WebView\      WebView2 profile (instead of WebView2's default in AppData)
//! <app>\Data\Cache\Temp\         TEMP/TMP for CAPTURE and every process it starts
//! <app>\Data\Cache\Update\       a staged update, only while one is being installed
//! <app>\Data\Logs\               capture.log (+ capture.old.log)
//! <app>\Uninstall CAPTURE.exe    installed copies only
//! ```
//!
//! Recordings and the replay buffer never live here; they go to the folders chosen in Settings.
//!
//! If `<app>\Data` cannot be written (for example CAPTURE was copied into Program Files),
//! CAPTURE does not silently move its data elsewhere: it explains the problem and only uses
//! `%LOCALAPPDATA%\CAPTURE\Data` if the user agrees. That choice is remembered there.

use std::path::{Path, PathBuf};

pub const UNINSTALLER: &str = "Uninstall CAPTURE.exe";
const FALLBACK_MARKER: &str = "fallback-for.txt";

#[derive(Debug, Clone)]
pub struct Paths {
    pub app_dir: PathBuf,
    pub runtime: PathBuf,
    pub resources: PathBuf,
    pub data: PathBuf,
    pub settings: PathBuf,
    pub webview: PathBuf,
    pub temp: PathBuf,
    pub update: PathBuf,
    pub logs: PathBuf,
    /// Installed with the Windows installer (an uninstaller sits next to CAPTURE.exe).
    pub installed: bool,
    /// Data lives in `%LOCALAPPDATA%\CAPTURE\Data` because the app folder is read-only.
    pub data_fallback: bool,
}

impl Paths {
    /// The layout for an app folder and a data folder. Creates nothing.
    pub fn layout(app_dir: &Path, data: &Path, data_fallback: bool) -> Self {
        let cache = data.join("Cache");
        Self {
            app_dir: app_dir.to_path_buf(),
            runtime: app_dir.join("Runtime"),
            resources: app_dir.join("Resources"),
            data: data.to_path_buf(),
            settings: data.join("Settings"),
            webview: cache.join("WebView"),
            temp: cache.join("Temp"),
            update: cache.join("Update"),
            logs: data.join("Logs"),
            installed: app_dir.join(UNINSTALLER).is_file(),
            data_fallback,
        }
    }

    pub fn create_dirs(&self) -> std::io::Result<()> {
        for d in [&self.settings, &self.webview, &self.temp, &self.logs] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    /// `Runtime\capture-worker.exe`. Development builds (cargo's `target\<profile>\`) have the
    /// worker beside CAPTURE.exe instead.
    pub fn worker_exe(&self) -> PathBuf {
        let packaged = self.runtime.join("capture-worker.exe");
        let dev = self.app_dir.join("capture-worker.exe");
        if !packaged.is_file() && dev.is_file() {
            dev
        } else {
            packaged
        }
    }
}

pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .expect("CAPTURE cannot find its own folder")
}

/// Windows Explorer can run an exe straight out of a zip by extracting just that file to a
/// temporary folder. CAPTURE would then start without its Runtime folder.
pub fn running_from_zip_preview(app_dir: &Path) -> bool {
    let s = app_dir.to_string_lossy().to_ascii_lowercase();
    let temp = std::env::var("TEMP").unwrap_or_default().to_ascii_lowercase();
    (!temp.is_empty() && s.starts_with(&temp) && s.contains(".zip")) || s.contains(".zip\\")
}

pub fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

pub enum Resolved {
    Ready(Paths),
    /// The app folder is read-only and the user has not yet agreed to a fallback location.
    NeedsConsent { app_dir: PathBuf, fallback: PathBuf },
}

fn fallback_data_dir() -> PathBuf {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| "C:\\".into());
    PathBuf::from(local).join("CAPTURE").join("Data")
}

/// Where this copy of CAPTURE keeps its data. Never creates anything outside `<app>\Data`
/// unless the user already agreed to the fallback for this exact app folder.
pub fn resolve(app_dir: &Path) -> Resolved {
    let data = app_dir.join("Data");
    if writable(&data) {
        return Resolved::Ready(Paths::layout(app_dir, &data, false));
    }
    let fb = fallback_data_dir();
    let agreed = std::fs::read_to_string(fb.join(FALLBACK_MARKER))
        .is_ok_and(|s| s.lines().any(|l| same_path(Path::new(l.trim()), app_dir)));
    if agreed && writable(&fb) {
        return Resolved::Ready(Paths::layout(app_dir, &fb, true));
    }
    Resolved::NeedsConsent { app_dir: app_dir.to_path_buf(), fallback: fb }
}

/// Record the user's consent to keep this app folder's data in the fallback location.
pub fn accept_fallback(app_dir: &Path, fallback: &Path) -> std::io::Result<Paths> {
    std::fs::create_dir_all(fallback)?;
    let marker = fallback.join(FALLBACK_MARKER);
    let mut body = std::fs::read_to_string(&marker).unwrap_or_default();
    if !body.lines().any(|l| same_path(Path::new(l.trim()), app_dir)) {
        body.push_str(&format!("{}\r\n", app_dir.display()));
    }
    std::fs::write(marker, body)?;
    Ok(Paths::layout(app_dir, fallback, true))
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase();
    norm(a) == norm(b)
}

/// Builds before 0.5 kept `settings.json` and the WebView2 profile (`webview\`) directly in
/// `<app>\data` (the same folder as `Data` on Windows, which ignores case). Move them into the
/// new subfolders. The original settings file is also kept in `Settings\Backups\`.
/// Returns true if anything was migrated.
pub fn migrate_legacy(old: &Path, p: &Paths) -> bool {
    let mut moved = false;
    let legacy = old.join("settings.json");
    let target = p.settings.join("settings.json");
    if legacy.is_file() && !target.exists() {
        let backups = p.settings.join("Backups");
        if std::fs::create_dir_all(&backups).is_ok() {
            let _ = std::fs::copy(&legacy, backups.join("settings-v1-from-0.4.json"));
        }
        let _ = std::fs::create_dir_all(&p.settings);
        if std::fs::rename(&legacy, &target).is_ok() || std::fs::copy(&legacy, &target).is_ok() {
            moved = true;
        }
        // Earlier last-known-good and damaged copies follow the file.
        for e in std::fs::read_dir(old).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("settings.json.") && e.path().is_file() {
                let _ = std::fs::rename(e.path(), p.settings.join(&name));
            }
        }
    }
    let wv = old.join("webview");
    if wv.is_dir() && !same_path(&wv, &p.webview) && !p.webview.join("EBWebView").exists() {
        let _ = std::fs::create_dir_all(p.webview.parent().unwrap());
        let _ = std::fs::remove_dir(&p.webview); // only if empty
        if std::fs::rename(&wv, &p.webview).is_ok() {
            moved = true;
        }
    }
    moved
}

/// Program files that 0.3–0.4 kept at the top of the folder. When someone extracts a 0.5 zip
/// over an old portable folder, these are obsolete copies of what is now in `Runtime\` and
/// `Resources\`. Removed only when the new runtime is present, and only these exact names.
const LEGACY_FILES: [&str; 7] = ["capture-worker.exe", "avcodec-62.dll", "avformat-62.dll", "avutil-60.dll", "swresample-6.dll", "swscale-9.dll", "QUICKSTART.md"];

pub fn remove_legacy_program_files(p: &Paths) -> usize {
    if !p.runtime.join("capture-worker.exe").is_file() {
        return 0;
    }
    let mut n = 0;
    for f in LEGACY_FILES {
        if std::fs::remove_file(p.app_dir.join(f)).is_ok() {
            n += 1;
        }
    }
    let lic = p.app_dir.join("licenses");
    for f in ["FFmpeg-LICENSE.txt", "THIRD_PARTY.md"] {
        if std::fs::remove_file(lic.join(f)).is_ok() {
            n += 1;
        }
    }
    let _ = std::fs::remove_dir(lic); // only if empty
    n
}

/// Show the folder as `Data` rather than the lower-case `data` older builds created.
pub fn normalize_data_case(app_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(app_dir) else { return };
    for e in entries.flatten() {
        if e.file_name() == "data" {
            let tmp = app_dir.join(format!("Data.renaming-{}", std::process::id()));
            if std::fs::rename(e.path(), &tmp).is_ok() && std::fs::rename(&tmp, app_dir.join("Data")).is_err() {
                let _ = std::fs::rename(&tmp, e.path());
            }
        }
    }
}

/// Point TEMP/TMP at CAPTURE's own temp folder, so temporary files written by CAPTURE, WebView2
/// and the capture engine (which inherit the environment) stay inside `Data\Cache\Temp`.
pub fn use_private_temp(p: &Paths) {
    if std::fs::create_dir_all(&p.temp).is_ok() {
        std::env::set_var("TEMP", &p.temp);
        std::env::set_var("TMP", &p.temp);
    }
}

/// Remove temp files older than an hour (only inside CAPTURE's own temp folder).
pub fn clean_temp(p: &Paths) {
    let hour = std::time::Duration::from_secs(3600);
    for e in std::fs::read_dir(&p.temp).into_iter().flatten().flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > hour);
        if old {
            let path = e.path();
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

/// Send this process's stderr (and so the engine's, which inherits it) to `Logs\capture.log`
/// when there is no console or pipe to write to, as in every normal launch. The previous log
/// is kept as `capture.old.log` once the current one passes 2 MB.
pub fn init_logging(p: &Paths) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_CHAR, FILE_TYPE_DISK, FILE_TYPE_PIPE};
    use windows::Win32::System::Console::{GetConsoleMode, GetStdHandle, SetStdHandle, CONSOLE_MODE, STD_ERROR_HANDLE};
    // Keep a real destination (a console, a pipe from a test harness, a redirect to a file).
    // No handle, or the NUL device that launchers often pass, means the log would be lost.
    if let Ok(h) = unsafe { GetStdHandle(STD_ERROR_HANDLE) } {
        if !h.is_invalid() && !h.0.is_null() {
            let kind = unsafe { GetFileType(h) };
            let console = kind == FILE_TYPE_CHAR && unsafe { GetConsoleMode(h, &mut CONSOLE_MODE::default()) }.is_ok();
            if console || kind == FILE_TYPE_PIPE || kind == FILE_TYPE_DISK {
                return;
            }
        }
    }
    let _ = std::fs::create_dir_all(&p.logs);
    let log = p.logs.join("capture.log");
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > 2_000_000) {
        let _ = std::fs::rename(&log, p.logs.join("capture.old.log"));
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&log) else { return };
    let h = HANDLE(file.as_raw_handle());
    if unsafe { SetStdHandle(STD_ERROR_HANDLE, h) }.is_ok() {
        std::mem::forget(file); // stays open for the life of the process
    }
}

pub fn timestamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("capture-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn layout_is_relative_to_the_app_folder() {
        let p = Paths::layout(Path::new(r"E:\Tools\CAPTURE"), Path::new(r"E:\Tools\CAPTURE\Data"), false);
        assert_eq!(p.worker_exe(), Path::new(r"E:\Tools\CAPTURE\Runtime\capture-worker.exe"));
        assert_eq!(p.settings, Path::new(r"E:\Tools\CAPTURE\Data\Settings"));
        assert_eq!(p.webview, Path::new(r"E:\Tools\CAPTURE\Data\Cache\WebView"));
        assert!(!p.installed);
    }

    #[test]
    fn writable_app_folder_keeps_data_beside_the_exe() {
        let app = temp("resolve");
        match resolve(&app) {
            Resolved::Ready(p) => {
                assert_eq!(p.data, app.join("Data"));
                assert!(!p.data_fallback);
            }
            Resolved::NeedsConsent { .. } => panic!("a writable folder must not need consent"),
        }
        std::fs::remove_dir_all(&app).unwrap();
    }

    #[test]
    fn pre_0_5_data_folder_is_migrated_and_backed_up() {
        let app = temp("legacy");
        let old = app.join("data");
        std::fs::create_dir_all(old.join("webview").join("EBWebView")).unwrap();
        std::fs::write(old.join("settings.json"), br#"{"replay_secs": 300}"#).unwrap();
        std::fs::write(old.join("settings.json.bak"), br#"{"replay_secs": 200}"#).unwrap();
        normalize_data_case(&app);
        let p = Paths::layout(&app, &app.join("Data"), false);
        assert!(migrate_legacy(&p.data, &p));
        assert_eq!(std::fs::read(p.settings.join("settings.json")).unwrap(), br#"{"replay_secs": 300}"#);
        assert!(p.settings.join("settings.json.bak").is_file());
        assert!(p.settings.join("Backups").join("settings-v1-from-0.4.json").is_file());
        assert!(p.webview.join("EBWebView").is_dir(), "WebView2 profile kept");
        assert!(!p.data.join("settings.json").exists());
        assert!(!migrate_legacy(&p.data, &p), "second run does nothing");
        std::fs::remove_dir_all(&app).unwrap();
    }

    #[test]
    fn zip_preview_folder_is_recognised() {
        assert!(running_from_zip_preview(Path::new(r"C:\Users\x\AppData\Local\Temp\Temp1_CAPTURE-0.5.0-portable.zip\CAPTURE.exe\..")));
        assert!(!running_from_zip_preview(Path::new(r"D:\Apps\CAPTURE")));
    }
}
