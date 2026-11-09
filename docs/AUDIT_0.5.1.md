# CAPTURE 0.5.1: release-candidate audit

**Date:** 2026-11-08 · **Branch:** `audit/rc-hardening` (from `a14a307`, tagged `v0.5.0-baseline`)
**Machine:** Windows 11 Pro 26200, i5-13600KF, 32 GB, RTX 4090 (driver 617.14), 4K@143 Hz. One PC only.
**Status:** **Release candidate, not a verified release.** Unsigned (SmartScreen warns), not tested on a
clean PC, AMD/Intel GPUs, exclusive fullscreen or anti-cheat games. See "Not verified".

## A. Summary
Three independent code reviews (engine, audio, app/UI) plus my own review of every file operation found
**2 release blockers, 10 high-priority and about 20 medium defects**. All P0/P1 defects and the valuable P2s
are fixed in 16 commits, each with a unit or end-to-end regression test. Six real-app tests were run
against the **unchanged 0.5.0 build first**, to prove each one actually catches its bug. The
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

## C. Tests
See the "Test results" section (filled from the runs below). Skipped and unverified items are listed
separately and never counted as passes.

## D. Performance
See "Performance". No comparison is claimed where the workload differs from the 0.4.1 measurements.

## E. Release artifacts
See "Artifacts".

## F. Known limitations / not verified
See "Not verified".

## G. Deferred work
See "Deferred".

## H. Safety confirmation
See "Safety".
