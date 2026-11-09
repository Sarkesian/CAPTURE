//! Loading `capture-hook.dll` into a game so CAPTURE's notices are drawn inside its frames
//! (the only way to reach games in exclusive fullscreen), and talking to it through shared
//! memory (`capture_protocol::hook`).
//!
//! Never loaded into a game protected by anti-cheat (those can block or ban for an unknown
//! DLL); such games keep the window notices and hotkeys. 64-bit games get `capture-hook.dll`
//! directly; 32-bit games get `capture-hook32.dll` through `capture-inject32.exe` (a 64-bit
//! process cannot start LoadLibraryW in a 32-bit one).

use std::sync::atomic::Ordering;

use capture_protocol::hook::{self as proto, Header};
use windows::core::{HSTRING, PCSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::*;
use windows::Win32::System::Performance::QueryPerformanceFrequency;
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_UNKNOWN;
use windows::Win32::System::Threading::*;

/// Files or folders next to a game (or in its parent folders) that mean anti-cheat.
const AC_FILES: &[&str] = &[
    "easyanticheat",
    "easyanticheat_eos",
    "start_protected_game.exe",
    "battleye",
    "beclient_x64.dll",
    "beservice.exe",
    "gameguard",
    "xigncode",
    "pnkbstra.exe",
    "punkbuster",
    "equ8",
    "anticheatexpert",
    "acesdk",
    "nprotect",
    "mhyprot",
    "faceit",
];
/// Modules loaded in a game process that mean anti-cheat.
const AC_MODULES: &[&str] = &["easyanticheat", "beclient", "bedaisy", "gameguard", "xigncode", "eos_anticheat", "vgc", "ace-base", "acesdk", "mhyprot", "npggnt", "equ8"];

/// Why CAPTURE will not load its hook into `pid`, if any.
pub fn refusal(pid: u32) -> Option<String> {
    let exe = exe_path(pid)?;
    let lower = |s: &str| s.to_ascii_lowercase();
    let mut dir = exe.parent();
    for _ in 0..3 {
        let Some(d) = dir else { break };
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let name = lower(&e.file_name().to_string_lossy());
                if let Some(ac) = AC_FILES.iter().find(|a| name.contains(*a)) {
                    return Some(format!("anti-cheat ({ac}) found next to the game"));
                }
            }
        }
        dir = d.parent();
    }
    for m in modules(pid) {
        let m = lower(&m);
        if let Some(ac) = AC_MODULES.iter().find(|a| m.contains(*a)) {
            return Some(format!("anti-cheat module ({ac}) loaded in the game"));
        }
    }
    None
}

/// The game is a 32-bit process (on 64-bit Windows).
fn is_32bit(pid: u32) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let (mut machine, mut native) = (Default::default(), Default::default());
        let r = IsWow64Process2(h, &mut machine, Some(&mut native));
        let _ = CloseHandle(h);
        r.is_ok() && machine != IMAGE_FILE_MACHINE_UNKNOWN
    }
}

fn exe_path(pid: u32) -> Option<std::path::PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]).into())
    }
}

fn modules(pid: u32) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) else { return out };
        let mut e = MODULEENTRY32W { dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32, ..Default::default() };
        if Module32FirstW(snap, &mut e).is_ok() {
            loop {
                let n = e.szModule.iter().position(|&c| c == 0).unwrap_or(e.szModule.len());
                out.push(String::from_utf16_lossy(&e.szModule[..n]));
                if Module32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// A file shipped next to the worker; in a development build, the 32-bit files come from the
/// i686 build output (`capture-hook32.dll` is built as `capture_hook.dll` there).
fn shipped(names: &[&str]) -> Option<std::path::PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let dev32 = dir.parent()?.join("i686-pc-windows-msvc").join("release");
    names
        .iter()
        .flat_map(|n| [dir.join(n), dev32.join(if *n == "capture-hook32.dll" { "capture_hook.dll" } else { n })])
        .find(|p| p.exists())
}

/// The hook stays loaded in a game until the game exits (unloading it safely is not possible
/// while other overlays may hook the same functions), so the game gets a copy in the temp
/// folder, one per build: CAPTURE's own files stay replaceable by an update while games run.
fn private_copy(src: &std::path::Path, bits32: bool) -> Result<std::path::PathBuf, String> {
    let meta = std::fs::metadata(src).map_err(|e| e.to_string())?;
    let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let dir = std::env::temp_dir().join("CAPTURE-hook").join(format!("{}-{stamp}", meta.len()));
    let dst = dir.join(if bits32 { "capture-hook32.dll" } else { "capture-hook.dll" });
    if !dst.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot prepare the in-game overlay: {e}"))?;
        let tmp = dir.join(format!("{}.part", std::process::id()));
        std::fs::copy(src, &tmp).map_err(|e| format!("cannot prepare the in-game overlay: {e}"))?;
        // Another worker may have put it there meanwhile (or a game holds it): either is fine.
        if std::fs::rename(&tmp, &dst).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    Ok(dst)
}

/// CAPTURE's hook inside one game process. Dropping it unloads the hook from the game.
pub struct GameHook {
    map: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    qpf: u64,
}

unsafe impl Send for GameHook {}
unsafe impl Sync for GameHook {}

impl GameHook {
    /// Create the shared memory and load the hook into `pid`.
    pub fn attach(pid: u32) -> Result<Self, String> {
        if let Some(why) = refusal(pid) {
            return Err(why);
        }
        let bits32 = is_32bit(pid);
        let shipped_dll = if bits32 { shipped(&["capture-hook32.dll"]) } else { shipped(&["capture-hook.dll", "capture_hook.dll"]) }.ok_or("the in-game overlay files are missing next to the worker")?;
        let dll = private_copy(&shipped_dll, bits32)?;
        unsafe {
            let name = HSTRING::from(proto::mapping_name(pid));
            let map = CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, proto::MAPPING_SIZE as u32, &name).map_err(|e| format!("shared memory: {e}"))?;
            let view = MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, proto::MAPPING_SIZE);
            if view.Value.is_null() {
                let _ = CloseHandle(map);
                return Err("shared memory could not be mapped".into());
            }
            let mut qpf = 0i64;
            let _ = QueryPerformanceFrequency(&mut qpf);
            let hook = Self { map, view, qpf: qpf.max(1) as u64 };
            let h = hook.header();
            h.detach.store(0, Ordering::Relaxed);
            h.claimed.store(0, Ordering::Relaxed);
            h.hook_state.store(proto::state::LOADING, Ordering::Relaxed);
            h.version.store(proto::VERSION, Ordering::Relaxed);
            h.magic.store(proto::MAGIC, Ordering::Release);
            if bits32 {
                let helper = shipped(&["capture-inject32.exe"]).ok_or("capture-inject32.exe is missing next to the worker")?;
                let out = std::process::Command::new(helper).arg(pid.to_string()).arg(&dll).output().map_err(|e| format!("32-bit loader: {e}"))?;
                if !out.status.success() {
                    return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
                }
            } else {
                load_library_in(pid, &dll)?;
            }
            Ok(hook)
        }
    }

    pub fn header(&self) -> &Header {
        unsafe { &*(self.view.Value as *const Header) }
    }

    /// The hook can draw in this game right now: the game is presenting frames through it with
    /// a graphics API it draws with. Each notice goes either in-game (this) or to the notice
    /// window, never both; until the hook is drawing (just loaded, or an API it cannot draw
    /// with yet), notices use the window so none is lost.
    pub fn usable(&self) -> bool {
        let h = self.header();
        match h.hook_state.load(Ordering::Acquire) {
            proto::state::PRESENTING => {
                let now = {
                    let mut c = 0i64;
                    unsafe {
                        let _ = windows::Win32::System::Performance::QueryPerformanceCounter(&mut c);
                    }
                    c as u64
                };
                let age = now.saturating_sub(h.last_present_qpc.load(Ordering::Relaxed)) as f64 / self.qpf as f64;
                matches!(h.api.load(Ordering::Relaxed), proto::api::D3D11 | proto::api::D3D9) && age < 2.0
            }
            _ => false,
        }
    }

    /// Hand a notice image (BGRA, straight alpha) to the hook; it shows it for `show_ms`, then
    /// fades it out over `fade_ms`.
    pub fn show(&self, w: u32, h: u32, scale_pct: u32, pixels: &[u8], show_ms: u32, fade_ms: u32) {
        if w == 0 || h == 0 || w > proto::NOTICE_MAX_W || h > proto::NOTICE_MAX_H || pixels.len() < (w * h * 4) as usize {
            return;
        }
        let hd = self.header();
        let seq = hd.notice_seq.load(Ordering::Relaxed);
        let writing = if seq % 2 == 0 { seq + 1 } else { seq + 2 };
        hd.notice_seq.store(writing, Ordering::Release);
        unsafe {
            let dst = (self.view.Value as *mut u8).add(proto::PIXELS_OFFSET);
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), dst, (w * h * 4) as usize);
        }
        hd.notice_w.store(w, Ordering::Relaxed);
        hd.notice_h.store(h, Ordering::Relaxed);
        hd.notice_scale_pct.store(scale_pct, Ordering::Relaxed);
        hd.notice_show_ms.store(show_ms, Ordering::Relaxed);
        hd.notice_fade_ms.store(fade_ms, Ordering::Relaxed);
        hd.notice_seq.store(writing + 1, Ordering::Release);
    }

    /// The hook's last error message, if any.
    pub fn message(&self) -> String {
        let bytes: Vec<u8> = self.header().message.iter().flat_map(|w| w.load(Ordering::Relaxed).to_le_bytes()).collect();
        let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..n]).into_owned()
    }
}

impl Drop for GameHook {
    fn drop(&mut self) {
        // The hook restores the game's functions and unloads itself; the mapping lives on in
        // the game until it lets go of it.
        let h = self.header();
        eprintln!(
            "in-game overlay: state {} api {} presents {} notice frames {} backbuffer {}x{} fmt {}{}",
            h.hook_state.load(Ordering::Relaxed),
            h.api.load(Ordering::Relaxed),
            h.presents.load(Ordering::Relaxed),
            h.drawn_frames.load(Ordering::Relaxed),
            h.backbuffer_w.load(Ordering::Relaxed),
            h.backbuffer_h.load(Ordering::Relaxed),
            h.backbuffer_format.load(Ordering::Relaxed),
            match self.message() {
                m if m.is_empty() => String::new(),
                m => format!("; {m}"),
            }
        );
        h.detach.store(1, Ordering::Release);
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.map);
        }
    }
}

/// `LoadLibraryW(dll)` on a new thread inside `pid`.
fn load_library_in(pid: u32, dll: &std::path::Path) -> Result<(), String> {
    unsafe {
        let access = PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_VM_READ;
        let proc = OpenProcess(access, false, pid).map_err(|e| format!("cannot open the game process: {e}"))?;
        let wide: Vec<u16> = dll.as_os_str().encode_wide_nul();
        let bytes = wide.len() * 2;
        let remote = VirtualAllocEx(proc, None, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        let result = (|| {
            if remote.is_null() {
                return Err("cannot allocate memory in the game".to_string());
            }
            WriteProcessMemory(proc, remote, wide.as_ptr() as *const _, bytes, None).map_err(|e| e.to_string())?;
            let k32 = GetModuleHandleW(windows::core::w!("kernel32.dll")).map_err(|e| e.to_string())?;
            let load = GetProcAddress(k32, PCSTR(b"LoadLibraryW\0".as_ptr())).ok_or("LoadLibraryW not found")?;
            let start: LPTHREAD_START_ROUTINE = Some(std::mem::transmute(load));
            let thread = CreateRemoteThread(proc, None, 0, start, Some(remote), 0, None).map_err(|e| format!("cannot start the hook in the game: {e}"))?;
            let waited = WaitForSingleObject(thread, 5000);
            let mut code = 0u32;
            let _ = GetExitCodeThread(thread, &mut code);
            let _ = CloseHandle(thread);
            if waited != WAIT_OBJECT_0 {
                return Err("the game did not load the hook in time".into());
            }
            if code == 0 {
                return Err("the game refused to load the hook".into());
            }
            Ok(())
        })();
        if !remote.is_null() {
            let _ = VirtualFreeEx(proc, remote, 0, MEM_RELEASE);
        }
        let _ = CloseHandle(proc);
        result
    }
}

trait WideNul {
    fn encode_wide_nul(&self) -> Vec<u16>;
}

impl WideNul for std::ffi::OsStr {
    fn encode_wide_nul(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}
