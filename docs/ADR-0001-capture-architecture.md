# ADR-0001: Native capture worker (WGC + WASAPI + LGPL libav) instead of libobs

**Status:** Accepted, 2026-11-08 · **Evidence:** `docs/PHASE0_CAPABILITY_REPORT.md`

## Context
CAPTURE must save the last X minutes as a standard H.264/AAC MP4 that has one `Preview Mix` plus
isolated, synchronized `Game Audio`, `Microphone` and `Voice Chat` tracks. It must probe
encoders honestly, never inject into games by default, stay small, and avoid accumulating data on C:.

Two working spikes were built and measured on this machine.

- **A: native.** Windows Graphics Capture → D3D11 texture → hardware encoder through LGPL FFmpeg 8.1
  (`libavcodec`/`libavformat`), plus WASAPI process loopback, endpoint loopback and mic capture.
- **B: libobs.** OBS 33.0.1 (portable) driven over obs-websocket, using Application Audio
  Capture sources, NVENC and its built-in replay buffer.

## Findings that decided it

| Criterion | A: native | B: libobs |
|---|---|---|
| Video capture | ✅ WGC window/display, real game captured, no injection | ✅ plus an injection hook (`graphics-hook64.dll`) for exclusive fullscreen |
| Per-process audio isolation | ✅ ≈127 dB (raw), ≈108 dB after AAC | ✅ ≈80 dB after AAC |
| Track names in MP4 | ✅ `handler_name` per stream | ❌ every stream is `SoundHandler` |
| Unwanted default sources | none: everything is explicit | auto-created Desktop Audio + Mic/Aux that cannot be removed |
| Hardware encode | ✅ NVENC zero-copy, 506 fps @1080p in process | ✅ NVENC |
| Constant frame rate | ⚠ must be implemented (WGC is variable rate) | ✅ built in |
| A/V clock | one process, one QPC timeline under our control | handled by libobs internally |
| Footprint | ≈123 MB of FFmpeg DLLs (can be trimmed with a custom build later) | ≈553 MB, Qt, 668 MB RAM in the spike |
| Licence | FFmpeg **LGPL** (v3, because of `--enable-version3`), dynamically linked; CAPTURE keeps its own licence | libobs **GPL-2.0+**: embedding it makes CAPTURE GPL. Driving `obs64.exe` at arm's length avoids that but ships all of OBS and its UI. |
| Fit with "simple, small, no hooks by default" | strong | weak: it fights OBS's own defaults and configuration model |

## Decision
Build **A**: a dedicated native Rust capture worker.

1. **Video:** WGC (`windows` crate; `CreateForWindow` / `CreateForMonitor`, border off) produces BGRA
   D3D11 textures. These go to a libav `AV_PIX_FMT_D3D11` hardware frames pool and then to the first encoder that opens from
   `h264_nvenc` → `h264_amf` → `h264_qsv` (D3D11 hwmapped to QSV) → `h264_mf` (hardware MFT) →
   `h264_mf` software with CPU readback. The chosen encoder and the reason for each rejection are shown in Settings › Advanced.
   CAPTURE produces constant frame rate output by re-submitting the last texture on the target-fps clock, with timestamps taken from QPC.
2. **Audio:** WASAPI event-driven shared mode at 48 kHz float.
   - *Game* and *Voice chat* each use process loopback (`INCLUDE_TARGET_PROCESS_TREE`).
   - *System output* uses endpoint loopback, labelled as such and never mixed into an isolated track.
   - *Microphone* uses endpoint capture and is OFF by default.
   Each stream fills gaps against the QPC timeline. Each is encoded separately with AAC, and Preview Mix is encoded from a sum of the enabled sources.
3. **Replay ring:** encoded packets are written by libavformat into short (~2 s) keyframe-aligned segments in
   `<root>\Buffer`. A segment index has generation IDs and protection references. Saving = protect
   the segments that cover the window → remux with `-c copy` semantics into `Clips\<Game>\…mp4`
   (`+faststart`, `handler_name` labels, Preview Mix as default) → verify with a probe → atomic rename → `Saved`.
   The segment container (fragmented MP4 or MPEG-TS) will be chosen in Phase 1 by a forced-kill recovery test.
4. **Shell:** Tauri 2 + Svelte UI. The capture worker runs as a **separate process** behind a versioned, typed IPC
   (JSON over a named pipe, `protocol_version` field). A UI crash never stops recording, and a worker crash is reported
   and restarted.
5. **Bindings:** `ffmpeg-sys-next` 9.0.1 bindings, generated against the pinned FFmpeg headers. libclang is a build-time
   tool only, kept in `third_party/`.

## Consequences
- We own CFR pacing, A/V alignment and the segment ring. These are the main risks for Phases 1–2 and are covered by tests.
- Exclusive-fullscreen games that WGC cannot see get an honest "can't capture in this mode, try
  borderless or Display capture" message. **No hook is installed.** If real-game testing shows WGC coverage is
  unacceptable, revisit with a separately-licensed, opt-in libobs worker. This ADR records that as **Plan B, not a commitment**.
- AMF and QSV paths cannot be verified on this machine. They are probed honestly and labelled "unverified on this
  build machine" in docs until tested on such hardware.

## Licence duties (engineering notes, not legal advice)
- Ship the FFmpeg DLLs unmodified and replaceable, include `LICENSE.txt` (LGPL v3), and point to the exact source
  (BtbN `autobuild-2026-11-07-12-47`, FFmpeg `n8.1.3-34-ga35157abb9`). Keep a dependency bill of materials.
- Do not use `libopenh264` from this build (patent coverage applies only to Cisco's own binaries). The CPU fallback uses the
  OS-provided Media Foundation encoder.
- AAC: FFmpeg's native `aac` encoder is the default; `aac_mf` (OS-licensed) is the alternative if distribution
  requires it. **Open question for the user before any public release.**
- No OBS code or binaries ship with CAPTURE. OBS appears in this repository only as an optional spike download.
