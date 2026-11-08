#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! CAPTURE UI process: window, tray and settings. All capture work happens in
//! `capture-worker.exe`, supervised by [`worker::Worker`].

mod library;
mod settings;
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
    data_dir: PathBuf,
    last_state: Mutex<Option<EngineState>>,
    engine_down: Mutex<Option<String>>,
    tray: Mutex<Option<Tray>>,
}

#[derive(Serialize)]
struct Bootstrap {
    settings: AppSettings,
    data_dir: String,
    default_root: String,
    last_state: Option<EngineState>,
    engine_down: Option<String>,
}

fn apply(state: &AppState) {
    let s = state.settings.lock().unwrap().clone();
    let _ = state.worker.send(Command::Configure(s.engine_config()));
    let _ = state.worker.send(Command::SetReplay { enabled: s.replay_enabled });
}

fn persist(state: &AppState, s: &AppSettings) -> Result<(), String> {
    settings::save(&state.data_dir, s).map_err(|e| format!("Could not save settings: {e}"))
}

#[tauri::command]
fn bootstrap(state: State<AppState>) -> Bootstrap {
    Bootstrap {
        settings: state.settings.lock().unwrap().clone(),
        data_dir: state.data_dir.to_string_lossy().into_owned(),
        default_root: settings::default_root(),
        last_state: state.last_state.lock().unwrap().clone(),
        engine_down: state.engine_down.lock().unwrap().clone(),
    }
}

#[tauri::command]
fn update_settings(app: AppHandle, state: State<AppState>, settings: AppSettings) -> Result<AppSettings, String> {
    let s = settings.sanitized();
    if s.capture_root != state.settings.lock().unwrap().capture_root {
        validate_destination(&s.capture_root)?;
    }
    persist(&state, &s)?;
    let replay_changed = state.settings.lock().unwrap().replay_enabled != s.replay_enabled;
    *state.settings.lock().unwrap() = s.clone();
    state.worker.send(Command::Configure(s.engine_config()))?;
    if replay_changed {
        state.worker.send(Command::SetReplay { enabled: s.replay_enabled })?;
    }
    refresh_tray(&app);
    allow_media(&app, &s.capture_root);
    let _ = app.emit("settings", &s); // keep every window (main + overlay) in sync
    Ok(s)
}

/// A new capture folder must be creatable and writable before it is used. Existing recordings
/// stay where they are; nothing is moved.
fn validate_destination(root: &str) -> Result<(), String> {
    let p = PathBuf::from(root);
    if !p.is_absolute() {
        return Err(r"Choose a full folder path (for example D:\Captures).".into());
    }
    std::fs::create_dir_all(&p).map_err(|e| format!("CAPTURE can't use {root}: {e}"))?;
    let probe = p.join(".capture-write-test");
    std::fs::write(&probe, b"ok").map_err(|e| format!("CAPTURE can't save files in {root}: {e}"))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// Let the webviews play/view files inside the capture folder only (reviews, clips, thumbs).
fn allow_media(app: &AppHandle, root: &str) {
    if !root.trim().is_empty() {
        let _ = app.asset_protocol_scope().allow_directory(root, true);
    }
}

/// Any engine command from either window; the answer arrives as an `engine` event whose
/// `re` equals the returned id.
/// Start with Windows: a per-user Run entry that starts CAPTURE in the tray. Removing it
/// leaves nothing behind.
#[tauri::command]
fn set_start_with_windows(app: AppHandle, state: State<AppState>, enabled: bool) -> Result<(), String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::*;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let value = format!("\"{}\" --tray", exe.display());
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None)
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
        r.ok().map_err(|e| format!("Could not change startup setting: {e}"))?;
    }
    let s = {
        let mut g = state.settings.lock().unwrap();
        g.start_with_windows = enabled;
        g.clone()
    };
    persist(&state, &s)?;
    let _ = app.emit("settings", &s);
    Ok(())
}

#[tauri::command]
fn engine_command(state: State<AppState>, command: Command) -> Result<u64, String> {
    if matches!(command, Command::Configure(_) | Command::Shutdown) {
        return Err("Not allowed from the UI".into());
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
    let _ = w.set_focus();
    let _ = app.emit_to("overlay", "overlay_shown", ());
}

#[tauri::command]
fn set_replay(app: AppHandle, state: State<AppState>, enabled: bool) -> Result<(), String> {
    set_replay_inner(&app, &state, enabled)
}

fn set_replay_inner(app: &AppHandle, state: &AppState, enabled: bool) -> Result<(), String> {
    let s = {
        let mut g = state.settings.lock().unwrap();
        g.replay_enabled = enabled;
        g.clone()
    };
    persist(state, &s)?;
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
    let p = inside_root(&state, &path)?;
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

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
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
        Event::HotkeyPressed { action: capture_protocol::HotkeyAction::ToggleOverlay } => {
            let a = app.clone();
            let _ = app.run_on_main_thread(move || toggle_overlay_window(&a));
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

fn main() {
    let data_dir = settings::data_dir();
    let loaded = settings::load(&data_dir);
    let worker = Worker::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            worker: worker.clone(),
            settings: Mutex::new(loaded),
            data_dir: data_dir.clone(),
            last_state: Mutex::new(None),
            engine_down: Mutex::new(None),
            tray: Mutex::new(None),
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
            set_start_with_windows
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            // Started from Windows sign-in: stay in the tray until opened.
            let start_hidden = std::env::args().any(|a| a == "--tray");
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .visible(!start_hidden)
                .title("CAPTURE")
                .inner_size(1000.0, 760.0)
                .min_inner_size(760.0, 600.0)
                .data_directory(data_dir.join("webview"))
                .build()?;
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
                .data_directory(data_dir.join("webview"))
                .build()?;
            overlay.on_window_event(|e| {
                if let WindowEvent::CloseRequested { api, .. } = e {
                    api.prevent_close();
                }
            });
            allow_media(&handle, &app.state::<AppState>().settings.lock().unwrap().capture_root.clone());
            {
                let handle = handle.clone();
                window.on_window_event(move |e| {
                    if let WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        let state = handle.state::<AppState>();
                        if state.settings.lock().unwrap().close_to_tray {
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.hide();
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
        .build(tauri::generate_context!())
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
