//! Automatic game detection, independent of any launcher.
//!
//! The foreground window is treated as a game when (first match wins):
//! 1. its exe is in the user's *ignore* list → never a game;
//! 2. its exe is in the user's *games* list → game;
//! 3. it is a known non-game app (browsers, chat, editors, Windows shell…) → not a game;
//! 4. its exe lives in a game-library folder (Steam, Epic, GOG, itch, Xbox, EA, Ubisoft,
//!    Riot, Rockstar, Battle.net, or any `\Games\` folder) → game;
//! 5. its window covers the whole monitor (fullscreen / borderless) → game.
//!
//! Nothing here reads launcher databases or injects anything; it only looks at the window,
//! its process path and its size.

use capture_protocol::WindowTarget;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};

use crate::targets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    UserList,
    Library,
    Fullscreen,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Reason::UserList => "your games list",
            Reason::Library => "game library folder",
            Reason::Fullscreen => "fullscreen app",
        }
    }
}

const NOT_GAMES: &[&str] = &[
    "explorer.exe", "chrome.exe", "msedge.exe", "firefox.exe", "opera.exe", "brave.exe", "vivaldi.exe",
    "discord.exe", "slack.exe", "teams.exe", "ms-teams.exe", "zoom.exe", "spotify.exe", "steam.exe",
    "steamwebhelper.exe", "epicgameslauncher.exe", "galaxyclient.exe", "itch.exe", "battle.net.exe",
    "eadesktop.exe", "upc.exe", "redlauncher.exe", "code.exe", "devenv.exe", "notepad.exe", "notepad++.exe",
    "winword.exe", "excel.exe", "powerpnt.exe", "outlook.exe", "windowsterminal.exe", "cmd.exe",
    "powershell.exe", "pwsh.exe", "mstsc.exe", "obs64.exe", "vlc.exe", "mpc-hc64.exe", "mpv.exe",
    "applicationframehost.exe", "textinputhost.exe", "shellexperiencehost.exe", "searchhost.exe",
    "startmenuexperiencehost.exe", "lockapp.exe", "systemsettings.exe", "taskmgr.exe", "capture.exe",
    "capture-worker.exe", "claude.exe", "msedgewebview2.exe", "photos.exe", "blender.exe",
];

const LIBRARY_MARKERS: &[&str] = &[
    "\\steamapps\\common\\", "\\epic games\\", "\\gog galaxy\\games\\", "\\gog games\\", "\\itch\\apps\\",
    "\\itch\\games\\", "\\xboxgames\\", "\\riot games\\", "\\ea games\\", "\\origin games\\",
    "\\ubisoft game launcher\\games\\", "\\rockstar games\\", "\\battle.net\\", "\\games\\",
];

pub fn classify(exe_path: &str, exe: &str, covers_monitor: bool, games: &[String], ignore: &[String]) -> Option<Reason> {
    let has = |list: &[String]| list.iter().any(|g| g.eq_ignore_ascii_case(exe));
    if has(ignore) {
        return None;
    }
    if has(games) {
        return Some(Reason::UserList);
    }
    let lower = exe.to_ascii_lowercase();
    if NOT_GAMES.contains(&lower.as_str()) {
        return None;
    }
    let path = exe_path.to_ascii_lowercase().replace('/', "\\");
    if LIBRARY_MARKERS.iter().any(|m| path.contains(m)) {
        return Some(Reason::Library);
    }
    covers_monitor.then_some(Reason::Fullscreen)
}

/// The game to record right now:
/// 1. the foreground window, if it qualifies (any rule, including fullscreen);
/// 2. otherwise any visible window that is a game by path or by the user's list (not by
///    fullscreen — that rule is only trusted for the window the user is looking at), largest
///    first. This covers games that opened behind another window, or the user clicking back
///    to CAPTURE after launching a game.
pub fn current_game(games: &[String], ignore: &[String]) -> Option<(WindowTarget, Reason)> {
    if let Some(hit) = foreground_game(games, ignore) {
        return Some(hit);
    }
    targets::windows()
        .into_iter()
        .filter_map(|w| {
            let path = targets::process_path(w.pid).unwrap_or_default();
            classify(&path, &w.exe, false, games, ignore).map(|r| (w, r))
        })
        .max_by_key(|(w, _)| w.width as u64 * w.height as u64)
}

/// The foreground window if it qualifies as a game.
pub fn foreground_game(games: &[String], ignore: &[String]) -> Option<(WindowTarget, Reason)> {
    let h = unsafe { GetForegroundWindow() };
    if h.is_invalid() {
        return None;
    }
    let w = targets::describe(h, std::process::id())?;
    let path = targets::process_path(w.pid).unwrap_or_default();
    let covers = unsafe {
        let mut r = RECT::default();
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetWindowRect(h, &mut r).is_ok()
            && GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool()
            && r.left <= mi.rcMonitor.left
            && r.top <= mi.rcMonitor.top
            && r.right >= mi.rcMonitor.right
            && r.bottom >= mi.rcMonitor.bottom
    };
    classify(&path, &w.exe, covers, games, ignore).map(|r| (w, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_folders_and_fullscreen_are_games() {
        let g = |p: &str, e: &str, fs: bool| classify(p, e, fs, &[], &[]);
        assert_eq!(g(r"D:\Programs\itch\Games\faultheart\FAULTHEART.exe", "FAULTHEART.exe", false), Some(Reason::Library));
        assert_eq!(g(r"D:\SteamLibrary\steamapps\common\The Witcher 3\bin\x64_dx12\witcher3.exe", "witcher3.exe", false), Some(Reason::Library));
        assert_eq!(g(r"C:\Tools\SomeGame\game.exe", "game.exe", true), Some(Reason::Fullscreen));
        assert_eq!(g(r"C:\Tools\SomeTool\tool.exe", "tool.exe", false), None);
    }

    #[test]
    fn browsers_and_shell_are_never_games_even_fullscreen() {
        assert_eq!(classify(r"C:\Program Files\Google\Chrome\Application\chrome.exe", "chrome.exe", true, &[], &[]), None);
        assert_eq!(classify(r"C:\Windows\explorer.exe", "explorer.exe", true, &[], &[]), None);
        // ...unless the user explicitly says so.
        assert_eq!(classify(r"C:\x\chrome.exe", "chrome.exe", false, &["Chrome.exe".into()], &[]), Some(Reason::UserList));
    }

    #[test]
    fn ignore_list_wins() {
        let ig = vec!["FAULTHEART.exe".to_string()];
        assert_eq!(classify(r"D:\itch\Games\f\FAULTHEART.exe", "faultheart.exe", true, &ig.clone(), &ig), None);
    }
}
