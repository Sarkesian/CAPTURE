//! Shared memory between the capture worker and the in-game hook (`capture-hook.dll`).
//!
//! One named mapping per game process, created by the worker before it loads the hook into the
//! game: [`Header`] followed by the notice image (BGRA, straight alpha, `NOTICE_MAX_W` x
//! `NOTICE_MAX_H`, rows of `notice_w` pixels). All fields are atomics so both processes can read
//! and write them without locks. The notice image is published with a sequence lock:
//! `notice_seq` is odd while the worker writes it and even (and new) once it is complete.

use std::sync::atomic::{AtomicU32, AtomicU64};

pub const MAGIC: u32 = 0x4B4F_4F48; // "HOOK"
pub const VERSION: u32 = 2;
pub const NOTICE_MAX_W: u32 = 1600;
pub const NOTICE_MAX_H: u32 = 480;
/// Offset of the notice pixels from the start of the mapping.
pub const PIXELS_OFFSET: usize = 4096;
pub const MAPPING_SIZE: usize = PIXELS_OFFSET + (NOTICE_MAX_W * NOTICE_MAX_H * 4) as usize;

/// Name of the mapping for game process `pid`.
pub fn mapping_name(pid: u32) -> String {
    format!("Local\\CAPTURE_hook_{pid}")
}

pub mod state {
    pub const LOADING: u32 = 0;
    /// Present functions hooked; no frame seen yet.
    pub const HOOKED: u32 = 1;
    /// The game is presenting frames through the hook.
    pub const PRESENTING: u32 = 2;
    pub const FAILED: u32 = 3;
    /// Unhooked and unloaded (after `detach`).
    pub const DETACHED: u32 = 4;
}

pub mod api {
    pub const UNKNOWN: u32 = 0;
    pub const D3D11: u32 = 1;
    /// Presents through DXGI with a device the hook cannot draw with yet (Direct3D 12).
    pub const OTHER_DXGI: u32 = 2;
    pub const D3D9: u32 = 3;
}

#[repr(C)]
pub struct Header {
    pub magic: AtomicU32,
    pub version: AtomicU32,
    // ---- worker -> hook ----
    /// 1: unhook and unload.
    pub detach: AtomicU32,
    pub notice_seq: AtomicU32,
    pub notice_w: AtomicU32,
    pub notice_h: AtomicU32,
    /// Scale the image was rendered at, in percent of its 1080p size (e.g. 200 = rendered at
    /// twice the size it should appear on a 1080-line frame).
    pub notice_scale_pct: AtomicU32,
    /// How long the notice stays fully visible, then how long it fades.
    pub notice_show_ms: AtomicU32,
    pub notice_fade_ms: AtomicU32,
    // ---- hook -> worker ----
    /// Set by the one hook that serves this session (a game can hold several hook copies, e.g.
    /// from an earlier CAPTURE build): 0 = unclaimed, else that hook's id. Others stay dormant.
    pub claimed: AtomicU32,
    pub hook_state: AtomicU32,
    pub api: AtomicU32,
    pub backbuffer_w: AtomicU32,
    pub backbuffer_h: AtomicU32,
    pub backbuffer_format: AtomicU32,
    /// Notice (by `notice_seq`) last drawn in the game, and frames it was drawn on.
    pub drawn_seq: AtomicU32,
    pub drawn_frames: AtomicU32,
    pub presents: AtomicU64,
    /// QueryPerformanceCounter value at the latest present.
    pub last_present_qpc: AtomicU64,
    /// Last error from the hook, UTF-8, NUL-terminated.
    pub message: [AtomicU32; 64],
}

const _: () = assert!(std::mem::size_of::<Header>() <= PIXELS_OFFSET);
