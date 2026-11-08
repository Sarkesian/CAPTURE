# CAPTURE quick start (v0.3)

1. **Start** `CAPTURE.exe`. No installer is needed; its own settings live in the `data` folder next to it.
   (Windows may warn that the app is unrecognised because it isn't code-signed. Choose *More info → Run anyway*.)
2. Turn on **Instant replay**. **What to record** is set to **Auto (games)**: start a game and CAPTURE
   finds it (Steam, Epic, GOG, itch, Xbox, EA, Ubisoft, Riot, Battle.net, any `Games` folder, or anything
   fullscreen). If it misses one, use **Missed a game?**. If it records something that isn't a game, press
   **Not a game**. You can also use *Pick an app* or *Whole display*.
3. A small corner notice says **Recording <game>**. It never appears in your recordings.

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
Recording never stops while the overlay is open. Over *exclusive fullscreen* games Windows can't show
the overlay without leaving the game. Use borderless, or just the keys, which always work.

## Audio (Capture tab → Audio)
- **Game sound**: only the game's own sound (*Whole display*: everything you hear except CAPTURE).
- **Your microphone**: **off until you turn it on.** Pick the device, watch the meter, **Test**
  (records 3 s and plays it back) and **Mute**. If your chosen mic is unplugged, CAPTURE says so and never
  switches to another one.
- **Voice chat**: pick the app (e.g. Discord). It's recorded from that app only. Voices in a game's
  *built-in* chat are part of the game sound and can't be separated.
- **Other computer audio**: optional extra track of everything; it is not added to Preview Mix.
- Sliders change the Preview Mix only. Every source is also saved untouched as its own track.

## Your files
```
D:\Captures\Clips\<Game>\…_Clip.mp4         saved replays (never deleted automatically)
D:\Captures\Sessions\<Game>\…_Session.mp4   full-length recordings
D:\Captures\Screenshots\<Game>\….png
D:\Captures\Buffer\                          temporary footage (size-capped, removed on exit)
```
A clip is a normal MP4: **Preview Mix** (default, sounds normal anywhere) plus Game Audio / Microphone /
Voice Chat tracks for editing. In an editor that shows all tracks, mute *Preview Mix*. For editors that
can't use MP4 audio tracks: Library → **Editor audio files** writes synced WAVs to `<clip>_Audio\`.

## Library
Clips · Sessions · Screenshots, grouped by game, with search, Play, Show in folder, Copy path, Editor
audio files, Rename, and Delete (to the Recycle Bin, after confirmation).

## Good to know
- If CAPTURE or Windows stops unexpectedly, the next start offers to **save the last footage as a clip**.
- Settings → Games: per-game replay length, quality and sound.
- Settings → Storage: temporary-buffer limit and free-space reserve. When space runs low, buffering
  pauses with a message instead of filling the drive. Saved files are never touched.
- *Start with Windows* starts CAPTURE in the tray. *Keep running in the tray* keeps it recording when the
  window is closed.
