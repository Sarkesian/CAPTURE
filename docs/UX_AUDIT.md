# CAPTURE UX/UI audit and overhaul (v0.4.0)

Before screenshots: `docs/ux/before/`. After screenshots: `docs/ux/after/` (same sizes, same pages).

## 1. Audit: what was wrong (v0.3.0)

| Area | Problem | Evidence |
|---|---|---|
| Window chrome | The default white/grey Windows caption sat above a near-black app, so the window looked like two products glued together. | Before shots; a real window showed the light caption. |
| Navigation | Three top tabs (Capture / Library / Settings). Replay had no page of its own: it was reachable only from the in-game overlay. | `home-1000x760.png` |
| Home | The status card said "Buffering", which is engine jargon. The main action ("Save clip") sat in a second card under a big red toggle, so the strongest colour went to a setting, not the action. Recent clips and free space were not on Home. | `home-*.png` |
| Colour | Red meant several things: brand, toggles, primary buttons, "recording" and danger. Nothing was left to signal that the app is recording. | All pages |
| Audio meters | They updated only with the engine's 1 Hz state event and had no ballistics, so they jumped once per second and lagged speech by up to a second. They showed no RMS and no peak hold. Clipping, muted, disconnected and "not recording yet" all drew the same empty bar. | `AudioSources.svelte` (v0.3.0), `Meter.svelte` |
| Settings | One long page ordered by when each feature was added (capture and hotkeys, markers, games, quality, then "Storage & privacy" holding startup, tray, notices and overlay size). There was no index. The temporary-buffer path was shown without a backslash (`D:\CapturesBuffer`). | `settings-*-full.png` |
| Library | Each card had a row of seven text buttons. There was no sort, no game filter and no loading state, and the only empty state was one plain line. | `library-*.png` |
| Overlay | Its colours, buttons and icons (Unicode glyphs ⏮ ▶ ⏭) did not match the main window. The review controls were text glyphs. | `overlay-*.png` |
| Feedback | Hover was only a subtle border change, there was no pressed state, focus rings showed on mouse clicks, and nothing honoured reduced motion. | |
| Small windows | At 760×600 the top bar wrapped and Home cards stacked unevenly. | `*-760x600.png` |

## 2. Decisions

- **Design tokens** (`app/src/app.css`): surfaces, text, accent, status colours, spacing (4 px scale), radii, type scale, control heights and motion timings. Dark is the default; light follows Windows (`prefers-color-scheme`). The in-game overlay is always dark.
- **One meaning per colour**:
  - Blue accent: primary actions and selection.
  - Red: "recording" and destructive actions only.
  - Amber: warnings and markers.
  - Green: success.
- **Window frame**: DWM caption and border colours match the app chrome (#121418 dark, #EEF0F3 light). They are re-applied when the Windows theme changes. The native caption, snap, resize and Win+arrow behaviour are kept: no custom title bar to break them.
- **Navigation**: a sidebar with Home, Replay, Library and Settings. It has arrow-key movement and `aria-current`, collapses to icons below 900 px, and remembers scroll position per page. A status line at its foot (for example "Replay on · FAULTHEART") is visible from every page.
- **Home answers five questions at a glance**:
  - Is it recording? The kicker and the red dot.
  - What is it recording? The title: the game, or a plain state such as "Recording is off" or "No game running".
  - How much can I save? The replay-ready bar, with the length selector next to it.
  - Is audio working? Live meters per source.
  - What do I do? "Save replay" (primary, with its hotkey), plus Marker, Record and Review.

  Recent clips and the storage line sit underneath.
- **Replay page**: wraps the existing review player. It has empty states for "recording off" and "nothing yet", and releases its snapshot when you leave the page.
- **Meters, real data only**:
  - The engine computes peak and RMS per track over each ~33 ms window and sends them (`Levels` event, ~30/s) only while a visible window has subscribed. The subscription stops when the window is hidden or no meter is on screen.
  - The UI draws them in one `requestAnimationFrame` loop that writes CSS variables into `transform: scaleX()`, so drawing is compositor-only with no layout.
  - Ballistics: one-pole smoothing in the dB domain. Peak bar: 30 ms attack, 90 ms release constant (about 270 ms to settle). RMS bar: 60 ms attack, 120 ms release. Bars glide between the 30 updates per second instead of stepping, but they only ever move towards the measured level. The peak-hold tick (0.9 s) and the clip mark (≥ −0.1 dBFS, held 1.5 s) use the raw peak, so short overloads are never smoothed away.
  - Drawing is capped at about 60 frames/s whatever the display rate, and stops entirely when every meter is at rest.
  - Muted, off, error/disconnected and "waiting" states draw a flat bar with a label. Nothing animates without signal.
- **Settings by task**: Recording, Audio, Hotkeys, Markers, Overlay & notices, Games, Storage, Startup and Advanced, with a sticky section index that tracks scrolling. Every control still applies immediately. Sliders preview while dragging and save on release, instead of writing settings on every pixel. The Custom quality preset was not added: Balanced, High and Maximum cover the encoder's tested range, and a custom bitrate field would be an untested path.
- **Library**:
  - Search, game filter and sort (newest, oldest, largest, name), with a count and total size.
  - Hover actions: show in folder, plus a "…" menu with Play, Copy path, editor audio files, Rename, and Delete with confirmation. The menu closes with Esc or a click outside.
  - Skeleton cards while loading, and real empty states that name the hotkey to press.
- **Overlay**: the same tokens and icons, the four large actions with icons, and audio and replay length side by side. "Buffering" was renamed "Replay on", matching the main window.
- **Speed**:
  - 80 ms and 140 ms transitions on colour and opacity only, and a pressed scale of 0.985.
  - No entrance animations.
  - `prefers-reduced-motion` zeroes every transition and stops the pulsing dot.
  - Focus rings appear only for keyboard focus (`:focus-visible`).

## 3. Problems found while measuring, and fixed

- **Meters drew at the display rate.** They drew 144 frames/s on a 144 Hz monitor. Now capped at 60 and idle when silent.
- **Minimised windows kept working.** WebView2 kept reporting `document.hidden = false` while the window was minimised or hidden. Pages kept animating, and the hidden overlay (which shows meters too) kept the 30/s level stream running all the time. The app now sets the WebView2 controller's visibility on every minimise, restore, hide, show and tray start. A minimised window now costs 0.03% UI CPU and gets no level traffic.
- **Jumpy meters.** The user reported the meters looked erratic. They now use smoothed ballistics (above), still driven only by real levels.

## 4. Measurements (0.4.0 package, recording FAULTHEART, i5-13600KF / RTX 4090 / 144 Hz)

From `scripts/ui-perf.mjs`, run by `scripts/run-acceptance.ps1`:

| | Home visible | Window minimised |
|---|---|---|
| Level updates received | 30.0 /s | 0 /s |
| Meter frames drawn | 48 /s (60 cap on a 144 Hz display) | 0 /s |
| UI CPU (CAPTURE.exe + its WebView2 processes; % of 20 logical CPUs) | 1.8–2.8 % | 0.01–0.04 % |
| Engine CPU | 0.13–0.37 % | 0.15–0.31 % |

- Meter cost on the engine: within noise (−0.02 to +0.06 %).
- Page switch, from click to painted frame: 10–22 ms.
- After a restore, the page is visible again within about 1 ms and the meters resume.

Acceptance on the same package:
- `ui-auto-test` 10/10: auto-detection, notice excluded from capture, numpad hotkeys.
- `ui-overlay-test` 18/18: overlay actions, markers, review, range save, sessions.
- `ui-perf` 7/7.

## 5. Not done or not verified

- A Custom quality preset (optional in the brief); not added, because it would be an untested encoder path.
- No screen reader was used to test. Controls have labels and keyboard paths.
- The light theme is checked through emulation (`home-light-*`, `settings-light-*` screenshots), not on a machine set to light mode.
