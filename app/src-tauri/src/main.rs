#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! CAPTURE UI process: window, tray and settings. All capture work happens in
//! `capture-worker.exe`, supervised by [`worker::Worker`].

mod library;
mod owned;
mod paths;
mod settings;
mod update;
mod worker;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use capture_protocol::{Command, EngineState, Event, Message, Status};
use serde::Serialize;
use settings::AppSettings;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use worker::Worker;

struct Tray {
    icon: TrayIcon,
    status: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
    save: MenuItem<tauri::Wry>,
}

struct AppState {
    worker: Arc<Worker>,
    settings: Mutex<AppSettings>,
    paths: paths::Paths,
    last_state: Mutex<Option<EngineState>>,
    engine_down: Mutex<Option<String>>,
    tray: Mutex<Option<Tray>>,
    /// One-time messages for the UI (update finished or failed).
    notices: Mutex<Vec<String>>,
    /// Version of a verified update waiting in `Data\Cache\Update\Staged`.
    staged: Mutex<Option<String>>,
}

/// What this copy of CAPTURE is and where it keeps its files (Settings › Advanced).
#[derive(Serialize)]
struct AppInfo {
    version: String,
    installed: bool,
    app_dir: String,
    data_dir: String,
    logs_dir: String,
    data_fallback: bool,
}

/// A saved folder that is not usable on this PC right now (drive unplugged, CAPTURE moved to
/// another computer). The setting is kept; the user decides whether to change it.
#[derive(Serialize)]
struct PathIssue {
    which: &'static str,
    path: String,
}

#[derive(Serialize)]
struct Bootstrap {
    settings: AppSettings,
    data_dir: String,
    default_root: String,
    last_state: Option<EngineState>,
    engine_down: Option<String>,
    app: AppInfo,
    path_issues: Vec<PathIssue>,
    notices: Vec<String>,
}

fn app_info(p: &paths::Paths) -> AppInfo {
    AppInfo {
        version: env!("CAPTURE_VERSION").to_string(),
        installed: p.installed,
        app_dir: p.app_dir.to_string_lossy().into_owned(),
        data_dir: p.data.to_string_lossy().into_owned(),
        logs_dir: p.logs.to_string_lossy().into_owned(),
        data_fallback: p.data_fallback,
    }
}

/// The drive (or network share) a folder lives on is present. The folder itself may not exist
/// yet: CAPTURE creates it when it first saves.
fn location_available(dir: &str) -> bool {
    Path::new(dir.trim()).ancestors().filter(|a| !a.as_os_str().is_empty()).last().is_some_and(|root| root.exists())
}

fn path_issues(s: &AppSettings) -> Vec<PathIssue> {
    let mut v = Vec::new();
    if !s.capture_root.trim().is_empty() && !location_available(&s.capture_root) {
        v.push(PathIssue { which: "capture_root", path: s.capture_root.clone() });
    }
    if let Some(b) = s.buffer_root.as_deref().filter(|b| !b.trim().is_empty()) {
        if !location_available(b) {
            v.push(PathIssue { which: "buffer_root", path: b.to_string() });
        }
    }
    v
}

fn apply(state: &AppState) {
    let s = state.settings.lock().unwrap().clone();
    let _ = state.worker.send(Command::Configure(s.engine_config()));
    let _ = state.worker.send(Command::SetReplay { enabled: s.replay_enabled });
}

fn persist(state: &AppState, s: &AppSettings) -> Result<(), String> {
    settings::save(&state.paths.settings, s).map_err(|e| format!("Could not save settings: {e}"))
}

/// The one way settings change: apply `f` to the current settings and save, all under the
/// settings lock, so two writers (a window, the tray, the mute hotkey) can never undo each
/// other or write the file out of order. Memory changes only once the file is saved.
fn mutate(state: &AppState, f: impl FnOnce(&mut AppSettings)) -> Result<AppSettings, String> {
    let mut g = state.settings.lock().unwrap();
    let mut s = g.clone();
    f(&mut s);
    persist(state, &s)?;
    *g = s.clone();
    Ok(s)
}

/// `current` with the fields present in `patch` replaced (a window sends only what it changed).
fn merged(current: &AppSettings, patch: &serde_json::Value) -> Result<AppSettings, String> {
    let mut v = serde_json::to_value(current).map_err(|e| e.to_string())?;
    if let (Some(dst), Some(src)) = (v.as_object_mut(), patch.as_object()) {
        for (k, val) in src {
            dst.insert(k.clone(), val.clone());
        }
    }
    serde_json::from_value::<AppSettings>(v).map(AppSettings::sanitized).map_err(|e| format!("Invalid setting: {e}"))
}

#[tauri::command]
fn bootstrap(window: tauri::WebviewWindow, state: State<AppState>) -> Bootstrap {
    let s = state.settings.lock().unwrap().clone();
    // One-time notices belong to the main window, not the hidden overlay.
    let notices = if window.label() == "main" { std::mem::take(&mut *state.notices.lock().unwrap()) } else { Vec::new() };
    Bootstrap {
        path_issues: path_issues(&s),
        settings: s,
        data_dir: state.paths.data.to_string_lossy().into_owned(),
        default_root: settings::default_root(),
        last_state: state.last_state.lock().unwrap().clone(),
        engine_down: state.engine_down.lock().unwrap().clone(),
        app: app_info(&state.paths),
        notices,
    }
}

#[tauri::command]
fn check_paths(state: State<AppState>) -> Vec<PathIssue> {
    path_issues(&state.settings.lock().unwrap())
}

/// Open one of CAPTURE's own folders in Explorer.
#[tauri::command]
fn open_app_folder(state: State<AppState>, which: String) -> Result<(), String> {
    let p = &state.paths;
    let dir = match which.as_str() {
        "data" => &p.data,
        "logs" => &p.logs,
        "app" => &p.app_dir,
        "resources" => &p.resources,
        _ => return Err("Unknown folder".into()),
    };
    let _ = std::fs::create_dir_all(dir);
    std::process::Command::new("explorer").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct StagedUpdate {
    version: String,
    current: String,
    /// -1 older, 0 same, 1 newer than the running version.
    order: i8,
}

/// Check a downloaded release zip and get it ready to install. Changes nothing in the app folder.
#[tauri::command]
async fn stage_update(app: AppHandle, path: String) -> Result<StagedUpdate, String> {
    let state = app.state::<AppState>();
    let p = state.paths.clone();
    let m = tauri::async_runtime::spawn_blocking(move || update::stage(Path::new(&path), &p)).await.map_err(|e| e.to_string())??;
    let current = env!("CAPTURE_VERSION").to_string();
    let order = match update::cmp_versions(&m.version, &current) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    *state.staged.lock().unwrap() = Some(m.version.clone());
    Ok(StagedUpdate { version: m.version, current, order })
}

/// Install the staged update: back up settings, hand over to the updater and quit. Refused
/// while a recording is running or a clip is being saved, so nothing is interrupted.
#[tauri::command]
fn apply_update(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let Some(version) = state.staged.lock().unwrap().clone() else {
        return Err("Choose an update file first.".into());
    };
    if let Some(st) = state.last_state.lock().unwrap().as_ref() {
        if st.session_secs.is_some() {
            return Err("A recording is running. Stop it first, then install the update.".into());
        }
        if st.saves_in_progress > 0 {
            return Err("A clip is being saved. Try again in a moment.".into());
        }
    }
    let s = state.settings.lock().unwrap().clone();
    persist(&state, &s)?;
    let backups = state.paths.settings.join("Backups");
    let _ = std::fs::create_dir_all(&backups);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    std::fs::copy(state.paths.settings.join("settings.json"), backups.join(format!("settings-before-{version}-{stamp}.json")))
        .map_err(|e| format!("Could not back up settings before updating: {e}"))?;
    update::launch_apply(&state.paths)?;
    eprintln!("{} update: handing over to the {version} updater", paths::timestamp());
    quit(&app);
    Ok(())
}

/// `patch` holds only the fields a window changed; they are merged into the current settings
/// here, so a change made meanwhile elsewhere (mute hotkey, tray, the other window) is kept.
#[tauri::command]
fn update_settings(app: AppHandle, state: State<AppState>, patch: serde_json::Value) -> Result<AppSettings, String> {
    // Validate new folders first, without holding the lock (a slow drive must not stall the rest).
    let (before, candidate) = {
        let cur = state.settings.lock().unwrap().clone();
        let cand = merged(&cur, &patch)?;
        (cur, cand)
    };
    if candidate.capture_root != before.capture_root {
        validate_destination(&candidate.capture_root)?;
    }
    if let Some(b) = candidate.buffer_root.as_deref().filter(|b| !b.trim().is_empty()) {
        if Some(b) != before.buffer_root.as_deref() {
            validate_destination(b)?;
        }
    }
    let mut replay_changed = false;
    let mut err = None;
    let s = mutate(&state, |cur| match merged(cur, &patch) {
        Ok(next) => {
            replay_changed = cur.replay_enabled != next.replay_enabled;
            *cur = next;
        }
        Err(e) => err = Some(e),
    })?;
    if let Some(e) = err {
        return Err(e);
    }
    // Saved: every window, the tray and the engine follow. While the engine is down the send
    // fails, which is fine — it gets the saved configuration when it restarts (`apply`).
    let _ = state.worker.send(Command::Configure(s.engine_config()));
    if replay_changed {
        let _ = state.worker.send(Command::SetReplay { enabled: s.replay_enabled });
    }
    refresh_tray(&app);
    allow_media(&app, &s);
    let _ = app.emit("settings", &s); // keep every window (main + overlay) in sync
    Ok(s)
}

/// A new capture folder must be creatable and writable before it is used. Existing recordings
/// stay where they are; nothing is moved.
fn validate_destination(root: &str) -> Result<(), String> {
    let p = PathBuf::from(root);
    if !p.is_absolute() {
        return Err(r"Choose a full folder path (for example G:\Videos\Captures).".into());
    }
    std::fs::create_dir_all(&p).map_err(|e| format!("CAPTURE can't use {root}: {e}"))?;
    let probe = p.join(".capture-write-test");
    std::fs::write(&probe, b"ok").map_err(|e| format!("CAPTURE can't save files in {root}: {e}"))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// Let the webviews play/view files inside the capture folder (clips, screenshots) and the
/// temporary-footage folder (review snapshots live there when it is set) only.
fn allow_media(app: &AppHandle, s: &AppSettings) {
    if !s.capture_root.trim().is_empty() {
        let _ = app.asset_protocol_scope().allow_directory(&s.capture_root, true);
    }
    if let Some(b) = s.buffer_root.as_deref().filter(|b| !b.trim().is_empty()) {
        // `<temporary-footage folder>\Buffer`, as the engine's `storage::buffer_dir_for`.
        let _ = app.asset_protocol_scope().allow_directory(Path::new(b.trim()).join("Buffer"), true);
    }
}

const RUN_KEY: windows::core::PCWSTR = windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// The command line of CAPTURE's per-user Run entry, if there is one. This (not the saved
/// setting) is the truth for "Start with Windows".
fn startup_entry() -> Option<String> {
    use windows::core::w;
    use windows::Win32::System::Registry::*;
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("CAPTURE"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    r.is_ok().then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
}

/// The executable a Run entry starts (`"C:\path\CAPTURE.exe" --tray` -> `C:\path\CAPTURE.exe`).
fn entry_exe(cmd: &str) -> PathBuf {
    let t = cmd.trim();
    PathBuf::from(match t.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => t.split(" --").next().unwrap_or(t),
    })
}

/// At launch: make the toggle show what Windows will actually do, and keep a working entry
/// working after CAPTURE was moved to a new folder (the old exe is gone). An entry that points
/// to another existing copy of CAPTURE is left alone, and so is one whose drive is simply not
/// connected right now (a portable copy on a USB drive): CAPTURE never takes it over.
/// Exception: `upgraded_from` is the earlier version folder whose settings this copy just
/// imported (side-by-side update). Its entry moves to this copy, or Windows would keep starting
/// the old version at sign-in, and single-instance would then hand this copy's launch to it.
fn reconcile_startup(s: &mut AppSettings, upgraded_from: Option<&Path>) -> bool {
    let entry = startup_entry();
    let mut changed = s.start_with_windows != entry.is_some();
    s.start_with_windows = entry.is_some();
    if let (Some(cmd), Ok(me)) = (entry, std::env::current_exe()) {
        let target = entry_exe(&cmd);
        let drive_present = target.ancestors().filter(|a| !a.as_os_str().is_empty()).last().is_some_and(|r| r.exists());
        let from_old_version = upgraded_from.is_some_and(|old| target.parent().is_some_and(|d| paths::same_path(d, old)));
        if ((drive_present && !target.exists()) || from_old_version) && !paths::same_path(&target, &me) && write_startup(true, &me).is_ok() {
            eprintln!("startup entry pointed to {cmd}; now {}", me.display());
            changed = true;
        }
    }
    changed
}

fn write_startup(enabled: bool, exe: &Path) -> Result<(), String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::*;
    let value = format!("\"{}\" --tray", exe.display());
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None)
            .ok()
            .map_err(|e| format!("Could not change startup setting: {e}"))?;
        let r = if enabled {
            let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
            RegSetValueExW(key, w!("CAPTURE"), None, REG_SZ, Some(std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2)))
        } else {
            let r = RegDeleteValueW(key, w!("CAPTURE"));
            if r == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND { windows::Win32::Foundation::ERROR_SUCCESS } else { r }
        };
        let _ = RegCloseKey(key);
        r.ok().map_err(|e| format!("Could not change startup setting: {e}"))
    }
}

/// Start with Windows: a per-user Run entry that starts CAPTURE in the tray. Removing it
/// leaves nothing behind.
#[tauri::command]
fn set_start_with_windows(app: AppHandle, state: State<AppState>, enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    write_startup(enabled, &exe)?;
    // Report what Windows now has, not what was asked for.
    let actual = startup_entry().is_some();
    if actual != enabled {
        return Err("Windows did not accept the startup change. Check that no policy blocks startup apps.".into());
    }
    let s = mutate(&state, |g| g.start_with_windows = enabled)?;
    let _ = app.emit("settings", &s);
    Ok(())
}

#[tauri::command]
fn engine_command(state: State<AppState>, command: Command) -> Result<u64, String> {
    if matches!(command, Command::Configure(_) | Command::Shutdown) {
        return Err("Not allowed from the UI".into());
    }
    // File paths the engine opens (FFmpeg also accepts URLs such as `http:`) must be CAPTURE
    // recordings inside the capture folder.
    if let Command::ExportStems { path } | Command::ClipInfo { path } = &command {
        recording_path(&state, path)?;
    }
    state.worker.send(command)
}

#[tauri::command]
fn toggle_overlay(app: AppHandle) {
    toggle_overlay_window(&app);
}

#[tauri::command]
fn hide_overlay(app: AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.hide();
        set_page_visible(&w, false);
    }
}

/// Bring the full settings window forward, optionally on a given tab.
#[tauri::command]
fn show_main(app: AppHandle, tab: Option<String>) {
    hide_overlay(app.clone());
    show_window(&app);
    if let Some(t) = tab {
        let _ = app.emit_to("main", "goto", t);
    }
}

/// Move a clip to the Recycle Bin (recoverable). Only files inside the capture folder.
#[tauri::command]
fn delete_clip(state: State<AppState>, path: String) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::*;
    let p = inside_root(&state, &path)?;
    if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4") || e.eq_ignore_ascii_case("png")) {
        return Err("Only clips, sessions and screenshots can be deleted from here.".into());
    }
    let mut from: Vec<u16> = p.as_os_str().to_string_lossy().encode_utf16().collect();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(format!("Could not move the clip to the Recycle Bin (code {r})."));
    }
    let mut side = p.as_os_str().to_owned();
    side.push(".markers.json");
    let _ = std::fs::remove_file(side);
    Ok(())
}

fn toggle_overlay_window(app: &AppHandle) {
    let Some(w) = app.get_webview_window("overlay") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        set_page_visible(&w, false);
        return;
    }
    // Centre on the monitor under the mouse (the one being played on).
    if let Ok(cursor) = app.cursor_position() {
        if let Ok(Some(mon)) = app.monitor_from_point(cursor.x, cursor.y) {
            if let Ok(size) = w.outer_size() {
                let (mp, ms) = (mon.position(), mon.size());
                let x = mp.x + (ms.width as i32 - size.width as i32) / 2;
                let y = mp.y + (ms.height as i32 - size.height as i32) / 2;
                let _ = w.set_position(PhysicalPosition::new(x, y));
            }
        }
    }
    let _ = w.show();
    set_page_visible(&w, true);
    let _ = w.set_focus();
    let _ = app.emit_to("overlay", "overlay_shown", ());
}

#[tauri::command]
fn set_replay(app: AppHandle, state: State<AppState>, enabled: bool) -> Result<(), String> {
    set_replay_inner(&app, &state, enabled)
}

fn set_replay_inner(app: &AppHandle, state: &AppState, enabled: bool) -> Result<(), String> {
    // Saved first: if the file cannot be written, nothing claims replay is on (or off).
    let s = mutate(state, |g| g.replay_enabled = enabled)?;
    state.worker.send(Command::SetReplay { enabled })?;
    let _ = app.emit("settings", &s);
    refresh_tray(app);
    Ok(())
}

#[tauri::command]
fn save_replay(state: State<AppState>) -> Result<u64, String> {
    state.worker.send(Command::SaveReplay)
}

#[tauri::command]
fn list_targets(state: State<AppState>) -> Result<u64, String> {
    state.worker.send(Command::ListTargets)
}

#[tauri::command]
fn clear_temp_cache(state: State<AppState>) -> Result<u64, String> {
    state.worker.send(Command::ClearTempCache)
}

#[tauri::command]
fn list_clips(state: State<AppState>, kind: Option<String>) -> Vec<library::ClipEntry> {
    let root = PathBuf::from(&state.settings.lock().unwrap().capture_root);
    library::list(&root, kind.as_deref().unwrap_or("clips"), 1000)
}

#[tauri::command]
fn rename_clip(state: State<AppState>, path: String, name: String) -> Result<String, String> {
    let p = recording_path(&state, &path)?;
    library::rename(&p, &name).map(|t| t.to_string_lossy().into_owned())
}

/// Only paths inside the capture folder may be opened from the UI.
fn inside_root(state: &AppState, path: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(&state.settings.lock().unwrap().capture_root);
    let p = PathBuf::from(path);
    let canon = |x: &Path| std::fs::canonicalize(x).map_err(|_| format!("{} does not exist", x.display()));
    let (pc, rc) = (canon(&p)?, canon(&root)?);
    if pc.starts_with(&rc) {
        Ok(p)
    } else {
        Err("Only files in your capture folder can be opened from here.".into())
    }
}

/// An existing `.mp4`/`.png` file strictly inside the capture folder (never the folder itself).
fn recording_path(state: &AppState, path: &str) -> Result<PathBuf, String> {
    let p = inside_root(state, path)?;
    let ext_ok = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4") || e.eq_ignore_ascii_case("png"));
    if !ext_ok || !p.is_file() {
        return Err("Only recordings in your capture folder can be used here.".into());
    }
    Ok(p)
}

#[tauri::command]
fn open_path(state: State<AppState>, path: String) -> Result<(), String> {
    let p = inside_root(&state, &path)?;
    std::process::Command::new("explorer").arg(&p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_path(state: State<AppState>, path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let p = inside_root(&state, &path)?;
    std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_capture_folder(state: State<AppState>) -> Result<(), String> {
    let root = PathBuf::from(&state.settings.lock().unwrap().capture_root);
    std::fs::create_dir_all(&root).map_err(|e| format!("Cannot create {}: {e}", root.display()))?;
    std::process::Command::new("explorer").arg(&root).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    quit(&app);
}

fn quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.worker.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    let (app, worker) = (app.clone(), state.worker.clone());
    std::thread::spawn(move || {
        worker.shutdown(Duration::from_secs(30)); // lets in-flight saves finish
        app.exit(0);
    });
}

/// Make the native Windows 11 title bar part of the app: caption, caption text and border take
/// the app's own surface colours (dark or light, following Windows). Native behaviour — Snap
/// layouts, caption buttons, resize, double-click maximise, active/inactive state — is untouched,
/// because the frame stays the real system frame. On Windows 10 these attributes are ignored.
fn theme_frame(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::Graphics::Dwm::*;
    let Ok(hwnd) = window.hwnd() else { return };
    let dark = !matches!(window.theme(), Ok(tauri::Theme::Light));
    let rgb = |r: u32, g: u32, b: u32| COLORREF(r | (g << 8) | (b << 16));
    // Must match --chrome / --text-2 / --chrome-line in app.css.
    let (caption, text, border) = if dark { (rgb(0x12, 0x14, 0x18), rgb(0xA4, 0xAB, 0xB8), rgb(0x12, 0x14, 0x18)) } else { (rgb(0xEE, 0xF0, 0xF3), rgb(0x4A, 0x52, 0x60), rgb(0xEE, 0xF0, 0xF3)) };
    let h = HWND(hwnd.0);
    unsafe {
        let set = |attr: DWMWINDOWATTRIBUTE, v: &u32| {
            let _ = DwmSetWindowAttribute(h, attr, v as *const u32 as *const _, 4);
        };
        set(DWMWA_USE_IMMERSIVE_DARK_MODE, &(dark as u32));
        set(DWMWA_CAPTION_COLOR, &caption.0);
        set(DWMWA_TEXT_COLOR, &text.0);
        set(DWMWA_BORDER_COLOR, &border.0);
    }
}

/// Tell WebView2 whether its page is on screen. Hiding or minimising the native window alone
/// leaves the page "visible" (measured: `document.hidden` stayed false), so it kept animating and
/// kept live meters streaming. With this the page sees `document.hidden` and stops both.
fn set_page_visible(w: &tauri::WebviewWindow, visible: bool) {
    let _ = w.with_webview(move |wv| unsafe {
        let _ = wv.controller().SetIsVisible(visible);
    });
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        set_page_visible(&w, true);
        let _ = w.set_focus();
    }
}

fn fmt_secs(s: f64) -> String {
    let s = s.round() as u64;
    if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

fn refresh_tray(app: &AppHandle) {
    let state = app.state::<AppState>();
    let replay_on = state.settings.lock().unwrap().replay_enabled;
    let hotkey = state.settings.lock().unwrap().save_hotkey.clone();
    let st = state.last_state.lock().unwrap().clone();
    let down = state.engine_down.lock().unwrap().clone();
    let (line, active) = match (&down, &st) {
        (Some(d), _) => (format!("Needs attention: {d}"), false),
        (None, Some(s)) => match s.status {
            Status::Ready => ("Ready · replay is off".to_string(), false),
            Status::Starting => ("Starting…".to_string(), false),
            Status::Waiting => (s.attention.clone().unwrap_or_else(|| "Watching for games".into()), false),
            Status::Buffering => (
                format!("Buffering · {} · {}", s.target_name.clone().unwrap_or_default(), fmt_secs(s.buffered_secs)),
                true,
            ),
            Status::NeedsAttention => (format!("Needs attention: {}", s.attention.clone().unwrap_or_default()), false),
        },
        (None, None) => ("Starting…".to_string(), false),
    };
    let tray = state.tray.lock().unwrap();
    if let Some(t) = tray.as_ref() {
        let short: String = line.chars().take(120).collect();
        let _ = t.status.set_text(&short);
        let _ = t.icon.set_tooltip(Some(format!("CAPTURE — {}", short.chars().take(100).collect::<String>())));
        let _ = t.toggle.set_text(if replay_on { "Turn instant replay off" } else { "Turn instant replay on" });
        let _ = t.save.set_text(if hotkey.is_empty() { "Save replay".to_string() } else { format!("Save replay ({hotkey})") });
        let _ = t.save.set_enabled(replay_on);
        let png: &[u8] = if active { include_bytes!("../icons/tray-active.png") } else { include_bytes!("../icons/tray-idle.png") };
        if let Ok(img) = Image::from_bytes(png) {
            let _ = t.icon.set_icon(Some(img));
        }
    }
    drop(tray);
}

fn on_message(app: &AppHandle, m: Message) {
    let state = app.state::<AppState>();
    match &m.event {
        Event::HotkeyPressed { action: capture_protocol::HotkeyAction::ToggleMicMute } => {
            // Settings live here, so the UI process flips mute and pushes it to the engine.
            // Mute must follow the key even if the settings file can't be written right now.
            let s = mutate(&state, |g| g.mic_muted = !g.mic_muted).unwrap_or_else(|_| {
                let mut g = state.settings.lock().unwrap();
                g.mic_muted = !g.mic_muted;
                g.clone()
            });
            let _ = state.worker.send(Command::Configure(s.engine_config()));
            let _ = app.emit("settings", &s);
        }
        Event::HotkeyPressed { action: capture_protocol::HotkeyAction::ToggleOverlay } => {
            let a = app.clone();
            let _ = app.run_on_main_thread(move || toggle_overlay_window(&a));
        }
        Event::Levels { .. } => {
            // High-rate meter data: forward straight to the webviews, nothing else to do.
            let _ = app.emit("levels", &m.event);
            return;
        }
        Event::State(s) => {
            *state.last_state.lock().unwrap() = Some(s.clone());
            *state.engine_down.lock().unwrap() = None;
            refresh_tray(app);
        }
        _ => {}
    }
    let _ = app.emit("engine", &m);
}

fn build_tray(app: &AppHandle) -> tauri::Result<Tray> {
    let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
    let save = MenuItem::with_id(app, "save", "Save replay", false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Turn instant replay on", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show CAPTURE", true, None::<&str>)?;
    let overlay = MenuItem::with_id(app, "overlay", "Open overlay", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit CAPTURE", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &sep1, &save, &toggle, &overlay, &show, &sep2, &quit_i])?;
    let icon = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../icons/tray-idle.png"))?)
        .tooltip("CAPTURE")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| {
            let state = app.state::<AppState>();
            match ev.id().as_ref() {
                "save" => {
                    let _ = state.worker.send(Command::SaveReplay);
                }
                "toggle" => {
                    let on = state.settings.lock().unwrap().replay_enabled;
                    let _ = set_replay_inner(app, &state, !on);
                }
                "show" => show_window(app),
                "overlay" => toggle_overlay_window(app),
                "quit" => quit(app),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(Tray { icon, status, toggle, save })
}

const SINGLE_INSTANCE_MUTEX: windows::core::PCWSTR = windows::core::w!("app.capture.local-sim"); // tauri-plugin-single-instance: "<identifier>-sim"

fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

/// A native message box, for the few things that must be said before (or without) the app
/// window: no WebView2, a read-only folder, the uninstaller's questions.
fn message(title: &str, text: &str, style: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE) -> i32 {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(title), style | MB_SETFOREGROUND | MB_TOPMOST).0 }
}

fn instance_running() -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, SINGLE_INSTANCE_MUTEX) } {
        Ok(h) => {
            let _ = unsafe { CloseHandle(h) };
            true
        }
        Err(_) => false,
    }
}

/// WebView2 is part of Windows 11 and of current Windows 10. A fixed-version runtime shipped in
/// `Runtime\WebView2\` (optional, see scripts/package-portable.ps1) is used instead when present.
fn webview2_version(p: &paths::Paths) -> Option<String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
    use windows::core::{HSTRING, PCWSTR, PWSTR};
    let fixed = p.runtime.join("WebView2");
    let folder = fixed.join("msedgewebview2.exe").is_file().then(|| HSTRING::from(fixed.as_os_str()));
    if let Some(f) = &folder {
        std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", f.to_os_string());
    }
    let mut out = PWSTR::null();
    let arg = folder.as_ref().map(|f| PCWSTR(f.as_ptr())).unwrap_or(PCWSTR::null());
    unsafe {
        GetAvailableCoreWebView2BrowserVersionString(arg, &mut out).ok()?;
        if out.is_null() {
            return None;
        }
        let v = out.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(out.0 as *const _));
        v
    }
}

/// `--uninstall-ask-data` / `--uninstall-cleanup temp|all [--silent]`, run by the uninstaller
/// before it removes the program files. Works on CAPTURE's own recordings only (see owned.rs).
fn uninstall_main(args: &[String], app_dir: &Path) -> i32 {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let paths::Resolved::Ready(p) = paths::resolve(app_dir) else { return 0 };
    if !p.settings.join("settings.json").exists() {
        return 0; // never configured: CAPTURE never recorded anything
    }
    let s = settings::load(&p.settings, None);
    let buffer = s.buffer_root.as_deref().map(PathBuf::from);
    let scan = owned::scan(Path::new(&s.capture_root), buffer.as_deref());
    let silent = args.iter().any(|a| a == "--silent");
    if args.iter().any(|a| a == "--uninstall-ask-data") {
        if scan.recordings.is_empty() && scan.temporary.is_empty() {
            message("Uninstall CAPTURE", &owned::describe(&scan), MB_OK | MB_ICONINFORMATION);
            return IDNO.0;
        }
        let text = format!("{}\nRemove these files?", owned::describe(&scan));
        return message("Uninstall CAPTURE: remove recordings?", &text, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2);
    }
    let all = args.iter().position(|a| a == "--uninstall-cleanup").and_then(|i| args.get(i + 1)).is_some_and(|m| m == "all");
    let failed = owned::remove(&scan, all);
    eprintln!("uninstall cleanup ({}): {} item(s) could not be removed", if all { "all" } else { "temp" }, failed.len());
    if !failed.is_empty() && !silent {
        let list: Vec<String> = failed.iter().take(8).map(|(p, e)| format!("    {} ({e})", p.display())).collect();
        let more = if failed.len() > 8 { format!("\n    …and {} more", failed.len() - 8) } else { String::new() };
        message("Uninstall CAPTURE", &format!("These files could not be removed, probably because another program has them open. You can delete them yourself:\n\n{}{more}", list.join("\n")), MB_OK | MB_ICONWARNING);
    }
    i32::from(!failed.is_empty())
}

fn main() {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let args: Vec<String> = std::env::args().collect();
    // Updater mode: this exe is a staged release replacing the app's program files.
    if args.get(1).map(String::as_str) == Some("--apply-update") {
        std::process::exit(update::apply_main(&args[2..]));
    }
    let app_dir = paths::app_dir();
    if args.iter().any(|a| a == "--uninstall-ask-data" || a == "--uninstall-cleanup") {
        std::process::exit(uninstall_main(&args, &app_dir));
    }
    // CAPTURE already runs: hand this launch's arguments (a plain launch, `--tray`, `--quit`)
    // to it and exit, without touching any files.
    if instance_running() {
        let _ = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|_, _, _| {})).build(context());
        return;
    }
    if args.iter().any(|a| a == "--quit") {
        return; // nothing to quit
    }
    if paths::running_from_zip_preview(&app_dir) {
        message("CAPTURE", "CAPTURE is running from inside the zip file, so it can't find its other files.\n\nRight-click the zip and choose Extract All, then open CAPTURE.exe from the extracted folder.", MB_OK | MB_ICONINFORMATION);
        return;
    }
    let p = match paths::resolve(&app_dir) {
        paths::Resolved::Ready(p) => p,
        paths::Resolved::NeedsConsent { app_dir, fallback } => {
            let text = format!(
                "CAPTURE keeps its settings in a Data folder next to CAPTURE.exe, but Windows does not allow changes in\n\n    {}\n\nBest: move the CAPTURE folder somewhere you can write to (for example D:\\Apps\\CAPTURE), or reinstall CAPTURE there.\n\nOr choose Yes to keep CAPTURE's data for this Windows account in\n\n    {}\n\nYes: use that folder.   No: quit.",
                app_dir.display(),
                fallback.display()
            );
            if message("CAPTURE can't save its settings here", &text, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES.0 {
                return;
            }
            match paths::accept_fallback(&app_dir, &fallback) {
                Ok(p) => p,
                Err(e) => {
                    message("CAPTURE", &format!("CAPTURE could not create {}: {e}", fallback.display()), MB_OK | MB_ICONERROR);
                    return;
                }
            }
        }
    };
    let mut notices = Vec::new();
    if !p.data_fallback {
        paths::normalize_data_case(&p.app_dir);
    }
    let legacy = if p.data_fallback { p.data.parent().unwrap().to_path_buf() } else { p.data.clone() };
    let migrated = paths::migrate_legacy(&legacy, &p);
    let obsolete = paths::remove_legacy_program_files(&p);
    if let Err(e) = p.create_dirs() {
        message("CAPTURE", &format!("CAPTURE could not create its data folder {}: {e}", p.data.display()), MB_OK | MB_ICONERROR);
        return;
    }
    paths::init_logging(&p);
    eprintln!(
        "{} CAPTURE {} starting ({}) app={} data={}{}",
        paths::timestamp(),
        env!("CAPTURE_VERSION"),
        if p.installed { "installed" } else { "portable" },
        p.app_dir.display(),
        p.data.display(),
        if migrated { " (moved 0.4 data into the 0.5 layout)" } else { "" }
    );
    if obsolete > 0 {
        eprintln!("removed {obsolete} program file(s) left over from the 0.4 folder layout");
    }
    paths::use_private_temp(&p);
    match webview2_version(&p) {
        Some(v) => eprintln!("WebView2 {v}"),
        None => {
            message(
                "CAPTURE needs Microsoft Edge WebView2",
                "CAPTURE's window uses Microsoft Edge WebView2, which is part of Windows 11 and current Windows 10, but it is missing or damaged on this PC.\n\nInstall the \"Evergreen\" WebView2 Runtime from Microsoft (search for \"WebView2 Runtime\" on microsoft.com), then open CAPTURE again.",
                MB_OK | MB_ICONERROR,
            );
            return;
        }
    }
    match update::on_startup(&p) {
        update::Startup::Resume => return,
        update::Startup::Updated { from, to } => notices.push(format!("CAPTURE was updated from {from} to {to}. Your settings and recordings were kept.")),
        update::Startup::Failed(m) => notices.push(m),
        update::Startup::Nothing => {}
    }
    let first_launch = !p.settings.join("settings.json").exists();
    let mut loaded = settings::load(&p.settings, Some(&p.app_dir));
    let upgraded_from = first_launch.then(|| settings::previous_copy(&p.app_dir)).flatten();
    if reconcile_startup(&mut loaded, upgraded_from.as_deref()) {
        let _ = settings::save(&p.settings, &loaded);
    }
    let webview_dir = p.webview.clone();
    let worker_exe = p.worker_exe();
    let worker = Worker::new(worker_exe.clone(), worker_exe.parent().unwrap_or(&p.app_dir).to_path_buf());

    tauri::Builder::default()
        // A second launch never starts a second engine: it brings the running one forward.
        // The sign-in launch (`--tray`) while CAPTURE already runs stays silent. `--quit` (used
        // by the installer and uninstaller) closes it the same way as Quit in the tray: saves
        // in progress finish first.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--quit") {
                quit(app);
            } else if !args.iter().any(|a| a == "--tray") {
                show_window(app);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            worker: worker.clone(),
            settings: Mutex::new(loaded),
            paths: p.clone(),
            last_state: Mutex::new(None),
            engine_down: Mutex::new(None),
            tray: Mutex::new(None),
            notices: Mutex::new(notices),
            staged: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            update_settings,
            set_replay,
            save_replay,
            list_targets,
            clear_temp_cache,
            list_clips,
            open_path,
            reveal_path,
            open_capture_folder,
            copy_text,
            quit_app,
            engine_command,
            toggle_overlay,
            hide_overlay,
            show_main,
            delete_clip,
            rename_clip,
            set_start_with_windows,
            check_paths,
            open_app_folder,
            stage_update,
            apply_update
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            // This is now the only CAPTURE running: safe to tidy its temp folder.
            paths::clean_temp(&app.state::<AppState>().paths);
            // Started from Windows sign-in: stay in the tray until opened.
            let start_hidden = std::env::args().any(|a| a == "--tray");
            let dark_bg = tauri::window::Color(0x12, 0x14, 0x18, 0xff);
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .visible(!start_hidden)
                .background_color(dark_bg) // no white flash before the page paints
                .title("CAPTURE")
                .inner_size(1120.0, 780.0)
                .min_inner_size(720.0, 540.0)
                .data_directory(webview_dir.clone())
                .build()?;
            if start_hidden {
                set_page_visible(&window, false);
            }
            // The in-game overlay: created once, hidden until its hotkey. A hidden WebView2 does
            // not render, so it costs nothing while you play.
            let overlay = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("index.html#overlay".into()))
                .title("CAPTURE overlay")
                .inner_size(1120.0, 800.0)
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .visible(false)
                .data_directory(webview_dir.clone())
                .build()?;
            set_page_visible(&overlay, false);
            overlay.on_window_event(|e| {
                if let WindowEvent::CloseRequested { api, .. } = e {
                    api.prevent_close();
                }
            });
            let s = app.state::<AppState>().settings.lock().unwrap().clone();
            allow_media(&handle, &s);
            theme_frame(&window);
            {
                let handle = handle.clone();
                let w2 = window.clone();
                window.on_window_event(move |e| {
                    if let WindowEvent::ThemeChanged(_) = e {
                        theme_frame(&w2);
                    }
                    // WebView2 keeps reporting "visible" while the window is minimised, so pages
                    // keep animating and live meters keep streaming. Tell the controller instead:
                    // the page then sees `document.hidden` and rAF/timers are throttled.
                    // Decide from the new size (a minimised window is 0x0): `is_minimized()` can still
                    // report the old state while the restore is being processed.
                    if let WindowEvent::Resized(size) = e {
                        let visible = size.width > 0 && size.height > 0 && w2.is_visible().unwrap_or(true);
                        set_page_visible(&w2, visible);
                        // Minimize to tray (optional): a minimised window is 0x0.
                        if size.width == 0 && size.height == 0 && handle.state::<AppState>().settings.lock().unwrap().minimize_to_tray {
                            let _ = w2.hide();
                        }
                    }
                    if let WindowEvent::Focused(true) = e {
                        set_page_visible(&w2, true);
                    }
                    if let WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        let state = handle.state::<AppState>();
                        if state.settings.lock().unwrap().close_to_tray {
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.hide();
                                set_page_visible(&w, false);
                            }
                        } else {
                            quit(&handle);
                        }
                    }
                });
            }
            let tray = build_tray(&handle)?;
            *app.state::<AppState>().tray.lock().unwrap() = Some(tray);

            let (h1, h2, h3) = (handle.clone(), handle.clone(), handle.clone());
            let started = worker.start(
                Arc::new(move |m| on_message(&h1, m)),
                Arc::new(move || apply(&h2.state::<AppState>())),
                Arc::new(move |msg| {
                    *h3.state::<AppState>().engine_down.lock().unwrap() = Some(msg.clone());
                    let _ = h3.emit("engine_down", msg);
                    refresh_tray(&h3);
                }),
            );
            match started {
                Ok(()) => apply(&app.state::<AppState>()),
                Err(e) => {
                    *app.state::<AppState>().engine_down.lock().unwrap() = Some(e);
                }
            }
            refresh_tray(&handle);
            Ok(())
        })
        .build(context())
        .expect("failed to build CAPTURE")
        .run(|app, ev| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = ev {
                let state = app.state::<AppState>();
                if code.is_none() && !state.worker.quitting.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    quit(app);
                }
            }
        });
}
