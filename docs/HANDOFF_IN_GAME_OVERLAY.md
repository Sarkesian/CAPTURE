# Handoff: in-game overlay (CAPTURE 0.6.x), 2026-11-09

> **Update, 2026-11-09 (later the same day).** The open bug below was attacked as follows; none of it has been
> seen on Skyrim yet (build `dist\CAPTURE-0.6.1-setup.exe`, not installed).
> - Where the overlay opens is now decided **by the worker at the key press**, from the window in front at that
>   moment (`Command::OverlayOpen` -> `Event::OverlayOpened { mode }`, `Engine::overlay_mode`), not by the UI from the
>   last published state. A window is opened only when nothing fullscreen is in front; fullscreen = covers >= 98 % of
>   its monitor (`targets::placement`), or Windows reports a Direct3D fullscreen application, or the hook reports
>   exclusive. In-game needs the captured game in front with the hook drawing; otherwise the notice.
> - Every request logs one line `overlay: asked for; ...` with all the facts (rectangles, hook state, exclusive).
> - `capture-inject32.exe` is started without a console window (one appearing takes focus from a fullscreen game).
> - The hook takes the game's input only once the overlay is really drawn; the worker closes an in-game overlay
>   when the game stops drawing or Windows gives no picture of the overlay window for 2 s.
> - The overlay window is no longer raised with `set_always_on_top(false)` (that put it above the game).
> - **Open request from the user (not started):** Replay must be on the overlay's FIRST screen, in one clean,
>   simply organized menu with the other actions; no button press to reach the footage. Also unknown: whether the
>   Replay video renders inside the in-game (mirrored) overlay; `capture.log` now has `replay: footage ...` lines.
> - Since this note: Replay tab/overlay Replay show a closed game's footage, a timeline that follows the recording,
>   saved clips and screenshots on it; `Buffer` folder renamed `Background Recording`; overlay colours converted
>   for sRGB/HDR back buffers (unverified on screen); overlay input goes through DevTools `Input.dispatch...`.
> - Build with `scripts\package-portable.ps1` (it now builds the 64-bit hook on its own: together with
>   `--bin capture-worker` it was never rebuilt). The one-line `cargo build ... -p capture-app` below overflows
>   bindgen's stack.


For the next engineer/agent. Written at the point of escalation: **opening the overlay over
fullscreen Skyrim still minimizes Skyrim.** Everything below is what is true now, with evidence.

## What the user wants (non-negotiable)

1. **Overlay like Steam / NVIDIA / Xbox Game Bar:** pressing the overlay hotkey (user's: **Num 8**)
   in a fullscreen game shows CAPTURE's overlay panel **inside the game**. The game must **never
   minimize, flash or lose focus**. Simple and clean; the user called the current state "janky".
2. **Background footage viewable in the overlay** (the Review Recording player) inside the game.
3. **Fallback:** when the overlay cannot be shown in a fullscreen game, **do not open a window**
   (it minimizes the game); show a notice instead ("Overlay unavailable in fullscreen"). Fallback
   only, the real fix is wanted.
4. **One notice at a time, never doubled** (in-game OR notice window, never both). Done.
5. **Universal:** any store (Steam/Epic/local), any API eventually (DX9/11 now; DX12/Vulkan/OpenGL
   later). Never inject into games with anti-cheat. Admin rights acceptable if needed.
6. **Testing:** the user wants to **test together**, not have the agent run fullscreen tests on
   their screen unannounced (it steals their focus, minimizes their game, looks broken). Ask first,
   tell them exactly what to expect, let them report.

## Repo state

- `main` @ `1299b99` = **0.6.0**: in-game **notices** via the hook (committed, verified).
- **Uncommitted** (this work, 0.6.1, installed on the user's PC at `D:\Programs\CAPTURE`):
  - `crates/capture-protocol/src/{hook.rs,lib.rs}`: shared-memory v4 (overlay picture, input
    window, `exclusive`), `Command::OverlayMirror{hwnd}`, `Command::OverlayUnavailable`,
    `EngineState::{in_game, game_fullscreen}`.
  - `crates/capture-hook/src/lib.rs`, `detour.rs`: layers (dim / overlay / pointer / notice),
    DX11 + DX9 drawing, input takeover (window subclass, WM_INPUT swallow, DirectInput hooks),
    `exclusive` reporting.
  - `crates/capture-engine/src/{mirror.rs (new), inject.rs, lib.rs, targets.rs}`: WGC capture of
    the overlay window into shared memory; `in_game()` decision + logging; `fills_monitor()`.
  - `app/src-tauri/src/main.rs`: overlay toggle (in-game mirror / fallback notice / window),
    WebView2 switches (`BROWSER_ARGS`).
  - `scripts/e2e-ingame-mirror.mjs` (new): mirror + click/key forwarding test (passes).
  - **Not mine (another session's UI work, uncommitted in the same tree):** `app/src-tauri/icons/*`,
    `app/src/components/FirstRun.svelte`, `HotkeyField.svelte`, maybe parts of `tauri.conf.json`.
    Stage only your own paths when committing (`git add <files>`, never `-A`).
- Architecture decision recorded in `docs/ADR-0001-capture-architecture.md` (Amendment 0.6.0).

## How it works (pieces)

| Piece | Where | Role |
|---|---|---|
| Hook DLL | `crates/capture-hook` (`capture-hook.dll` 64-bit, `capture-hook32.dll` 32-bit) | Inline detours (iced-x86, `detour.rs`) on DXGI Present/Present1/ResizeBuffers, D3D9 Present/PresentEx/SwapChain Present, DirectInput GetDeviceState/GetDeviceData. Draws layers into the back buffer with saved/restored state. Never unloads (dormant between sessions; one hook claims a session). |
| 32-bit loader | `crates/capture-inject32` | 64-bit worker cannot CreateRemoteThread(LoadLibraryW) into 32-bit games. |
| Injector | `capture-engine/src/inject.rs` | Anti-cheat refusal list, private temp copy of the DLL, shared memory (`capture_protocol::hook`), `usable()` (hook presenting D3D9/11 within 2 s), `exclusive()`. |
| Mirror | `capture-engine/src/mirror.rs` | WGC capture of the overlay HWND -> staging -> shared memory (`overlay_frame`). Finds WebView2's `Chrome_RenderWidgetHostHWND` for input. |
| Decision | `capture-engine/src/lib.rs` `Engine::in_game()` | `in_game = hook usable && (exclusive || fills_monitor)`; `game_fullscreen = exclusive || fills_monitor`. Published in `EngineState`; logs `in-game overlay: available/not available (...)` on change. |
| UI toggle | `app/src-tauri/src/main.rs` `toggle_overlay_window` | Reads `last_state`: `in_game` -> show window `SW_SHOWNOACTIVATE` + `HWND_BOTTOM`, `SetIsVisible(true)`, send `OverlayMirror`; `!in_game && game_fullscreen` -> `OverlayUnavailable` notice, no window; else normal window (`show()` + `set_focus()` -> **minimizes a fullscreen game**). Logs `overlay: opening ...`. |

Log: `D:\Programs\CAPTURE\Data\Logs\capture.log` (UI + worker stderr).

## Verified (automated, DX11 test game `crates/hook-testapp`)

- In-game notices: windowed + **true exclusive fullscreen** (legacy and flip model), never doubled,
  dormant hook re-wakes on the next session. Skyrim (32-bit D3D9 + Steam overlay): notices drawn
  in-game, user confirmed (`api 3`, hundreds of notice frames).
- `e2e-ingame-mirror.mjs` (windowed test game): a stand-in window is drawn **inside the game**
  (opaque, centered, game dimmed), a click and a key posted to the **game window** reach the
  stand-in through the hook; closing restores the game.
- Real CAPTURE overlay, DX11 test game in exclusive fullscreen, simulated Num 8: log shows
  `overlay: opening inside the game` / `in-game overlay: showing the overlay in the game`, game
  **stayed fullscreen**. BUT the panel was **see-through** (WGC alpha = 0). Fixed afterwards by
  drawing the overlay layer opaque (`alpha.y` in the shader / `D3DTOP_SELECTARG2` on DX9).
  **The fixed version has not been seen working with the real overlay UI by anyone.**

## Open bug: Num 8 over fullscreen Skyrim minimizes Skyrim

User repeated it on every build. Latest log (0.6.1 with the exclusive-fullscreen detection):

```
2026-11-09 13:27:32 CAPTURE 0.6.1 starting (installed)
in-game overlay: loaded into process 32276        <- Skyrim (restarted by the user)
overlay: opening as a window                      <- Num 8: in_game=false AND game_fullscreen=false
```

No `in-game overlay: available` line appeared for 32276 (it logs only on change, and the initial
value false is not logged). So at the key press the worker's last published state had
`in_game=false, game_fullscreen=false`. Since `game_fullscreen = exclusive || fills_monitor`,
**both** were false: the hook was not yet `usable()` (or `exclusive` read 0) **and**
`fills_monitor()` was false. Hence the UI took the plain window path (show + set_focus) -> Skyrim
minimized.

Earlier run with the previous Skyrim instance (12116):
```
in-game overlay: available (hook drawing: true, game fills its screen: true)
in-game overlay: not available (hook drawing: true, game fills its screen: false)   <- flipped
overlay: opening as a window
...state 2 api 3 presents 321 notice frames 254 backbuffer 3840x2160 fmt 22
```
So for Skyrim `fills_monitor()` **flips to false while the game is running and in front**, and the
hook is drawing D3D9 at 3840x2160. And in an even earlier instance (29816) the hook stayed at
`state 0` (never claimed the session) — cause unknown (stale older hook copies in the same game
process are a suspect; versions now differ so they should ignore it).

### Hypotheses, most likely first

1. **`fills_monitor()` is unreliable for Skyrim.** `targets.rs`: compares `GetWindowRect(game)`
   with `GetMonitorInfo(MonitorFromWindow).rcMonitor`. Skyrim (DPI-unaware, D3D9 "fullscreen"
   possibly run by Windows as fullscreen-optimized flip, 4K display at 150 % scaling) may report a
   virtualized or shifted rect, or the window may be 1 px off / have a hidden border. Log the two
   rects when they differ. A tolerant check (covers >= 95 % of the monitor, or the game window is
   the foreground window and `WS_POPUP` without caption) is probably right.
2. **`exclusive` never becomes 1 for Skyrim.** DX9 path reads
   `dev.GetSwapChain(0).GetPresentParameters().Windowed`. Skyrim may present windowed (FSO /
   borderless-at-mode) even with `bFull Screen=1`; then exclusive=0 and only (1) can save it.
   Also `exclusive()` requires `usable()` first. Add the hook's `exclusive` value to the
   `in-game overlay:` log line so this is visible.
3. **Stale state at the key press.** The UI decides from `last_state` (published only on change,
   ~2/s). After a Skyrim restart the first states have the hook not yet presenting. Safer: decide
   in the **worker** at the moment of the request (UI sends `OverlayRequest{hwnd}`, worker replies
   mirror / unavailable / window), or at least re-read state.
4. **Even when `in_game` is true, verify nothing in the in-game path steals focus**:
   `ShowWindow(SW_SHOWNOACTIVATE)` + `SetWindowPos(HWND_BOTTOM, SWP_NOACTIVATE)` on a window that
   Tauri created topmost; WebView2 `SetIsVisible(true)`; page `overlay_shown` handler. This path
   kept the DX11 test game fullscreen, but has not been observed on Skyrim (D3D9, DirectInput,
   Steam overlay present).
5. **Never fall back to a focus-taking window when the foreground window is the captured game and
   it covers its screen** — regardless of the hook. The fallback rule should be conservative:
   if in doubt, show the "Overlay unavailable in fullscreen" notice, not a window.

### Quick diagnostics to add first

- Log at every Num 8: `in_game`, `game_fullscreen`, hook state/api/exclusive/age of last present,
  game window rect, monitor rect, foreground window == game window.
- Log once per session: Skyrim's D3D9 present parameters (Windowed, BackBufferWidth/Height,
  FullScreen_RefreshRateInHz).

## Not done / known limits

- **The real overlay UI inside a game has never been seen working end to end** (content, clicks,
  Review player). Input forwarding is proven with a WinForms stand-in only. WebView2 input via
  `PostMessage` to `Chrome_RenderWidgetHostHWND` is unproven; if Chromium ignores posted input,
  options: WebView2 CompositionController `SendMouseInput` (needs visual hosting, not supported by
  Tauri), or render the overlay UI differently.
- WebView2 must keep rendering while the window sits behind the game: `BROWSER_ARGS` in `main.rs`
  disables native occlusion + background throttling. The overlay page also resets on
  `document.hidden` (`Overlay.svelte`); if the page still thinks it is hidden, Review breaks.
- DirectInput takeover (Skyrim) written but **never exercised** (no test game uses DirectInput).
- Notices/overlay drawn in-game also appear in the recording while shown (planned: hand CAPTURE the
  clean frame from the hook before drawing).
- DX12, Vulkan, OpenGL: no drawing path yet (window notices + fallback notice).

## Build, install, test

```powershell
$env:RUSTUP_HOME='D:\Programs\Rust\rustup'; $env:CARGO_HOME='D:\Programs\Rust\cargo'
cargo build --release -p capture-hook -p capture-engine --bin capture-worker -p capture-app
cargo build --release -p capture-hook -p capture-inject32 --target i686-pc-windows-msvc
powershell -ExecutionPolicy Bypass -File scripts\package-portable.ps1   # dist\CAPTURE-<v>\
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1    # dist\CAPTURE-<v>-setup.exe
# install over the user's copy (keeps settings); wait for makensis to finish first, verify hashes:
dist\CAPTURE-0.6.1-setup.exe /S /D=D:\Programs\CAPTURE
```

Tests (DX11 test game; they put windows on screen, **ask the user first**):
`node scripts/e2e-ingame-overlay.mjs <worker> <root> target\release\hook-testapp.exe [--fullscreen]`,
`node scripts/e2e-ingame-mirror.mjs <worker> <root> target\release\hook-testapp.exe`,
`attach:TESV.exe --fullscreen` attaches to a running Skyrim.

Gotchas seen in this session: the installer silently fails to replace files if run while a build
(`makensis`) or a test still holds them — verify hashes after installing. A fullscreen game
minimizes whenever any other window takes focus (including console windows spawned by test
scripts: use `windowsHide`). Original Skyrim is installed (`D:\Programs\Steam\steamapps\common\Skyrim`,
32-bit D3D9, `bFull Screen=1`, Steam overlay active); its launcher needs a click on PLAY.
