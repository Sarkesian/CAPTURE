# Indexed history implementation handoff

Implemented on `implementation/indexed-history`, based on `0c190cec36d135bc1de71a7b62c3435dc7f7cb52`. The installed application and its user settings/media were not modified or replaced. All new media tests used disposable fixtures under this repository's ignored `test-output` directories.

## Architecture implemented

The existing WGC/D3D11, H.264, AAC, TS segments, Ring, stream-copy MP4 saving and filesystem Library remain. A shared `history::Catalog` adds SQLite WAL/FULL transactions, indexed segment lookup, cross-session retention, range leases, bounded/reusable playback windows, and persisted events. There is no OBS replacement, new background service, HEVC/AV1 production path, or manual-export workflow.

Settings schema 3 adds `history_secs`, default 14400, maximum 21600. Existing `replay_secs`, byte budget and free-space reserve are preserved. The existing budget is shared by history and playback; migration does not allocate a larger budget. Duration is cumulative recorded segment coverage, excluding wall-clock idle periods. Settings show measured-rate estimates when available and otherwise a parameter-based estimate.

SQLite schema version 3 contains:

- `sessions`: stable folder-derived ID, game, directory, start time, ended status/end time, configuration and ownership flag.
- `epochs`: immutable codec/resolution/audio configuration for each recording epoch. Native resolution changes create another session/epoch rather than changing an existing stream's dimensions.
- `segments`: session/sequence, path, media interval, byte size, completion state; indexed by session/time.
- `events`: stable IDs and serialized marker, screenshot, clip, or removal events.
- `jobs`: protected expanded boundary ranges, save/playback kind, active/failed status and error.
- `pins`: bounded marker protection and temporary full-recording protection.
- `previews`: playback path, byte accounting, interval and reader references.

Segment files remain the video source. Startup enumerates metadata and incrementally probes unknown/incomplete media. Replay opening does not rescan all media. Marker/screenshot events also go to a flushed append-only `events.jsonl` journal so event rows can be rebuilt. Missing metadata uses honest recovery defaults; lost original labels cannot be invented.

## Retention, saving and feedback

New recordings survive game switches and restarts. Global cleanup evicts oldest unprotected, owned segments for duration, bytes or free-space reserve. Failed deletions remain accounted. Leases are acquired before acknowledging saves and shared with cleanup. Failed/interrupted saves retain source protection and expose an inline Retry save action. Shutdown waits for saves across catalog instances. Permanent MP4s and PNGs are independent of rolling history.

**Migration exception:** imported pre-existing media is never automatically deleted. Active/failed jobs and pins can also prevent satisfying a cap. The UI reports budget/protection constraints; quality is not reduced to fit. This favors preservation over silently enforcing the cap by deleting original user media.

Historical saves query only nearby segments and audio boundaries. MP4 creation retains the existing stream-copy/finalize/verify/rename path. Required unreadable media now fails rather than being silently skipped. Keyframe-aligned starts remain approximate; no frame-accurate claim is made. Selections crossing unavailable footage fail visibly. Different game/resolution epochs are separately selectable; cross-epoch single-file clip stitching is not implemented.

Immediate native/audio interaction feedback is preserved. Clip acknowledgement says **Saving Clip**. Successful completion updates Library silently; there is no extra success notification or progress dialog. Failures remain visible.

Replay uses approximately 30-second windows, shortened by a 64 MiB source-window target, with a 256 MiB aggregate preview reservation ceiling. Windows are reused with reader counts; unused files are released, stale files reconciled on restart, and upcoming windows prefetched. Extremely high bitrates or large individual segments can exceed that cache ceiling and produce an explicit error instead of lowering recording quality.

Replay now includes game-grouped session navigation, cumulative global navigation, available intervals/gaps, zoom, scrubbing, range controls, event jumps, screenshot previews, marker rename/removal, and one-action saving. The overlay reuses this component.

## Fidelity and limitations

History and immediate saves share original encoded video. No second lossy video pass is used when saving. Silent software FPS capping and automatic NVENC preset/CQ reductions were removed. Native dimension changes start another epoch; manual full recording continues in another file. Unsupported dimensions may fail encoder initialization instead of being cropped to fit.

An explicit NVIDIA **Archival 4:4:4** option uses H.264 lossless coding after RGB conversion, constant QP 0, no bitrate ceiling and no lower-quality fallback. Balanced/High/Maximum remain lossy H.264. Existing settings are not silently changed to Archival.

This is **not end-to-end mathematically lossless**. The WGC path is 8-bit BGRA SDR; RGB/YUV conversion and alpha handling prevent a bit-exact original-frame claim, and audio remains lossy AAC. Known unsupported HDR/wide-color output is rejected at native capture startup rather than silently converted. HDR/10-bit preservation, color-mode changes during recording, multi-adapter coverage and full render-to-output fidelity need further validation. The new color gate may reject previously recordable environments; that is an explicit fidelity conflict, not a tested HDR implementation.

Counters distinguish source arrivals, emitted video packets, skipped/queue-dropped recording ticks, ticks without a picture, repeated source-generation samples, and source arrivals not selected. They do not count game presents never delivered by Windows or prove every frame's pixel content is unique. Intended FPS is not advertised as evidence of complete capture. Demanding-game frame-time impact and six-hour stability remain unmeasured.

Storage is content-dependent. At a total 16 Mbit/s, 2/4/6 hours consume 14.4/28.8/43.2 decimal GB; at 60 Mbit/s, 54/108/162 GB. These are arithmetic estimates. Lossless video has no guaranteed compression ratio. Uncompressed 4K60 BGRA8 is about 1.99 GB/s or 43 TB over six hours; this is a raw upper planning reference, not measured encoded storage. Lossless/4:4:4 playback compatibility is substantially narrower than ordinary H.264 4:2:0. No universal zero-loss or gameplay-performance claim is made.

Technical references: [FFmpeg NVENC implementation](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/nvenc_h264.c), [NVIDIA encoder guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html), [Microsoft HDR capture guidance](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture), [DXGI color information](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_6/nf-dxgi1_6-idxgioutput6-getdesc1).

## Validation performed

- Final workspace suite: **83 passed, 0 failed**, one hardware-specific test excluded from the default run and exercised separately.
- Frontend production build and Rust engine/app checks passed. Release worker build passed. A new installer or release package was not created or installed.
- Nine catalog tests cover cumulative retention, leases, failed-save persistence, permanent media preservation, event/restart/journal recovery, gaps/missing files, failed deletion, legacy preservation and a synthetic 10800-segment catalog.
- Synthetic catalog timing in an earlier run: 10800 inserts about 143 ms; a 30-second metadata lookup about 1.2 ms. These are local synthetic measurements, not a gameplay benchmark.
- Final `scripts/e2e-history.mjs`: historical media timestamps beyond four hours, bounded preview creation, verified original encoded slices, repeated-save identity, corrupt-fixture failure/retry, shared preview ownership, restart persistence and permanent-media preservation all passed. It used prerecorded 320×180 fixtures, not four hours of live recording. Preview creation about **92 ms**, four-second saves about **83–89 ms**, acknowledgements about **20–27 ms**, preview about **2.18 MB**, **121 original video slices verified**.
- `scripts/ui-history-fixture.mjs`: **11 checks passed** in isolated headless Edge with mocked IPC, including session navigation, gaps, range controls, Saving Clip wording, silent completion/Library refresh, compact layout and late-response cancellation. This is not a real in-game overlay test.
- `scripts/fidelity-proof.mjs`: synthetic system-memory NVENC YUV444 source/decoded bytes matched exactly for 60 frames. The RGB round trip was not exact, with maximum channel error 2/255.
- Explicit synthetic D3D11 production-encoder test passed: 60/60 frames, native 256×144 at 60 FPS, decoded YUV444, maximum RGB channel error 2/255. This does not include WGC, HDR or a running game.
- Formatting tool was unavailable (`rustfmt` component missing); no toolchain component was installed. `git diff --check` passed.

Not executed: real six-hour capture, demanding-game performance/VRAM profiling, real device hotkeys/microphone tests, full existing process-control recording suites, live fullscreen overlay tests, genuine power-loss tests, AMD/Intel/HDR hardware coverage, complete editor/recipient compatibility testing. No such passes are claimed.

## Changed modules and rollback

Core changes: `crates/capture-engine/src/history.rs`, `lib.rs`, `pipeline.rs`, `ring.rs`, `save.rs`, `shot.rs`, `video.rs`, `wgc.rs`; protocol types/commands; SQLite dependency and Cargo.lock. UI changes: settings, store/types, Replay page, shared `HistoryPanel`, ReviewPanel wrapper, overlay eligibility/quality options, and current-rate estimation on Home. Three fixture scripts, migration tests, user guide and third-party notices were added/updated.

The original commit remains available as the rollback baseline. Reverting the feature commit restores source behavior, but rolling back a deployed app must not delete history catalogs/journals or original media. This work did not deploy the new executable, alter installed settings, or operate on user recordings. Real-game acceptance and the fidelity/compatibility conflicts above remain explicit follow-up work.
