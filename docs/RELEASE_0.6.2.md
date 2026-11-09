# CAPTURE 0.6.2

Integrates the current overlay/replay work, resolution-change handling, immediate capture feedback, direct video previews, and softer capture sounds.

- Resolution changes release WGC frames before rebuilding the pool, reject incomplete surfaces, and preserve the recording canvas and source aspect ratio.
- Screenshots process asynchronously. Clips, screenshots, and session actions confirm immediately; successful completion stays silent. Replay confirmation reads **Clip Saved**. Actual errors remain visible.
- Library videos decode representative frames from existing media with two concurrent decoders and a bounded memory cache. No persistent thumbnails are created.
- Original, quieter action-specific sounds use a warmed WASAPI player with overlap/backlog suppression. Existing mute, volume, and output-device preferences are retained.
- Includes the accumulated in-game overlay, replay review, input, audio, and storage improvements from the working branch.

Validation before packaging: 72 workspace tests passed, release app/worker builds passed, real recording/screenshot/error-path acceptance passed, and five saved MP4s decoded cleanly. Physical screenshot/replay hotkeys reached native notice painting within 4–11 ms on the test machine. Real Svelte preview and notification tests passed, including 20 video cards, black openings, corrupt media, cancellation, cache eviction, and silent completion.

Validation is limited to this Windows machine. Subsequent exclusive-fullscreen resolution test attempts could not enter fullscreen (DXGI_ERROR_NOT_CURRENTLY_AVAILABLE); broad commercial-game/GPU compatibility remains unverified. One earlier DirectX run completed exclusive fullscreen and minimize/restore successfully.
