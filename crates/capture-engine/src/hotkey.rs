//! Global save hotkey on a dedicated message-loop thread.
//!
//! Two mechanisms, chosen by key:
//! - **Regular keys** (F-keys, letters, digits, symbols, PrintScreen…): `RegisterHotKey` with
//!   `MOD_NOREPEAT`. Windows reports conflicts with other apps, which we surface.
//! - **Numpad keys**: a low-level keyboard hook (`WH_KEYBOARD_LL`). With NumLock off, Numpad 1
//!   sends the same virtual key as the navigation-cluster End key; only the hook can tell them
//!   apart (numpad keys are *not* flagged extended). So `Num1` works with NumLock on or off and
//!   never steals the real End/Home/arrow keys. The hook callback only posts a message — it
//!   does no work — and swallows just the matching key-down/up pair.
//!
//! In both cases a debounce window ignores bounces closer than [`DEBOUNCE`], and a held key
//! fires once.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::HotkeyState;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const DEBOUNCE: Duration = Duration::from_millis(750);
const HOTKEY_ID: i32 = 1;
const WM_RECONFIGURE: u32 = WM_APP + 1;
const WM_HOOK_HIT: u32 = WM_APP + 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub mods: u32,
    pub vk: u32,
    /// Set for numpad keys: the virtual key the same physical key sends with NumLock off
    /// (`None` for keys that do not change, e.g. NumAdd).
    pub numpad: Option<Option<u32>>,
}

/// Parse "Ctrl+Alt+S", "Alt+F10", "Num5", "Ctrl+NumAdd", "Alt+Minus". Letters, digits and
/// main-keyboard symbols require a modifier so a plain typing key can never be stolen.
pub fn parse(s: &str) -> Result<Combo, String> {
    let mut mods = 0u32;
    let mut key: Option<Key> = None;
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
                key = Some(key_code(k).ok_or_else(|| format!("\"{part}\" is not a supported key"))?);
            }
        }
    }
    let k = key.ok_or_else(|| format!("\"{s}\" has no key"))?;
    if k.typing && mods == 0 {
        return Err("Letter, number and symbol keys need Ctrl, Alt, Shift or Win".into());
    }
    if matches!(k.numpad, Some(Some(_))) && mods & MOD_SHIFT.0 != 0 {
        // Windows turns Shift+numpad digit into a navigation key and fakes a Shift release.
        return Err("Shift can't be combined with numpad digits. Use Ctrl or Alt instead.".into());
    }
    Ok(Combo { mods, vk: k.vk, numpad: k.numpad })
}

struct Key {
    vk: u32,
    typing: bool,
    numpad: Option<Option<u32>>,
}

fn key_code(k: &str) -> Option<Key> {
    let plain = |vk: VIRTUAL_KEY| Some(Key { vk: vk.0 as u32, typing: false, numpad: None });
    let typing = |vk: u32| Some(Key { vk, typing: true, numpad: None });
    let pad = |vk: VIRTUAL_KEY, off: Option<VIRTUAL_KEY>| Some(Key { vk: vk.0 as u32, typing: false, numpad: Some(off.map(|o| o.0 as u32)) });
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then(|| Key { vk: VK_F1.0 as u32 + n - 1, typing: false, numpad: None });
    }
    if let Some(n) = k.strip_prefix("num").and_then(|n| n.parse::<u32>().ok()) {
        // NumLock-off equivalents of Num0..Num9.
        let off = [VK_INSERT, VK_END, VK_DOWN, VK_NEXT, VK_LEFT, VK_CLEAR, VK_RIGHT, VK_HOME, VK_UP, VK_PRIOR];
        return (n <= 9).then(|| Key { vk: VK_NUMPAD0.0 as u32 + n, typing: false, numpad: Some(Some(off[n as usize].0 as u32)) });
    }
    if k.len() == 1 {
        let c = k.chars().next()?.to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return typing(c as u32);
        }
    }
    match k {
        "numadd" => pad(VK_ADD, None),
        "numsubtract" => pad(VK_SUBTRACT, None),
        "nummultiply" => pad(VK_MULTIPLY, None),
        "numdivide" => pad(VK_DIVIDE, None),
        "numdecimal" => pad(VK_DECIMAL, Some(VK_DELETE)),
        "minus" => typing(VK_OEM_MINUS.0 as u32),
        "equal" => typing(VK_OEM_PLUS.0 as u32),
        "bracketleft" => typing(VK_OEM_4.0 as u32),
        "bracketright" => typing(VK_OEM_6.0 as u32),
        "backslash" => typing(VK_OEM_5.0 as u32),
        "semicolon" => typing(VK_OEM_1.0 as u32),
        "quote" => typing(VK_OEM_7.0 as u32),
        "comma" => typing(VK_OEM_COMMA.0 as u32),
        "period" => typing(VK_OEM_PERIOD.0 as u32),
        "slash" => typing(VK_OEM_2.0 as u32),
        "backquote" => typing(VK_OEM_3.0 as u32),
        "printscreen" | "prtsc" => plain(VK_SNAPSHOT),
        "insert" | "ins" => plain(VK_INSERT),
        "delete" | "del" => plain(VK_DELETE),
        "home" => plain(VK_HOME),
        "end" => plain(VK_END),
        "pageup" | "pgup" => plain(VK_PRIOR),
        "pagedown" | "pgdn" => plain(VK_NEXT),
        "pause" => plain(VK_PAUSE),
        "scrolllock" => plain(VK_SCROLL),
        _ => None,
    }
}

// ---- low-level hook state (one hook per process, owned by the hotkey thread) -------------

static HOOK_ACTIVE: AtomicBool = AtomicBool::new(false);
static HOOK_VK: AtomicU32 = AtomicU32::new(0);
static HOOK_OFF_VK: AtomicU32 = AtomicU32::new(0);
static HOOK_MODS: AtomicU32 = AtomicU32::new(0);
static HOOK_TID: AtomicU32 = AtomicU32::new(0);
static HOOK_HELD: AtomicBool = AtomicBool::new(false);

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

fn mods_now() -> u32 {
    let mut m = 0;
    if down(VK_CONTROL) {
        m |= MOD_CONTROL.0;
    }
    if down(VK_MENU) {
        m |= MOD_ALT.0;
    }
    if down(VK_SHIFT) {
        m |= MOD_SHIFT.0;
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        m |= MOD_WIN.0;
    }
    m
}

/// Pure matcher (unit-tested): does this key event belong to the configured numpad key?
pub fn numpad_matches(vk: u32, extended: bool, want_vk: u32, want_off_vk: u32) -> bool {
    if vk == want_vk {
        // VK_NUMPADn / VK_ADD… are unique to the numpad. VK_DIVIDE is legitimately extended.
        return true;
    }
    want_off_vk != 0 && vk == want_off_vk && !extended
}

unsafe extern "system" fn ll_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && HOOK_ACTIVE.load(Ordering::Relaxed) {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let ext = k.flags.0 & LLKHF_EXTENDED.0 != 0;
        if numpad_matches(k.vkCode, ext, HOOK_VK.load(Ordering::Relaxed), HOOK_OFF_VK.load(Ordering::Relaxed)) {
            let msg = wp.0 as u32;
            let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            if is_down {
                if HOOK_HELD.load(Ordering::Relaxed) {
                    return LRESULT(1); // auto-repeat of a key we already consumed
                }
                if mods_now() == HOOK_MODS.load(Ordering::Relaxed) {
                    HOOK_HELD.store(true, Ordering::Relaxed);
                    let _ = PostThreadMessageW(HOOK_TID.load(Ordering::Relaxed), WM_HOOK_HIT, WPARAM(0), LPARAM(0));
                    return LRESULT(1);
                }
            } else if HOOK_HELD.swap(false, Ordering::Relaxed) {
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
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

enum Active {
    None,
    Registered,
    Hook(HHOOK),
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
    let tid = unsafe { GetCurrentThreadId() };
    HOOK_TID.store(tid, Ordering::Relaxed);
    let _ = ready.send(tid);
    let mut active = Active::None;
    let release = |a: &mut Active| {
        match a {
            Active::Registered => unsafe {
                let _ = UnregisterHotKey(None, HOTKEY_ID);
            },
            Active::Hook(h) => unsafe {
                HOOK_ACTIVE.store(false, Ordering::Relaxed);
                let _ = UnhookWindowsHookEx(*h);
            },
            Active::None => {}
        }
        *a = Active::None;
    };
    let apply = |active: &mut Active| {
        release(active);
        let combo = wanted.lock().unwrap().clone();
        let st = if combo.trim().is_empty() {
            HotkeyState { combo, registered: false, message: Some("No save hotkey set".into()) }
        } else {
            match parse(&combo) {
                Err(e) => HotkeyState { combo, registered: false, message: Some(e) },
                Ok(c) if c.numpad.is_some() => {
                    HOOK_VK.store(c.vk, Ordering::Relaxed);
                    HOOK_OFF_VK.store(c.numpad.flatten().unwrap_or(0), Ordering::Relaxed);
                    HOOK_MODS.store(c.mods, Ordering::Relaxed);
                    HOOK_HELD.store(false, Ordering::Relaxed);
                    match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), None, 0) } {
                        Ok(h) => {
                            HOOK_ACTIVE.store(true, Ordering::Relaxed);
                            *active = Active::Hook(h);
                            HotkeyState { combo, registered: true, message: None }
                        }
                        Err(e) => HotkeyState { combo, registered: false, message: Some(format!("Could not listen for numpad keys: {e}")) },
                    }
                }
                Ok(c) => match unsafe { RegisterHotKey(None, HOTKEY_ID, HOT_KEY_MODIFIERS(c.mods | MOD_NOREPEAT.0), c.vk) } {
                    Ok(()) => {
                        *active = Active::Registered;
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
    apply(&mut active);
    let mut last: Option<Instant> = None;
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        let hit = match msg.message {
            WM_HOTKEY => msg.wParam.0 as i32 == HOTKEY_ID,
            WM_HOOK_HIT => true,
            WM_RECONFIGURE => {
                apply(&mut active);
                false
            }
            _ => false,
        };
        if hit {
            let now = Instant::now();
            if last.is_none_or(|l| now.duration_since(l) >= DEBOUNCE) {
                last = Some(now);
                on_trigger();
            }
        }
    }
    release(&mut active);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_combos() {
        assert_eq!(parse("Alt+F10").unwrap(), Combo { mods: MOD_ALT.0, vk: VK_F10.0 as u32, numpad: None });
        assert_eq!(parse("ctrl + shift + s").unwrap().vk, 'S' as u32);
        assert_eq!(parse("F13").unwrap().mods, 0);
        assert_eq!(parse("Win+PrintScreen").unwrap().vk, VK_SNAPSHOT.0 as u32);
        assert_eq!(parse("Alt+Minus").unwrap().vk, VK_OEM_MINUS.0 as u32);
    }

    #[test]
    fn numpad_keys_parse_with_numlock_off_equivalents_and_need_no_modifier() {
        let c = parse("Num1").unwrap();
        assert_eq!((c.vk, c.mods, c.numpad), (VK_NUMPAD1.0 as u32, 0, Some(Some(VK_END.0 as u32))));
        assert_eq!(parse("Num5").unwrap().numpad, Some(Some(VK_CLEAR.0 as u32)));
        assert_eq!(parse("Ctrl+NumAdd").unwrap().numpad, Some(None));
        assert_eq!(parse("NumDecimal").unwrap().numpad, Some(Some(VK_DELETE.0 as u32)));
        assert!(parse("Num10").is_err());
        assert!(parse("Shift+Num1").is_err());
        assert!(parse("Shift+NumAdd").is_ok());
    }

    #[test]
    fn numpad_matcher_never_takes_the_navigation_keys() {
        let (n1, end) = (VK_NUMPAD1.0 as u32, VK_END.0 as u32);
        assert!(numpad_matches(n1, false, n1, end)); // NumLock on
        assert!(numpad_matches(end, false, n1, end)); // NumLock off: numpad End is not extended
        assert!(!numpad_matches(end, true, n1, end)); // the real End key is extended
        assert!(numpad_matches(VK_DIVIDE.0 as u32, true, VK_DIVIDE.0 as u32, 0));
        assert!(!numpad_matches(VK_HOME.0 as u32, false, n1, end));
    }

    #[test]
    fn rejects_unsafe_or_malformed_combos() {
        assert!(parse("S").is_err());
        assert!(parse("Minus").is_err());
        assert!(parse("Alt").is_err());
        assert!(parse("Alt+F25").is_err());
        assert!(parse("Alt+A+B").is_err());
        assert!(parse("Alt+Banana").is_err());
    }
}
