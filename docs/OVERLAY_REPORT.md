# Overlay & replay addendum: milestone report (v0.2.0)

**Date:** 2026-11-08 · **Branch:** `phase0/spikes` · **Build:** `dist\CAPTURE-0.2.0\CAPTURE.exe`
**Spec:** `docs/spec/CAPTURE_Overlay_Addendum.md`. The section numbers below refer to it.

## Implemented and tested

| Area (§) | What works now | Evidence |
|---|---|---|
| Overlay (§2) | Hotkey overlay (default **Alt+F11**). Status line, four large actions (Save Replay, Add Marker, Review Recording, Start/Stop Recording), replay-length presets, live game-sound meter, 3 recent clips with real thumbnails and duration (Play in overlay / Folder / Delete → Recycle Bin, with confirmation), Quick settings, Open folder, Full settings. **Esc** closes it. A hidden overlay does not render. | `scripts/ui-overlay-test.mjs`, 18/18 pass on FAULTHEART with real Alt+F11 / Alt+F8 key events |
| Instant replay (§3) | Direct-to-MP4 saves through the verified remux. Cuts are now keyframe-precise (1 s GOP): a 6 s replay saves as 6.32 s (previously up to +2 s). Saving never interrupts capture. | `scripts/e2e-review.mjs` |
| Review timeline (§4) | Plays buffered footage while recording continues (stream-copied snapshot ready in 0.55 s for 14 s at 4K). Play/pause, ±5 s, scrub by click/drag, beginning, **Live** (refreshes to the newest footage), previous/next marker, **Create clip** (default −15 s / +5 s around the playhead), two drag handles, **Save clip**. Footage being reviewed is protected from overwrite. | range 3–8 s saved as 5.02 s; overlay test "review plays", "selected range saved" |
| Markers (§5) | Global hotkey (default **Alt+F8**), about 0.1 s. Optional chime (volume setting). Markers show on the timeline; click one to select around it; **Save around** in one click; rename and remove. Status is honest: *Temporary*, *Kept until saved or session ends*, *Saved in a clip*, *Footage gone*. Protection is ±N s (default 30/15) within half the buffer limit; the oldest protection is released first, with an explicit note. Markers persist in `<session>\markers.json`; saved clips get `<clip>.markers.json`. | `scripts/e2e-markers.mjs` |
| Chime isolation (§5) | Game Audio = process loopback of the game only. Whole-display System Audio = process loopback **excluding CAPTURE's own processes**, so the chime cannot enter any recorded track. | chime peak −27 dB on the device; **−240 dB (absent)** in the clip's System Audio and Preview Mix |
| Session recording (§2) | Start/Stop (button, or optional hotkey) records any length to `Sessions\<Game>\…_Session.mp4` using the same capture (no second encoder). Finalized automatically when the game closes, recording is turned off, the free-space reserve is reached, or CAPTURE quits. | `scripts/e2e-session.mjs`: 10.55 s session with a 5 s replay length |
| Settings sync (§7) | One settings store in the UI process. Every change from the main window or the overlay is broadcast to both windows. | overlay presets ↔ main window |
| Hotkeys (§7) | Save / marker / overlay / session, each with conflict and duplicate detection, numpad (NumLock on or off) and symbol keys. Defaults avoid NVIDIA's Alt+Z / Alt+F9. | unit tests + real-key runs |
| Capture quality | Window capture now records only the game's client area (no title bar or borders): FAULTHEART 1600×900. | frame check |

Resource use while buffering FAULTHEART: worker about 100 MB, UI process 31 MB (WebView2 renderer processes not counted). Overlay open/closed FPS impact on a demanding game has **not** been measured yet.

## Not implemented yet (by design order, not forgotten)
- **Microphone** row/dropdown/mute/test and **voice-chat** track (§6): next milestone. The overlay shows no mic controls until they work.
- Screenshots hotkey (§13), per-game preferences, custom replay length typed in, per-notification toggles, overlay size/opacity, separate buffer/sessions paths.
- Device recovery for audio hot-plug (§6): comes with the mic work.
- Performance benchmark with the overlay open vs closed (§12) on a demanding game.

## Known limitations
- **Exclusive fullscreen**: Windows will not show a normal topmost window over an exclusive-fullscreen game. Opening the overlay will switch away from the game. Hotkeys (save, marker, session) keep working. Borderless/windowed games are fine.
- The review is a snapshot. **Live** refreshes it; it does not stream continuously.
- A microphone near speakers can still pick up the chime acoustically (no software can prevent that).
- Marker protection ends when the session ends (game closed, CAPTURE quit). Save a clip to keep footage permanently.
