# CAPTURE

Windows-first, local-first game recording and instant replay. Press a hotkey and get a normal MP4 in
`Captures\<Game>\Clips\`, with a playable Preview Mix plus separate Game / Microphone / Voice Chat tracks
for editing.

**Status:** v0.7.3 development source, including saved-clip trimming (unsigned, tested on one PC). Instant replay,
overlay with review timeline and markers, sessions, screenshots, separate game/mic/voice-chat tracks, editor audio
files, library, portable folder and per-user installer. See `docs/STATUS.md` for every requirement and its evidence
and `docs/AUDIT_0.5.1.md` for the release audit.
Build with `scripts\package-portable.ps1` (payload + zip) and `scripts\build-installer.ps1` (setup.exe). User guide: `docs/QUICKSTART.md`.

| Doc | Purpose |
|---|---|
| `docs/spec/CAPTURE_Design.md` | Product + technical specification (acceptance contract) |
| `docs/spec/CAPTURE_Agent_Prompt.md` | Build brief |
| `docs/PHASE0_CAPABILITY_REPORT.md` | What was measured on this machine |
| `docs/ADR-0001-capture-architecture.md` | Native WGC + WASAPI + LGPL libav, chosen over libobs |
| `docs/DISTRIBUTION.md` | Portable/installer architecture, data locations, updates, uninstall, compatibility |
| `docs/CAPTURE_OVERHAUL_REPORT.md` | **0.7.0 recording/replay/library overhaul: root causes, changes, verification, limits** |
| `docs/STATUS.md` | **Current status: every requirement, tested evidence, gaps** |
| `docs/spec/CAPTURE_Overlay_Addendum.md` | Overlay & replay spec |
| `docs/OVERLAY_REPORT.md` | Overlay milestone report |
| `docs/PHASE1_REPORT.md` | What works now, evidence, limitations, playtest steps |
| `docs/QUICKSTART.md` | User quick start (also shipped in the build) |
| `docs/DISTRIBUTION.md` | Portable folder, installer, updates, uninstall, what is written where |
| `docs/AUDIT_0.5.1.md` | Release-candidate audit: defects found and fixed, tests, limits |
| `docs/FRAMECUT_IMPORT_CONTRACT.md` | Draft track/stem contract for FrameCut (not implemented there) |

## Dev setup

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-deps.ps1   # pinned FFmpeg LGPL + libclang into third_party\
# Install Rust (MSVC), Visual Studio C++ build tools, and Node.js before building.
# Set RUSTUP_HOME/CARGO_HOME only if your Rust installation uses custom locations.
cd app
npm ci
npm run build
cd ..
cargo test --workspace                                              # unit tests
powershell -ExecutionPolicy Bypass -File scripts\package-portable.ps1   # -> dist\CAPTURE-<version>\ + portable zip
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1    # -> dist\CAPTURE-<version>-setup.exe
node scripts\e2e-distribution.mjs <version> D:\some\work\dir          # portable + installer end to end
```

| Path | What |
|---|---|
| `crates/capture-protocol` | Versioned UI <-> worker messages |
| `crates/capture-engine` | Capture, encode, replay ring, save; `capture-worker` binary |
| `app/` | Tauri 2 + Svelte 5 UI (`CAPTURE.exe`) |
| `scripts/run-acceptance.ps1` | Real-app acceptance suites (auto detection, overlay) |
| `scripts/e2e-*.mjs` | Worker-level tests: replay, markers, review, session, audio, stems, crash, low disk, window closed, resize, colour; `e2e-lifecycle` / `e2e-distribution` drive the packaged app |
| `scripts/validate-media.mjs` | Independent check of recordings with ffprobe/ffmpeg (full decode, A/V offsets) |
| `scripts/soak.mjs` | Long-run soak (memory, CPU, buffer, sync) |

Spikes live in `spikes/`. Their media output goes to `spikes/out/` (git-ignored, because it contains real screen content).
