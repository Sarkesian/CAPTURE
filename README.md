# CAPTURE

Windows-first, local-first game recording and instant replay. Press a hotkey and get a normal MP4 in
`Captures\Clips\<Game>\`, with a playable Preview Mix plus separate Game / Microphone / Voice Chat tracks
for editing.

**Status:** Phase 0 complete (capability audit + architecture spikes). **There is no app yet.**

| Doc | Purpose |
|---|---|
| `docs/spec/CAPTURE_Design.md` | Product + technical specification (acceptance contract) |
| `docs/spec/CAPTURE_Agent_Prompt.md` | Build brief |
| `docs/PHASE0_CAPABILITY_REPORT.md` | What was measured on this machine |
| `docs/ADR-0001-capture-architecture.md` | Native WGC + WASAPI + LGPL libav, chosen over libobs |
| `docs/FRAMECUT_IMPORT_CONTRACT.md` | Draft track/stem contract for FrameCut (not implemented there) |

## Dev setup

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-deps.ps1   # pinned FFmpeg LGPL + libclang into third_party\
$env:RUSTUP_HOME='D:\Programs\Rust\rustup'; $env:CARGO_HOME='D:\Programs\Rust\cargo'
```

Spikes live in `spikes/`. Their media output goes to `spikes/out/` (git-ignored, because it contains real screen content).
