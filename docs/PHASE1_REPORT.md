# Phase 1 report: first playable vertical slice

**Date:** 2026-11-08 · **Branch:** `phase0/spikes` · **Build:** `dist\CAPTURE-0.1.0\CAPTURE.exe` (portable, 134 MB,
made by `scripts\package-portable.ps1`)

## Implemented
- **App + tray:** Tauri 2 / Svelte 5 UI (`CAPTURE.exe`) with three tabs. Capture and encoding run in a separate
  `capture-worker.exe` over a versioned JSON-lines protocol. The worker is restarted automatically if it
  crashes (at most 3 times a minute). The tray shows live status and offers Save replay, replay on/off, Show and Quit.
- **Replay ring:** 2 s MPEG-TS segments in `<root>\Buffer\<session>`. Retention is replay length + 6 s, with a byte cap and a
  free-space reserve (buffering pauses with a message instead of filling the drive). The buffer is deleted
  when the session ends. Saved clips are never touched by retention.
- **Save:** protects the segments it needs, waits for late audio, stream-copies to MP4 (`.partial` → probe → atomic rename),
  and reports only verified results. Repeated and overlapping saves get unique names. A young buffer saves what exists and
  says so.
- **Capture:** Windows Graphics Capture (window or display, no injection) produces a constant 60 fps output stream
  through NVENC (zero-copy). Encoders are tried in this order, with reasons recorded: NVENC → AMF → Media Foundation (hardware, then software).
- **Audio:** `Preview Mix` (default) plus `Game Audio` (process loopback of the game and its child processes) or `System Audio`
  (endpoint loopback for display capture). Audio is aligned to QPC with gap padding and drift correction.
- **Targets:** the user picks a running app (remembered by its exe) or a display. While the app isn't running the status says
  *Waiting for X* and buffering starts automatically when it launches.
- **Hotkey:** global `RegisterHotKey` in the worker, with no key-repeat, a 750 ms debounce, a recorder UI, and plain-language
  conflict messages.
- **Files:** `<root>\Clips\<Game Name>\YYYY-MM-DD_HH-MM-SS_<Game>_Clip.mp4`. Open file, Show in folder and Copy path
  work. The Library lists clips straight from the folders.
- **Storage hygiene:** settings and the WebView2 profile live in `<exe folder>\data` (portable). Nothing CAPTURE-specific
  is written to C: except what Windows itself keeps (notification registration, and WebView2 runtime files that are
  already installed). The default capture root is `D:\Captures`.

## Evidence (this machine: RTX 4090, Win11 26200)

| Test | Result |
|---|---|
| Unit tests (`cargo test --workspace`) | 22 pass: ring selection/protection/retention, audio alignment, hotkey parsing, names, settings |
| Display replay via worker harness (`scripts/e2e-worker.mjs`) | young-buffer save 2.79 s of 10 s; two overlapping saves 10.81 s / 11.11 s; buffer emptied on exit |
| FAULTHEART (itch.io, non-Steam) window replay | same scenario, all saves verified, `Game Audio` = process loopback with real signal |
| Output probe | H.264 High, 3840×2160 (display) / 1604×946 (window), **constant 60 fps** (648 frames / 10.8 s), Preview Mix + isolated track, full decode OK. Audio starts 16–21 ms after video (one AAC frame). |
| Real app UI driven over WebView2 DevTools (`scripts/ui-drive.mjs`) | replay switch → Buffering; **real global Alt+F10 (SendInput)** saved; Save clip button saved; Copy path reached the Windows clipboard; Library listed 2 clips; Settings shows `✓ Alt+F10 is active`; closing the window quit both processes and left only clips |
| Settings robustness | a UTF-8 BOM settings file now loads; an unreadable file is kept as `settings.json.bad` (bug found by the UI run and fixed) |
| Resource use (FAULTHEART buffering, 1604×946@60) | worker 0.19 % CPU (of 20 threads), 98 MB RAM; NVENC 7 %; game CPU unchanged (1.66 % → 1.59 %). UI process 35 MB (WebView2 renderer processes not counted). |

Screenshots of the driven UI: `test-output\ui-shots\` (git-ignored).

## Bugs found by testing and fixed
1. Two saves in the same second picked the same file name. Names are now reserved atomically with a create-new `.partial` file.
2. A save could read the still-open newest segment. Only closed segments are used now.
3. A per-track `title` produced MP4 udta warnings. Only `handler_name` is written now.
4. bindgen's stack overflowed in dev builds. Build scripts are now optimized.
5. Settings with a UTF-8 BOM were silently replaced by defaults (see above).
6. The young-buffer toast wording was circular.

## Not implemented / known limitations
- **Steam game test pending:** no Steam game is installed (The Witcher 3 is installing). This is required for the playtest.
- Microphone, voice-chat app track, WAV editor stems, and the A/V sync drift test over long sessions: **Phase 2**.
- Auto game detection, first-run flow, thumbnails, profiles: **Phase 3**. Game FPS overhead, long soak, crash/kill recovery
  of leftover buffers (currently only *deleted* on request, not salvaged), installer: **Phase 4**.
- If the game closes, its footage can still be saved until a new session starts; once the game relaunches, the old buffer is
  finalized and removed.
- Window capture includes the window border and title bar for windowed games. Borderless/fullscreen games are unaffected.
- A clip can be up to 2 s longer than requested, because it starts on a keyframe.
- NVENC tags colour as BT.601 (`bt470bg`) because its RGB→YUV conversion uses that matrix. Players render it correctly.
- AMF/QSV are unverified on this hardware. QSV is not implemented, and the UI says so under Advanced.
- Exclusive-fullscreen games may produce no frames under WGC. After 5 s the UI says so and suggests borderless or Display capture.

## What to playtest now
1. Run `D:\Projects\CAPTURE\dist\CAPTURE-0.1.0\CAPTURE.exe`.
2. Start a game (The Witcher 3 once installed, or FAULTHEART), then in CAPTURE press **Refresh**, pick it, and turn on **Instant replay**.
3. Play for a few minutes, press **Alt+F10** a few times (including twice quickly), and alt-tab in and out.
4. Check `D:\Captures\Clips\<Game>\`: double-click a clip (it should sound normal), then drop one into your editor and confirm
   there are two audio tracks (Preview Mix + Game Audio).
5. Tell me anything confusing in the UI, any wrong game name, and whether exclusive fullscreen worked.

## Next highest-value work (Phase 2)
Microphone (off by default, named devices, meter, no silent fallback), voice-chat app track, the `Editor audio files`
WAV stems, and a long-run A/V sync measurement (a flash+beep test pattern).
