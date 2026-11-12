# CAPTURE quick start

## Indexed background history

- **Instant Replay length** controls the hotkey clip only. **Background history** targets four hours of recorded gameplay across sessions, selectable up to six hours, within your existing storage budget. Updating does not increase that budget.
- Each game has one **Background Recording** in the Library. Opening it shows everything recorded for that game on one timeline: every stretch end to end, with a break wherever nothing was recorded. Hover to see the frame at that moment, click to go there, scroll to zoom, press **Clip**, drag the two handles and **Save Clip**. That creates a normal permanent MP4; there is no separate export.
- **Save Replay** saves the last minutes of the session even if the game changed resolution or restarted a moment ago. When less than the configured length exists, the notice says so. Saving the same footage twice does not create a second file.
- Capturing a clip immediately says **Saving Clip** and plays the existing feedback. Completion updates Library silently. A failure is shown; protected source footage can be retried from the recording's timeline.
- Original encoded video is shared by history and immediate clips. Saving does not re-encode it. Previews use short byte-bounded playback windows rather than whole-history copies.
- Balanced, High and Maximum are lossy H.264 settings. **Archival 4:4:4** uses NVIDIA lossless coding after RGB conversion, requires compatible hardware/players, and can use much more storage. It is not bit-exact RGB or an end-to-end zero-loss mode. Audio remains AAC.
- Native resolution changes start another recording epoch instead of scaling the new gameplay into an older canvas. Manual full recordings continue into another file across that boundary. FPS and encoder quality are never automatically reduced. Odd window sizes lose their last pixel row or column (4:2:0 H.264 needs even dimensions).
- The implemented capture path is 8-bit SDR. **With Windows HDR on, recording still works: Windows converts the picture to SDR before CAPTURE receives it, so recordings are SDR and HDR highlights are not preserved** (the session's `session.json` records `source_display_hdr`). Native HDR/10-bit recording is separate, unfinished work. Complete game-present accounting is not established. Diagnostics distinguish delivered source frames, emitted video packets, repeated samples, unselected source arrivals, missed clock ticks and ticks without a source picture.
- Imported old media is preserved during migration, even if it exceeds the budget. Active/failed saves and marker pins can also limit cleanup. Permanent clips and PNGs remain independent of history retention.

1. **Start** `CAPTURE.exe` (portable: extract the whole zip first; installed: Start menu). CAPTURE keeps its
   settings, cache and logs in the `Data` folder next to it, and asks four first-run questions.
   (Windows may warn that the app is unrecognised because it isn't code-signed. Choose *More info → Run anyway*.)
2. On **Home**, switch **Recording** on. **What to record** is set to **Auto (games)**: start a game and CAPTURE
   finds it (Steam, Epic, GOG, itch, Xbox, EA, Ubisoft, Riot, Battle.net, any `Games` folder, or anything
   fullscreen). If it misses one, use **Missed a game?**. If it records something that isn't a game, press
   **Not a game**. You can also use *An app* or *Whole display*.
3. A small corner notice says **Recording <game>**. It never appears in your recordings.
4. Home now shows the game, how much replay is ready, live audio meters and **Save replay**. The sidebar
   has **Home**, **Library** (background recordings, clips and screenshots; watch and clip recent gameplay
   without stopping) and **Settings**.

## While playing (default keys; change any of them in Settings, numpad keys work too)
| Key | Does |
|---|---|
| **Alt+F10** | Save the last 2 minutes (or your length) as a clip |
| **Alt+F8** | Add a marker: bookmarks the moment, with a short sound that is never recorded |
| **Alt+F11** | Open the overlay (Esc closes it) |
| **Alt+F12** | Screenshot (PNG) |
| *(optional)* | Start/stop a full-length session recording |

## The overlay (Alt+F11)
Save Replay · Add Marker · **Review Recording** (watch recent gameplay, click a marker, **Create clip**,
drag the two handles, **Save clip**; or **Save around** a marker in one click) · Start/Stop Recording ·
audio rows with live meters and mic mute · recent clips · quick settings · full settings.
Recording never stops while the overlay is open. The overlay is at its best over a game in
*borderless window*: set that in the game's display options where it has it. Over *exclusive fullscreen*
CAPTURE draws the overlay inside the game instead (Direct3D 9 and 11 games without anti-cheat), which is
less sharp; where it can't, a notice and a short falling sound say so, and the keys always work.

## Audio (Home → Audio; levels and devices in Settings → Audio)
- **Game sound**: only the game's own sound (*Whole display*: everything you hear except CAPTURE).
- **Your microphone**: **off until you turn it on.** Pick the device, watch the meter, **Test**
  (records 3 s and plays it back) and **Mute**. If your chosen mic is unplugged, CAPTURE says so and never
  switches to another one. If Windows has the mic muted or at 0%, or the mic sends pure silence (headset
  off, mute switch on), CAPTURE says that under the device list. CAPTURE never changes Windows settings.
  **Mic check:** put the headset on and switch it on, turn on *Your microphone*, press **Test** and speak
  for 3 seconds. You should hear yourself played back and see "Peaked at … dB".
- **Voice chat**: pick the app (e.g. Discord). It's recorded from that app only. Voices in a game's
  *built-in* chat are part of the game sound and can't be separated.
- **All computer audio (extra track)**: optional track of everything you hear. It also contains the game and voice chat (Windows can only leave out one app), so it is never added to Preview Mix. In an editor, keep it muted unless you need it.
- Sliders change the Preview Mix only. Every source is also saved untouched as its own track.
- Meters show the real level of each source (bar = loudness, lighter bar = peaks, tick = recent peak,
  red end mark = clipping). A flat bar is labelled *Off*, *Muted*, *Waiting for a game* or in amber
  when a device is missing; meters never move without signal.

## Your files
```
G:\Videos\Captures\<Game>\Clips\…_Clip.mp4         saved replays (never deleted automatically)
G:\Videos\Captures\<Game>\Sessions\…_Session.mp4   full-length recordings
G:\Videos\Captures\<Game>\Screenshots\….png
G:\Videos\Captures\Background Recording\            indexed rolling history across games and restarts
```
A clip is a normal MP4: **Preview Mix** (default, sounds normal anywhere) plus Game Audio / Microphone /
Voice Chat tracks for editing. In an editor that shows all tracks, mute *Preview Mix*. For editors that
can't use MP4 audio tracks: Library → **Editor audio files** writes synced WAVs to `<clip>_Audio\`, or turn on
Settings → Audio → **Create separate audio files for editors** to get them after every save (off by default:
each WAV track takes about 12× the space of the same track inside the MP4).

## Library
Two sections. **Background recordings**: one card per game (temporary: the most recent gameplay, kept
automatically up to your history length and storage limit). **Saved media**: your clips, recordings and
screenshots, with search, a game filter and a type filter. Thumbnails are shown clean; hover a card for
its name, date and type. Click a card to open it. The **…** menu on
saved media: Show in folder, Save a copy, Open externally, Copy path, Editor audio files, Rename, and
Delete (to the Recycle Bin, after confirmation). On a background recording: **Delete footage**, which
removes only that game's temporary footage; clips and screenshots you saved are kept.

## Good to know
- If CAPTURE or Windows stops unexpectedly, the next start offers to **save the last footage as a clip**.
- Settings → Games: per-game replay length, quality and sound.
- Settings → Storage: temporary-buffer limit and free-space reserve. When space runs low, buffering
  pauses with a message instead of filling the drive. Saved files are never touched.
- *Start with Windows* starts CAPTURE in the tray. Closing the window keeps CAPTURE recording in the tray
  (turn off *Keep running in the tray* to make close quit). *Minimize to the tray* is optional. **Quit
  CAPTURE** in the tray menu really exits.
- Updating: Settings › About & updates › **Choose file…** and pick the new release zip. Every file is checked,
  settings are backed up, CAPTURE restarts. (Installed copies can also run the new setup. A new portable folder
  next to an old `CAPTURE-*` folder still copies its settings on first start.)
- Removing: portable, quit from the tray and delete the folder (turn off Start with Windows first). Installed,
  use Installed apps; recordings are kept unless you choose *Remove CAPTURE and all its data*.
- If the capture drive is unplugged, CAPTURE says so and carries on by itself when it is back.
