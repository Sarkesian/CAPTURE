# CAPTURE

Windows-first, local-first game recording and instant replay. Press a hotkey and get a normal MP4 in
`Captures\Clips\<Game>\`, with a playable Preview Mix plus separate Game / Microphone / Voice Chat tracks
for editing.

**Status:** Phase 1 playtest build. A portable app records instant replays with a Preview Mix plus an isolated Game Audio track.
Run `dist\CAPTURE-0.1.0\CAPTURE.exe` (build it with `scripts\package-portable.ps1`). See `docs/QUICKSTART.md`.

| Doc | Purpose |
|---|---|
| `docs/spec/CAPTURE_Design.md` | Product + technical specification (acceptance contract) |
| `docs/spec/CAPTURE_Agent_Prompt.md` | Build brief |
| `docs/PHASE0_CAPABILITY_REPORT.md` | What was measured on this machine |
| `docs/ADR-0001-capture-architecture.md` | Native WGC + WASAPI + LGPL libav, chosen over libobs |
| `docs/PHASE1_REPORT.md` | What works now, evidence, limitations, playtest steps |
| `docs/QUICKSTART.md` | User quick start (also shipped in the build) |
| `docs/FRAMECUT_IMPORT_CONTRACT.md` | Draft track/stem contract for FrameCut (not implemented there) |

## Dev setup

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-deps.ps1   # pinned FFmpeg LGPL + libclang into third_party\
$env:RUSTUP_HOME='D:\Programs\Rust\rustup'; $env:CARGO_HOME='D:\Programs\Rust\cargo'
cargo test --workspace                                              # unit tests
powershell -ExecutionPolicy Bypass -File scripts\package-portable.ps1   # -> dist\CAPTURE-<version>\
```

| Path | What |
|---|---|
| `crates/capture-protocol` | Versioned UI <-> worker messages |
| `crates/capture-engine` | Capture, encode, replay ring, save; `capture-worker` binary |
| `app/` | Tauri 2 + Svelte 5 UI (`CAPTURE.exe`) |
| `scripts/e2e-worker.mjs`, `scripts/ui-drive.mjs` | Headless worker scenario and real-UI acceptance drive |

Spikes live in `spikes/`. Their media output goes to `spikes/out/` (git-ignored, because it contains real screen content).
