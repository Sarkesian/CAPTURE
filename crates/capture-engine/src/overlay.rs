//! Small on-screen notice ("Recording FAULTHEART", "Clip saved · 1m 42s").
//!
//! A native layered popup on its own thread: topmost, click-through, never activated (so it
//! cannot pull a game out of focus), excluded from screen capture via
//! `WDA_EXCLUDEFROMCAPTURE` (it never appears in recordings), shown on the monitor of the
//! foreground window, and faded out after ~3 s. Over exclusive-fullscreen games Windows may
//! not show it at all; borderless/windowed games are fine.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Recording,
    Saved,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub title: String,
    pub detail: String,
    pub kind: Kind,
}

const WM_SHOW: u32 = WM_APP + 2;
const SHOW_MS: u32 = 2800;
const ALPHA: u8 = 238;

static CURRENT: Mutex<Option<Notice>> = Mutex::new(None);
static SCALE_PCT: AtomicU32 = AtomicU32::new(100);
static FADE: AtomicU32 = AtomicU32::new(ALPHA as u32);

pub struct Overlay {
    tid: u32,
    pending: Arc<Mutex<Option<Notice>>>,
}

impl Overlay {
    pub fn start() -> Arc<Self> {
        let pending = Arc::new(Mutex::new(None));
        let (tx, rx) = std::sync::mpsc::channel();
        let p = pending.clone();
        std::thread::Builder::new()
            .name("overlay".into())
            .spawn(move || run(p, tx))
            .expect("spawn overlay thread");
        let tid = rx.recv().unwrap_or(0);
        Arc::new(Self { tid, pending })
    }

    pub fn show(&self, n: Notice) {
        *self.pending.lock().unwrap() = Some(n);
        unsafe {
            let _ = PostThreadMessageW(self.tid, WM_SHOW, WPARAM(0), LPARAM(0));
        }
    }
}

fn s(v: i32) -> i32 {
    v * SCALE_PCT.load(Ordering::Relaxed) as i32 / 100
}

fn run(pending: Arc<Mutex<Option<Notice>>>, ready: std::sync::mpsc::Sender<u32>) {
    unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        let _ = ready.send(GetCurrentThreadId());
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("CaptureOverlay");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            eprintln!("overlay: RegisterClassW failed: {:?}", windows::core::Error::from_thread());
        }
        let created = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class,
            w!("CAPTURE notice"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinst.into()),
            None,
        );
        let hwnd = match created {
            Ok(h) => h,
            Err(e) => {
                eprintln!("overlay: CreateWindowExW failed: {e}");
                return;
            }
        };
        // Never appear in any screen capture, including our own Display capture.
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_SHOW {
                if let Some(n) = pending.lock().unwrap().take() {
                    present(hwnd, n);
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn present(hwnd: HWND, n: Notice) {
    let mon = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(mon, &mut mi);
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    SCALE_PCT.store(dx * 100 / 96, Ordering::Relaxed);
    let (wdt, hgt) = (s(380), s(66));
    let x = mi.rcMonitor.right - wdt - s(24);
    let y = mi.rcMonitor.top + s(24);
    *CURRENT.lock().unwrap() = Some(n);
    let _ = SetWindowRgn(hwnd, Some(CreateRoundRectRgn(0, 0, wdt + 1, hgt + 1, s(18), s(18))), true);
    FADE.store(ALPHA as u32, Ordering::Relaxed);
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), ALPHA, LWA_ALPHA);
    let _ = KillTimer(Some(hwnd), 2);
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, wdt, hgt, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    let _ = InvalidateRect(Some(hwnd), None, true);
    SetTimer(Some(hwnd), 1, SHOW_MS, None);
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wp.0 == 1 => {
            let _ = KillTimer(Some(hwnd), 1);
            SetTimer(Some(hwnd), 2, 16, None);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == 2 => {
            let a = FADE.load(Ordering::Relaxed).saturating_sub(24);
            FADE.store(a, Ordering::Relaxed);
            if a == 0 {
                let _ = KillTimer(Some(hwnd), 2);
                let _ = ShowWindow(hwnd, SW_HIDE);
            } else {
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), a as u8, LWA_ALPHA);
            }
            LRESULT(0)
        }
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let bg = CreateSolidBrush(rgb(22, 24, 30));
            FillRect(dc, &rc, bg);
            let _ = DeleteObject(bg.into());
            if let Some(n) = CURRENT.lock().unwrap().clone() {
                let dot = match n.kind {
                    Kind::Recording => rgb(239, 68, 68),
                    Kind::Saved => rgb(52, 211, 153),
                    Kind::Failed => rgb(251, 191, 36),
                };
                let br = CreateSolidBrush(dot);
                let old_b = SelectObject(dc, br.into());
                let old_p = SelectObject(dc, GetStockObject(NULL_PEN));
                let cy = rc.bottom / 2;
                let _ = Ellipse(dc, s(18), cy - s(7), s(32), cy + s(7));
                SelectObject(dc, old_p);
                SelectObject(dc, old_b);
                let _ = DeleteObject(br.into());
                SetBkMode(dc, TRANSPARENT);
                let font = |h: i32, wt: i32| {
                    CreateFontW(-s(h), 0, 0, 0, wt, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"))
                };
                let f1 = font(16, 600);
                let f2 = font(13, 400);
                let draw = |f: HFONT, color: COLORREF, top: i32, bottom: i32, text: &str| {
                    let old = SelectObject(dc, f.into());
                    SetTextColor(dc, color);
                    let mut r = RECT { left: s(44), top, right: rc.right - s(14), bottom };
                    let mut wide: Vec<u16> = text.encode_utf16().collect();
                    DrawTextW(dc, &mut wide, &mut r, DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
                    SelectObject(dc, old);
                };
                draw(f1, rgb(240, 242, 246), s(11), s(34), &n.title);
                draw(f2, rgb(160, 168, 182), s(35), s(56), &n.detail);
                let _ = DeleteObject(f1.into());
                let _ = DeleteObject(f2.into());
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
