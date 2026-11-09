# CAPTURE distribution: portable folder and Windows installer (v0.5.0)

One application payload, two ways to get it onto a PC. The installer deploys the exact files of
the portable zip (verified against the same `manifest.json`), so recording behaviour, features and
settings are identical.

| Artifact | Built by | Size |
|---|---|---|
| `dist\CAPTURE-<v>\` (the payload, runnable in place) | `scripts\package-portable.ps1` | 136 MB, 10 files |
| `dist\CAPTURE-<v>-portable.zip` | same | 56 MB |
| `dist\CAPTURE-<v>-setup.exe` (NSIS 3.11, per user) | `scripts\build-installer.ps1` | 41 MB |
| `dist\SHA256SUMS-<v>.txt` | same | |

## Layout

```text
CAPTURE\
  CAPTURE.exe                 UI, tray, settings (Tauri 2 + WebView2)
  manifest.json               version + SHA-256/size of every program file
  Runtime\                    capture-worker.exe + FFmpeg 8.1 LGPL DLLs (never written at run time)
  Resources\                  QUICKSTART.md, Licenses\
  Data\                       created on first run, never shipped
    Settings\                 settings.json, settings.json.bak, Backups\ (migration/update copies)
    Cache\WebView\            WebView2 profile (instead of WebView2's AppData default)
    Cache\Temp\               TEMP/TMP for CAPTURE, WebView2 and the engine
    Cache\Update\             a staged update, only while installing one
    Logs\                     capture.log (+ capture.old.log after 2 MB), UI and engine
  Uninstall CAPTURE.exe       installed copies only
```

All paths come from `app/src-tauri/src/paths.rs`, resolved from the location of `CAPTURE.exe`;
nothing depends on the working directory. Recordings, sessions, screenshots and the replay buffer
go to the folders chosen in Settings and never into the app folder.

**Read-only app folder** (for example someone copies CAPTURE into Program Files): CAPTURE does
not silently move its data. A native dialog explains the problem and offers to keep data in
`%LOCALAPPDATA%\CAPTURE\Data` for that account (remembered in `fallback-for.txt` there) or quit.
The installer refuses Program Files / Windows folders and drive roots up front, and checks the
folder is writable without creating anything.

## Writes outside the app folder (complete list)

| What | When | Removed by |
|---|---|---|
| `HKCU\...\Run\CAPTURE` | only if Start with Windows is on | the toggle; uninstall (if it points to that copy) |
| Recording folders (`<root>\<Game>\...`, `<buffer>\Buffer\`) | when recording | user; uninstall "all data" (CAPTURE's own files only) |
| Installed only: `HKCU\...\Uninstall\CAPTURE`, `Start Menu\Programs\CAPTURE.lnk`, optional `Desktop\CAPTURE.lnk` | install | uninstall |
| `%LOCALAPPDATA%\CAPTURE\Data` | only after consenting to the read-only-folder fallback | user |
| Windows itself (not CAPTURE-controlled): WebView2 runtime's own updates, Windows Error Reporting, Explorer's recent/jump lists, tray icon cache | — | Windows |

Verified by `e2e-distribution.mjs`: after running, recording, moving and deleting a portable copy,
nothing CAPTURE-named exists in `%LOCALAPPDATA%`, `%APPDATA%` or `%TEMP%`, and no startup entry.

## Dependencies

| Dependency | How it is provided |
|---|---|
| FFmpeg 8.1.3 LGPL (avcodec, avformat, avutil, swresample, swscale) | bundled in `Runtime\`, unmodified shared DLLs (licence in `Resources\Licenses`) |
| C runtime | Rust built with `+crt-static`; FFmpeg uses the Universal CRT, part of Windows 10/11. **No VC++ redistributable.** (Checked with `dumpbin /dependents`: only Windows system DLLs and `api-ms-win-crt-*`.) |
| WebView2Loader | statically linked |
| WebView2 runtime | **system Evergreen runtime** (built into Windows 11; current Windows 10 has it via Edge). Missing → native message before any window, pointing to Microsoft's installer. |
| Hardware encoders | NVENC / AMF / Media Foundation probed at run time; Software fallback. No vendor setting is saved, so moving to another PC cannot carry an invalid GPU configuration. |
| Rust, Node, pnpm, Python, libclang, NSIS | build-time only |

### WebView2: why not bundled
A Fixed Version runtime adds ~170 MB (zip ~60 → ~230 MB) and freezes the browser engine: every
Chromium security fix would need a new CAPTURE release. Since WebView2 is an OS component on the
supported systems, CAPTURE uses it and checks for it at start. The option stays open:
`package-portable.ps1 -WebView2Cab <Microsoft.WebView2.FixedVersionRuntime.*.cab>` puts it in
`Runtime\WebView2\`, and CAPTURE then uses it automatically (`WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`).
Not tested here.

## Start-up, tray, single instance
- One running CAPTURE per Windows session (`app.capture.local` mutex). A second launch hands its
  arguments to the running one before touching any file: plain launch shows the window, `--tray`
  stays silent, `--quit` quits exactly like the tray's Quit (in-progress saves finish first).
- Close hides to the tray (recording continues); Quit ends the UI and the engine.
- Start with Windows = one per-user Run value. A value pointing to a moved copy is repointed on
  launch, but **not** when the target's drive is merely absent (portable copy on a USB drive).

## Settings safety
- Schema version 2. A file from another schema version is copied to `Settings\Backups\` before it
  is rewritten (also when an older build reads a newer file).
- 0.3/0.4 `data\settings.json` + `data\webview` are moved into the new layout on first start, with
  a backup. Extracting the 0.5 zip over a 0.4 folder also removes the nine obsolete top-level
  program files (exact names only).
- A new portable folder beside older `CAPTURE*` folders copies the newest one's settings (copy only).
- Unchanged from 0.4.1: last-known-good `.bak`, field-by-field salvage, damaged copies kept.
- First run asks four things only (recordings folder, automatic recording, Start with Windows,
  microphone); people who already used 0.3/0.4 are not asked again.

## Moving to another PC
Saved recording/buffer folders on a drive that is not present are reported on Home with a fix
button; the setting itself is kept. A missing microphone keeps its existing "your chosen
microphone is not installed" message, with no silent fallback. Encoders are re-probed.

## Updates
Settings › About & updates › **Choose file…** takes a release zip (the portable zip works for
installed copies too):
1. Extract to `Data\Cache\Update\Staged`, verify every file's SHA-256 and size against the
   release manifest (unsafe paths and `Data\` entries refused). App folder untouched.
2. Refused while a session is recording or a clip is saving. Settings backed up
   (`Backups\settings-before-<v>-*.json`).
3. The staged `CAPTURE.exe --apply-update` waits for the running app to exit, moves current
   program files to `Update\Rollback`, copies and re-verifies the new ones, restores everything
   on any failure, and records each step in `Update\journal.json`. An interrupted update resumes
   from the staged copy at next start. Installed copies get their Installed-apps version updated.
4. The new app reports the result once and deletes staged and rollback files, which also removes
   program files the new release no longer has.

There is no online update check: CAPTURE has no release server and makes no network requests.
Running a newer `setup.exe` also upgrades an installed copy in place.

## Uninstall
- **Remove CAPTURE** (default, and `/S`): closes CAPTURE (`--quit`), deletes temporary replay
  footage, the startup entry (if it is this copy's), shortcuts it created, `CAPTURE.exe`,
  `manifest.json`, `Runtime\`, `Resources\`, `Data\` (settings, cache, logs), the uninstaller and
  the Installed-apps entry. The install folder is removed only if then empty (never `RMDir /r` on
  it). Recordings stay.
- **Remove CAPTURE and all its data** (`/S /ALLDATA` unattended): CAPTURE first lists the
  locations, file counts and sizes and asks. Then only files it can verify as its own
  (`owned.rs`: MP4s carrying `Recorded with CAPTURE` (0.5+) or CAPTURE's track names (0.3+),
  stamped screenshots, stem folders with CAPTURE's manifest, buffer segments) go to the Recycle
  Bin. Other files in those folders and unrelated folders are never touched; folders are removed
  only when empty.
- Files that could not be removed (locked) are listed.

## Test results (this machine, packaged builds)
- Unit tests: 58 pass (20 app: paths, migration, schema backup, ownership, update apply/rollback/
  tamper; 35 engine; 3 protocol).
- `e2e-distribution.mjs`: **69/69**. Covers: zip = manifest; first run creates only `Data\`;
  WebView2 profile inside `Data`; setup card; whole-display replay clip and screenshot recorded by
  the packaged engine (ffprobe: plays, ownership tag); `--quit`; log file; folder moved; missing
  drive reported and kept; damaged update refused; in-app update applied, verified, settings kept
  and backed up, cleanup; folder deleted with no leftovers. Installer: silent install to
  `D:\...\Install Dir\CAPTURE` (non-C, space in path); files = payload; Installed-apps entry,
  Start menu and desktop shortcuts, Run entry; record; close to tray; upgrade while running
  (closed, settings kept, one install); uninstall keeping recordings (buffer removed, shortcuts,
  Run entry and folder gone); reinstall; uninstall with all data (only CAPTURE's files removed).
- `e2e-lifecycle.mjs` on the 0.5.0 payload: **19/19** (single instance, tray, startup entry, crash
  restart, no orphan engine, damaged settings, import from a 0.4-layout folder).
- In-place upgrade of a copy of a real 0.4.0 folder: data migrated, settings intact, 9 obsolete
  files removed.
- Installer wizard pages and the Program Files refusal checked by screenshots; cancelling leaves
  nothing. Uninstall choice page checked by screenshot.

## Not verified / limits
- **No clean-VM test.** Everything ran on the development PC (dev tools present). The dependency
  audit (`dumpbin`) shows no dev-tool dependency, but a clean Windows 10/11 VM run is still owed.
- **Offline**: CAPTURE makes no network requests (CSP `connect-src ipc:` only), but no run with
  networking disabled was done.
- **WebView2 absent**: the message path was not exercised (WebView2 cannot be removed from Windows 11).
- **CAPTURE's own "remove recordings?" confirmation dialog** was not captured visually; the
  silent `/ALLDATA` path and the list text are tested.
- Interactive install with the Finish page's launch, and the folder-exists/portable-copy
  messages, were not clicked through.
- Fixed-version WebView2 packaging (`-WebView2Cab`) untested.
- Unsigned: SmartScreen warns on first run of `setup.exe` and `CAPTURE.exe`. Public distribution
  should use Authenticode signing (not bypassed or worked around here).
- Reboot/sign-in startup not exercised (registry value verified only).

## Compatibility matrix
| Dimension | Tested | Expected / notes |
|---|---|---|
| Windows | 11 Pro 26200 x64 | Windows 10 22H2 x64 and Windows 11: expected (WGC per-window capture needs 10 2004+; process loopback audio needs 10 2004+). Untested. |
| CPU arch | x64 (i5-13600KF) | ARM64 not built; x86 not supported |
| GPU / encoder | RTX 4090 NVENC; Software (Media Foundation) | AMD AMF code path untested; Intel Quick Sync not implemented (Software fallback) |
| Audio | system output, process loopback, G06-BT (silent device) | live mic speech still unverified (see STATUS.md) |
| Capture | windowed / borderless games, whole display | exclusive fullscreen and anti-cheat untested |
| Install path | `D:\...\Install Dir\CAPTURE` (spaces), moved portable folder with spaces/parentheses | Program Files refused by design |
| Network | online machine; no requests made | offline expected to work; not run |

## Licences
FFmpeg LGPL 2.1+: unmodified shared DLLs, licence and source pointer in `Resources\Licenses`
(`docs/THIRD_PARTY.md`). NSIS (zlib licence) is build-time only; the generated installer
contains NSIS's stub under its zlib/libpng terms (no notice required).
