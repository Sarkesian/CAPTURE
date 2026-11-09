//! Updating CAPTURE from a release zip: the same file updates a portable folder and an
//! installed copy, because both hold the same payload.
//!
//! 1. **Stage** (in the running app): the zip is extracted to `Data\Cache\Update\Staged` and
//!    every file is checked against the release's `manifest.json` (SHA-256 and size).
//!    Nothing in the app folder changes.
//! 2. **Apply** (in a separate process: the staged copy's own `CAPTURE.exe --apply-update`,
//!    started after settings are backed up and only when no recording is running). It waits
//!    for the running CAPTURE to exit so no program file is in use, moves the current program
//!    files to `Update\Rollback`, copies the new ones in and verifies each one. On any failure
//!    it moves the old files back. A journal (`Update\journal.json`) records each step, so an
//!    interrupted update is finished from the staged copy on the next start.
//! 3. **Clean up** (in the new app): the staged copy and the rollback files are deleted, which
//!    also removes program files that the new release no longer has.
//!
//! `Data\` (settings, logs) is never part of an update, and neither are recordings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::Paths;

pub const MANIFEST: &str = "manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub product: String,
    pub version: String,
    pub files: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Relative path with backslashes, e.g. `Runtime\capture-worker.exe`.
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Applying,
    Done,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub state: State,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub message: String,
}

/// Program files of 0.3–0.4 builds, which had no manifest.
const LEGACY_FILES: [&str; 10] = [
    "CAPTURE.exe",
    "capture-worker.exe",
    "avcodec-62.dll",
    "avformat-62.dll",
    "avutil-60.dll",
    "swresample-6.dll",
    "swscale-9.dll",
    "QUICKSTART.md",
    "licenses\\FFmpeg-LICENSE.txt",
    "licenses\\THIRD_PARTY.md",
];

pub fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let b = std::fs::read(dir.join(MANIFEST)).map_err(|_| "This is not a CAPTURE release (manifest.json is missing).".to_string())?;
    let m: Manifest = serde_json::from_slice(b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&b)).map_err(|e| format!("The release manifest is damaged: {e}"))?;
    if m.product != "CAPTURE" || m.files.is_empty() {
        return Err("This is not a CAPTURE release.".into());
    }
    for f in &m.files {
        if !safe_rel(&f.path) {
            return Err(format!("The release manifest lists an unsafe path: {}", f.path));
        }
    }
    Ok(m)
}

/// A relative path that stays inside its folder and never touches `Data`.
fn safe_rel(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && path.is_relative()
        && path.components().all(|c| matches!(c, std::path::Component::Normal(_)))
        && !path.components().next().is_some_and(|c| c.as_os_str().eq_ignore_ascii_case("Data"))
}

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn check_file(p: &Path, e: &Entry) -> Result<(), String> {
    let len = std::fs::metadata(p).map_err(|_| format!("{} is missing", e.path))?.len();
    if len != e.size {
        return Err(format!("{} has the wrong size", e.path));
    }
    let h = sha256_file(p).map_err(|err| format!("{}: {err}", e.path))?;
    if !h.eq_ignore_ascii_case(&e.sha256) {
        return Err(format!("{} is damaged (checksum mismatch)", e.path));
    }
    Ok(())
}

pub fn verify_dir(dir: &Path, m: &Manifest) -> Result<(), String> {
    m.files.iter().try_for_each(|e| check_file(&dir.join(&e.path), e))
}

/// Compare dotted versions numerically (`0.10.0` > `0.9.3`).
pub fn cmp_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| v.split(['.', '-']).map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parse(a).cmp(&parse(b))
}

fn staged_dir(p: &Paths) -> PathBuf {
    p.update.join("Staged")
}
fn rollback_dir(update: &Path) -> PathBuf {
    update.join("Rollback")
}
fn journal_path(update: &Path) -> PathBuf {
    update.join("journal.json")
}

fn write_journal(update: &Path, j: &Journal) {
    let _ = std::fs::create_dir_all(update);
    let tmp = update.join("journal.json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(j).unwrap()).is_ok() {
        let _ = std::fs::rename(tmp, journal_path(update));
    }
}

pub fn read_journal(update: &Path) -> Option<Journal> {
    serde_json::from_slice(&std::fs::read(journal_path(update)).ok()?).ok()
}

/// Extract a release zip into `Update\Staged` and verify it. Returns the staged manifest.
pub fn stage(zip_path: &Path, p: &Paths) -> Result<Manifest, String> {
    let staged = staged_dir(p);
    if read_journal(&p.update).is_some_and(|j| j.state == State::Applying) {
        return Err("Another update is still being installed. Restart CAPTURE to finish it.".into());
    }
    let _ = std::fs::remove_dir_all(&p.update);
    std::fs::create_dir_all(&staged).map_err(|e| format!("Could not prepare the update folder: {e}"))?;
    let file = std::fs::File::open(zip_path).map_err(|e| format!("Could not open {}: {e}", zip_path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "This file is not a zip archive.".to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("The zip is damaged: {e}"))?;
        let Some(rel) = entry.enclosed_name() else { return Err("The zip contains an unsafe path.".into()) };
        let out = staged.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| format!("Could not extract the update: {e}"))?;
        std::io::copy(&mut entry, &mut f).map_err(|e| format!("Could not extract the update: {e}"))?;
    }
    // Accept a zip with the files at the top level or inside one folder.
    let root = if staged.join(MANIFEST).is_file() {
        staged.clone()
    } else {
        let dirs: Vec<PathBuf> = std::fs::read_dir(&staged).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|d| d.join(MANIFEST).is_file()).collect();
        match dirs.as_slice() {
            [one] => one.clone(),
            _ => return Err("This zip does not contain a CAPTURE release.".into()),
        }
    };
    if root != staged {
        // Move the payload up so Staged\ always holds it directly.
        let tmp = p.update.join("Staged.tmp");
        std::fs::rename(&root, &tmp).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&staged);
        std::fs::rename(&tmp, &staged).map_err(|e| e.to_string())?;
    }
    let m = read_manifest(&staged)?;
    verify_dir(&staged, &m).map_err(|e| format!("The update is incomplete or damaged: {e}. Download it again."))?;
    if !m.files.iter().any(|f| f.path.eq_ignore_ascii_case("CAPTURE.exe")) {
        return Err("This release has no CAPTURE.exe.".into());
    }
    Ok(m)
}

/// Start the staged copy's updater, which waits for this process to exit.
pub fn launch_apply(p: &Paths) -> Result<(), String> {
    let staged = staged_dir(p);
    let exe = staged.join("CAPTURE.exe");
    std::process::Command::new(&exe)
        .arg("--apply-update")
        .arg(&p.app_dir)
        .arg(&p.data)
        .arg(std::process::id().to_string())
        .current_dir(&staged)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start the updater: {e}"))
}

fn wait_for_pid(pid: u32, secs: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let _ = WaitForSingleObject(h, secs * 1000);
            let _ = CloseHandle(h);
        }
    }
}

/// Retry a file operation while another process (the exiting app, a virus scanner) still
/// holds the file.
fn retry<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut last = None;
    let tries = if cfg!(test) { 2 } else { 60 };
    for _ in 0..tries {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(last.unwrap())
}

/// The program files an app folder holds now, by its manifest (or the 0.3–0.4 list).
fn current_files(app_dir: &Path) -> (String, Vec<String>) {
    match read_manifest(app_dir) {
        Ok(m) => (m.version, m.files.into_iter().map(|f| f.path).chain([MANIFEST.to_string()]).collect()),
        Err(_) => ("0.4".into(), LEGACY_FILES.iter().map(|s| s.to_string()).collect()),
    }
}

/// Swap the program files in `app_dir` for the verified ones in `staged`. Idempotent: running
/// it again after an interruption finishes the job.
pub fn apply(app_dir: &Path, update: &Path, staged: &Path) -> Result<String, String> {
    let new = read_manifest(staged)?;
    verify_dir(staged, &new)?;
    let rollback = rollback_dir(update);
    let resumed = read_journal(update).filter(|j| j.state == State::Applying);
    let from = resumed.as_ref().map(|j| j.from.clone()).unwrap_or_else(|| current_files(app_dir).0);
    write_journal(update, &Journal { state: State::Applying, from: from.clone(), to: new.version.clone(), message: String::new() });

    let result = (|| -> Result<(), String> {
        // 1. Move every current program file aside (keep the first backup if resuming).
        let mut names = current_files(app_dir).1;
        names.extend(new.files.iter().map(|f| f.path.clone()));
        names.push(MANIFEST.into());
        names.sort_by_key(|n| n.to_ascii_lowercase());
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        for n in &names {
            let src = app_dir.join(n);
            let dst = rollback.join(n);
            if src.is_file() && !dst.exists() {
                std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
                retry(|| std::fs::rename(&src, &dst)).map_err(|e| format!("{n} is in use ({e})"))?;
            }
        }
        // 2. Copy the new files in and verify each one.
        for e in &new.files {
            let dst = app_dir.join(&e.path);
            std::fs::create_dir_all(dst.parent().unwrap()).map_err(|err| err.to_string())?;
            retry(|| std::fs::copy(staged.join(&e.path), &dst)).map_err(|err| format!("Could not write {}: {err}", e.path))?;
            check_file(&dst, e)?;
        }
        std::fs::copy(staged.join(MANIFEST), app_dir.join(MANIFEST)).map_err(|e| e.to_string())?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            // Folders the old layout used and the new one does not (e.g. `licenses\` at the top).
            for d in ["licenses"] {
                let _ = std::fs::remove_dir(app_dir.join(d));
            }
            write_journal(update, &Journal { state: State::Done, from, to: new.version.clone(), message: String::new() });
            Ok(new.version)
        }
        Err(msg) => {
            restore(app_dir, &rollback, &new);
            write_journal(update, &Journal { state: State::RolledBack, from, to: new.version, message: msg.clone() });
            Err(msg)
        }
    }
}

/// Put the backed-up program files back and remove files that only the new release had.
fn restore(app_dir: &Path, rollback: &Path, new: &Manifest) {
    for e in &new.files {
        if !rollback.join(&e.path).exists() {
            let _ = std::fs::remove_file(app_dir.join(&e.path));
        }
    }
    for f in walk(rollback) {
        let rel = f.strip_prefix(rollback).unwrap();
        let dst = app_dir.join(rel);
        let _ = std::fs::create_dir_all(dst.parent().unwrap());
        let _ = std::fs::remove_file(&dst);
        let _ = retry(|| std::fs::rename(&f, &dst));
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// `CAPTURE.exe --apply-update <app_dir> <data_dir> <pid>`, run from the staged copy.
pub fn apply_main(args: &[String]) -> i32 {
    let (Some(app_dir), Some(data), Some(pid)) = (args.first(), args.get(1), args.get(2).and_then(|p| p.parse::<u32>().ok())) else {
        return 2;
    };
    let app_dir = PathBuf::from(app_dir);
    let update = PathBuf::from(data).join("Cache").join("Update");
    let staged = update.join("Staged");
    wait_for_pid(pid, 120);
    let r = apply(&app_dir, &update, &staged);
    if let Ok(v) = &r {
        update_uninstall_entry(&app_dir, v);
    }
    let exe = app_dir.join("CAPTURE.exe");
    let _ = std::process::Command::new(&exe).arg("--after-update").current_dir(&app_dir).spawn();
    i32::from(r.is_err())
}

/// An installed copy's "Apps & features" entry shows the new version.
fn update_uninstall_entry(app_dir: &Path, version: &str) {
    use windows::core::{w, HSTRING};
    use windows::Win32::System::Registry::*;
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\CAPTURE");
    unsafe {
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let ok = RegGetValueW(HKEY_CURRENT_USER, key, w!("InstallLocation"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)).is_ok();
        let loc = String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]);
        if ok && crate::paths::same_path(Path::new(&loc), app_dir) {
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key, w!("DisplayVersion"), REG_SZ.0, Some(HSTRING::from(version).as_ptr() as *const _), ((version.len() + 1) * 2) as u32);
        }
    }
}

/// At start-up: report a finished update and remove its leftovers, or finish an interrupted
/// one. Returns a message for the UI, if any.
pub enum Startup {
    Nothing,
    Updated { from: String, to: String },
    Failed(String),
    /// An update was interrupted; the staged copy has been started to finish it. Exit now.
    Resume,
}

pub fn on_startup(p: &Paths) -> Startup {
    let Some(j) = read_journal(&p.update) else {
        return Startup::Nothing;
    };
    match j.state {
        State::Applying => {
            let exe = staged_dir(p).join("CAPTURE.exe");
            if exe.is_file() && launch_apply(p).is_ok() {
                Startup::Resume
            } else {
                Startup::Failed("An update was interrupted and could not be finished. Your settings are safe; download CAPTURE again to repair it.".into())
            }
        }
        State::Done => {
            cleanup_later(p);
            Startup::Updated { from: j.from, to: j.to }
        }
        State::RolledBack => {
            cleanup_later(p);
            Startup::Failed(format!("The update to {} could not be installed, so CAPTURE {} was kept: {}", j.to, j.from, j.message))
        }
    }
}

/// Delete `Update\` once the staged updater process has exited.
fn cleanup_later(p: &Paths) {
    let dir = p.update.clone();
    std::thread::spawn(move || {
        for _ in 0..30 {
            if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("capture-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn release(dir: &Path, version: &str, files: &[(&str, &str)]) {
        let mut entries = Vec::new();
        for (path, body) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
            entries.push(Entry { path: path.to_string(), sha256: sha256_file(&p).unwrap(), size: body.len() as u64 });
        }
        let m = Manifest { product: "CAPTURE".into(), version: version.into(), files: entries };
        std::fs::write(dir.join(MANIFEST), serde_json::to_vec(&m).unwrap()).unwrap();
    }

    #[test]
    fn update_replaces_program_files_and_keeps_data() {
        let root = temp("apply");
        let app = root.join("CAPTURE");
        release(&app, "0.5.0", &[("CAPTURE.exe", "ui-old"), ("Runtime\\capture-worker.exe", "w-old"), ("Runtime\\old.dll", "gone")]);
        std::fs::create_dir_all(app.join("Data").join("Settings")).unwrap();
        std::fs::write(app.join("Data").join("Settings").join("settings.json"), "{}").unwrap();
        let update = app.join("Data").join("Cache").join("Update");
        let staged = update.join("Staged");
        release(&staged, "0.5.1", &[("CAPTURE.exe", "ui-new"), ("Runtime\\capture-worker.exe", "w-new"), ("Resources\\QUICKSTART.md", "guide")]);

        assert_eq!(apply(&app, &update, &staged).unwrap(), "0.5.1");
        assert_eq!(std::fs::read_to_string(app.join("CAPTURE.exe")).unwrap(), "ui-new");
        assert_eq!(read_manifest(&app).unwrap().version, "0.5.1");
        assert!(!app.join("Runtime").join("old.dll").exists(), "obsolete file not left in the app folder");
        assert!(app.join("Data").join("Settings").join("settings.json").exists());
        assert_eq!(read_journal(&update).unwrap().state, State::Done);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn damaged_release_is_rejected_and_nothing_changes() {
        let root = temp("damaged");
        let app = root.join("CAPTURE");
        release(&app, "0.5.0", &[("CAPTURE.exe", "ui-old")]);
        let update = app.join("Data").join("Cache").join("Update");
        let staged = update.join("Staged");
        release(&staged, "0.5.1", &[("CAPTURE.exe", "ui-new")]);
        std::fs::write(staged.join("CAPTURE.exe"), "tampered").unwrap();
        assert!(apply(&app, &update, &staged).is_err());
        assert_eq!(std::fs::read_to_string(app.join("CAPTURE.exe")).unwrap(), "ui-old");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failed_copy_rolls_back() {
        let root = temp("rollback");
        let app = root.join("CAPTURE");
        release(&app, "0.5.0", &[("CAPTURE.exe", "ui-old"), ("Runtime\\a.dll", "a-old")]);
        let update = app.join("Data").join("Cache").join("Update");
        let staged = update.join("Staged");
        release(&staged, "0.5.1", &[("CAPTURE.exe", "ui-new"), ("Runtime\\a.dll", "a-new")]);
        // A folder where a new file must go makes the copy fail after the old files were moved.
        let mut m = read_manifest(&staged).unwrap();
        std::fs::write(staged.join("Runtime").join("zz-blocked"), "").unwrap();
        m.files.push(Entry { path: "Runtime\\zz-blocked".into(), sha256: sha256_file(&staged.join("Runtime").join("zz-blocked")).unwrap(), size: 0 });
        std::fs::write(staged.join(MANIFEST), serde_json::to_vec(&m).unwrap()).unwrap();
        std::fs::create_dir_all(app.join("Runtime").join("zz-blocked").join("x")).unwrap();
        let err = apply(&app, &update, &staged).unwrap_err();
        assert!(err.contains("zz-blocked"), "{err}");
        assert_eq!(read_journal(&update).unwrap().state, State::RolledBack);
        assert_eq!(std::fs::read_to_string(app.join("CAPTURE.exe")).unwrap(), "ui-old");
        assert_eq!(std::fs::read_to_string(app.join("Runtime").join("a.dll")).unwrap(), "a-old");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unsafe_manifest_paths_are_refused() {
        assert!(!safe_rel("..\\evil.exe"));
        assert!(!safe_rel("C:\\Windows\\x.dll"));
        assert!(!safe_rel("Data\\Settings\\settings.json"));
        assert!(safe_rel("Runtime\\capture-worker.exe"));
    }

    #[test]
    fn versions_compare_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_versions("0.10.0", "0.9.3"), Greater);
        assert_eq!(cmp_versions("0.5.0", "0.5.0"), Equal);
        assert_eq!(cmp_versions("0.4.1", "0.5.0"), Less);
    }
}
