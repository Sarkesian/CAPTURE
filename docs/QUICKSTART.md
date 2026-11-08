# CAPTURE quick start (v0.2)

1. **Start** `CAPTURE.exe`. No installer is needed. Its own settings live in the `data` folder next to it.
2. **Capture tab → What to record** is set to **Auto (games)**. Start a game and CAPTURE finds it by itself
   (Steam, Epic, GOG, itch, Xbox, EA, Ubisoft, Riot, Battle.net, any `Games` folder, or anything fullscreen).
   If it misses one, choose it under **Missed a game?**. If it records something that isn't a game, press **Not a game**.
   You can also use **Pick an app** or **Whole display**.
3. Turn on **Instant replay** and choose **Save last** (default 2 minutes).
4. Wait until the status says **Buffering**, then play.
5. A small notice in the corner says **Recording <game>**. It never appears in your clips.
6. Press **Alt+F10** (or click **Save clip**). Then:
   - a small notice says **Clip saved**, but only after the file has been written *and* checked;
   - the clip is in `D:\Captures\Clips\<Game Name>\YYYY-MM-DD_HH-MM-SS_<Game>_Clip.mp4`;
   - **Open file**, **Show in folder** and **Copy path** take you straight to it.

## In-game overlay (Alt+F11)

Press **Alt+F11** while playing (Esc closes it). Recording keeps running the whole time.
- **Save Replay**: same as Alt+F10.
- **Add Marker**: bookmark this moment (also **Alt+F8**, with a short sound that is never recorded).
- **Review Recording**: watch your recent gameplay, click a marker, press **Create clip**, drag the two handles, **Save clip**.
  **Save around** on a marker saves the moments around it in one click.
- **Start Recording**: a full session of any length, saved to `Sessions\<Game>\` when you stop.
- Recent clips with thumbnails, quick settings, and a shortcut to the full settings.

Over **exclusive fullscreen** games Windows can't show the overlay without leaving the game. Use borderless, or just use
the hotkeys, which always work.

## What is in a clip

A normal MP4 (H.264 video, AAC audio) that plays anywhere and drops into any editor. It has:
- **Preview Mix**: the default track, which sounds normal when you double-click the file;
- **Game Audio**: only the game's own sound, isolated per app. With *Whole display* this track is
  **System Audio**, meaning everything you hear.

Editors that show every audio track: mute **Preview Mix**, or you will hear the game twice.

## Good to know

- Any hotkey can be changed in Settings, including numpad keys (`Num1`, `NumAdd`…). Numpad keys work with NumLock
  on or off and never take over the normal End/Home/arrow keys. Shift can't be combined with numpad digits
  (Windows turns those into arrow keys).

- If less than the chosen length has been buffered, the clip is shorter. The notification shows the real length.
- A clip can start up to 2 s earlier than requested, because clips start on a keyframe.
- Temporary footage lives in `D:\Captures\Buffer` under a size cap (Settings). It is removed when CAPTURE closes.
  **Saved clips are never deleted automatically.**
- Closing the window quits CAPTURE and stops recording, unless you turn on *Keep running in the tray* in Settings.
- Exclusive-fullscreen games may show no frames. Switch the game to borderless/windowed fullscreen or use *Whole display*.
  CAPTURE never injects into games.

## Not in this build yet

Microphone and voice-chat tracks, the WAV "Editor audio files" export, screenshots, per-game settings, and an installer.
