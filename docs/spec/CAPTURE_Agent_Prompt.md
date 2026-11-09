# AGENT BUILD PROMPT | Build CAPTURE from zero context

You are the principal engineer, product designer and test owner for a new **Windows-first, local-first game recording and instant replay desktop app**, working title **CAPTURE**.

**Read `CAPTURE_DESIGN_SPEC.md` completely before doing anything else.** Treat it as the product/engineering specification and acceptance contract. The user wants Steam's effortless background game recording, AMD Adrenalin's clean automatic organization and replay behavior, and selected OBS-grade capabilities, in a much smaller, friendlier interface that does not depend on Steam or a particular GPU vendor.

## Mission: what must actually work
Build a real, usable, independently installable app, **not** a mock interface or prototype that never saves a genuine clip.

The core user journey is:
1. Open app; see clear `Capture`, `Library`, `Settings` sections.
2. Choose game/window/display recording through clean, low-friction toggles; ideally games are detected automatically.
3. Select a microphone from real device names with a level meter and test; select game/system audio and optionally a separate voice-chat app just as easily.
4. Enable rolling replay, set `Save last X minutes`, and use a configurable global hotkey.
5. Press hotkey during a game; see a genuine MP4 automatically saved under `G:\Videos\Captures\Clips\[Game Name]\`, immediately accessible to any editor without Steam or any mandatory conversion/export step.
6. Open that clip in an editor and adjust **game audio, microphone, and external voice chat independently**, because they were recorded as separate synchronized streams, not mixed at capture time. Video must be its own editable stream.

A combined preview audio track should make the MP4 sound normal in ordinary players, but do **not** cause duplicated audio in editors. For editor compatibility include an optional `Editor audio files` command that extracts synchronized 48 kHz WAV stems without re-encoding the video. Ensure `Preview Mix` is clearly labeled and automatically muted or excluded when an integrated editor expands isolated tracks.

**Critical boundary:** capture app records separate sources; editor imports/displays them. It cannot retroactively isolate in-game voice chat already mixed by the game. Don't promise universal separation.

## Engineering process and authority
- First inspect the workspace, repo, current application dependencies, filesystem conventions, Windows version, available capture hardware, encoder availability and any existing FrameCut editor import architecture. Do not assume the app or repository already exists.
- Choose a sane project location aligned with the user's shallow, self-contained storage organization, preferably on the D: data drive if available. **Do not relocate or delete unrelated projects.** Ask only if an essential destructive decision or unavailable requirement truly blocks progress.
- Produce a short architecture decision record comparing two small working spikes: **native Windows Graphics Capture + Windows audio loopback + proven encoding/muxing libraries**, versus leveraging **libobs**. Evaluate complexity, capture coverage, per-process audio isolation, performance, packaging and GPL/license duties. Choose based on evidence, not fashion. Document why.
- Recommended UI/backend arrangement: **Tauri 2 + Rust + Svelte or similarly lightweight frontend**, with capture/encoding isolated from the UI. You may choose another implementation if measurements and reliability materially favor it. Keep IPC typed and versioned.
- Prefer mature capture/muxer code and actual GPU hardware encoders (NVIDIA NVENC, AMD AMF, Intel Quick Sync), with capability probing and documented fallback. Avoid a home-built codec or game-hook stack, and never evade anti-cheat/DRM protections.
- Keep default UI to **three tabs**; use large clear status, one main replay toggle, simple duration/quality selectors, and unobtrusive audio source cards. Advanced functionality goes behind disclosure, not on the home screen.
- Use clean Rust/TypeScript patterns, narrowly scoped modules, deterministic tests and recoverable errors. No fake status indicators, dummy waveform data, unimplemented buttons or brittle global state.
- Work autonomously through non-destructive implementation tasks, creating small commits and checkpoints. Preserve any existing application features and known-good behavior. Use reversible migrations and small merges; do not rewrite unrelated modules.

## Non-negotiable functional details
**Audio:** Microphone is OFF until explicitly enabled. Device selection by friendly name plus stable ID; test meter and clear disconnected state. Separate source selection for game process, system output fallback, external voice-chat process, optional other desktop audio. Use Windows-supported process loopback where available and distinguish it from whole-device loopback. Avoid double capture, feedback and wrong-device fallback. Maintain A/V sync under long recordings and source changes. Capture separate streams even when the main preview mix is present.

**Files:** One standard playable H.264/AAC MP4 for saved clips by default, with video + labeled audio tracks. Use safe filenames and shallow paths under user-chosen capture root. Prefer `G:\Videos\Captures` only if the drive exists and user has not chosen another location. Temporary ring footage belongs in `Buffer`; saved clips in `Clips\Game\`; full sessions in `Sessions`. Never auto-delete saved clips. Never trap footage in a private Steam-like export system.

**Replay:** Disk-backed rolling segments with configurable duration, strict temporary cap, free-space reserve, atomic protection of the triggered replay window, asynchronous finalization and verified `Saved` notification. Support hotkey debounce, repeated saves, overlap without deleting in-use segments, shortened replay when the buffer is young, and crash recovery. User must have a one-click route to the actual file.

**Capture and hardware:** Auto detect a game independent of launcher when possible; otherwise easy manual target picker. Fallback to window/display capture when game capture is blocked. Honor protected content and anti-cheat restrictions. Support GPU encoder probing and honest CPU fallback, rather than promise vendor/hardware independence without capability checks. No heavyweight continuously rendering preview when minimized.

**Settings and UX:** Live source status and meters; plain-language actionable errors; working `Open file`, `Open folder`, `Copy path`; saved device/hotkey/profile choices; customizable hotkeys with conflict detection; per-game optional rules; accessible controls; high-DPI support. No unnecessary panels, ads, cloud accounts or analytics. Explicit mic-active and desktop-active indicators. Preserve capture operation in the tray only when configured to do so.

**Media compatibility:** Test output streams using `ffprobe` and actual playback/import. Provide optional separate WAV stems for editors that do not expose MP4 audio streams. The user is building **FrameCut** separately, so document a precise import contract for it, but do not modify FrameCut unless the repository/work scope clearly includes that project and the user has authorized the integration. A proposed future integration is not a completed feature.

**Storage hygiene:** The user prefers a shallow, understandable, self-contained D: layout and minimal C: accumulation. Avoid writing bulk cache/recordings into AppData or the installation folder; only OS-required metadata where unavoidable. Explain what remains on C: and why. Do not move, uninstall or reinstall unrelated applications.

## Phased build, no paper-only 'completion'
**Phase 0: capability audit and recording spikes.** Validate real window/game frame capture, audio process separation, mic input, encoder init, multitrack media file and libobs/license tradeoffs. Record factual results. Select architecture.

**Phase 1: first playable vertical slice.** Working app/process + tray, real replay ring, configurable duration, global save hotkey, **actual verified playable MP4** in user-chosen folder. Test a real game. Commit and expose a build for user playtest.

**Phase 2: polished audio.** Device picker, level meters, tests, capture routing, separate game/mic/voice-app streams, preview mix, WAV editor stems and synchronization tests. Verify stream count/content. Commit and user playtest.

**Phase 3: exceptional ease of use.** Three-tab UI, first-run flow, library, game detection, profiles, storage controls, notifications and useful fixes for errors. Commit and user playtest.

**Phase 4: release hardening.** Memory/CPU/GPU/FPS benchmark against no-capture baseline; long-run tests; low disk and crash recovery; hardware/device changes; safe capture fallbacks; signed/unsigned installer behavior clearly described; portable build if feasible; documentation. Commit and produce reproducible build.

Do not silently skip a gate or call unfinished behavior complete. If a feature is technically blocked, deliver the closest useful working implementation, document the exact limitation, and keep related UI honest.

## Test matrix and required evidence
Prove rather than claim:
- Real clip from a Steam game and a non-Steam game or standalone app.
- Save X-minute replay repeatedly, including a buffer that is younger than X and two overlapping hotkeys.
- Show MP4 is immediately playable and editable without requiring a second app-managed export.
- `ffprobe` reports one video stream and separate labeled `Preview Mix`, `Game Audio`, `Microphone`, `Voice Chat` streams when available; use audio-content checks to prove they are not duplicates.
- Open saved clip in at least one target editor and verify isolated editing or demonstrate WAV-stem fallback; no double-volume default playback.
- Disable mic and prove no mic samples were recorded. Disconnect device and verify no silent switch to a private microphone.
- Long capture audio sync, FPS overhead, memory growth, source switches, blocked game capture, low disk, forced process kill, recovery.
- File retention never deletes permanent clips; disk cap protects against C: growth and full drives.
- Every major UI button performs a real action; hotkeys function reliably without getting 'stuck'.

Measure resource use rather than making unverified performance promises. Collect clear before/after metrics and actual failure observations.

## Final handoff format
At each useful milestone report briefly:
1. **Implemented** vs **not implemented**.
2. Repository path, branch, latest commit and runnable build location.
3. Demonstrated real recordings, exact file paths and media/track probe summaries.
4. Tests run and results, performance overhead and known limitations.
5. What a human should playtest now and how.
6. Next highest-value work.

Provide a compact user-facing quick-start guide. Keep design coherent and simple. The highest priority is an excellent, reliable one-hotkey recording experience with editor-ready independent audio, not a broad checklist of unfinished features.

## Technical research to verify before implementation
- OBS multitrack recording: https://obsproject.com/kb/multiple-audio-track-recording-guide/
- OBS media containers and hybrid MP4: https://obsproject.com/kb/audio-video-formats-guide/ and https://obsproject.com/kb/hybrid-mp4
- Microsoft process audio capture sample: https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/
- Microsoft process loopback minimum OS requirement: https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-audioclient_activation_type
- Microsoft WGC: https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture
- FFmpeg stream mapping: https://ffmpeg.org/ffmpeg.html
- NVIDIA encoding SDK: https://developer.nvidia.com/video-codec-sdk
- OBS capture limitations: https://obsproject.com/kb/game-capture-troubleshooting

**Begin with an environment audit and a real audio/video capture spike. Then build and test sequentially. Do not ask the user for repeated confirmation of normal development actions; do not take destructive actions or move unrelated user data.**
