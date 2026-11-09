//! Inline function hooks ("detours"), 32- and 64-bit.
//!
//! Function tables (vtables) are not a reliable way into a game's graphics calls: other
//! overlays (Steam's, for one) give each device its own private copy of the table, so patching
//! any one table misses the game's device. Hooking the function itself catches every caller.
//!
//! How (the classic MinHook design):
//! - the function's first whole instructions covering 5 bytes are copied to a trampoline, with
//!   relative addresses fixed up for their new place (iced-x86's block encoder), followed by a
//!   jump back to the rest of the function. Calling the trampoline = calling the original.
//! - the function's first 5 bytes become `jmp rel32` to our function (on 64-bit, to a relay
//!   in the trampoline page, which is allocated within 2 GB so the 5-byte jump reaches; the
//!   relay jumps anywhere).
//! - those 5 bytes are written while every other thread of the game is suspended, and never
//!   while one of them is executing inside them.
//! A function already hooked by someone else starts with their jump; it is relocated like any
//! other instruction, so both hooks run (theirs after ours).

use std::ffi::c_void;

use iced_x86::{BlockEncoder, BlockEncoderOptions, Decoder, DecoderOptions, Instruction, InstructionBlock};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::{FlushInstructionCache, GetThreadContext, CONTEXT};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
use windows::Win32::System::Memory::*;
use windows::Win32::System::Threading::*;

const BITNESS: u32 = if cfg!(target_pointer_width = "64") { 64 } else { 32 };
const JMP_LEN: usize = 5;

/// Hook `target` so it calls `detour`; returns the address to call for the original function.
pub unsafe fn hook(target: usize, detour: usize) -> Result<usize, String> {
    // Whole instructions covering the 5 bytes we overwrite.
    let code = std::slice::from_raw_parts(target as *const u8, 64);
    let mut decoder = Decoder::with_ip(BITNESS, code, target as u64, DecoderOptions::NONE);
    let mut instrs: Vec<Instruction> = Vec::new();
    let mut len = 0usize;
    while len < JMP_LEN {
        let i = decoder.decode();
        if i.is_invalid() {
            return Err(format!("cannot decode the function at {target:#x}"));
        }
        len += i.len();
        instrs.push(i);
    }

    let page = alloc_near(target).ok_or("no memory for the trampoline")?;
    // Trampoline: relocated instructions, then a jump back after them.
    let enc = BlockEncoder::encode(BITNESS, InstructionBlock::new(&instrs, page as u64), BlockEncoderOptions::NONE).map_err(|e| format!("cannot relocate the function's start: {e}"))?;
    let mut tramp = enc.code_buffer;
    let at = page + tramp.len();
    jmp_abs(&mut tramp, at, target + len);
    // Relay (64-bit): reachable from the target with a 5-byte jump, jumps anywhere.
    let relay = page + 128;
    if tramp.len() > 128 {
        return Err("trampoline too large".into());
    }
    let mut relay_code = Vec::new();
    jmp_abs(&mut relay_code, relay, detour);
    std::ptr::copy_nonoverlapping(tramp.as_ptr(), page as *mut u8, tramp.len());
    std::ptr::copy_nonoverlapping(relay_code.as_ptr(), relay as *mut u8, relay_code.len());
    let mut old = PAGE_PROTECTION_FLAGS(0);
    let _ = VirtualProtect(page as *const c_void, 4096, PAGE_EXECUTE_READ, &mut old);

    // The patch: jmp rel32 to the relay, rest of the moved instructions padded with nops.
    let dest = if BITNESS == 64 { relay } else { detour };
    let rel = (dest as i64 - (target as i64 + JMP_LEN as i64)) as i32;
    let mut patch = vec![0xE9u8];
    patch.extend_from_slice(&rel.to_le_bytes());
    patch.resize(len, 0x90);
    write_code(target, &patch)?;
    Ok(page)
}

/// Absolute jump to `to`, placed at `at`.
fn jmp_abs(out: &mut Vec<u8>, at: usize, to: usize) {
    if BITNESS == 64 {
        out.extend_from_slice(&[0xFF, 0x25, 0, 0, 0, 0]); // jmp [rip+0]
        out.extend_from_slice(&(to as u64).to_le_bytes());
    } else {
        let rel = (to as i64 - (at as i64 + 5)) as i32;
        out.push(0xE9);
        out.extend_from_slice(&rel.to_le_bytes());
    }
}

/// An executable page within 2 GB of `target` (anywhere on 32-bit).
unsafe fn alloc_near(target: usize) -> Option<usize> {
    let flags = MEM_RESERVE | MEM_COMMIT;
    if BITNESS == 32 {
        let p = VirtualAlloc(None, 4096, flags, PAGE_EXECUTE_READWRITE);
        return (!p.is_null()).then_some(p as usize);
    }
    const GRAN: usize = 0x10000;
    const RANGE: usize = 0x7000_0000;
    let base = target & !(GRAN - 1);
    let mut d = GRAN;
    while d < RANGE {
        for a in [base.wrapping_sub(d), base.wrapping_add(d)] {
            let p = VirtualAlloc(Some(a as *const c_void), 4096, flags, PAGE_EXECUTE_READWRITE);
            if !p.is_null() {
                return Some(p as usize);
            }
        }
        d += GRAN;
    }
    None
}

/// Write `bytes` over code at `at` with every other thread of this process suspended and none
/// of them executing inside the range.
unsafe fn write_code(at: usize, bytes: &[u8]) -> Result<(), String> {
    for _ in 0..100 {
        let threads = suspend_others();
        let busy = threads.0.iter().any(|&h| ip_of(h).is_some_and(|ip| ip >= at && ip < at + bytes.len()));
        if !busy {
            let mut old = PAGE_PROTECTION_FLAGS(0);
            let ok = VirtualProtect(at as *const c_void, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old).is_ok();
            if ok {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len());
                let _ = VirtualProtect(at as *const c_void, bytes.len(), old, &mut old);
                let _ = FlushInstructionCache(GetCurrentProcess(), Some(at as *const c_void), bytes.len());
            }
            resume(threads);
            return if ok { Ok(()) } else { Err("the function's code could not be made writable".into()) };
        }
        resume(threads);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Err("the game kept executing the code being hooked".into())
}

/// Suspend every other thread of this process. Nothing may allocate memory between the first
/// suspension and the matching `resume`: a suspended thread may hold the heap lock, and this
/// thread would then wait for it forever. So the thread list is read first, and the result
/// vector is sized up front.
unsafe fn suspend_others() -> (Vec<HANDLE>, Vec<u32>) {
    let (pid, me) = (GetCurrentProcessId(), GetCurrentThreadId());
    let mut ids: Vec<u32> = Vec::with_capacity(512);
    if let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) {
        let mut e = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
        if Thread32First(snap, &mut e).is_ok() {
            loop {
                if e.th32OwnerProcessID == pid && e.th32ThreadID != me {
                    ids.push(e.th32ThreadID);
                }
                if Thread32Next(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    let mut out: Vec<HANDLE> = Vec::with_capacity(ids.len());
    for &id in &ids {
        if let Ok(h) = OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT, false, id) {
            if SuspendThread(h) != u32::MAX {
                out.push(h); // within capacity: no allocation
            } else {
                let _ = CloseHandle(h);
            }
        }
    }
    // `ids` is returned (and freed only after `resume`), not freed here.
    (out, ids)
}

/// Resume what `suspend_others` suspended; its memory is freed only once all are running.
unsafe fn resume((threads, ids): (Vec<HANDLE>, Vec<u32>)) {
    for &h in &threads {
        ResumeThread(h);
        let _ = CloseHandle(h);
    }
    drop(threads);
    drop(ids);
}

/// Instruction pointer of a suspended thread.
unsafe fn ip_of(h: HANDLE) -> Option<usize> {
    #[repr(C, align(16))]
    struct Aligned(CONTEXT);
    let mut c: Aligned = std::mem::zeroed();
    #[cfg(target_pointer_width = "64")]
    {
        c.0.ContextFlags = windows::Win32::System::Diagnostics::Debug::CONTEXT_CONTROL_AMD64;
        GetThreadContext(h, &mut c.0).ok()?;
        Some(c.0.Rip as usize)
    }
    #[cfg(target_pointer_width = "32")]
    {
        c.0.ContextFlags = windows::Win32::System::Diagnostics::Debug::CONTEXT_CONTROL_X86;
        GetThreadContext(h, &mut c.0).ok()?;
        Some(c.0.Eip as usize)
    }
}
