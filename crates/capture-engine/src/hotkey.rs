//! Global save hotkey via `RegisterHotKey` on a dedicated message-loop thread.
//!
//! - `MOD_NOREPEAT` means holding the key does not auto-repeat triggers.
//! - A debounce window ignores bounces/double taps closer than [`DEBOUNCE`].
//! - Registration failure (another app owns the combo) is reported, never silently ignored.
//! - WM_HOTKEY is a key-down notification; there is no key-up state to get "stuck".

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::HotkeyState;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const DEBOUNCE: Duration = Duration::from_millis(750);
const HOTKEY_ID: i32 = 1;
const WM_RECONFIGURE: u32 = WM_APP + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub mods: u32,
    pub vk: u32,
}

/// Parse "Ctrl+Alt+S", "Alt+F10", "Shift+PrintScreen". Letters/digits require a modifier so a
/// plain typing key can never be stolen from games.
pub fn parse(s: &str) -> Result<Combo, String> {
    let mut mods = 0u32;
    let mut key: Option<u32> = None;
    let mut needs_mod = false;
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL.0,
            "alt" => mods |= MOD_ALT.0,
            "shift" => mods |= MOD_SHIFT.0,
            "win" | "windows" => mods |= MOD_WIN.0,
            k => {
                if key.is_some() {
                    return Err(format!("\"{s}\" has more than one key"));
                }
                let (vk, letter_like) = key_code(k).ok_or_else(|| format!("\"{part}\" is not a supported key"))?;
                needs_mod = letter_like;
                key = Some(vk);
            }
        }
    }
    let vk = key.ok_or_else(|| format!("\"{s}\" has no key"))?;
    if needs_mod && mods == 0 {
        return Err("Letter and number keys need Ctrl, Alt, Shift or Win".into());
    }
    Ok(Combo { mods, vk })
}

fn key_code(k: &str) -> Option<(u32, bool)> {
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some((VK_F1.0 as u32 + n - 1, false));
    }
    if k.len() == 1 {
        let c = k.chars().next()?.to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some((c as u32, true));
        }
    }
    let vk = match k {
        "printscreen" | "prtsc" => VK_SNAPSHOT,
        "insert" | "ins" => VK_INSERT,
        "delete" | "del" => VK_DELETE,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "pause" => VK_PAUSE,
        "scrolllock" => VK_SCROLL,
        _ => return None,
    };
    Some((vk.0 as u32, false))
}

pub struct HotkeyService {
    thread_id: u32,
    wanted: Arc<Mutex<String>>,
    pub state: Arc<Mutex<HotkeyState>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl HotkeyService {
    pub fn start(
        combo: &str,
        on_trigger: impl Fn() + Send + 'static,
        on_change: impl Fn(HotkeyState) + Send + 'static,
    ) -> Self {
        let wanted = Arc::new(Mutex::new(combo.to_string()));
        let state = Arc::new(Mutex::new(HotkeyState::default()));
        let (tx, rx) = std::sync::mpsc::channel();
        let (w, st) = (wanted.clone(), state.clone());
        let join = std::thread::Builder::new()
            .name("hotkey".into())
            .spawn(move || run(w, st, tx, on_trigger, on_change))
            .expect("spawn hotkey thread");
        let thread_id = rx.recv().unwrap_or(0);
        Self { thread_id, wanted, state, join: Some(join) }
    }

    pub fn set(&self, combo: &str) {
        *self.wanted.lock().unwrap() = combo.to_string();
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_RECONFIGURE, WPARAM(0), LPARAM(0));
        }
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn run(
    wanted: Arc<Mutex<String>>,
    state: Arc<Mutex<HotkeyState>>,
    ready: std::sync::mpsc::Sender<u32>,
    on_trigger: impl Fn(),
    on_change: impl Fn(HotkeyState),
) {
    let mut msg = MSG::default();
    unsafe {
        // Force creation of this thread's message queue before announcing the thread id.
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
    }
    let _ = ready.send(unsafe { GetCurrentThreadId() });
    let mut registered = false;
    let apply = |registered: &mut bool| {
        if *registered {
            unsafe {
                let _ = UnregisterHotKey(None, HOTKEY_ID);
            }
            *registered = false;
        }
        let combo = wanted.lock().unwrap().clone();
        let st = if combo.trim().is_empty() {
            HotkeyState { combo, registered: false, message: Some("No save hotkey set".into()) }
        } else {
            match parse(&combo) {
                Err(e) => HotkeyState { combo, registered: false, message: Some(e) },
                Ok(c) => match unsafe { RegisterHotKey(None, HOTKEY_ID, HOT_KEY_MODIFIERS(c.mods | MOD_NOREPEAT.0), c.vk) } {
                    Ok(()) => {
                        *registered = true;
                        HotkeyState { combo, registered: true, message: None }
                    }
                    Err(_) => HotkeyState {
                        message: Some(format!("{combo} is already used by another app. Choose a different hotkey.")),
                        combo,
                        registered: false,
                    },
                },
            }
        };
        *state.lock().unwrap() = st.clone();
        on_change(st);
    };
    apply(&mut registered);
    let mut last: Option<Instant> = None;
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        match msg.message {
            WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_ID => {
                let now = Instant::now();
                if last.is_none_or(|l| now.duration_since(l) >= DEBOUNCE) {
                    last = Some(now);
                    on_trigger();
                }
            }
            WM_RECONFIGURE => apply(&mut registered),
            _ => {}
        }
    }
    if registered {
        unsafe {
            let _ = UnregisterHotKey(None, HOTKEY_ID);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_combos() {
        assert_eq!(parse("Alt+F10").unwrap(), Combo { mods: MOD_ALT.0, vk: VK_F10.0 as u32 });
        assert_eq!(parse("ctrl + shift + s").unwrap(), Combo { mods: MOD_CONTROL.0 | MOD_SHIFT.0, vk: 'S' as u32 });
        assert_eq!(parse("F13").unwrap().mods, 0);
        assert_eq!(parse("Win+PrintScreen").unwrap().vk, VK_SNAPSHOT.0 as u32);
    }

    #[test]
    fn rejects_unsafe_or_malformed_combos() {
        assert!(parse("S").is_err());
        assert!(parse("Alt").is_err());
        assert!(parse("Alt+F25").is_err());
        assert!(parse("Alt+A+B").is_err());
        assert!(parse("Alt+Banana").is_err());
    }
}
