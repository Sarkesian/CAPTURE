# CAPTURE status: requirement-by-requirement (v0.3.0)

**Date:** 2026-11-08 · **Branch:** `phase0/spikes` · **Build:** `dist\CAPTURE-0.3.0\` and `dist\CAPTURE-0.3.0-portable.zip`
**Specs:** `docs/spec/CAPTURE_Design.md`, `docs/spec/CAPTURE_Agent_Prompt.md`, `docs/spec/CAPTURE_Overlay_Addendum.md`
**Test machine:** Windows 11 Pro 26200, i5-13600KF, 32 GB, RTX 4090 (driver 617.14), 4K@143 Hz.

Legend: ✅ done and tested on this machine · 🟡 done, but only partly verifiable here · ❌ not done

## Core recording
| Requirement | Status | Evidence |
|---|---|---|
| One-hotkey replay saves a normal MP4 to `Clips\<Game>\` | ✅ | `ui-overlay-test` (real Alt+F10), `e2e-worker` |
| Disk-backed rolling buffer, configurable length (presets + custom), size cap, free-space reserve | ✅ | ring unit tests; `e2e-lowdisk` (below) |
| Young buffer saves what exists and says so; repeated + overlapping saves; unique names | ✅ | `e2e-worker`: 2.79 s of 10 s; two overlapping saves |
| Clip starts at the keyframe ≤ requested time (1 s GOP) | ✅ | `e2e-review`: 3–8 s range → 5.02 s |
| "Saved" only after the file is verified; atomic rename | ✅ | `save.rs` verify + `.partial` |
| Crash / forced-kill recovery, never erasing without consent | ✅ | `e2e-crash`: 12.0 s recovered after TerminateProcess; recovery banner in UI |
| Auto game detection without Steam (library folders, user list, fullscreen) + manual app / display | ✅ | `ui-auto-test` on FAULTHEART (itch.io) |
| Steam game tested | ❌ | No Steam game was installed during testing (The Witcher 3 was still installing). The detection rule (`\steamapps\common\`) is unit-tested only. |
| WGC capture, no injection; client area only for windowed games | ✅ | FAULTHEART 1600×900 without title bar |
| Constant frame rate output | ✅ | 648 frames in 10.8 s at 60 fps |
| Encoder probing NVENC → AMF → Media Foundation (hardware → software) with reasons; Software option | 🟡 | NVENC ✅ in use. Software (Media Foundation) ✅: a real 4K clip, capped at 30 fps above 1080p60 because a 4K60 CPU encode measured only ~48 fps. **AMF and Quick Sync could not be tested: there is no AMD or Intel GPU here.** QSV is not implemented. |
| Exclusive fullscreen / anti-cheat | 🟡 | No hooks are used. If no frames arrive within 5 s, the UI suggests borderless mode or Whole display. Not tested with a real exclusive-fullscreen or anti-cheat game. |

## Audio
| Requirement | Status | Evidence |
|---|---|---|
| Separate tracks: Preview Mix (default) + Game + Microphone + Voice Chat (+ Other Desktop) | ✅ | `e2e-audio`: 5 tracks in contract order, content verified |
| Per-process isolation (game vs voice app) | ✅ | ≈127 dB raw isolation; game↔voice correlation 0.003 |
| Microphone off by default; friendly names + IDs; virtual devices labelled | ✅ | settings unit test; device list shows "Steam Streaming Microphone — virtual device" |
| No silent fallback when the chosen mic is missing | ✅ | `e2e-audio`: "Your chosen microphone is not installed any more" |
| Mute (live, records silence), meter, 3 s test with playback, auto-reconnect | 🟡 | Live mute ✅ (no restart). The meter and test paths run, but **your G06-BT mic only ever delivered digital silence**, so a live voice level has not been verified. |
| Voice-chat app follows restarts | ✅ | process-exit watch; `e2e-audio` restart check |
| Other Desktop never mixed (no doubling); voice not mixed twice in Whole-display mode | ✅ | `audio_sources` + unit test |
| Marker chime never recorded | ✅ | −27 dB on the device, −240 dB (absent) in the clip tracks |
| A/V sync, no drift | ✅ | flash+beep pattern: 60 s window capture, audio ≈7 ms after video relative to the source's own baseline. 25 min soak: +33 ms at 5 min and +30 ms at 25 min (raw, including the looping player's own latency): **no drift**. |
| Editor audio files (48 kHz/24-bit WAV per track, aligned to clip time 0) | ✅ | `e2e-stems`: stems identical in timing to the MP4 tracks |
| Import into a real editor | 🟡 | FrameCut was not modified (not authorised). Its current importer would use Preview Mix (no doubled sound). Contract: `docs/FRAMECUT_IMPORT_CONTRACT.md`. **Opening a clip in your editor is a playtest item.** |

## Overlay & replay addendum
| Requirement | Status | Evidence |
|---|---|---|
| Hotkey overlay with the four actions, status, audio rows, recent clips, quick/full settings | ✅ | `ui-overlay-test` 18/18 |
| Review timeline: play, scrub, ±5 s, start/live, marker jumps, two handles, save clip, save around marker | ✅ | `ui-overlay-test`, `e2e-review` |
| Markers: hotkey, chime (volume, on/off), rename/remove, protection with honest states, sidecar | ✅ | `e2e-markers` |
| Session recording (start/stop, any length, auto-finalize) | ✅ | `e2e-session`, `e2e-lowdisk` |
| Screenshots hotkey → `Screenshots\<Game>\` | ✅ | PNG 1600×900 check |
| Hotkeys: save, marker, overlay, session, screenshot, mic mute; conflicts and duplicates; numpad and symbols | ✅ | unit tests; real-key runs |
| Settings in one store, synced between overlay and main window | ✅ | |
| Notices (each type can be switched off), overlay size | ✅ | |
| Library: Clips/Sessions/Screenshots, thumbnails, rename, delete to Recycle Bin, editor audio files | ✅ | screenshots `test-output\shots-ui` |
| Per-game preferences | ✅ | protocol unit test + UI |
| Choosing the chime's playback device; separate paths for buffer/sessions/screenshots; migrating old files when the folder changes | ❌ | Chime plays on the default output. One capture root with fixed subfolders. Old files are never moved. |
| Overlay opacity, controller shortcuts, favourites, hover thumbnails | ❌ | Optional items, not built |

## Storage, privacy, packaging
| Requirement | Status | Evidence |
|---|---|---|
| Nothing bulky on C: | ✅ | Settings + WebView2 profile in `<exe folder>\data`. Search of AppData/Temp/registry found nothing. The startup entry exists only when you enable it. |
| Destination validated before use; files never moved | ✅ | `validate_destination` |
| Saved clips never auto-deleted; low disk pauses buffering | ✅ | `e2e-lowdisk` 6/6: buffering pauses with a plain message, the running session is finalized, earlier clips are untouched, and buffering resumes when space is back |
| Portable build + zip; unsigned | ✅ | Windows SmartScreen will warn (no code-signing certificate). There is no installer. |
| Third-party licences | ✅ | `docs/THIRD_PARTY.md` (LGPL FFmpeg, unmodified DLLs) |

## Performance and reliability (measured)
- Buffering FAULTHEART 1604×946@60: worker 0.19 % CPU, about 100 MB RAM, NVENC 7 %. Game CPU unchanged.
- **25-minute soak** (`scripts/soak.mjs`, window capture, 2 min replay, a save every 5 min): worker memory
  79 → 85 MB (peak 88, no unbounded growth), 0.31 % CPU average, buffer held at its cap, 6/6 saves at
  120.2–120.8 s, 0 failures, buffer emptied on exit.
- Forced kill → recovery: 12.0 s recovered, full decode.
- Acceptance suites on the final 0.3.0 package: `ui-auto-test` 10/10, `ui-overlay-test` 18/18.
- Unit tests: 39 pass.
- **Not measured:** game FPS impact (needs a frame-time tool such as PresentMon and a demanding game), and
  overhead with the overlay open on a heavy game.
