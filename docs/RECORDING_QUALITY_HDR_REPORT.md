# Capture Point recording quality and HDR implementation report

Implemented in D:\Projects\CAPTURE, branch implementation/native-hdr, based on commit 009b309. The existing uncommitted HDR conversion draft was reviewed, preserved and integrated. This is a preview implementation, not a claim of completed hardware/workload certification. The installed recorder, its settings and user recordings were not replaced. Test captures used a dedicated DX11 fixture and disposable test-output folders; Windows display settings were not changed.

## Implemented behavior and root causes

The previous production path used BGRA8 SDR and rejected HDR displays. The uncommitted Main10 converter already passed a small synthetic encoding proof, but was disconnected from live capture and settings. Native capture now selects FP16 linear scRGB on HDR output; ordinary SDR retains BGRA8. Display color detection uses DXGI output characteristics, refreshed factory enumeration after invalidation, monitor identity and Windows SDR white level. Display HDR is identified as composition state, never as proof of a game's rendering mode or mastering metadata. The per-frame check uses factory invalidation rather than full output enumeration; slower color queries run during periodic housekeeping or actual display changes.

Two output preferences are exposed. Compatible Recording creates H.264 SDR MP4, with an explicit warning that HDR is tone mapped. Original HDR Recording selects NVIDIA HEVC Main10 for HDR displays and H.264 SDR for SDR displays. Quality remains a separate existing control. Archival 4:4:4 is an advanced SDR option and is refused for HDR sources without fallback.

The HDR converter performs a linear Rec.709/scRGB to Rec.2020 transform, absolute ST.2084/PQ encoding (scRGB 1.0 = 80 nits), and limited-range P010 conversion on the GPU. Main10, BT.2020 primaries, PQ transfer and BT.2020 nonconstant-luminance matrix are signaled on the encoder and individual frames. MP4 uses hvc1. No mastering-display, MaxCLL or MaxFALL values are invented: WGC does not provide game mastering metadata. PQ representation clips values outside its representable positive, 0–10,000-nit range; this is not a promise of bit-exact scRGB preservation.

Compatible HDR conversion normalizes using Windows' actual SDR white setting and applies a documented luminance shoulder above 75% of that white, then sRGB encoding and an 8-bit SDR encode. The shoulder intentionally changes highlights; it is not an HDR-preserving or perceptually certified tone mapper. Its implementation was checked against numerical references and decoded test patches. Saturated out-of-gamut colors can clip in SDR. Pure SDR desktop sources keep the existing SDR path.

Additional fixes address hardcoded SDR session manifests, incorrect SDR fallback tagging of unspecified HEVC, FP16 screenshot copies into BGRA textures, and browser audio-only playback masquerading as successful video playback. PNG screenshots remain explicitly tone-mapped SDR. Replay now explains missing video codec support even when the audio track loads. The build script also tracks Vite's output directory, avoiding stale embedded UI after content-hashed frontend assets change.

The existing indexed history, retention, range leases, single continuous encoder, TS segmentation and stream-copy MP4 saving remain. Resolution/display color changes create new epochs; footage on each side stays navigable. Manual recordings continue automatically as separate MP4s. Cross-epoch single-file stitching is not implemented. Remuxing refuses mixed codec/dimension/audio layouts rather than producing a malformed file. No extra continuous encoder, export screen or conversion button was added. Existing immediate Saving Clip feedback and silent successful completion remain.

## Supported paths

| Source / preference | Capture / conversion | Encoder and output | Verified coverage |
|---|---|---|---|
| SDR display, either preference | BGRA8, existing BT.709 SDR | H.264 NVENC, AMF or Media Foundation; MP4 | Synthetic NVENC/MF tests; live HDR-disabled display was not exercised |
| HDR display, Compatible | FP16 scRGB → explicit SDR tone map → BGRA8 | Same H.264 choices; SDR MP4 | Live NVIDIA capture through 4K; GPU and MF synthetic color checks |
| HDR display, Original HDR | FP16 scRGB → BT.2020/PQ/P010 | NVIDIA hevc_nvenc Main10, 10-bit 4:2:0 MP4 | Live SDR-on-HDR and native FP16 HDR fixture; independent FFmpeg/VLC decoding |
| Advanced Archival, SDR only | Existing RGB conversion → 4:4:4 | NVIDIA H.264 lossless coding; MP4 | Synthetic production D3D11 test; max RGB channel error 2/255 |

AMD/Intel HDR, software HDR and direct QSV input are not implemented. H.264 AMD/MF hardware availability is retained but not newly tested on those vendors. Unsupported HDR encoding, color formats and odd native 4:2:0 dimensions report errors; there is no crop, resize or SDR fallback. Audio stays on the existing AAC path and track layout; it is not mathematically lossless audio.

## Resolution, FPS, drops and color measurements

Hardware: RTX 4090, driver 32.0.16.1714, 3840×2160 HDR desktop; Windows reported approximately 143 Hz. Tests used a controlled DX11 target, not demanding commercial gameplay. Live native HDR used an FP16 scRGB swap chain with known 1,000-nit and 0.8-nit patches. Both patches survived WGC, encoding, TS and saved MP4: expected luma codes 722.6005 and 185.6490, measured 723 and 186 at all three resolutions. This tests absolute luminance preservation for those patches; it is not a complete color-volume/perceptual assessment.

| Live path / quality | Native dimensions | Encoded packets at sample | Missed deadline ticks | Repeated source samples | Saved MP4 frames / duration | Maximum packet gap |
|---|---:|---:|---:|---:|---|---|
| Original HDR High, final run | 1920×1080 | 384 | 0 | 77 | 331 / 5.5167 s | 16.667 ms |
| Original HDR High, final run | 2560×1440 | 376 | 3 | 76 | 333 / 5.5500 s | 16.667 ms |
| Original HDR High, final run | 3840×2160 | 381 | 0 | 77 | 336 / 5.6000 s | 16.667 ms |
| Original HDR Maximum | 3840×2160 | 342 | 41 | 153 | 291 / 5.4500 s | 66.667 ms |
| Compatible Maximum | 3840×2160 | 362 | 0 | 76 | 333 / 5.5500 s | 16.667 ms |

Counters are sampled before requesting each clip; the selected clip and whole-epoch counters cover different intervals. The 1440p High run's three missed ticks preceded its saved clip, whose packet spacing was continuous. An earlier High run had zero missed ticks at all three resolutions. These are measured observations, not zero-drop guarantees. The fixture consistently delivered roughly 48 WGC updates/sec, so a 60 FPS encoded file repeats source samples; why this fixture/WGC combination delivers 48 updates on the 143 Hz desktop was not established. No unique-60-FPS gameplay claim is made. Frames Windows never delivered cannot be counted by these counters. Advanced diagnostics expose the counts; ended epochs write recording-metrics.json. The measured overload does not change quality, FPS or native size.

Synthetic 180-frame tests independently decoded all 180 frames at 1920×1080, 2560×1440 and 3840×2160, in both SDR and HDR, with 60/1 timestamps and correct pixel formats. With the bounded production-style queue, Maximum 4K sequential fixture throughput was approximately 74.0 FPS SDR and 52.0 FPS HDR. Thus Maximum HDR is demonstrably not sustainable at 4K60 on this setup in that test. High achieved continuous 4K60 output timing in the live fixture, while still repeating the approximately 48-Hz source.

Synthetic HDR patch error after MP4 decoding was under 0.5 of a 10-bit YUV code value at sampled patch centers. The tone-mapped SDR reference round trip differed by at most 2.891/255 on NVENC and 4.486/255 on Microsoft's software encoder. The Archival RGB result was max 2/255. These patch-specific metrics are not universal quality scores, not mathematically lossless claims, and do not measure fine chroma edges, all gamut boundaries, motion compression or arbitrary game rendering.

## MP4 compatibility, history fidelity and latency

H.264 and Main10 MP4s decoded independently with FFmpeg. VLC also opened and decoded both 4K samples through its real decoder in a headless dummy-output run; software decoding produced first pictures and clean completion. This does not verify its HDR display renderer. An initial dummy-output attempt using hardware surfaces produced VLC overlay-blending warnings; the clean software-decoder check is the result reported here.

Capture Point's actual Replay component in an isolated Edge harness presented H.264 video (15 video frames observed). The HEVC+AAC fixture exposed an environment limitation: audio loaded but video dimensions were zero. The new codec notice appeared correctly. This is a verified graceful failure, not a pass for HEVC playback in this browser. Actual WebView2/Windows Media Player HDR rendering remains unverified and depends on installed HEVC/HDR support.

FrameCut Studio was launched for a real import attempt. Its native dialog could not be addressed reliably by the available computer-use helper; the attempt was canceled without importing media or changing projects. FrameCut import/rendering remains unverified. Discord upload/recipient behavior was not tested and no files were sent externally.

H.264 and HEVC history fixtures beyond the four-hour timestamp mark passed preview, save, identical repeat-save packet hashes, independent decode, corrupted-source failure/retry, source protection and restart persistence. For each format, 121 saved encoded VCL slices matched the original segment slices exactly. HEVC Main10, 10-bit pixel format, PQ/BT.2020 tags and hvc1 survived historical saves. These fixture timestamps simulate history placement; they are not four hours of sustained capture. The fixtures include AAC and verify original clip decode; physical microphone/voice hotkeys and multi-device synchronization were not newly exercised.

Measured four-second historical save completion was roughly 58–144 ms across the fixture runs; acknowledgement roughly 22–33 ms. HDR preview preparation was approximately 63 ms in an earlier run. Synthetic three-second native remux ranged approximately 41–436 ms depending on format and file size. Live hotkey-equivalent replay commands took roughly 2.7–4.2 seconds to finish because the existing ring waits for the tail segment and late packets to close. Interaction acknowledgement remains separate from file completion. Physical hotkey input latency was not measured in this session.

## Performance, storage and regression status

There is one video encoder, with no separate history encode. Original HDR adds two GPU pixel-shader passes and P010 buffers; Compatible HDR adds a GPU tone-map pass. At 4K, a single FP16 surface is about 63.3 MiB, BGRA8 about 31.6 MiB, and P010 about 23.7 MiB. These are pixel-storage calculations, not measured total VRAM. Actual GPU utilization, CPU/game frame-time impact, storage throughput and six-hour stability remain unmeasured. Source clips were simple fixtures and their sizes must not be extrapolated to gameplay.

At 4K60, the existing High and Maximum rate ceilings correspond to approximately 59.7 and 99.5 Mbit/s before audio, respectively; actual CQ/VBR rate depends on content. Four hours at 60 Mbit/s is about 108 decimal GB before additional audio/overhead. Retention limits shorten history and preserve protected/permanent files; they never reduce encoding settings. Archival storage remains content dependent.

Final workspace tests: **86 passed, 0 failed, 4 explicit hardware tests excluded from the default suite**. All four hardware tests were run separately and passed. The UI fixture passed 12 checks for H.264 playback and 12 for HEVC unsupported-video feedback/history behavior. Production frontend build, release worker build and official Tauri no-bundle application build passed. git diff --check passed. New Rust modules were formatted with rustfmt. The portable preview includes the final worker/UI and existing unchanged overlay helpers; it is not installed and has not undergone full installer/distribution acceptance.

Still required for release certification: demanding representative games, live HDR-disabled display testing, physical HDR/SDR toggle and multi-monitor transitions, long-duration recording and retention, AMD/Intel coverage, actual Windows/FrameCut HDR rendering, Discord recipients, hardware audio/hotkeys/overlay regressions, and measured gameplay impact. Detection/restart code is implemented, but those unexecuted scenarios are not represented as passes.

## Files changed

- Engine: crates/capture-engine/Cargo.toml; src/lib.rs, pipeline.rs, video.rs, wgc.rs, save.rs, shot.rs; new src/hdr.rs and src/quality_tests.rs.
- Protocol: crates/capture-protocol/src/lib.rs.
- App: app/src-tauri/build.rs and src/settings.rs; app/src/lib/types.ts; app/src/tabs/SettingsTab.svelte; app/src/overlay/HistoryPanel.svelte.
- Fixture: crates/hook-testapp/Cargo.toml and src/main.rs (DPI-aware native sizing, visible test window with hidden console, FP16 HDR source).
- Verification: scripts/e2e-history.mjs, ui-history-fixture.mjs; new scripts/e2e-native-quality.mjs.
- Documentation: docs/QUICKSTART.md and this report. No history catalog/retention or audio architecture was rebuilt.

Reproduce from the repository with FFmpeg DLLs on PATH: cargo test --workspace; cargo test -p capture-engine video::tests -- --ignored --nocapture --test-threads=1; cargo test -p capture-engine quality_tests -- --ignored --nocapture --test-threads=1; node scripts/e2e-history.mjs [--hdr]; node scripts/e2e-native-quality.mjs --hdr [--high | --compatible]; node scripts/ui-history-fixture.mjs. The UI fixture consumes the latest successful history fixture. The live script uses only its dedicated test window and never toggles Windows HDR.

Technical basis: [Microsoft HDR capture formats](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture), [Windows Advanced Color/scRGB](https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range), [DXGI factory refresh](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_6/nf-dxgi1_6-idxgioutput6-getdesc1), [Windows SDR white level](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_sdr_white_level), [NVIDIA NVENC programming guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html). Measurements above come from local tests, not these reference documents.
