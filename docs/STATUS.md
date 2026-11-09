# CAPTURE status: requirement-by-requirement (v0.3.1)

**Date:** 2026-11-08 · **Branch:** `stabilize/post-redesign` (worktree `D:\Projects\CAPTURE-stabilize`, based on
`53ebc58` = tag `v0.3.0-baseline`) · **Build:** `D:\Projects\CAPTURE-stabilize\dist\CAPTURE-0.3.1\`. The 0.3.0
build in `D:\Projects\CAPTURE\dist\CAPTURE-0.3.0\` is untouched. Not yet merged with the UX/UI redesign, which
was still in progress on `main` during this pass (see "Merging" below).

## 0.3.1 stabilization pass: what changed and how it was verified
| Area | Change | Evidence |
|---|---|---|
| Microphone | Root cause of the G06-BT "only silence": the device is **unmuted at 25% in Windows but delivers pure digital zeros** (headset off/unpaired or its own mute). CAPTURE now says so after 5 s, and names a Windows mute / 0% level exactly (read-only; never changes Windows). Shown on the track, the level monitor and the 3 s test. | unit test; real device: `e2e-isolation` + UI smoke screenshot show the message for the G06-BT |
| Source labels | "Other Desktop" → **All Desktop Audio**: Windows process loopback can exclude only one process tree, so this track always contains the game and voice chat. It is still never in Preview Mix. UI: "All computer audio (extra track)". | `e2e-isolation` (440 Hz game / 880 Hz voice): Game Audio = 440 only, Voice Chat = 880 only (≈28 dB apart), All Desktop Audio = both |
| No doubling | Preview Mix holds each source once with All Desktop Audio and voice chat on. | `e2e-isolation`: mix vs track level 0.0 dB difference (doubling would be +6 dB) |
| Editor WAVs | New **Create separate audio files for editors** (Settings › Storage, off by default). Writes the per-track WAVs after every save; existing per-clip button kept. Stem export never deletes a folder it did not create. | `e2e-isolation`: 4 WAVs written automatically, levels match the MP4 tracks within 0.1 dB, length = clip length |
| Settings safety | New version folder copies settings from the newest `CAPTURE-*` beside it (copy only). Last-known-good `settings.json.bak`. One unreadable value no longer resets everything; non-JSON falls back to the backup; the damaged file is kept as `settings.json.bad-<time>`. | 4 unit tests; `e2e-lifecycle` (update import, restart persistence, damaged file) |
| Tray / close / minimize | Close keeps recording in the tray by default (new installs; a saved choice is respected). Optional *Minimize to the tray*. Tray **Quit** ends the UI and the engine. | `e2e-lifecycle` |
| Start with Windows | Toggle shows the real Run entry; an entry pointing to a moved/deleted CAPTURE.exe is repointed on launch. A sign-in (`--tray`) launch while CAPTURE already runs stays silent. | `e2e-lifecycle`; HKCU Run restored afterwards |
| Single instance | Repeated launches never start a second engine. | `e2e-lifecycle` |
| Engine failures | Killed engine restarts by itself; a hard-killed UI does not leave an orphan engine. | `e2e-lifecycle` |
| Drive unplugged | **Defect fixed:** after the buffer drive came back, 0.3.0 stayed in "needs attention" forever, even though buffering had resumed. It now recovers by itself, and the message names the folder and says the gap is not recorded. | `e2e-unplug` (via `subst`): 0.3.0 fails, 0.3.1 3/3 |
| Markers after a crash | **Defect fixed:** recovery deleted the session's `markers.json`. A recovered clip now gets its markers sidecar. | `e2e-crash` 4/4: marker 7.1 s before the end vs 7.2 s expected |
| FrameCut | Checked the installed FrameCut's importer (read-only): it uses the first audio stream (Preview Mix) and imports audio-only files. Today's workaround and the exact FrameCut change are in `docs/FRAMECUT_IMPORT_CONTRACT.md`. FrameCut was not modified. | code reading of `desktop/tools.cjs` |

### Test results of this pass (all on the 0.3.1 build unless noted)
- Unit tests: **43 pass** (`cargo test --workspace`: 32 engine, 8 app/settings, 3 protocol).
- Worker suites: `e2e-worker`, `e2e-markers`, `e2e-review` 3/3, `e2e-session` 4/4, `e2e-crash` 4/4,
  `e2e-lowdisk` 6/6, `e2e-stems`, `e2e-audio` (all pass, packaged worker), new `e2e-isolation` 12/12,
  new `e2e-unplug` 3/3.
- App: new `e2e-lifecycle` **19/19** and `ui-stabilize-smoke` 6/6. These ran on a code-identical copy built
  with a different app identifier, because the UX agent's own CAPTURE instance was running and CAPTURE's
  single-instance guard would otherwise have handed every launch to it.
- **Not run in this pass:** `ui-auto-test` and `ui-overlay-test`. They start and stop FAULTHEART and use the
  global hotkeys, which would have disrupted the UX agent's runs. Run `scripts\run-acceptance.ps1` after the merge.

### Performance (measured; `scripts/perf-sample.mjs`, 1080p60 test pattern, 2 min, NVENC)
- Worker CPU **0.27 %** of the machine on average (max 0.41 %); memory flat (working set 107 → 108 MB).
- Output: 60.88 s clip with exactly 3653 frames = 60.00 fps. Windows delivered about 48 fps from the ffplay
  pattern; that rate is set by ffplay's own presentation and could not be separated without a frame-time tool.
- NVENC 10–24 % machine-wide, but the UX agent's capture of FAULTHEART was running at the same time, so this is
  **not** CAPTURE's share.
- **Still not measured:** game frame-time impact (needs PresentMon or an ETW trace, neither available here
  without a download or elevation), and overlay/meter latency.

### Not verified (needs you or real hardware)
- **Speech from a real microphone.** The G06-BT delivered only zeros on every test. Mic check: switch the
  headset on, Capture → Your microphone → **Test**, speak; expect playback and "Peaked at … dB".
- Sleep/wake, real sign-in startup and Explorer restart were not exercised (no reboots or session disruption).
  The tray library re-adds its icon on Explorer's `TaskbarCreated` message (tray-icon 0.25.1 source); audio
  sources retry every 2 s and follow the device. Unverified.
- Exclusive fullscreen, anti-cheat games, AMD/Intel encoders (unchanged; see below).

### Remaining issues, by impact on everyday use
1. **Live microphone speech unverified.** Needs you to switch the G06-BT headset on and run the mic check.
2. **Not yet merged with the redesign**, and the real-app acceptance suites have not run on the merged build.
3. **Editors that play every track** (e.g. Resolve, Premiere) play Preview Mix *and* the separate tracks at
   once unless you mute Preview Mix. This is documented in QUICKSTART; CAPTURE cannot control the editor.
4. **All Desktop Audio overlaps** game and voice chat by nature (Windows limit). It is off by default and labelled.
5. **Sleep/wake and Explorer restart** were not exercised (see above).
6. Game frame-time impact not measured (needs PresentMon; ask before it is downloaded).

### Deferred (intentionally not in this pass)
FrameCut multitrack import (contract written) · AMD/Intel hardware tests (AMF code untouched; Quick Sync
not implemented) · frame-time benchmarking harness · exclusive-fullscreen/anti-cheat testing · separate
Sessions/Screenshots folders · overlay opacity, controller shortcuts · installer and code signing.

### Merging with the UX/UI redesign
This branch touches UI files only minimally: two Settings rows (`SettingsTab.svelte`, same Toggle/field
pattern), label and device text in `AudioSources.svelte`, and two fields in `types.ts`. The engine files the
redesign also changed (`audio.rs`, `lib.rs`, `mic.rs`, `capture-protocol`) were edited in different places, but
expect small textual conflicts. Merge after the redesign is committed, then run all suites and `run-acceptance.ps1`.

## Earlier (0.3.0) status, with 0.3.1 corrections inline
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
| Steam game tested | ✅ | **The Witcher 3** (Steam, DX12): auto-detected from the game library folder, 3840×2160 at a true 60 fps in, Game Audio with signal, 20.8 s clip saved (`scripts/e2e-auto-game.mjs`). It ran in its default display mode (not changed by me); a game set to true exclusive fullscreen has not been tested. |
| WGC capture, no injection; client area only for windowed games | ✅ | FAULTHEART 1600×900 without title bar |
| Constant frame rate output | ✅ | 648 frames in 10.8 s at 60 fps |
| Encoder probing NVENC → AMF → Media Foundation (hardware → software) with reasons; Software option | 🟡 | NVENC ✅ in use. Software (Media Foundation) ✅: a real 4K clip, capped at 30 fps above 1080p60 because a 4K60 CPU encode measured only ~48 fps. **AMF and Quick Sync could not be tested: there is no AMD or Intel GPU here.** QSV is not implemented. |
| Exclusive fullscreen / anti-cheat | 🟡 | No hooks are used. If no frames arrive within 5 s, the UI suggests borderless mode or Whole display. Not tested with a real exclusive-fullscreen or anti-cheat game. |

## Audio
| Requirement | Status | Evidence |
|---|---|---|
| Separate tracks: Preview Mix (default) + Game + Microphone + Voice Chat (+ All Desktop Audio, formerly "Other Desktop") | ✅ | `e2e-audio`, `e2e-isolation`: 5 tracks in contract order, content verified with distinct tones |
| Per-process isolation (game vs voice app) | ✅ | ≈127 dB raw isolation; game↔voice correlation 0.003 |
| Microphone off by default; friendly names + IDs; virtual devices labelled | ✅ | settings unit test; device list shows "Steam Streaming Microphone — virtual device" |
| No silent fallback when the chosen mic is missing | ✅ | `e2e-audio`: "Your chosen microphone is not installed any more" |
| Mute (live, records silence), meter, 3 s test with playback, auto-reconnect | 🟡 | Live mute ✅ (no restart). The meter and test paths run, but **your G06-BT mic only ever delivered digital silence** (unmuted in Windows; 0.3.1 now reports this). Live speech capture is **not verified**. |
| Voice-chat app follows restarts | ✅ | process-exit watch; `e2e-audio` restart check |
| All Desktop Audio never mixed (no doubling); voice not mixed twice in Whole-display mode. It overlaps game + voice by nature and is labelled so | ✅ | `e2e-isolation`: Preview Mix level = source level (0.0 dB) |
| Marker chime never recorded | ✅ | −27 dB on the device, −240 dB (absent) in the clip tracks |
| A/V sync, no drift | ✅ | flash+beep pattern: 60 s window capture, audio ≈7 ms after video relative to the source's own baseline. 25 min soak: +33 ms at 5 min and +30 ms at 25 min (raw, including the looping player's own latency): **no drift**. |
| Editor audio files (48 kHz/24-bit WAV per track, aligned to clip time 0) | ✅ | `e2e-stems`: stems identical in timing to the MP4 tracks |
| Import into a real editor | 🟡 | FrameCut was not modified. Its importer (checked in the installed app) uses Preview Mix and accepts the WAVs as audio clips. No other editor (Premiere/Resolve) is installed here. Streams verified with ffprobe: H.264 CFR + AAC tracks, start 0, `handler_name` labels, only Preview Mix default. **Opening a clip in your editor is a playtest item.** |

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
