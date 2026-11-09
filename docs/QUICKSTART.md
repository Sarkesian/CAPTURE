# CAPTURE quick start (v0.4.1)

1. **Start** `CAPTURE.exe`. No installer is needed; its own settings live in the `data` folder next to it.
   (Windows may warn that the app is unrecognised because it isn't code-signed. Choose *More info → Run anyway*.)
2. On **Home**, switch **Recording** on. **What to record** is set to **Auto (games)**: start a game and CAPTURE
   finds it (Steam, Epic, GOG, itch, Xbox, EA, Ubisoft, Riot, Battle.net, any `Games` folder, or anything
   fullscreen). If it misses one, use **Missed a game?**. If it records something that isn't a game, press
   **Not a game**. You can also use *An app* or *Whole display*.
3. A small corner notice says **Recording <game>**. It never appears in your recordings.
4. Home now shows the game, how much replay is ready, live audio meters and **Save replay**. The sidebar
   has **Home**, **Replay** (watch and clip recent gameplay without stopping), **Library** and **Settings**.

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
G:\Videos\Captures\Buffer\                          temporary footage (size-capped, removed on exit)
```
A clip is a normal MP4: **Preview Mix** (default, sounds normal anywhere) plus Game Audio / Microphone /
Voice Chat tracks for editing. In an editor that shows all tracks, mute *Preview Mix*. For editors that
can't use MP4 audio tracks: Library → **Editor audio files** writes synced WAVs to `<clip>_Audio\`, or turn on
Settings → Audio → **Create separate audio files for editors** to get them after every save (off by default:
each WAV track takes about 12× the space of the same track inside the MP4).

## Library
Clips · Sessions · Screenshots with search, a game filter and sorting (newest, oldest, largest, name).
Click a thumbnail to play. Hover for **Show in folder** and the **…** menu: Copy path, Editor audio
files, Rename, and Delete (to the Recycle Bin, after confirmation).

## Good to know
- If CAPTURE or Windows stops unexpectedly, the next start offers to **save the last footage as a clip**.
- Settings → Games: per-game replay length, quality and sound.
- Settings → Storage: temporary-buffer limit and free-space reserve. When space runs low, buffering
  pauses with a message instead of filling the drive. Saved files are never touched.
- *Start with Windows* starts CAPTURE in the tray. Closing the window keeps CAPTURE recording in the tray
  (turn off *Keep running in the tray* to make close quit). *Minimize to the tray* is optional. **Quit
  CAPTURE** in the tray menu really exits.
- Updating: unzip the new version next to the old one (e.g. `CAPTURE-0.3.1` beside `CAPTURE-0.3.0`). On first
  start it copies your settings from the old folder; the old folder is left as it was.
- If the capture drive is unplugged, CAPTURE says so and carries on by itself when it is back.
