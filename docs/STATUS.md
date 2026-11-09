# CAPTURE status: requirement-by-requirement (v0.4.0)

**Date:** 2026-11-08 · **Branch:** `main` · **Build:** `dist\CAPTURE-0.4.0\` and `dist\CAPTURE-0.4.0-portable.zip`
**Specs:** `docs/spec/CAPTURE_Design.md`, `docs/spec/CAPTURE_Agent_Prompt.md`, `docs/spec/CAPTURE_Overlay_Addendum.md`, UX/UI overhaul brief (see `docs/UX_AUDIT.md`)
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
| Steam game tested | ✅ | **The Witcher 3** (Steam, DX12): auto-detected from the game library folder, 3840×2160 at a true 60 fps in, Game Audio with signal, 20.8 s clip saved (`scripts/e2e-auto-game.mjs`). It ran in its default display mode (not changed by me); a game set to true exclusive fullscreen has not been tested. |
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
| Choosing the chime's playback device | ✅ | Settings › Markers. Chime on the monitor output: 0 samples on the default speakers, −12 dB on the chosen device. |
| Separate temporary-buffer location | ✅ | Settings › Storage. Segments in the chosen folder, clips in the capture folder, emptied on exit. |
| Separate Sessions/Screenshots paths; migrating old files when the folder changes | ❌ | One capture root with fixed subfolders. Old files are never moved (by design; no migration tool). |
| Overlay opacity, controller shortcuts, favourites, hover thumbnails | ❌ | Optional items, not built |

## Storage, privacy, packaging
| Requirement | Status | Evidence |
|---|---|---|
| Nothing bulky on C: | ✅ | Settings + WebView2 profile in `<exe folder>\data`. Search of AppData/Temp/registry found nothing. The startup entry exists only when you enable it. |
| Destination validated before use; files never moved | ✅ | `validate_destination` |
| Saved clips never auto-deleted; low disk pauses buffering | ✅ | `e2e-lowdisk` 6/6: buffering pauses with a plain message, the running session is finalized, earlier clips are untouched, and buffering resumes when space is back |
| Portable build + zip; unsigned | ✅ | Windows SmartScreen will warn (no code-signing certificate). There is no installer. |
| Third-party licences | ✅ | `docs/THIRD_PARTY.md` (LGPL FFmpeg, unmodified DLLs) |

## UX/UI overhaul (v0.4.0)
Details, decisions and before/after screenshots: `docs/UX_AUDIT.md`, `docs/ux/before/`, `docs/ux/after/`.

| Requirement | Status | Evidence |
|---|---|---|
| Audit first, with before screenshots | ✅ | `docs/ux/before/` (1000×760 and 760×600, plus overlay) |
| Design tokens, dark-first plus light theme | ✅ | `app/src/app.css`; light follows Windows. The overlay stays dark over games. |
| Window frame matches the app; native behaviour kept | ✅ | DWM caption, text and border colours, re-applied on theme change; `docs/ux/after/window-frame.png` |
| Navigation; Home answers the key questions | ✅ | Sidebar: Home, Replay, Library, Settings. Home shows the state, the game, replay readiness, audio and Save replay. |
| Audio meters: 30–60 updates/s, fast attack, smooth release, peak/RMS, hold, clip, off/muted/disconnected states, no fake motion | ✅ | `ui-perf`: 30.0 level updates/s, draws capped at 60/s (48/s on a 144 Hz display). Smoothing: attack 30 ms, release about 270 ms to 95%. Flat, labelled bar when there is no signal. |
| Meters cost nothing when not seen | ✅ | `ui-perf`: minimised gives 0 updates/s, 0 draws and 0.03% UI CPU (2.8% visible). The hidden overlay no longer keeps the level stream running. |
| Fast feel: press feedback, short transitions, reduced motion | ✅ | 80/140 ms colour and opacity transitions; `prefers-reduced-motion` honoured. Page switch paints in 10–22 ms (`ui-perf`). |
| Settings grouped by task, with an index | ✅ | Recording, Audio, Hotkeys, Markers, Overlay & notices, Games, Storage, Startup, Advanced |
| Quality: Balanced / High / Maximum (Custom optional) | 🟡 | Three presets. Custom was not added because a free bitrate field would be untested. |
| Replay and Library restyled; sort/filter, hover actions, empty states | ✅ | Replay page; Library search, game filter, 4 sorts, hover actions and a "…" menu, skeleton and empty states |
| Overlay consistent with the app | ✅ | Same tokens and icons; `ui-overlay-test` passes |
| Recording, tray, settings and overlay not broken | ✅ | `ui-auto-test`, `ui-overlay-test` and `ui-perf` on the 0.4.0 package (below) |
| Screen reader names, keyboard navigation | 🟡 | Labelled controls, arrow keys in the sidebar and segmented controls, visible keyboard focus. No screen reader was used to test. |

## Performance and reliability (measured)
- Buffering FAULTHEART 1604×946@60: worker 0.19 % CPU, about 100 MB RAM, NVENC 7 %. Game CPU unchanged.
- **25-minute soak** (`scripts/soak.mjs`, window capture, 2 min replay, a save every 5 min): worker memory
  79 → 85 MB (peak 88, no unbounded growth), 0.31 % CPU average, buffer held at its cap, 6/6 saves at
  120.2–120.8 s, 0 failures, buffer emptied on exit.
- Forced kill → recovery: 12.0 s recovered, full decode.
- Acceptance suites on the final 0.4.0 package: `ui-auto-test` 10/10, `ui-overlay-test` 18/18, `ui-perf` 7/7.
- Unit tests: 39 pass.
- **Not measured:** game FPS impact (needs a frame-time tool such as PresentMon and a demanding game), and
  overhead with the overlay open on a heavy game.
