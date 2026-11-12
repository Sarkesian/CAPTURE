# CAPTURE 0.7.0 — recording, replay and library overhaul

Source of requirements: "Software Overhaul and Improvement Backlog", 11 November 2026, chapter 2
(Capture), sections 2.1–2.12 and the Final Objective. This report gives, per section, the root
causes found, what was changed, how it was verified, and what is still open.

Branch `overhaul/capture-backlog`, based on `feature/replay-experience` plus the uncommitted
0.6.5 work that was sitting in the `CAPTURE-replay` worktree (the build that is installed).
Nothing was installed, and the installed app, its settings and the recordings in
`G:\Videos\Captures` were only read, never changed.

## What was actually wrong (evidence from the real recordings)

The installed app's history index and capture folder were inspected read-only.

| Symptom in the backlog | What the data shows | Root cause |
|---|---|---|
| "Eight visually identical thumbnails" | Eight clips in `Hookshot\Clips`, 02:44–02:45 on 11 Nov, each **5,433,672 bytes and 5.435 s**, byte-identical | The game switched from 3840×2160 to 1920×1080 while closing. A resolution change starts a new internal recording, so the last 5 s became a recording of their own. Instant Replay only ever looked at the *newest* internal recording, and a closed game's last recording stayed saveable for as long as no other game started. Each press of the save key (`Num1`, pressed an hour after the game closed) wrote the same 5 seconds again. |
| "Unexplained five-second recordings" although Instant Replay is 2 minutes | Same eight files; also 1–5 s internal recordings at every game start and exit | Same cause: the replay window never reached back across a resolution change or restart, so a press shortly after one saved only the seconds since. Nothing told the player why. |
| "Fragmented footage", "missing gameplay history" | 60 internal recordings in two days; 27 still holding footage; many of 1–50 s | Every game launch, every resolution change and every capture restart was shown as its own unrelated "background recording". The footage itself was continuous inside each one (e.g. 3,430.5 s in 1,716 segments with no hole); it was the presentation and the replay logic that were fragmented. |
| "Replay cache is busy. Close another Replay view and try again." | 4K at ~53 Mbit/s: one playback window reserved ~100–120 MB | Playback windows were admitted against a fixed 256 MiB budget counted by what each window *might* need (source segments plus 3 s margins each side). Two or three windows in flight (current, prefetch, a new seek) exceeded it. Every seek also started its own thread and its own remux, so scrubbing piled them up, and windows were only freed when the UI remembered to release them. The message blamed "another Replay view" for what was a single view's own requests. |
| Hover shows only a timestamp | Thumbnails came from the player's currently loaded window (≈10 s at 4K) | Outside that window there was no picture to show. |
| Hover `1:11:04.583` on a `0:57:10` recording | The 0:57:10 recording is `2026-11-11_00-24-16` (3,430.5 s) | Not reproduced exactly. The old viewer derived its times from three places (the bound zoom range, the interval list and the loaded window's own clock) and nothing forced a hover time to lie inside the recording. See 2.3 for what replaced it. |
| 2-hour history | 119.7 minutes kept, 49.8 GB, with a 50 GB limit and 2 h configured | Honoured, but only just: at this bitrate 2 h is ~48 GB, the playback cache was counted against the same limit, and 129 s of footage imported from an older version counted too but could never be removed. The app never said which limit was the one in force. |
| Home showed "Background recording 3 GB of 50 GB" | — | It showed the bytes of the current internal recording, not of the history. |

Two more defects found on the way:

- `cap_bytes as i64` turned an unlimited storage cap into −1, which made retention delete
  everything. No shipped setting reaches that value, but a test did.
- "Delete temporary footage" in Settings ran retention with a zero limit, which also deleted the
  closed segments of the recording in progress.

## The model now

- An **internal recording** is what one capture pipeline writes: one game, one resolution, one
  audio layout. A new one still starts at every launch and resolution change (recording at the
  game's native resolution is kept).
- A **session** is consecutive internal recordings of the same game with less than 15 minutes
  between them, on one clock (seconds since the session began). The break between two of them is
  a visible gap. The index stores this (`history.sqlite` schema 4); the 60 real recordings
  become 6 sessions.
- A **game's recording history** is every recorded stretch of all its sessions on one timeline,
  end to end, with the unrecorded time in between collapsed to a short break, as in Steam's
  viewer. This is what the Library card opens and what the overlay shows.
- Everything the UI sends or receives is session time; the engine translates to a recording and
  its media time in one place (`history.rs`).

## Section by section

### 2.1 Background recording reliability and missing footage

- Lifecycle traced end to end (capture → encode → 2 s segments → index → retention → cleanup).
  Within a recording nothing was being lost; see the table for what looked like loss.
- Retention keeps the configured duration within the storage limit and the free-space reserve,
  oldest footage first, never footage a save or a marker holds. The playback cache has its own
  budget and legacy imported footage no longer counts, so the whole limit is available to the
  history. When less than the configured duration is kept, Home, Settings and the Library say
  which limit is the cause.
- Timestamps: one clock per session; ordering by start time; a test covers a marker in a later
  internal recording landing at the right place.
- Segment rotation: the duplicate in-memory segment index (`ring.rs`) is gone. The catalog is the
  only index, so the two can no longer disagree about what exists.
- Crash recovery: verified by killing the worker mid-recording (`e2e-crash.mjs`): the footage is
  re-indexed, a 12 s clip with its marker is recovered, the footage stays in the history, and
  recovering again creates no duplicate.
- Recording is not interrupted by playback, browsing or saving: verified live (see 2.12).
- A gap on the timeline means "not recorded". A playback problem says so separately ("This
  moment could not be played… the recording itself is intact").

### 2.2 One timeline per game

- Library: one Background Recording card per game. Opening it shows the whole history: every
  stretch at once, in order, breaks between them, session dates above, saved clips (green),
  markers and screenshots at their times.
- Wheel zoom at the cursor, drag/shift-wheel pan, `0` for everything; the playhead, zoom and a
  selection keep their place while footage is added at the end or ages out at the start
  (`lib/gameline.ts`, tested).
- Long histories: publishing a 6-hour index (10,800 segments) takes ~10 ms and is cached until
  something changes (1.3 µs); a retention pass takes ~5.5 ms; the real 2-hour index builds in 4 ms.

### 2.3 Hover previews and navigation

- The engine decodes the frame straight from the recorded segment (`thumb.rs`): the keyframe at
  that second when zoomed out, the exact frame when zoomed in. Bounded in-memory cache (12 MB in
  the engine, 320 pictures in the UI); nothing is written to disk.
- Measured on the fixtures: keyframe ~9 ms, exact frame ~13 ms. On live 4K footage: worst case
  173 ms including the round trip.
- Accuracy: the test compares the thumbnail with the frame ffmpeg extracts independently at that
  position (difference 2.3 on a 0–255 scale; 14.0 against a frame 1.3 s away). The burned-in
  timecode of the test pattern was also read by eye: `10.467` for a thumbnail reported at 10.488
  minus the fixture's 0.021 s start offset.
- Fast movement: requests carry a viewer name and the engine keeps only the newest per viewer; the
  UI clears a picture as soon as the cursor is further from it than the precision in use.
- One clock: a hover time is clamped to the recording, the transport shows footage time and
  total footage, and the tooltip adds the time of day. The `1:11:04` vs `0:57:10` reading could
  not be reproduced; the code path that could produce a time outside the recording no longer
  exists, and a browser check asserts it.

### 2.4 Replay cache, seeking and playback

- The cache never refuses. It is a least-recently-used cache (512 MiB target, the six newest
  windows always kept, idle windows dropped after 10 minutes, everything cleaned at start).
- One worker builds windows, one makes thumbnails; a newer request from the same viewer replaces
  its pending ones, which are answered as superseded. Two viewers (app and overlay) do not
  cancel each other.
- A seek opens a short window (8 s / 32 MiB) that is ready fast; continuous playback uses long
  ones (30 s / 96 MiB). Windows are written in one pass (no second pass moving the index).
- Verified: 120 seeks fired in bursts from two viewers with prefetches in between → 0 failures,
  every viewer's newest request served; re-opening a place is served from the cache.
  Live at 4K: 24 seeks during recording, worst 417 ms.

### 2.5 Duplicates and short recordings

- Instant Replay now selects from the session, not from the newest internal recording. A clip is
  one continuous piece at one resolution, so it takes the longest piece inside the replay window
  (the most recent one when that is at least half as long) and the notice says why it is shorter:
  "15s saved: the recording was interrupted there (the game restarted or changed resolution)" or
  "6s saved: that is all that has been recorded so far".
- The same footage is never written twice: a second save of the same range answers "Already
  saved" and points at the existing file. This covers the key pressed repeatedly after a game
  closed, a repeated trim in the viewer, and "save all" recovery.
- Every path that creates a permanent file now goes through one function
  (`Catalog::save_clip`): Instant Replay, the viewer's Save Clip, full-session recording, and
  explicit recovery. The bulk "Save as clip" button in Settings (which would have turned the
  whole history into clips) is removed; the command remains for explicit use.
- Nothing was deleted. The eight identical 5-second clips are still in
  `G:\Videos\Captures\Hookshot\Clips` (`2026-11-11_02-44-17` … `02-45-38`); seven of them are
  exact copies of the first.

### 2.6 Library

- One grid, side by side: a Background Recording card per game (a real frame of its footage,
  total recorded time, "Recording" while live, clips saved from it), then saved clips,
  recordings and screenshots. Names, dates and durations are visible without hovering.
- Saved media: open, show in folder, **save a copy**, open externally, copy path, editor audio
  files, rename, move to the Recycle Bin.
- Background footage: **Delete footage…** removes that game's temporary footage after a
  confirmation that says saved clips and screenshots are kept. What is being recorded is never
  deleted from here.
- Internal recordings whose footage has all aged out are forgotten (rows, markers that point at
  nothing, empty folders), so the list does not fill with "expired" entries.
- Scroll position and focus are restored when coming back from the viewer (unchanged).

### 2.7 Playback and clip creation

- Play/pause, ±10 s (crossing breaks), previous/next marker when there are markers, volume slider
  (remembered) and mute, fullscreen, frame stepping, Clip.
- A click in a break lands on the nearest recorded moment instead of an error.
- Clip selects a range around the playhead; handles snap to real frames (start on a keyframe);
  the duration is shown; Save Clip acknowledges at once, the viewer stays usable, and completion
  is confirmed. A selection that includes an unrecorded moment is refused with that reason.
- Verified on real encoded footage: a clip across three internal segments contains exactly the
  original encoded frames in order (210 frames compared by hash); a frame-aligned selection of
  3.500 s saved as 3.500 s; live 4.4 s selection saved as 4.4 s.

### 2.8 In-game overlay

- The overlay uses the same viewer component and the same engine history, opens on the game
  being recorded, and has the same timeline, markers and Clip action. Its requests are separate
  from the main window's.
- Not re-verified inside a real exclusive-fullscreen game (see limits).

### 2.9 Interface polish

- Fewer controls where they were not needed (no session picker; marker buttons only when there
  are markers; the Library is one grid). Plain wording for every message touched.
- The viewer fits a 760×600 window (checked).

### 2.10 Regression protection

- Verified again on live capture: full-session recording (`e2e-session`), markers and their
  protection (`e2e-markers`), review and exact range save (`e2e-review`), crash recovery
  (`e2e-crash`). Recording at native resolution, SDR conversion on an HDR display, hotkey
  acknowledgement before any work, separate Instant Replay and history settings, and the
  free-space reserve are unchanged in code.
- Not re-run: the resolution-change, hotkey-feedback and low-disk scripts (they need the test
  game or physical keys).

### 2.11 Architecture

- Removed: the in-memory segment ring and its pin/protect bookkeeping (a second source of truth
  whose retention was never called), the legacy review-snapshot path, per-seek threads.
- Removed disk work: the engine walked every recording folder on every state publish to size
  "leftover" footage (3,700 files on this PC, several times a second); it now reads the cached
  catalog state. Retention used to load every segment row on every pass; it now reads one
  aggregate and only pages through rows when something must go.
- One place translates between session time and media time; one function writes clips.

### 2.12 Verification results

Automated, all passing:

| Check | Command | Result |
|---|---|---|
| Rust unit tests | `cargo test --workspace` | 82 passed (57 engine, 21 app, 3 protocol, 1 hook) |
| History on real encoded fixtures | `node scripts/e2e-overhaul.mjs` | 6 groups: timeline, thumbnails, seeking, clipping, library correctness, restart and delete |
| Live display capture, 4K NVENC | `node scripts/e2e-live-overhaul.mjs` | 6 groups, 0 dropped frames while reviewing |
| UI in a real browser | `node scripts/ui-overhaul-check.mjs` | 45 checks |
| Timeline model | `node scripts/test-timeline.mjs` | passed |
| Existing live scripts | `e2e-session`, `e2e-markers`, `e2e-review`, `e2e-crash` | passed |
| Migration | a copy of the real index, opened and migrated | 60 recordings → 6 sessions, 119.7 min, built in 4 ms |

The backlog's eight workflows:

1. **Two-hour recording** — not run as a live two-hour capture. Covered by: the real index
   holding 119.7 min at the 50 GB limit, retention tests, and the 6-hour synthetic index.
2. **Timeline overview** — verified (fixtures and browser).
3. **Hover previews** — verified against independently extracted frames.
4. **Seeking** — verified (burst test, live test, browser scrubbing).
5. **Library correctness** — causes found and corrected; verified.
6. **Clipping** — verified, including across segment boundaries.
7. **Overlay** — recording continues while reviewing and saving: verified at engine level and
   with the overlay's panel in a browser; not inside a real fullscreen game.
8. **Regression checks** — see 2.10.

## Remaining limits

- No live two-hour soak and no test inside a real game (the live test records the desktop).
- The overlay was not exercised in exclusive fullscreen or through the in-game hook.
- A clip is still one continuous piece at one resolution; a selection cannot span a break.
  Clip starts are on a keyframe (the viewer aligns the handle to one before saving).
- Playback is still a sequence of short stream-copied windows, not a gapless decoder; there can
  be a brief pause when playback crosses into the next window that was not prefetched in time.
- The index is now schema 4. 0.6.5 and older refuse to open it ("created by a newer version")
  and then cannot record; going back needs `history.sqlite` removed (it is rebuilt from the
  footage) or a copy of the old one.
- The save key on this PC is a bare `Num1`. It is caught everywhere while CAPTURE runs, whether
  or not a game is being recorded. Duplicate saves are now impossible, but the key still does not
  reach other programs; a modifier (or releasing bare keys when nothing is recorded) would fix that.
- Controller navigation was not addressed. True HDR remains postponed, as in the backlog.

## Files

Engine: `history.rs` (rewritten), `review.rs` and `thumb.rs` (new), `pipeline.rs`, `lib.rs`,
`save.rs`, `storage.rs`; `ring.rs` removed. Protocol: viewer-aware review, thumbnails, session
parts, save notes, delete. App: `export_clip` command, dialog save permission. UI:
`HistoryPanel.svelte`, `ReplayTimeline.svelte`, `LibraryTab.svelte`, `SessionThumb.svelte`,
`lib/gameline.ts`, `lib/thumbs.ts`, store and types, Home, Settings, Overlay. Scripts:
`e2e-overhaul.mjs`, `e2e-live-overhaul.mjs`, `ui-overhaul-check.mjs`, `overhaul-fixture.ts.txt`
(new); `e2e-history.mjs`, `ui-history-fixture.mjs`, `ui-replay-acceptance.mjs`,
`replay-fixture.ts.txt` (replaced by the above).
