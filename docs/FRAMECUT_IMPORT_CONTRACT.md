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
| next | AAC 48 kHz stereo | `Other Desktop` | no | explicitly enabled. **Never part of Preview Mix.** |

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

### Editor audio files (on request)
`<clip stem>_Audio\` contains one 48 kHz / 24-bit stereo WAV per isolated track (`GameAudio.wav`,
`SystemAudio.wav`, `Microphone.wav`, `VoiceChat.wav`, `OtherDesktop.wav`), plus `manifest.json` and
`README.txt`. Every WAV starts at the clip's time 0 and lasts the clip's full duration. Preview Mix
is deliberately not exported.

## FrameCut side (proposal, when you authorise it)
1. Today `desktop/tools.cjs:46` imports only the first audio stream. For CAPTURE clips that is Preview
   Mix: safe (nothing plays twice) but not separable.
2. Proposed: if any audio stream's `handler_name` is one of Game Audio / System Audio / Microphone /
   Voice Chat / Other Desktop, import each as its own lane at t = `start_time`, and **mute or exclude
   `Preview Mix`** (keep it as an optional "Combined" lane, off by default). Leave `Other Desktop` muted by
   default too, because it overlaps the other lanes.
3. If `<clip>.mp4.markers.json` exists, show its markers on the timeline.
4. Clips without these labels keep the existing behaviour.
