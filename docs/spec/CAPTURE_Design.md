# CAPTURE | Product Design Document + Technical Specification
**Status:** Implementation-ready design baseline | **Version:** 1.0 | **Date:** 2026-11-08  
**Working name:** Capture (replaceable) | **Target:** Windows 11 first, Windows 10 where APIs are verified

> **North star:** A capture tool you barely need to open. It quietly records what you choose, lets you save the last X seconds/minutes with one hotkey, and places a normal, properly named, editor-ready video directly in its permanent folder. Simple enough for a first-time user, reliable enough for an experienced editor.

## 1. Product definition
Capture is a **local-first, standalone, hardware-adaptive gameplay and desktop recording application**. It combines the effortless background workflow of Steam Game Recording, Adrenalin-style automatic organization and instant replay, and selected OBS-quality media features, without turning recording into an OBS-like configuration project.

### The four promises
1. **One hotkey, real file.** Save the past X minutes to a conventional MP4, playable and editable without launching Capture or manually exporting from a private format.
2. **Distinct original audio.** Preserve gameplay, microphone, and separate voice-chat applications as independent, synchronized audio streams wherever the underlying OS can capture them individually.
3. **No housekeeping.** Rotate temporary buffer footage, respect user-configured disk limits and reserves, sort permanent recordings into human-readable game folders, and never auto-delete saved clips.
4. **No vendor lock-in.** Auto-detect a supported encoder from NVIDIA, AMD, Intel, or a software fallback. Capture windows and displays without requiring Steam or a proprietary GPU recording overlay.

### Non-goals for initial release
Streaming, an in-app full nonlinear editor, cloud synchronization, mandatory user accounts, automatic clip uploads, heavy AI recognition, invasive game hooks as the default, bypassing anti-cheat or DRM, and guaranteeing perfect performance on every game/hardware combination. Future integrations must not delay a reliable core.

### Design values
- **Progressive disclosure:** simple switches and clear source names by default; power-user controls are one layer deeper.
- **Truthful status:** tell the user exactly whether frames and audio are being captured, not merely whether a toggle is on.
- **User owns their files:** ordinary directories, stable filenames, metadata sidecars optional, no app-only export barrier.
- **Graceful fallback:** always indicate what is unsupported and suggest the next safe compatible mode.
- **Performance-minded:** encoder/capture work is asynchronous; no live-video preview in the UI when closed.
- **Privacy by design:** local-only defaults, no silent screen/microphone recording, obvious recording indicators.

## 2. Primary workflows and acceptance stories
**First run (target: minimal interaction).** Select a permanent storage folder (recommended `G:\Videos\Captures` on this workstation); allow the app to detect a game; choose a microphone using human-readable names; test its meter; choose the default audio output / application sound; enable replay; select a replay duration; assign or accept a hotkey. A short self-test records video and the intended audio tracks, then opens a sample clip. All permissions or access limitations are explained contextually.

**Daily use.** Launch Windows or the app; tray icon shows `Ready`, `Buffering`, `Saving`, `Paused`, or `Needs attention`. When a recognized game is active, a configurable background buffer starts. Press `Save replay`; the app preserves past X minutes and writes a permanent MP4. A notification offers `Open file`, `Open folder`, and `Copy path`. No manual export.

**Manual session.** Click `Start recording` or use a hotkey; record as long as desired within limits; stop; the final playable MP4 lands in Sessions. A manual session and replay buffer can coexist only after the implementation proves they do not needlessly duplicate encoding or overrun the encoder.

**Editor use.** Drag the MP4 into a supported editor. Game, microphone, and voice-chat sources are available as distinct audio streams. If the editor cannot expose embedded streams individually, use `Export editor audio files` to produce synchronized stems without regenerating video. FrameCut integration, if implemented, must present audio sources on separate lanes and avoid playing a combined track simultaneously with its components.

**Failure.** A game switches resolutions, the headset disconnects, space runs low, a game blocks a capture method, or power is lost. The app must explain the problem, preserve recoverable footage, and never claim it saved a valid clip unless it exists and is playable.

## 3. Navigation and interaction design
Top-level interface has **three tabs only**: `Capture`, `Library`, and `Settings`. A tray menu supplies essential actions even when the window is closed. Default to a quiet dark UI with generous spacing, clear contrast, crisp typography, no dashboard clutter, no fake complexity. Avoid telemetry graphs on the landing screen.

### 3.1 Capture: the primary screen
Top row: prominent **status indicator** (`Ready`, `Buffering`, `Recording`, `Saving`, `Error`), current game/window, and the location of the current output. Never use an ambiguous green dot without a text label.

Main controls:
- **Instant replay:** large On/Off switch; when enabled, show `Save last [30 sec | 1 min | 2 min | 3 min | 5 min | 10 min | 15 min | 20 min | Custom]`.
- **Recording mode:** segmented chooser `Auto games | Selected window | Desktop`; desktop must be an explicit opt-in.
- **Save clip:** primary button with a visible editable hotkey. `Start/Stop session` is secondary.
- **Audio:** compact panel with `Game sound`, `Microphone`, `Voice chat`, each a toggle, a plain-language device/app selector, an input level meter, and an `Advanced` disclosure.
- **Quality:** `Balanced | High | Maximum | Custom`; contextual estimated file size per minute, displayed as an estimate rather than a guarantee.
- **Destination:** visible save folder with `Change` and `Open` actions.
- **Storage:** one unambiguous summary, e.g. `Temporary buffer: 7.3 of 20 GB · Permanent clips: never auto-deleted`.

The home screen must be fully usable with keyboard and screen readers, use tooltips only for non-essential explanation, and give direct feedback for all interactions. A low-power UI should not continuously decode preview frames in the background.

### 3.2 Audio source picker: exceptionally easy
Each source row must work independently, for example:

| Row | User-facing default | Picker behavior | Test feedback |
|---|---|---|---|
| Game audio | On: `Current game's audio` | Detect game's process; choose a specific running app or system-output fallback | Real-time level meter and `Test capture` |
| Microphone | Off until explicitly enabled | `System default`, then named connected input devices; remember stable device IDs | Input meter, mic test playback, optional mute |
| Voice chat | Off until explicitly enabled | Pick running app such as Discord, or select a different app/process | Voice-activity meter, warning on unsupported separation |
| Other desktop audio | Off | Choose system mix if wanted | Warn if source overlaps with game/chat and could double audio |

A source card should show `Detected`, `Not running`, `Disconnected`, `Unsupported`, or `Muted`, with a short action to fix it. Avoid cryptic WASAPI/endpoint jargon in the main interface. Detect the default mic/output automatically, but never silently turn a mic on. On device unplug or reconnection, maintain the selected source when possible; if not, show a safe fallback choice, never silently switch to a sensitive microphone.

**Privacy and monitoring:** show a persistent mic-active indicator while capturing; local test monitoring should be opt-in and automatically prevent obvious speaker-to-microphone feedback loops. Device IDs are preferences, not absolute identifiers; handle driver reinstalls.

### 3.3 Library
- Toggle `Clips | Sessions | Screenshots`; grouped primarily by game/application.
- Search filenames and game titles; filter by date, duration, favorite, tags, source and media type.
- Efficient video thumbnails and lightweight preview on selection; lazy-load large collections.
- Every item offers `Play`, `Open folder`, `Copy file/path`, `Rename`, `Favorite`, `Trim copy` (later), `Editor audio files`, and `Delete` with confirmation.
- Library index is rebuildable from the filesystem; app must still operate if the index is lost.
- Make clip metadata human-readable and portable: source name, timestamp, duration, codec, resolution, FPS, captured audio stream labels, creation state.

### 3.4 Settings
Four compact groups: `Capture & hotkeys`, `Audio`, `Video quality`, `Storage & privacy`. Put audio routing details, encoder override, bitrate, frame rate, color/HDR, container, buffer algorithm, logging, and debug traces behind `Advanced settings`. Support a `Reset this category` control and explicit `Restore defaults` that does not erase media.

## 4. Recording modes and timeline semantics
1. **Automatic replay:** app detects a permitted foreground game; begins capturing to a rotating ring of safe media segments; on hotkey, freezes the required range as an asynchronous save task. Buffer behavior continues unless resource constraints require a transparent pause.
2. **Manual replay:** user picks a window or display and enables a rolling buffer, whether or not a game was detected.
3. **Full session:** explicit start/stop, resilient ongoing file, optionally add markers.
4. **Screenshot:** optional separate hotkey; save standard PNG or JPG without interfering with recording.

Replay duration `X` is measured relative to the triggering hotkey, not relative to the beginning of the oldest file. The saved clip should cover the requested time within a documented frame/keyframe tolerance; if exact frame precision is offered it must be validated, and may need a bounded edge re-encode. Support configurable **post-roll** later, with clear semantics. Prevent hotkey bounce, duplicate task creation, and 'stuck input' states. Hotkey registration must detect conflicts and offer alternative assignments.

No UI should promise the exact last X seconds if the current buffer contains less than X; the notification must say `Saved 1m 42s available` or an equivalent precise description.

## 5. Audio and media contract: critical design requirement
### 5.1 Where separation happens
**The recorder captures source streams separately.** If it combines all sounds into one waveform first, no editor can reliably reconstruct the original microphone/game/chat sources. **The editor displays and edits the recorded streams independently.** A video track and multiple synchronized audio tracks can reside in a single MP4 container.

Windows audio process-loopback capture can isolate audio rendered by a selected process and its children on supported Windows builds. Endpoint-loopback captures the whole output mix instead. These are different capabilities; they must not be labeled interchangeably. Audio already mixed *inside a game* (for example, teammates using that game's built-in voice chat) generally cannot be unscrambled into independent people by the recording tool.

### 5.2 Default output contract
Primary saved file: `YYYY-MM-DD_HH-MM-SS_Game_Clip.mp4`.

- **Video stream:** one normally decodable H.264/AVC stream initially, chosen native resolution or preset resolution, target 60 FPS when feasible; timestamps synchronized to the capture clock.
- **Audio 1:** `Preview Mix` (game + mic + chat, but only sources enabled at capture time), stereo AAC, set as default for everyday playback.
- **Audio 2:** `Game Audio`, isolated AAC, if available.
- **Audio 3:** `Microphone`, isolated AAC, if enabled.
- **Audio 4:** `Voice Chat`, isolated AAC, if enabled and independently capturable.
- **Optional Audio 5:** `Other Desktop`, only when explicitly enabled and configured to avoid duplicate capture.
- Each track gets stable stream tags/title/role metadata wherever supported, accurate timeline start offsets, and predictable stream ordering.

**Why include Preview Mix?** Many general video players play only one audio stream. It makes the video sound correct when double-clicked. **Trade-off:** importing *all* streams and playing the mix plus isolated ones will double the sound. The app and integrated editors MUST automatically mute or exclude Preview Mix whenever isolated tracks are expanded. For generic editors, provide a clearly labeled `Editor audio files` action, and mention this behavior in help text.

### 5.3 Editor-ready interchange
Offer three *user-facing* choices, not a maze of codecs:
- `Standard clip` (default): one MP4 with a playable preview mix **and** independently preserved audio streams; direct file access.
- `Editor files`: keep the original MP4 and create a sibling `ClipName_Audio/` folder with synchronized `Game.wav`, `Microphone.wav`, `VoiceChat.wav`, plus a tiny manifest and import instructions. 48 kHz PCM WAV is the recommended compatibility target; generate files only on user request or when a profile asks for them. Export is asynchronous, without needless video re-encode.
- `Single mixed track` (opt-in compatibility): standard MP4 with just the playable mix; warn that this removes downstream source independence for that particular exported copy.

When FrameCut supports this contract, `Open in FrameCut` should import the MP4, create separate game/mic/chat tracks, mute the redundant preview mix, preserve timestamp alignment and immediately show editable waveforms. Do not describe this as implemented until the receiving editor truly supports it.

### 5.4 Audio DSP and sync
- First milestone: capture **unprocessed originals** at consistent sample rate/timebase, with gain/mute controls. Optional noise suppression, limiter, ducking, and push-to-talk may be applied in an opt-in nondestructive or separate-processing path later.
- Verify exact source assignment, no accidental loopback double capture, stereo mapping, clipping indicators, 48 kHz output, and no sound recorded from disabled sources.
- Timestamp packets using a common monotonic timeline; detect clock drift; align audio on device swaps/resets; validate long-recording lip sync and replay boundary continuity.
- If a process is silent or disappears, track may be silent/omitted per documented policy; the MP4 should still finalize correctly and warn if a selected required source was absent.

## 6. Capture engine and hardware strategy
**Windows first.** Prototype Windows Graphics Capture (WGC) for windows/displays and native desktop APIs, using `CreateForWindow` where appropriate. Support active-game foreground identification independently of Steam, while respecting Windows capture access and privacy behavior. If game/window capture fails, offer a clear **Window / Display** fallback. Do not bypass protected content or anti-cheat restrictions.

Encoder adapter layer:
1. Probe capabilities (codec, max resolution/framerate, concurrent sessions, HDR, GPU availability, encoder initialization).
2. Prefer supported hardware encoding: NVIDIA NVENC, AMD AMF, Intel Quick Sync; choose a documented software fallback only if acceptable for target workload.
3. Expose `Balanced`, `High`, `Maximum`, with sensible codec/quality settings derived from actual hardware support, not fictitious universal bitrates.
4. Use H.264 + AAC for default compatibility; AV1/HEVC optional if recording and target editor support is confirmed.
5. Handle GPU reset, display changes, fullscreen transitions, games moving monitors, rapid foreground switches, and unsupported modes.

**Critical build-vs-reuse evaluation:** run two time-boxed prototypes: (A) native capture worker + FFmpeg-based encoding/muxing and (B) the relevant libobs components. Compare real stability, per-game coverage, multi-track audio, performance, binary size, maintenance, and licensing. Because libobs/OBS Studio is GPL-based, do not embed it without an explicit license compliance decision. Do not blindly write a proprietary hook/encoder stack from scratch to avoid dependencies.

Recommended application shell: **Tauri 2 with a Rust backend and a compact frontend** (for example, Svelte), talking to an isolated/native capture worker. This is a design preference to benchmark, not an irrevocable mandate. Keep capture alive in the tray when the window is closed, with explicit user control of startup and background behavior.

## 7. Replay buffer, storage safety and crash recovery
### 7.1 Storage scheme
Default **user-selectable**, recommended `G:\Videos\Captures` for this machine, not a growing hidden cache under `C:\Users\...\AppData`. Allow a portable/uninstalled-friendly layout where system limitations permit. The program may retain a small OS-required settings/integration footprint; any unavoidable files on C: must be documented.

```text
G:\Videos\Captures\
  Buffer\                     # disposable/recoverable ring, cap strictly enforced
  Clips\
    Cyberpunk 2077\
      2026-11-08_143215_Cyberpunk-2077_Clip.mp4
  Sessions\
    Blender\
      2026-11-08_163001_Blender_Session.mp4
  Screenshots\
    Game Name\
  Data\                       # optional small rebuildable library index/settings
```

Do **not** silently create excessive nesting or copy clips into the app's private folder. `Buffer` can be omitted from library views. Only temp footage is managed by automatic retention: **never auto-delete clips or full-session recordings**, even when space is low.

### 7.2 Retention policy
- Configurable replay duration and temporary buffer size cap; both are enforced. Default values should be selected after benchmark and available disk-space checks, not asserted to work universally.
- Configurable **minimum free-space reserve**; when reserve is reached, delete oldest disposable segments first, refuse further temp writes when necessary, pause with explanation, and leave saved files untouched.
- Atomic segment index updates, generation IDs, ownership records, and unique clip names; no deleting files still needed by an in-flight save task.
- Protect clip snapshot segments immediately on hotkey, then finalize asynchronously to permanent destination with atomic rename. Notification first says `Saving…`, then `Saved` only after verification.
- Recovery on restart: enumerate incomplete files, verify and salvage possible footage, label partially recovered assets honestly, never erase recoverable footage without user approval.
- Choose a crash-resilient internal container/fragment model, then finalize a standard MP4. Evaluate fragmented/hybrid MP4 and MKV based on editor compatibility, multi-track fidelity and recovery tests. No implicit second transcode.

### 7.3 User-visible storage controls
Expose `Temporary buffer cap`, `Minimum free space`, `Delete temporary cache now`, `Open saved clips`, and `Move capture destination`. On destination changes, do not silently move old recordings. Offer a clear explicit migrate/copy option with verification; do not destroy old files.

## 8. Detection, source selection and per-game settings
- Maintain detection confidence states: `Known game`, `Selected application`, `Unknown foreground app`, `No capture target`.
- Match by executable/process path plus persisted user choice, not only window title or Steam ID. Handle game launchers and child processes.
- Default `Auto games` does not accidentally capture browsers, password windows, desktop notifications, or unrelated workspaces. User can allowlist/denylist apps.
- Per-game overrides: replay on/off, audio inclusion, quality preset, save duration, directory name (safe filename), capture mode.
- Manual `Select window` and `Select display` always available; show a small visual/source preview during setup only when user requests it.
- If target switches or disappears, pause, safely finalize segment, and show `Waiting for game` rather than silently switching to private content.
- Hotkeys globally registered by the OS where permitted; must be user-changeable, distinguish key-down from key-up, debounce repeats, and not interfere with games. Controller hotkey bindings are future scope unless reliably implemented.

## 9. Performance, observability and quality gates
Performance must be measured under *representative actual games*, not inferred from benchmarks. Test 1080p60 and native-resolution 1440p60/4K scenarios as supported. Track GPU encoder use, CPU consumption, RAM, disk writes, dropped/duplicated frames, input-to-buffer latency, capture-to-audio skew, and clip-finalization latency. Define release budgets only after baseline benchmarking, compare game FPS with capture off vs on, and include failures/variance rather than only best-case figures.

**Reliability targets and checks:**
- 100 replay hotkey triggers with unique valid MP4 outputs, source/length checks, and no stale key-down state.
- Repeated 1–2 hour sessions and long replay-buffer soak; no unbounded memory growth or buffer overflow.
- Verify multi-track stream count, stream titles, mix playback, isolated-track contents and timeline alignment via `ffprobe` plus actual editor import.
- Zero unintended microphone capture when mic is off; zero duplicate game audio from overlapping sources in default settings.
- Power loss/kill during recording, during buffer rotation and during clip finalization. Validate partial recovery and leave permanent files intact.
- Disconnect/reconnect microphone and headphones; change system defaults; swap active game; alt-tab; fullscreen/borderless; monitor resolution/refresh/HDR changes; low disk.
- Test at least one Steam game, one non-Steam game, a standard app/window and a display. Test anti-cheat-constrained capture **without bypasses**, reporting fallback results.
- UI accessible by keyboard, high DPI and scaling, small laptop viewport, and no confusing unsupported controls.

## 10. Failure-state UX and safety rules
- Avoid false success: show `Buffering` only with confirmed incoming frames; `Mic capturing` only with confirmed device samples; `Saved` only with a verified playable file.
- Status/error messages are actionable: `Your selected microphone disconnected. Choose another`, `Game cannot be captured in this mode. Try Window Capture`, `Low disk space. Replay paused to protect saved clips`.
- Never elevate to administrator automatically, never install game hooks or drivers without separate explicit user authorization, and never evade game protections.
- No cloud transfer, analytics, microphone recording or desktop recording by default; all optional future connectivity must be opt-in.
- Do not let third-party codecs or plug-ins run untrusted code silently; maintain a dependency bill of materials and verify redistribution/licenses.

## 11. Implementation phases and completion gates
**Phase 0: discovery and architecture spike.** Inspect current development environment/repo, verify Windows build, capture APIs, encoders, media licensing, existing FrameCut import expectations, then prototype separate game-window capture, game process audio, mic audio, voice-chat process audio and MP4 round trip. Produce an evidence-based decision record on native vs libobs. No giant UI rewrite before this passes.

**Phase 1: useful vertical slice.** One source, hardware encoding, replay ring, configurable duration, working global hotkey, final regular MP4 written to user-selected folder, safe crash behavior, tray status. Demonstrate a real game recording. **First user playtest checkpoint.**

**Phase 2: independent audio.** Device picker, visual meters, source mapping, voice chat app capture, preview mix + isolated tracks, editor WAV export, synchronization tests, hot-plug/fallback rules. Verify via `ffprobe` and real editor. **Second user checkpoint.**

**Phase 3: ease-of-use and library.** Three-tab polished UI, first-run guide, automatic game detection, profiles, disk rotation caps, game-based naming, thumbnails/search, meaningful notifications, keyboard accessibility. **Third user checkpoint.**

**Phase 4: hardening.** Soak tests, unusual games, anti-cheat safe fallbacks, low disk/crash recovery, performance comparison, settings migrations, installer/portable option, docs and rollback. Ship when user acceptance gates pass.

**Phase 5: optional integrations, only after solid core.** FrameCut one-click open with separated tracks, instant trim/copy, chapter/event markers, controller hotkeys, rules-based highlight detection, optional Linux backend if justified separately.

Each phase must leave a working build, reproducible setup and test record, and a revertible commit/checkpoint. Do not claim success from mock screens or screenshots alone.

## 12. Explicit acceptance criteria
- A first-time user can select mic, game audio and voice chat in the Capture tab without reading implementation jargon.
- Selecting mic shows a live meter; mic stays off until enabled. Unplugging it does not silently start recording a different mic.
- Pressing replay hotkey after gameplay preserves the available preceding duration and produces a **real playable, conventional MP4** in `Clips\Game Name\` automatically.
- Double-clicking MP4 plays a sensible combined mix. A supported editor can retrieve *independent* original game/mic/chat streams. An unsupported editor has one-click lossless-video `Editor files` stems fallback.
- Sample clip audio is in sync with video at start, at clip boundaries, and after a long recording; report measured tolerance.
- Buffer file cap and minimum-free-space policy are respected. Saved clips are never automatically deleted.
- App can be run without Steam or AMD software, and encoder/backends are chosen by detected capabilities.
- Recording does not silently capture unrelated windows when `Auto games` is chosen.
- Program closes to tray only when configured to, preserves background operation transparently and can be stopped/quit immediately.
- Installer/portable distribution does not impose an unmanaged growing C: cache when D: is selected; unavoidable OS integration storage is documented.
- No production task is marked complete without real capture evidence, media probing, automated checks and a manual test report.

## 13. Potential risks and mitigations
| Risk | Consequence | Mitigation |
|---|---|---|
| Audio already mixed in game | Cannot isolate player voices | Clearly label unsupported source separation; isolate chat app if separate process |
| Preview mix plus stems doubles sound | Too-loud audio in editor | Explicit `Preview Mix` labeling; auto-mute mix in FrameCut and editor instructions |
| Game or anti-cheat blocks capture | Black/blank video | WGC window/display fallback; clear error; no bypass |
| Disk nearly full | Corrupt files or data loss | Reserve, ring retention, atomic finalize and recovery |
| Format/editor mismatches | Track import issues | H.264/AAC MP4 default; optional 48 kHz WAV stems; real editor testing |
| Audio drift/devices change | Desync or wrong source | Common clock, compensation, hotplug tests, explicit source state |
| Encoder overhead | Game FPS decreases | HW encoding when supported, measured presets, throttle/quality fallback |
| GPL/codec redistribution | Legal/distribution blockers | License audit before bundling libobs/FFmpeg/encoder libraries |

## 14. Decision log: recommendations, not unverified facts
- Working title `Capture` until branding chosen.
- Windows-first and dedicated capture worker; Tauri/Rust frontend recommended subject to measured performance.
- Separate audio **must** be recorded at source; editing software should expose it, not synthesize it.
- MP4 H.264/AAC with preview mix + independent streams is proposed default; final choice gated on target editor import tests.
- Replay segments are disposable, saved clips permanent, storage defaults to a user-chosen data drive.
- No AI features, streaming, cloud or full editor in initial release.
- FrameCut integration only after a stable shared import contract and confirmed editor support.

## 15. Source references and engineering verification
Authoritative references reviewed 2026-11-08. These establish capabilities/constraints, **not** proof that the proposed application is already built:

1. OBS, Multiple Audio Track Recording Guide: https://obsproject.com/kb/multiple-audio-track-recording-guide/
2. OBS, Audio/Video Formats Guide: https://obsproject.com/kb/audio-video-formats-guide/
3. OBS, Hybrid MP4 & Hybrid MOV Formats: https://obsproject.com/kb/hybrid-mp4
4. Microsoft, Application loopback audio capture sample: https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/
5. Microsoft, process loopback API requirements (Windows 10 Build 20348): https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-audioclient_activation_type
6. Microsoft, WGC screen capture API: https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture
7. Microsoft, WGC CreateForWindow: https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow
8. FFmpeg, stream selection and mapping: https://ffmpeg.org/ffmpeg.html
9. NVIDIA Video Codec SDK: https://developer.nvidia.com/video-codec-sdk
10. OBS, game capture troubleshooting: https://obsproject.com/kb/game-capture-troubleshooting
11. Tauri, architecture: https://v2.tauri.app/concept/architecture/

**Final instruction:** Protect the effortless experience. If a feature needs an entire new screen or setup wizard step, justify it against the four product promises before adding it.
