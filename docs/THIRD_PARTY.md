# Third-party components in CAPTURE builds

| Component | Version | Licence | How it is used |
|---|---|---|---|
| FFmpeg (`avcodec-62`, `avformat-62`, `avutil-60`, `swresample-6`, `swscale-9`) | n8.1.3-34-ga35157abb9 (BtbN build `autobuild-2026-11-07-12-47`, LGPL variant) | LGPL v3 (`licenses/FFmpeg-LICENSE.txt`) | Unmodified shared DLLs, dynamically linked by `capture-worker.exe`. You may replace them with compatible builds. Source: https://github.com/FFmpeg/FFmpeg/tree/n8.1.3 and the build recipe at https://github.com/BtbN/FFmpeg-Builds |
| Tauri 2 and plugins (dialog, clipboard-manager, notification, single-instance) | 2.12.x | MIT / Apache-2.0 | UI shell (`CAPTURE.exe`) |
| windows-rs | 0.62 | MIT / Apache-2.0 | Windows API bindings |
| Svelte 5, Vite | 5.57 / 8.3 | MIT | UI (bundled into `CAPTURE.exe`) |
| Microsoft Edge WebView2 Runtime | provided by Windows | Microsoft | Renders the UI. It is not shipped with CAPTURE. |

Not used: `libopenh264` (patent coverage applies only to Cisco's own binaries), libobs/OBS (GPL; it was evaluated in Phase 0 only).
