# CAPTURE → FrameCut import contract (v1)

**Status:** CAPTURE writes everything below today (v0.3). FrameCut has **not** been changed; the
"FrameCut side" section is a proposal only.

## What CAPTURE writes

`<root>\Clips\<Game>\YYYY-MM-DD_HH-MM-SS_<Game>_Clip.mp4` (also `_Session.mp4` in `Sessions\`,
`_Recovered.mp4` after crash recovery).

| Order | Type | `handler_name` | Default | Present when |
|---|---|---|---|---|
| v:0 | H.264, constant frame rate, BT.601 matrix tag (NVENC RGB→YUV) | `Video` | yes | always |
| a:0 | AAC 48 kHz stereo | `Preview Mix` | **yes** | when any audio source is on (sum of the mixed sources) |
| next | AAC 48 kHz stereo | `Game Audio` (`System Audio` for whole-display capture) | no | game sound on |
| next | AAC 48 kHz stereo | `Microphone` | no | microphone on (silent while muted) |
| next | AAC 48 kHz stereo | `Voice Chat` | no | voice-chat app chosen |
| next | AAC 48 kHz stereo | `All Desktop Audio` | no | explicitly enabled. Everything you hear (it **also contains the game and voice chat**: Windows cannot exclude more than one app from a capture). **Never part of Preview Mix.** Named `Other Desktop` before 0.3.1. |

- Absent sources are **omitted**, so consumers must identify tracks by `handler_name`, never by index.
- Isolated tracks are unprocessed; per-source volume sliders affect Preview Mix only.
- All audio streams share one start offset (typically 5–21 ms, one AAC frame) relative to video;
  honour `start_time`. Measured A/V offset end to end: about 7 ms, with no drift over the soak test (see STATUS).

### Markers sidecar (only when the clip contains markers)
`<clip>.mp4.markers.json`
```json
{ "capture_markers": 1, "clip": "<file name>", "markers": [ { "t": 12.4, "label": "", "at": "16:13:49" } ] }
```
`t` = seconds from the clip's start.

### Editor audio files (on request, or after every save when "Create separate audio files for editors" is on)
`<clip stem>_Audio\` contains one 48 kHz / 24-bit stereo WAV per isolated track (`GameAudio.wav`,
`SystemAudio.wav`, `Microphone.wav`, `VoiceChat.wav`, `AllDesktopAudio.wav`), plus `manifest.json` and
`README.txt`. Every WAV starts at the clip's time 0 and lasts the clip's full duration. Preview Mix
is deliberately not exported.

## Works today, without changing FrameCut
Re-checked 2026-11-08 against the installed `D:\Programs\FrameCut Studio` (read-only, `app.asar`):
`probeMedia()` in `desktop/tools.cjs` uses `streams.find(codec_type === 'audio')`, i.e. the first audio
stream (Preview Mix), and accepts audio-only files as `kind: 'audio'`. So for separate lanes now:
turn on **Settings › Storage › Create separate audio files for editors** (or "Editor audio files" on a
clip in the Library), import the clip and the WAVs from `<clip>_Audio\`, place every WAV at 0 and mute
the clip's own audio. The WAVs are sample-aligned with the video (`e2e-isolation`, `e2e-stems`).

## FrameCut side (proposal, when you authorise it)
1. Today `probeMedia()` (`desktop/tools.cjs`) imports only the first audio stream. For CAPTURE clips that
   is Preview Mix: safe (nothing plays twice) but not separable.
   Smallest change: have `probeMedia` also return `audioStreams: [{ index, handler_name }]`; on import of a
   media item whose streams carry the labels below, add one audio lane per labelled stream that reads it
   with `-map 0:a:<n>`. The lane/compose code (`compose.cjs`, timeline) must accept a stream index per
   audio clip; that part is the real work and was not attempted from CAPTURE.
2. Proposed: if any audio stream's `handler_name` is one of Game Audio / System Audio / Microphone /
   Voice Chat / All Desktop Audio, import each as its own lane at t = `start_time`, and **mute or exclude
   `Preview Mix`** (keep it as an optional "Combined" lane, off by default). Leave `All Desktop Audio` muted by
   default too, because it overlaps the other lanes.
3. If `<clip>.mp4.markers.json` exists, show its markers on the timeline.
4. Clips without these labels keep the existing behaviour.
