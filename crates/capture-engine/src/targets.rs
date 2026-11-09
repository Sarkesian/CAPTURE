//! Enumerate capturable windows and displays with human-readable names.

use capture_protocol::{DisplayTarget, WindowTarget};
use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn windows() -> Vec<WindowTarget> {
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        (*(l.0 as *mut Vec<HWND>)).push(h);
        true.into()
    }
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut hwnds as *mut _ as isize));
    }
    let me = std::process::id();
    hwnds.into_iter().filter_map(|h| describe(h, me)).collect()
}

pub fn describe(h: HWND, own_pid: u32) -> Option<WindowTarget> {
    unsafe {
        if !IsWindowVisible(h).as_bool() {
            return None;
        }
        if GetWindow(h, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return None;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        if cloaked != 0 {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        let (w, hgt) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
        if w < 160 || hgt < 120 {
            return None;
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(h, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n as usize]);
        if title.trim().is_empty() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == own_pid || pid == 0 {
            return None;
        }
        let exe_path = process_path(pid).unwrap_or_default();
        let exe = exe_path.rsplit(['\\', '/']).next().unwrap_or("").to_string();
        if matches!(exe.to_ascii_lowercase().as_str(), "textinputhost.exe" | "applicationframehost.exe" | "capture.exe" | "capture-worker.exe") {
            return None;
        }
        let app_name = app_name(&exe_path, &exe, &title);
        Some(WindowTarget { hwnd: h.0 as u64, pid, exe, title, app_name, width: w, height: hgt })
    }
}

pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Best human name for folder naming: the exe's FileDescription/ProductName unless it names
/// a generic engine/runtime, then the window title, then the exe stem.
pub fn app_name(exe_path: &str, exe: &str, title: &str) -> String {
    let generic = |s: &str| {
        let l = s.to_ascii_lowercase();
        ["godot", "unity", "unreal", "java", "python", "electron", "launcher", "host", "runtime", "application"]
            .iter()
            .any(|g| l.contains(g))
    };
    for key in ["FileDescription", "ProductName"] {
        if let Some(v) = version_string(exe_path, key) {
            if !v.trim().is_empty() && !generic(&v) {
                return v.trim().to_string();
            }
        }
    }
    if !title.trim().is_empty() && title.len() <= 60 {
        return title.trim().to_string();
    }
    exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string()
}

fn version_string(path: &str, key: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    unsafe {
        let wpath: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let size = GetFileVersionInfoSizeW(PCWSTR(wpath.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wpath.as_ptr()), None, size, data.as_mut_ptr() as *mut _).ok()?;
        // Translation table → first language/codepage pair.
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let q: Vec<u16> = "\\VarFileInfo\\Translation".encode_utf16().chain(Some(0)).collect();
        let (lang, cp) = if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(q.as_ptr()), &mut ptr, &mut len).as_bool() && len >= 4 {
            let p = ptr as *const u16;
            (*p, *p.add(1))
        } else {
            (0x0409, 0x04b0)
        };
        let q: Vec<u16> = format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\{key}").encode_utf16().chain(Some(0)).collect();
        if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(q.as_ptr()), &mut ptr, &mut len).as_bool() && len > 0 {
            let s = std::slice::from_raw_parts(ptr as *const u16, len as usize);
            return Some(String::from_utf16_lossy(s).trim_end_matches('\0').to_string());
        }
        None
    }
}

pub struct Monitor {
    pub handle: isize,
    pub target: DisplayTarget,
}

pub fn monitors() -> Vec<Monitor> {
    let mut list: Vec<HMONITOR> = Vec::new();
    unsafe extern "system" fn cb(m: HMONITOR, _: HDC, _: *mut RECT, l: LPARAM) -> BOOL {
        (*(l.0 as *mut Vec<HMONITOR>)).push(m);
        true.into()
    }
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut list as *mut _ as isize));
    }
    list.into_iter()
        .enumerate()
        .map(|(i, m)| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            unsafe {
                let _ = GetMonitorInfoW(m, &mut info as *mut _ as *mut MONITORINFO);
            }
            let r = info.monitorInfo.rcMonitor;
            let primary = info.monitorInfo.dwFlags & 1 != 0; // MONITORINFOF_PRIMARY
            Monitor {
                handle: m.0 as isize,
                target: DisplayTarget {
                    index: i as u32,
                    name: format!("Display {}{}", i + 1, if primary { " (main)" } else { "" }),
                    width: (r.right - r.left) as u32,
                    height: (r.bottom - r.top) as u32,
                    primary,
                },
            }
        })
        .collect()
}

/// How a window sits on its monitor, for deciding whether it is a fullscreen game.
pub struct Placement {
    pub pid: u32,
    /// Covers (nearly) its whole monitor and is not a maximized ordinary window or the desktop.
    pub fullscreen: bool,
    /// The facts behind `fullscreen`, for the log.
    pub facts: String,
}

/// The window in front right now (0 if none).
pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Windows reports a Direct3D application running in exclusive fullscreen (the mode in which
/// a game minimizes as soon as any other window takes focus).
pub fn d3d_fullscreen_running() -> bool {
    // (Declared here: turning on the whole `Win32_UI_Shell` feature for one function is not worth it.)
    #[link(name = "shell32")]
    extern "system" {
        fn SHQueryUserNotificationState(state: *mut i32) -> i32;
    }
    const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;
    let mut state = 0i32;
    unsafe { SHQueryUserNotificationState(&mut state) >= 0 && state == QUNS_RUNNING_D3D_FULL_SCREEN }
}

/// Where window `hwnd` is and whether it is fullscreen (exclusive fullscreen, or borderless
/// over the whole screen). Tolerant on purpose: games are a few pixels off, keep window borders
/// in their style while in exclusive fullscreen, or report odd rectangles on scaled displays.
/// Deciding "not fullscreen" for a fullscreen game makes CAPTURE open a window over it, which
/// minimizes the game.
pub fn placement(hwnd: isize) -> Option<Placement> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    if hwnd == 0 {
        return None;
    }
    unsafe {
        let h = HWND(hwnd as *mut _);
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
            return None;
        }
        let m = mi.rcMonitor;
        let mut buf = [0u16; 64];
        let n = GetClassNameW(h, &mut buf);
        let class = String::from_utf16_lossy(&buf[..n as usize]);
        let shell = matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "XamlExplorerHostIslandWindow" | "Windows.UI.Core.CoreWindow");
        let minimized = IsIconic(h).as_bool();
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        // A maximized ordinary window (title bar) is not a fullscreen game, even where it covers
        // the whole monitor (auto-hidden taskbar).
        let maximized = IsZoomed(h).as_bool() && style & WS_CAPTION.0 == WS_CAPTION.0;
        let (iw, ih) = ((r.right.min(m.right) - r.left.max(m.left)).max(0) as i64, (r.bottom.min(m.bottom) - r.top.max(m.top)).max(0) as i64);
        let area = ((m.right - m.left) as i64 * (m.bottom - m.top) as i64).max(1);
        let covered = iw * ih * 100 / area;
        let fullscreen = covered >= 98 && !shell && !minimized && !maximized && IsWindowVisible(h).as_bool();
        let facts = format!(
            "window {hwnd:#x} class {class} pid {pid} rect {},{} {}x{} on monitor {},{} {}x{} covers {covered}%{}{}",
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            m.left,
            m.top,
            m.right - m.left,
            m.bottom - m.top,
            if minimized { " minimized" } else { "" },
            if maximized { " maximized" } else { "" },
        );
        Some(Placement { pid, fullscreen, facts })
    }
}

