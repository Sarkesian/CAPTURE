# Third-party components in CAPTURE builds

| Component | Version | Licence | How it is used |
|---|---|---|---|
| FFmpeg (`avcodec-62`, `avformat-62`, `avutil-60`, `swresample-6`, `swscale-9`) | n8.1.3-34-ga35157abb9 (BtbN build `autobuild-2026-11-07-12-47`, LGPL variant) | LGPL v3 (`licenses/FFmpeg-LICENSE.txt`) | Unmodified shared DLLs, dynamically linked by `capture-worker.exe`. You may replace them with compatible builds. Source: https://github.com/FFmpeg/FFmpeg/tree/n8.1.3 and the build recipe at https://github.com/BtbN/FFmpeg-Builds |
| Tauri 2 and plugins (dialog, clipboard-manager, notification, single-instance) | 2.12.x | MIT / Apache-2.0 | UI shell (`CAPTURE.exe`) |
| windows-rs | 0.62 | MIT / Apache-2.0 | Windows API bindings |
| rusqlite | 0.41.1 | MIT | Transactional local history catalog, statically linked into the worker |
| SQLite (libsqlite3-sys) | pinned by Cargo.lock | Public domain (Rust bindings MIT) | Bundled SQLite; no additional service or database DLL required |
| Svelte 5, Vite | 5.57 / 8.3 | MIT | UI (bundled into `CAPTURE.exe`) |
| Microsoft Edge WebView2 Runtime | provided by Windows | Microsoft | Renders the UI. It is not shipped with CAPTURE. |

Not used: `libopenh264` (patent coverage applies only to Cisco's own binaries), libobs/OBS (GPL; it was evaluated in Phase 0 only).

## rusqlite notice

Copyright (c) 2014 The rusqlite developers

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
