# CAPTURE 0.5.1: release-candidate audit

**Date:** 2026-11-08 · **Branch:** `audit/rc-hardening` (from `a14a307`, tagged `v0.5.0-baseline`)
**Machine:** Windows 11 Pro 26200, i5-13600KF, 32 GB, RTX 4090 (driver 617.14), 4K@143 Hz. One PC only.
**Status:** **Release candidate, not a verified release.** Unsigned (SmartScreen warns), not tested on a
clean PC, AMD/Intel GPUs, exclusive fullscreen or anti-cheat games. See "Not verified".

## A. Summary
Three independent code reviews (engine, audio, app/UI) plus my own review of every file operation found
**2 release blockers, 10 high-priority and about 20 medium defects**. All P0/P1 defects and the valuable P2s
are fixed in 16 commits, each with a unit or end-to-end regression test. New real-app tests were also run
against the **unchanged 0.5.0 build** to prove they catch their bug (installer deletion, resize
crop, closed-window save, missing audio at clip start). The
packaged 0.5.1 portable zip and installer pass their distribution and lifecycle suites. Recordings were
checked independently with ffprobe/ffmpeg, not with CAPTURE's own status.

Biggest remaining risks: no clean-machine or second-hardware test; real microphone speech still
unverified; no game frame-time measurement on a demanding game.

## B. Defects found and fixed
Severity: P0 release blocker · P1 high · P2 medium · P3 low. "Shown on 0.5.0" means the new test fails
on the old build.

| # | Sev | Area | Defect (root cause) | User impact | Fix | Evidence |
|---|---|---|---|---|---|---|
| 1 | **P0** | Installer | Installing into a non-empty folder ran `RMDir /r` on its `Runtime`, `Resources`, `licenses`; the uninstaller did the same on `Data`, whatever they contained | Another program's files deleted | Folder page **and** install section (silent installs) refuse such a folder without `CAPTURE.exe` (exit 2, nothing written) | Shown on 0.5.0: silent install deleted `Runtime\theirs.txt`, `Resources\theirs.txt`; uninstall deleted `Data\theirs.txt`. 0.5.1: refused, files intact (`e2e-distribution`) |
| 2 | **P0** | Storage | "Clear temporary footage", crash recovery and the old-layout migration used `remove_dir_all` on every sub-folder of `<chosen folder>\Buffer` and `<root>\Data\thumbs`, and moved anything under `<root>\Clips\*` | A capture/buffer folder chosen inside a shared folder could lose unrelated files | Only CAPTURE-named session folders and CAPTURE's own file names; never recursive | unit tests `session_cleanup_only_touches_capture_files`, `migrate_moves_old_layout…` (user files kept) |
| 3 | P1 | Sessions | Every session used the same buffer pin id; finishing the previous session's save released a session started meanwhile | A 20-min session saved as ~36 s | One pin id per session | unit `each_session_pin_is_released_on_its_own` |
| 4 | P1 | Recording | When the window closed the video thread stopped but audio kept the mux alive: last segment never closed, saves waited 8 s, lost the unflushed tail, could cut audio-only, buffer grew | Late/short clips after a game closes | Session stops as a whole; saves end where video ended; buffer kept saveable until a new game | `e2e-target-closed`: 0.5.0 save took 8.1 s and 8.00 s of footage; 0.5.1: 0.1 s, 8.82 s, second save works |
| 5 | P1 | Recording | Capture texture/encoder sized once; after a resize or resolution change only the top-left was copied (or padded) | Cropped recordings for the rest of the session | GPU scaling (D3D11 video processor, aspect kept) when sizes differ | `e2e-resize`: shown on 0.5.0 (corner missing); 0.5.1 passes |
| 6 | P1 | Auto mode | One 1-second tick on any qualifying window (launcher in a library folder, borderless app) replaced the pipeline and deleted the buffer | End-of-match replay gone when the launcher returns | Switch only after the new window stays 4 s (20 s after the game closed); Riot/Rockstar launchers, crash reporters excluded | unit `launchers_in_library_folders_are_not_games` |
| 7 | P1 | Startup | After a side-by-side update the Run entry kept starting the old version; single instance then handed the new copy's launches to it | User silently stuck on the old version | First launch of a copy that imported settings takes over that copy's entry | `e2e-lifecycle` new check (20/20) |
| 8 | P1 | Settings | UI sent full snapshots; the mute hotkey, tray and the other window could be reverted; two writers shared `settings.json.tmp` | Mic believed muted but recording; folder change lost | Windows send only changed fields, merged and saved under one lock | `e2e-lifecycle` settings checks; unit tests |
| 9 | P1 | Review | Review snapshots in a separate temporary-footage folder were outside the asset scope | Review never plays | Scope covers the buffer folder | code |
| 10 | P1 | Audio | Desktop-audio exclusion covered only the worker; clip playback in CAPTURE's own window was recorded | Old audio duplicated into new recordings | Exclude the UI's process tree | code (process-loopback exclude tree root) |
| 11 | P1 | Audio | A failed audio start leaked running capture threads (mic stays open); a timed-out process-loopback activation freed memory still used by Windows | Mic in use with nothing recording; possible crash | Encoders open before threads; activation buffers kept alive on timeout | code |
| 11a | P1 | Recording | Encoders deliver video after audio, so a clip's first 0.1–0.6 s of audio sat in the previous buffer segment (never read) or, at a session's start, was dropped before the first segment existed | Every clip started with 80–130 ms (NVENC) or ~550 ms (software encoder) of silence; editors show an offset | Saves read the preceding segment's audio; early audio is kept for the first segment | Found by `validate-media` (also on 0.5.0: +100–128 ms); 0.5.1 clips start within 18 ms, software clips at +0 ms |
| 11b | P2 | Recovery | The frame being written when the recorder was killed is cut short | Recovered clip ends with a damaged frame (decoder error) | Recovery ends just before the last frame on disk | `e2e-crash` clip now decodes without errors |
| 12 | P2 | Audio | Device timestamps marked wrong (or 0/garbage) were trusted | Track silent forever or GBs of padding | Implausible timestamps follow the clock | unit `implausible_device_timestamps_follow_the_clock` |
| 13 | P2 | Audio | Mic level monitor stored silent packets forever | Memory growth with Settings open | Monitor stores nothing | same unit test |
| 14 | P2 | Audio | Editor WAVs ignored timestamps | After any gap in a clip, WAVs out of sync with video | Silence inserted for gaps | unit `gaps_in_the_clip_are_kept_in_the_wav` |
| 15 | P2 | Audio | "Default microphone" never followed a change of the Windows default | Old mic recorded after plugging in a headset | Reopens on the new default within ~1 s | code (not tested with hardware) |
| 16 | P2 | Video | Truncated frame period (e.g. 166 666 × 100 ns) | A/V drift 14–58 ms per hour | Exact `t0 + n/fps` | unit `frame_times_do_not_drift` |
| 17 | P2 | Video | Software (Media Foundation) clips had no colour tags and BT.601 data; NVENC fine | Shifted colours in editors assuming BT.709 | BT.709 conversion, untagged clips tagged | `e2e-color`: patches within 2/255 on both encoders, tags bt709 |
| 18 | P2 | Engine | In Auto mode, encoder, buffer folder and per-game quality changes were ignored until the next game | Setting appears saved but isn't applied | Restart decision fixed | code |
| 19 | P2 | Engine | Quit/Shutdown killed saves after 30 s | Long session saves cut, `.partial` left | Waits up to 15 min (worker exits as soon as saves finish) | code |
| 20 | P2 | Engine | Frame lock held during a blocking channel send | A slow disk could stall capture and screenshots | Send after releasing the lock | code |
| 21 | P2 | App | Saved settings reported as an error while the engine was down; `set_replay` changed memory before saving | UI shows wrong state | Saved first; send errors ignored (resent on restart) | code |
| 22 | P2 | App | `ExportStems`, `ClipInfo`, rename accepted any path in the root (also the root itself; FFmpeg URL schemes) | Defence in depth | Only `.mp4`/`.png` files inside the capture folder | code |
| 23 | P2 | App | Worker restart racing with Quit could start a new worker | Orphan worker, update blocked | Checked under the child lock | code |
| 24 | P2 | UI | Mic monitor started by hidden windows (overlay, tray start) | "Microphone in use" with no window | Only while visible | code |
| 25 | P2 | UI | Meters stayed flat after an engine restart; stale engine state after a crash | Misleading status | Resubscribe on `hello`; state cleared on engine stop | code |
| 26 | P3 | Misc | Rename could replace a file created meanwhile, case-only rename refused; Library tab race; review keys stole Space/arrows; unreleased review kept segments; same-second session folders; `.partial` left on failed rename; damaged settings re-reported each start | Minor | Fixed | unit tests (rename, settings) |

Checked and found correct (no change): clip `.partial` + verify-before-rename, ring retention under one
lock, the mux's DTS monotonicity, game-process re-check of reused HWNDs, update zip-slip/manifest checks,
`owned.rs` uninstall ownership checks, single-instance mutex, tray/close behaviour.

## C. Tests (final pass on the packaged build, commit `a38d9fb`)
All on `dist\CAPTURE-0.5.1\` (the shipped worker) or the shipped zip/installer, one suite at a time.
NVENC unless noted. A local `llama-server` was using the GPU (~21 GB VRAM) during testing; whole-display
4K captures degraded to 24–50 fps for **both** 0.5.0 and 0.5.1 while it was busy, so that is a load
effect, not a regression.

| Suite | Result |
|---|---|
| Unit tests (`cargo test --workspace`) | **65 passed**, 0 failed (was 58) |
| Worker suites: markers, review, session, crash recovery, low disk (+ software encoder), drive unplug (`subst`), audio isolation (440/880 Hz), editor WAVs, replay saves, window closed, resize, colour | **12/12 passed** |
| `validate-media` on every clip those suites saved (full decode, frame count, A/V start/end) | **18/18 valid** |
| `e2e-lifecycle` (single instance, tray, startup entry, upgrade import + startup move, crash restart, no orphan, damaged settings) | **20/20** |
| `e2e-distribution` (zip = manifest, first run, record, move, update, damaged update refused, delete; installer: refuses a foreign folder, silent install on D: with spaces, shortcuts, Installed apps, record, tray, upgrade while running, uninstall keeping recordings, reinstall, uninstall with all data) | **all passed** (69 checks in the full run; exit 0 in the final run) |
| Soak (see D) | **partial: 11.5 of 20 minutes with a live source** (the test window was closed) |
| `e2e-audio` (needs the game FAULTHEART running) | **not run** |
| `run-acceptance.ps1`, `ui-auto-test`, `ui-overlay-test`, `ui-perf` (real UI clicks, need a game) | **not run** for 0.5.1. The UI changes in this pass are small (B #24–26) and the frontend builds, but they were not exercised by clicking |

## D. Performance (measured, not estimated)
- **Soak**, 640×360@60 window capture, 2-min replay, a save every 5 min, packaged 0.5.1: worker memory
  80 → 85 MB (working set; private 146 → 150 MB), flat from 5.5 min on; CPU 5.3 % of one core (≈0.3 % of the
  20-thread CPU) while buffering, 0.2 % of one core while waiting; buffer held at 120 s / 5 MB; 5/5 saves
  120.1–120.7 s, 0 failures, all valid. After the window closed at 11.5 min, the two later saves still held its
  last 2 minutes (fix #4). 0.4.1's 25-min soak: 79 → 85 MB, 0.31 % CPU: same order, not a like-for-like run.
- **A/V sync** (flash + beep together every second, 120 pairs per clip): **+17 ms** at 5 min, **+15 ms** at
  10 min; first/last quarter of each clip +18/+18 and +13/+17 ms: about one frame, **no drift**. Includes the
  test player's own latency (0.4.1, same method: +30–33 ms).
- **Clip start**: audio within 0–18 ms of the first video frame (0.5.0: +80–130 ms NVENC, +554 ms software).
- **Colour**: decoded patches within 2/255 of the source with NVENC and the software encoder.
- **Not measured**: game frame-time impact on a demanding game (`scripts/frametime.mjs` exists; not run for
  0.5.1); overlay cost on a heavy game.

## E. Release artifacts (built from `a38d9fb`, version 0.5.1)
| Artifact | Path | SHA-256 |
|---|---|---|
| Payload / runnable portable folder | `dist\CAPTURE-0.5.1\` (136 MB, 10 files; `manifest.json` has each file's SHA-256) | |
| Portable zip | `dist\CAPTURE-0.5.1-portable.zip` (58 MB) | `cacb27a0f921e9028e470dd344bfadc9a29ef827d6acf3acf0eecbeb12a60a78` |
| Installer (per user, NSIS) | `dist\CAPTURE-0.5.1-setup.exe` (43 MB) | `19ebce38cbf38cf1919b7efdc5eeceb5727cb9465aa3b86429da03c8ef93a0fb` |
| Checksums | `dist\SHA256SUMS-0.5.1.txt` | |
| Docs | `docs/STATUS.md`, `docs/DISTRIBUTION.md`, `docs/QUICKSTART.md` (shipped), `docs/THIRD_PARTY.md` (FFmpeg LGPL notice shipped in `Resources\Licenses`) | |

## F. Known limitations / not verified
- **Not a verified release.** Unsigned (SmartScreen warning); no clean-PC, offline or WebView2-missing test;
  one PC, one GPU (NVIDIA). AMD (AMF) and Intel paths are unchanged and **untested**.
- **Exclusive fullscreen and anti-cheat games: not tested.** Borderless and windowed are.
- **Real microphone speech unverified** (the headset on this PC delivers digital silence). Check: Settings ›
  Audio › Microphone › Test, speak, expect playback and a peak reading.
- Default-microphone switching (#15), exclusion of CAPTURE's own playback (#10) and the hidden-window mic fix
  (#24) are implemented and compile, but were **not exercised** with real devices or the real UI.
- After a resize the picture is scaled with its aspect ratio kept: a game that changes aspect ratio is
  letterboxed rather than recorded at its new native size.
- A setting change that restarts recording (quality, fps, encoder, folders, microphone) starts a new buffer:
  marker-protected footage of the previous one is not carried over ("kept until saved or session ends").
- All Desktop Audio contains game and voice chat by nature (Windows limit); editors that play every track need
  Preview Mix muted (unchanged, documented in QUICKSTART).
- Sleep/wake, Explorer restart and a real sign-in start were not exercised.
- Under heavy GPU load from another app, recordings can drop frames; CAPTURE counts them but does not warn yet.

## G. Deferred work (by user impact)
1. Run the UI acceptance suites and a real game session on 0.5.1 (needs FAULTHEART or another game). Small.
2. Live microphone speech check with a working headset (needs you). Minutes.
3. Clean-PC / VM install test, offline and without WebView2. Medium.
4. Code signing (removes the SmartScreen warning). Needs a certificate.
5. AMD and Intel hardware; exclusive-fullscreen and anti-cheat titles. Needs hardware and games.
6. Game frame-time impact on a demanding game with `scripts/frametime.mjs`. Small.
7. A notice when frames are being dropped under GPU load. Small.
8. Remaining P3s: mute applied by sample position (up to ~200 ms early/late), 20 ms clock corrections in
   isolated tracks, WAV stems clip above 0 dBFS (24-bit integer), Preview Mix meter shows the loudest source
   rather than the sum, `Num0`–`Num9` bindable without a modifier (kept: your own hotkeys use them).

## H. Safety confirmation
- **Your recordings:** untouched. No test wrote to `G:\Videos\Captures`; every test used disposable folders
  under `D:\Projects\CAPTURE\test-output\`.
- **Your settings:** `dist\CAPTURE-0.4.0\data\settings.json` was only read (the tests build a synthetic 0.4
  folder); a backup copy was taken first.
- **Stable build recoverable:** tag `v0.5.0-baseline` (`a14a307`); the 0.3.0–0.5.0 folders in `dist\` are unchanged.
- **Other locations:** tests touched only CAPTURE's own per-user entries (`Run\CAPTURE`, `Uninstall\CAPTURE`,
  `CAPTURE.lnk`) and restored them. **Exceptions:** one early test attached to and force-closed an already open
  Notepad (Windows 11 Notepad hosts all windows in one process); tests now only use windows they create, and that
  clip was deleted. The old 0.5.0 uninstaller window you saw came from reproducing the installer bug in a test folder.
- **Your Start-with-Windows toggle:** your 0.4.0 settings say it is on, but no CAPTURE Run entry existed when this
  audit started (likely removed by an earlier test run). CAPTURE shows the real state; turn it on again if wanted.
- **UI design** unchanged (behaviour fixes only). **No new features or services**: the GPU scaler and lead-in audio
  are internal fixes; no network access, telemetry or background service was added.
