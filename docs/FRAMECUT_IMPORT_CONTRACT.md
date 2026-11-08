# CAPTURE → FrameCut import contract (draft v0)

**Status:** proposal only. FrameCut has **not** been changed. Nothing here is implemented in FrameCut.
CAPTURE will honour this contract from Phase 2.

## What CAPTURE writes

`<root>\Clips\<Game>\YYYY-MM-DD_HH-MM-SS_<Game>_Clip.mp4`

| Order | Type | `handler_name` | Default | Present when |
|---|---|---|---|---|
| v:0 | H.264, CFR, BT.709 limited range | `Video` | yes | always |
| a:0 | AAC 48 kHz stereo | `Preview Mix` | **yes** | always (sum of the enabled sources) |
| a:1 | AAC 48 kHz stereo | `Game Audio` | no | game audio enabled |
| a:2 | AAC 48 kHz stereo | `Microphone` | no | mic enabled |
| a:3 | AAC 48 kHz stereo | `Voice Chat` | no | voice-chat app enabled |
| a:4 | AAC 48 kHz stereo | `Other Desktop` | no | explicitly enabled |

- Absent sources are **omitted**, not written as silent tracks. Consumers must identify tracks by `handler_name`, never by index alone.
- All streams share timeline zero, so no per-track offset is needed.
- Optional sidecar `<clip>.capture.json`, versioned:
  ```json
  {"capture_contract":1,"tracks":[{"index":1,"role":"preview_mix","contains":["game","mic","chat"]},
   {"index":2,"role":"game","source":"process-loopback","app":"FAULTHEART.exe"}, ...]}
  ```
- `Editor audio files` creates `<clip>_Audio\Game.wav | Microphone.wav | VoiceChat.wav | OtherDesktop.wav`
  (48 kHz, 24-bit PCM, same length and start as the clip) plus `manifest.json`. The video is not re-encoded.

## What FrameCut needs to do (when authorised)

1. Today, `desktop/tools.cjs:46` takes the **first** audio stream. For CAPTURE clips this is Preview Mix, which is
   safe (no doubled audio) but loses isolation.
2. Proposed: if more than one audio stream exists and any `handler_name` is in {Game Audio, Microphone, Voice Chat, Other Desktop},
   import each one as its own audio lane, aligned at t=0, and **mute or exclude `Preview Mix`** from playback and export.
   Keep Preview Mix available as a single "Combined" lane that is off by default.
3. Clips without these labels use the existing behaviour.
