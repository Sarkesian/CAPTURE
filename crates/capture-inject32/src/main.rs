//! `capture-inject32 <pid> <dll>`: load `dll` into the 32-bit process `pid` (LoadLibraryW on a
//! new thread there). The 64-bit capture worker runs this for 32-bit games, because the
//! address of LoadLibraryW it would pass is only valid in processes of its own bitness.
//! Exit code 0 = loaded; otherwise the reason is on stderr.

use std::os::windows::ffi::OsStrExt;

use windows::core::{w, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::*;
use windows::Win32::System::Threading::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(pid), Some(dll)) = (args.get(1).and_then(|p| p.parse::<u32>().ok()), args.get(2)) else {
        eprintln!("usage: capture-inject32 <pid> <dll>");
        std::process::exit(2);
    };
    if let Err(e) = inject(pid, dll) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn inject(pid: u32, dll: &str) -> Result<(), String> {
    unsafe {
        let access = PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_VM_READ;
        let proc = OpenProcess(access, false, pid).map_err(|e| format!("cannot open the game process: {e}"))?;
        let wide: Vec<u16> = std::ffi::OsStr::new(dll).encode_wide().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * 2;
        let remote = VirtualAllocEx(proc, None, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        let result = (|| {
            if remote.is_null() {
                return Err("cannot allocate memory in the game".to_string());
            }
            WriteProcessMemory(proc, remote, wide.as_ptr() as *const _, bytes, None).map_err(|e| e.to_string())?;
            let k32 = GetModuleHandleW(w!("kernel32.dll")).map_err(|e| e.to_string())?;
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
