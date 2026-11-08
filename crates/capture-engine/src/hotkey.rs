//! Global hotkeys (save replay, marker, overlay, session, screenshot) on one message-loop thread.
//!
//! Two mechanisms, chosen per key:
//! - **Regular keys** (F-keys, letters, digits, symbols, PrintScreen…): `RegisterHotKey` with
//!   `MOD_NOREPEAT`. Windows reports conflicts with other apps, which we surface.
//! - **Numpad keys**: a low-level keyboard hook (`WH_KEYBOARD_LL`). With NumLock off, Numpad 1
//!   sends the same virtual key as the navigation-cluster End key; only the hook can tell them
//!   apart (numpad keys are *not* flagged extended). So `Num1` works with NumLock on or off and
//!   never steals the real End/Home/arrow keys. The hook callback only posts a message — it
//!   does no work — and swallows just the matching key-down/up pair.
//!
//! Every action has its own debounce window ([`DEBOUNCE`]); a held key fires once; two actions
//! can never share one combination.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::{HotkeyAction, HotkeyState};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const DEBOUNCE: Duration = Duration::from_millis(750);
const HOTKEY_ID: i32 = 1; // + slot index
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

pub fn action_label(a: HotkeyAction) -> &'static str {
    match a {
        HotkeyAction::SaveReplay => "Save replay",
        HotkeyAction::AddMarker => "Add marker",
        HotkeyAction::ToggleOverlay => "Open overlay",
        HotkeyAction::ToggleSession => "Start/stop recording",
        HotkeyAction::Screenshot => "Screenshot",
        HotkeyAction::ToggleMicMute => "Mute microphone",
    }
}

// ---- low-level hook state (one hook per process, owned by the hotkey thread) -------------

const SLOTS: usize = 8;

struct Slot {
    active: AtomicBool,
    vk: AtomicU32,
    off_vk: AtomicU32,
    mods: AtomicU32,
    held: AtomicBool,
}

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY_SLOT: Slot = Slot {
    active: AtomicBool::new(false),
    vk: AtomicU32::new(0),
    off_vk: AtomicU32::new(0),
    mods: AtomicU32::new(0),
    held: AtomicBool::new(false),
};
static HOOK_SLOTS: [Slot; SLOTS] = [EMPTY_SLOT; SLOTS];
static HOOK_TID: AtomicU32 = AtomicU32::new(0);

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
    if code == HC_ACTION as i32 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let ext = k.flags.0 & LLKHF_EXTENDED.0 != 0;
        let msg = wp.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        for (i, s) in HOOK_SLOTS.iter().enumerate() {
            if !s.active.load(Ordering::Relaxed)
                || !numpad_matches(k.vkCode, ext, s.vk.load(Ordering::Relaxed), s.off_vk.load(Ordering::Relaxed))
            {
                continue;
            }
            if is_down {
                if s.held.load(Ordering::Relaxed) {
                    return LRESULT(1); // auto-repeat of a key we already consumed
                }
                if mods_now() == s.mods.load(Ordering::Relaxed) {
                    s.held.store(true, Ordering::Relaxed);
                    let _ = PostThreadMessageW(HOOK_TID.load(Ordering::Relaxed), WM_HOOK_HIT, WPARAM(i), LPARAM(0));
                    return LRESULT(1);
                }
            } else if s.held.swap(false, Ordering::Relaxed) {
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

pub type Bindings = Vec<(HotkeyAction, String)>;

/// Two actions may not share one key combination: returns the earlier action using it.
pub fn duplicate_of(bindings: &Bindings, i: usize) -> Option<HotkeyAction> {
    let me = parse(&bindings[i].1).ok()?;
    bindings[..i].iter().find(|(_, c)| parse(c).ok() == Some(me)).map(|(a, _)| *a)
}

pub struct HotkeyService {
    thread_id: u32,
    wanted: Arc<Mutex<Bindings>>,
    pub state: Arc<Mutex<Vec<HotkeyState>>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl HotkeyService {
    pub fn start(
        bindings: Bindings,
        on_trigger: impl Fn(HotkeyAction) + Send + 'static,
        on_change: impl Fn(Vec<HotkeyState>) + Send + 'static,
    ) -> Self {
        let wanted = Arc::new(Mutex::new(bindings));
        let state = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        let (w, st) = (wanted.clone(), state.clone());
        let join = std::thread::Builder::new()
            .name("hotkey".into())
            .spawn(move || run(w, st, tx, on_trigger, on_change))
            .expect("spawn hotkey thread");
        let thread_id = rx.recv().unwrap_or(0);
        Self { thread_id, wanted, state, join: Some(join) }
    }

    pub fn set(&self, bindings: Bindings) {
        *self.wanted.lock().unwrap() = bindings;
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_RECONFIGURE, WPARAM(0), LPARAM(0));
        }
    }

    pub fn state_for(&self, a: HotkeyAction) -> HotkeyState {
        self.state.lock().unwrap().iter().find(|s| s.action == Some(a)).cloned().unwrap_or_default()
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

fn release(registered: &mut Vec<i32>, hook: &mut Option<HHOOK>) {
    for id in registered.drain(..) {
        unsafe {
            let _ = UnregisterHotKey(None, id);
        }
    }
    for s in &HOOK_SLOTS {
        s.active.store(false, Ordering::Relaxed);
    }
    if let Some(h) = hook.take() {
        unsafe {
            let _ = UnhookWindowsHookEx(h);
        }
    }
}

fn apply(bindings: &Bindings, registered: &mut Vec<i32>, hook: &mut Option<HHOOK>) -> Vec<HotkeyState> {
    release(registered, hook);
    let mut states = Vec::new();
    for (i, (action, combo)) in bindings.iter().enumerate().take(SLOTS) {
        let st = |ok: bool, message: Option<String>| HotkeyState { action: Some(*action), combo: combo.clone(), registered: ok, message };
        if combo.trim().is_empty() {
            states.push(st(false, None));
            continue;
        }
        if let Some(other) = duplicate_of(bindings, i) {
            states.push(st(false, Some(format!("{combo} is already used for {}.", action_label(other)))));
            continue;
        }
        let state = match parse(combo) {
            Err(e) => st(false, Some(e)),
            Ok(c) if c.numpad.is_some() => {
                if hook.is_none() {
                    *hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), None, 0) }.ok();
                }
                if hook.is_some() {
                    let s = &HOOK_SLOTS[i];
                    s.vk.store(c.vk, Ordering::Relaxed);
                    s.off_vk.store(c.numpad.flatten().unwrap_or(0), Ordering::Relaxed);
                    s.mods.store(c.mods, Ordering::Relaxed);
                    s.held.store(false, Ordering::Relaxed);
                    s.active.store(true, Ordering::Relaxed);
                    st(true, None)
                } else {
                    st(false, Some("Could not listen for numpad keys.".into()))
                }
            }
            Ok(c) => {
                let id = HOTKEY_ID + i as i32;
                match unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(c.mods | MOD_NOREPEAT.0), c.vk) } {
                    Ok(()) => {
                        registered.push(id);
                        st(true, None)
                    }
                    Err(_) => st(false, Some(format!("{combo} is already used by another app. Choose a different key."))),
                }
            }
        };
        states.push(state);
    }
    states
}

fn run(
    wanted: Arc<Mutex<Bindings>>,
    state: Arc<Mutex<Vec<HotkeyState>>>,
    ready: std::sync::mpsc::Sender<u32>,
    on_trigger: impl Fn(HotkeyAction),
    on_change: impl Fn(Vec<HotkeyState>),
) {
    let mut msg = MSG::default();
    unsafe {
        // Force creation of this thread's message queue before announcing the thread id.
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
    }
    let tid = unsafe { GetCurrentThreadId() };
    HOOK_TID.store(tid, Ordering::Relaxed);
    let _ = ready.send(tid);
    let mut registered: Vec<i32> = Vec::new();
    let mut hook: Option<HHOOK> = None;
    let mut actions: Vec<HotkeyAction> = Vec::new();
    let mut last: Vec<Option<Instant>> = vec![None; SLOTS];
    let reapply = |registered: &mut Vec<i32>, hook: &mut Option<HHOOK>, actions: &mut Vec<HotkeyAction>| {
        let bindings = wanted.lock().unwrap().clone();
        *actions = bindings.iter().map(|(a, _)| *a).collect();
        let states = apply(&bindings, registered, hook);
        *state.lock().unwrap() = states.clone();
        on_change(states);
    };
    reapply(&mut registered, &mut hook, &mut actions);
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        let slot = match msg.message {
            WM_HOTKEY => Some(msg.wParam.0 as i64 - HOTKEY_ID as i64),
            WM_HOOK_HIT => Some(msg.wParam.0 as i64),
            WM_RECONFIGURE => {
                reapply(&mut registered, &mut hook, &mut actions);
                last = vec![None; SLOTS];
                None
            }
            _ => None,
        };
        if let Some(i) = slot.filter(|i| *i >= 0 && (*i as usize) < actions.len()).map(|i| i as usize) {
            let now = Instant::now();
            if last[i].is_none_or(|l| now.duration_since(l) >= DEBOUNCE) {
                last[i] = Some(now);
                on_trigger(actions[i]);
            }
        }
    }
    release(&mut registered, &mut hook);
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
    fn duplicate_combos_are_detected() {
        let b: Bindings = vec![
            (HotkeyAction::SaveReplay, "Alt+F10".into()),
            (HotkeyAction::AddMarker, "alt + f10".into()),
            (HotkeyAction::Screenshot, "".into()),
        ];
        assert_eq!(duplicate_of(&b, 1), Some(HotkeyAction::SaveReplay));
        assert_eq!(duplicate_of(&b, 0), None);
        assert_eq!(duplicate_of(&b, 2), None);
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
