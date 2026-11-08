# CAPTURE quick start (v0.1, Phase 1 playtest)

1. **Start** `CAPTURE.exe`. No installer is needed. Its own settings live in the `data` folder next to it.
2. **Capture tab → What to record**: pick your game from *Game or app* (start the game first and press
   **Refresh**), or choose *Whole display*. CAPTURE remembers the game, so next time it begins buffering
   by itself whenever that game is open.
3. Turn on **Instant replay** and choose **Save last** (default 2 minutes).
4. Wait until the status says **Buffering**, then play.
5. Press **Alt+F10** (or click **Save clip**). Then:
   - a notification says **Clip saved**, but only after the file has been written *and* checked;
   - the clip is in `D:\Captures\Clips\<Game Name>\YYYY-MM-DD_HH-MM-SS_<Game>_Clip.mp4`;
   - **Open file**, **Show in folder** and **Copy path** take you straight to it.

## What is in a clip

A normal MP4 (H.264 video, AAC audio) that plays anywhere and drops into any editor. It has:
- **Preview Mix**: the default track, which sounds normal when you double-click the file;
- **Game Audio**: only the game's own sound, isolated per app. With *Whole display* this track is
  **System Audio**, meaning everything you hear.

Editors that show every audio track: mute **Preview Mix**, or you will hear the game twice.

## Good to know

- If less than the chosen length has been buffered, the clip is shorter. The notification shows the real length.
- A clip can start up to 2 s earlier than requested, because clips start on a keyframe.
- Temporary footage lives in `D:\Captures\Buffer` under a size cap (Settings). It is removed when CAPTURE closes.
  **Saved clips are never deleted automatically.**
- Closing the window quits CAPTURE and stops recording, unless you turn on *Keep running in the tray* in Settings.
- Exclusive-fullscreen games may show no frames. Switch the game to borderless/windowed fullscreen or use *Whole display*.
  CAPTURE never injects into games.

## Not in this build yet

Microphone and voice-chat tracks, the WAV "Editor audio files" export, automatic game detection,
thumbnails, and an installer.
