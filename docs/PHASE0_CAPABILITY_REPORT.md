# Phase 0 capability report

**Date:** 2026-11-08 · **Branch:** `phase0/spikes` · **Machine:** the user's workstation (below).
Every result here comes from a run on this machine. Representative media (`mux1`, `game1`) is kept in
`spikes/out/`, which git ignores because it contains real screen content. Bulkier spike output (`audio1`,
`obs1`) was deleted after measurement; its numbers are recorded below. Re-run commands are listed at the end.

## 1. Environment

| Item | Finding |
|---|---|
| OS | Windows 11 Pro 25H2, build 26200. Process-loopback audio needs build ≥ 20348, so it is supported. |
| CPU / RAM | Intel i5-13600KF (14C/20T, no iGPU), 32 GB |
| GPU | NVIDIA RTX 4090, driver 617.14 (32.0.16.1714) |
| Display | 3840×2160 @ 143 Hz |
| Drives | C: 210 GB free · D: 1.5 TB free · `D:\Captures` does not exist yet |
| Toolchain | Rust 1.99.1 (rustup at `D:\Programs\Rust`, set `RUSTUP_HOME`/`CARGO_HOME`); VS 2022 17.14 + Win SDK 10.0.26100; Node 22.19; pnpm 10.33; WebView2 157 |
| Not present (fetched into `third_party/`, see `scripts/fetch-deps.ps1`) | FFmpeg (LGPL 8.1.3 shared), libclang 18.1.1 (build-time only), OBS 33.0.1 portable (spike B only) |
| Audio inputs | `Microphone (G06-BT)`: default, Bluetooth headset · `Microphone (Steam Streaming Microphone)`: virtual |
| Audio outputs | `Speakers (Logitech G560)`: default · `Speakers (G06-BT)` · `MSI G321CU (NVIDIA HDA)` · 2× Steam Streaming virtual endpoints |
| Games available | Steam: none (only Blender). itch: **FAULTHEART** (Godot), cabin-hollow. Epic: none. |
| FrameCut | `D:\Projects\ColdFusion\Main\workspace\framecut-studio-qol`. Electron + ffprobe-static. It imports **only the first audio stream** (`desktop/tools.cjs:46`). Read only; not modified. |

## 2. Encoders (FFmpeg 8.1.3 LGPL)

| Encoder | Init on this PC | Notes |
|---|---|---|
| `h264_nvenc` / `hevc_nvenc` / `av1_nvenc` | ✅ OK | In process, zero-copy from D3D11 textures: **506 fps at 1080p**, BT.709 tagged |
| `h264_amf` | ❌ `amfrt64.dll` missing | Correct: there is no AMD GPU. The AMF code path cannot be verified on this machine. |
| `h264_qsv` | ❌ MFX session -9 | Correct: the 13600**KF** has no iGPU. QSV also needs `qsv` frames (hwmap from D3D11), not raw D3D11 frames. |
| `h264_mf` (hardware MFT) | ✅ OK | ~5× real time at 1080p60 on test content, low CPU |
| `h264_mf` (software MFT) | ✅ OK | ~12× real time at 1080p60 on synthetic content. It **rejects D3D11 frames** (`Failed to set D3D manager: 80004001`), so the CPU fallback must read frames back to system memory. |
| `libopenh264` | ✅ OK | **Not to be used.** Cisco's patent coverage applies only to Cisco-distributed binaries. |

## 3. Video capture (Windows Graphics Capture)

| Test | Result |
|---|---|
| 4K desktop, `gfxcapture` → `h264_nvenc` on the GPU | ✅ 3840×2160 H.264 Main, 6.0 s, playable. **About 47 fps average: WGC delivers frames only when content changes.** |
| Real game window (FAULTHEART, by HWND) | ✅ 1600×900, 221 frames / 6 s (the menu presents at about 30 fps), mean luma 62.5, **no black frames**. A visually verified still is at `spikes/out/game1/frame3s.png`. |
| Yellow capture border | Not drawn: `display_border=0` is honoured on Win11. |

**Consequence:** CAPTURE must produce **constant frame rate** output itself, by re-submitting the
last texture on a target-fps clock, because WGC by itself produces variable frame rate, which editors handle poorly.

## 4. Audio capture (WASAPI, `spikes/audio_spike`)

Two `ffplay` processes played a 440 Hz "game" tone and an 880 Hz "voice chat" tone. All sources
were captured at the same time as 48 kHz stereo float and stamped with QPC time.

| Track | Mode | 440 Hz | 880 Hz | Notes |
|---|---|---|---|---|
| game (pid A) | process loopback, includes child processes | **−59.0 dB** | −186.1 dB | ≈127 dB isolation |
| chat (pid B) | process loopback, includes child processes | −186.2 dB | **−57.7 dB** | ≈128 dB isolation |
| loopback | **endpoint loopback (whole-device mix)** | −58.5 dB | −58.5 dB | contains both, as expected |
| mic (G06-BT) | endpoint capture | — | — | delivered **pure digital silence** (packets on time, all zeros) |

- Correlation between the game and chat tracks is **0.000**, so they are not duplicates.
- Start skew between streams after QPC alignment is 5.7–15.7 ms. Over 8 s there were 0 silent-flag packets and at most 1 discontinuity per stream.
- Process loopback delivers continuous packets even when the target is quiet. It requires an explicit format (48 kHz float plus `AUTOCONVERTPCM`).
- Real game audio: FAULTHEART's own process track captured the game's music (peak 0.026, −50.7 dBFS).
- **Mic not validated with a live signal.** The default mic, G06-BT, returned zeros, which most likely means the headset is idle or muted. A live voice test is a **user playtest item**. Product rule: a device that reports *active* but delivers exact zeros must show as `No signal`, not `Capturing`.
- **Bluetooth headset caveat:** opening a BT headset mic commonly switches the headset to the hands-free profile, which degrades its playback. The mic card must warn about this.
- **Virtual-device hazard:** "Steam Streaming Microphone" is a selectable input. The no-silent-fallback rule must never treat it as a substitute for the user's chosen mic.

## 5. Multitrack MP4 round trip (`spikes/out/mux1/clip.mp4`)

```
stream 0 h264 3840x2160                default=1 handler_name=Video
stream 1 aac 48k stereo                default=1 handler_name=Preview Mix
stream 2 aac 48k stereo                default=0 handler_name=Game Audio
stream 3 aac 48k stereo                default=0 handler_name=Microphone
stream 4 aac 48k stereo                default=0 handler_name=Voice Chat
```
- A full decode of all 5 streams succeeds.
- Stems were extracted with `-map 0:a:N -c:a pcm_s24le` (video untouched). The Game stem has **correlation +1.000** with the original capture, and after AAC encoding still has 108 dB of separation from the chat tone.
- Preview Mix contains both tones (correlation 0.70 against each source), so it sounds normal in a plain player.
- FFmpeg's MP4 muxer keeps names in `handler_name`. A per-stream `title` is not preserved, so `handler_name` plus a fixed stream order is the contract.

## 6. libobs comparison (OBS 33.0.1 portable driven over obs-websocket 5.7.6)

Same two-tone setup, using OBS "Application Audio Capture" sources routed to tracks 1+2 and
1+4, display capture, NVENC, and a 20 s replay buffer saved to MP4.

| Check | Result |
|---|---|
| Saved replay | ✅ 11.9 s, 3840×2160 **constant 60 fps**, 4 AAC tracks |
| Track isolation | ✅ track 2 is 440 Hz only (≈81 dB), track 4 is 880 Hz only (≈80 dB), track 1 is the mix |
| Track names | ❌ all streams written as `SoundHandler`: OBS track names are lost in plain MP4 |
| Defaults | ⚠ a fresh profile auto-creates global **Desktop Audio and Mic/Aux** sources that `RemoveInput` cannot delete (double-capture and privacy hazard) |
| Config | ⚠ OBS 31+ moved profile selection from `global.ini` to `user.ini`; the first run silently used a default profile |
| Resources | OBS reports 0.66 % CPU, **668 MB RAM** (includes Qt UI and preview). The process working set was 674 MB. |
| Footprint | 553 MB extracted (bin 143 MB, plugins 397 MB) versus about 123 MB for the five FFmpeg DLLs that CAPTURE needs |
| Game capture | libobs includes `graphics-hook64.dll` (DLL injection) for exclusive-fullscreen games. CAPTURE's spec forbids injection by default. |

## 7. Not yet verified (carried into later phases)

- A Steam game: none is installed. The user needs to install one for the Phase 1 playtest.
- A live microphone signal, mic hot-plug, and the BT profile switch.
- Long-run A/V sync, CPU/GPU/FPS overhead under a real game, and memory growth.
- AMD AMF and Intel QSV paths: no such hardware here. They are code paths with honest probing only.
- Exclusive-fullscreen games and anti-cheat-protected games under WGC.
- Import of the multitrack clip into a real editor. FrameCut today would import Preview Mix only.

## Re-run

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-deps.ps1 -WithObs
# audio:  spikes\audio_spike  (cargo build --release; audio_spike list | record ...)
# libav:  set FFMPEG_DIR + LIBCLANG_PATH as printed by fetch-deps, cargo build --release in spikes\libav_spike
# checks: python -I spikes\tools\tone_check.py 440,880 <wav files>
# libobs: spikes\tools\obs_spike.mjs (needs the portable profile described in section 6)
```
