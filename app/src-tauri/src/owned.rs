//! Files CAPTURE itself created in the recording folders. Used only by the uninstaller's
//! "Remove CAPTURE and all its data" choice, after the user confirmed a list of locations.
//!
//! A recording folder is often a broad, shared folder, so nothing here ever deletes a folder
//! recursively. A file counts as CAPTURE's only when it is inside CAPTURE's own structure
//! (`<root>\<Game>\Clips|Sessions|Screenshots`, `<Buffer>\<session>`) **and** passes a content
//! or name check:
//! - MP4: the file's `moov` box carries CAPTURE's tag (`Recorded with CAPTURE`, 0.5+) or its
//!   track names (`Preview Mix`, 0.3+);
//! - PNG screenshots: CAPTURE's `YYYY-MM-DD_HH-MM-SS_<Game>.png` name;
//! - editor audio folders: CAPTURE's `manifest.json` (`capture_stems`), then only the WAVs,
//!   manifest and README it wrote;
//! - replay buffer: `seg_*.ts`, `session.json`, `markers.json` in a CAPTURE session folder.
//!
//! Recordings go to the Recycle Bin; temporary buffer files are deleted. Folders are removed
//! only once empty. Anything else is left alone and counted as "not touched".

use std::path::{Path, PathBuf};

const KINDS: [&str; 3] = ["Clips", "Sessions", "Screenshots"];

#[derive(Debug, Default)]
pub struct Scan {
    /// Clips, sessions, screenshots, marker sidecars and editor audio files.
    pub recordings: Vec<PathBuf>,
    pub recording_bytes: u64,
    /// Replay-buffer segments and crash-recovery metadata.
    pub temporary: Vec<PathBuf>,
    pub temporary_bytes: u64,
    /// Folders that become removable once empty (deepest first).
    pub folders: Vec<PathBuf>,
    /// Files inside CAPTURE's folders that CAPTURE did not create (never touched).
    pub not_ours: usize,
    /// Roots that were scanned / not available (unplugged drive).
    pub roots: Vec<PathBuf>,
    pub unavailable: Vec<PathBuf>,
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn ext_is(p: &Path, e: &str) -> bool {
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case(e))
}

/// `YYYY-MM-DD_HH-MM-SS_` prefix, as every CAPTURE recording name starts.
fn stamped(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() > 20
        && b[..19].iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'_',
            13 | 16 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
        && b[19] == b'_'
}

/// True if an MP4's `moov` box contains CAPTURE's tag or track names.
pub fn is_capture_mp4(p: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(p) else { return false };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut pos = 0u64;
    for _ in 0..64 {
        if pos + 8 > len || f.seek(SeekFrom::Start(pos)).is_err() {
            return false;
        }
        let mut h = [0u8; 16];
        if f.read_exact(&mut h[..8]).is_err() {
            return false;
        }
        let mut sz = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = h[4..8].try_into().unwrap();
        let mut header = 8;
        if sz == 1 {
            if f.read_exact(&mut h[8..16]).is_err() {
                return false;
            }
            sz = u64::from_be_bytes(h[8..16].try_into().unwrap());
            header = 16;
        } else if sz == 0 {
            sz = len - pos;
        }
        if sz < header {
            return false;
        }
        if &kind == b"moov" {
            let body = (sz - header).min(256 << 20) as usize;
            let mut buf = vec![0u8; body];
            if f.read_exact(&mut buf).is_err() {
                return false;
            }
            let has = |needle: &[u8]| buf.windows(needle.len()).any(|w| w == needle);
            return has(b"Recorded with CAPTURE") || has(b"Preview Mix");
        }
        pos += sz;
    }
    false
}

fn is_stem_folder(dir: &Path) -> bool {
    std::fs::read(dir.join("manifest.json")).is_ok_and(|b| b.windows(13).any(|w| w == b"capture_stems"))
}

pub fn scan(capture_root: &Path, buffer_root: Option<&Path>) -> Scan {
    let mut s = Scan::default();
    if capture_root.as_os_str().is_empty() {
        return s;
    }
    if !capture_root.is_dir() {
        s.unavailable.push(capture_root.to_path_buf());
    } else {
        s.roots.push(capture_root.to_path_buf());
        scan_recordings(capture_root, &mut s);
    }
    let mut buffers: Vec<PathBuf> = BACKGROUND_DIRS.iter().map(|n| capture_root.join(n)).collect();
    if let Some(b) = buffer_root.filter(|b| !b.as_os_str().is_empty()) {
        if b.is_dir() {
            buffers.extend(BACKGROUND_DIRS.iter().map(|n| b.join(n)));
        } else {
            s.unavailable.push(b.to_path_buf());
        }
    }
    buffers.dedup_by(|a, b| crate::paths::same_path(a, b));
    for b in buffers {
        scan_buffer(&b, &mut s);
    }
    s
}

/// The background-recording folder's name, and its name before 0.6.1 (as the engine's `storage`).
pub const BACKGROUND_DIRS: [&str; 2] = ["Background Recording", "Buffer"];

fn scan_recordings(root: &Path, s: &mut Scan) {
    for game in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let g = game.path();
        if !g.is_dir() || BACKGROUND_DIRS.iter().any(|n| game.file_name().eq_ignore_ascii_case(n)) {
            continue;
        }
        let mut any_kind = false;
        for kind in KINDS {
            let dir = g.join(kind);
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            any_kind = true;
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                let lower = name.to_ascii_lowercase();
                let ours = if p.is_dir() {
                    if kind != "Screenshots" && lower.ends_with("_audio") && is_stem_folder(&p) {
                        for f in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                            let fp = f.path();
                            let fname = f.file_name().to_string_lossy().to_ascii_lowercase();
                            if fp.is_file() && (ext_is(&fp, "wav") || fname == "manifest.json" || fname == "readme.txt") {
                                s.recording_bytes += size(&fp);
                                s.recordings.push(fp);
                            } else {
                                s.not_ours += 1;
                            }
                        }
                        s.folders.push(p.clone());
                        continue;
                    }
                    false
                } else if kind == "Screenshots" {
                    (ext_is(&p, "png") && stamped(&name)) || (lower.ends_with(".png.partial") && stamped(&name))
                } else if ext_is(&p, "mp4") {
                    is_capture_mp4(&p)
                } else if lower.ends_with(".mp4.markers.json") {
                    // a sidecar belongs to CAPTURE when its clip does (or the clip is already gone)
                    let clip = p.with_file_name(&name[..name.len() - ".markers.json".len()]);
                    !clip.exists() || is_capture_mp4(&clip)
                } else {
                    lower.ends_with(".mp4.partial") && stamped(&name)
                };
                if ours {
                    s.recording_bytes += size(&p);
                    s.recordings.push(p);
                } else {
                    s.not_ours += 1;
                }
            }
            s.folders.push(dir);
        }
        if any_kind {
            s.folders.push(g);
        }
    }
}

fn scan_buffer(buffer: &Path, s: &mut Scan) {
    let Ok(sessions) = std::fs::read_dir(buffer) else { return };
    for sess in sessions.flatten() {
        let d = sess.path();
        let name = sess.file_name().to_string_lossy().into_owned();
        if !d.is_dir() || !stamped(&format!("{name}x")) {
            s.not_ours += usize::from(d.is_file());
            continue;
        }
        for f in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let fp = f.path();
            let n = f.file_name().to_string_lossy().to_ascii_lowercase();
            let ours = (n.starts_with("seg_") && n.ends_with(".ts")) || ["session.json", "markers.json", "markers.json.tmp", "ended.json"].contains(&n.as_str()) || (n.starts_with("review_") && n.ends_with(".mp4"));
            if ours && fp.is_file() {
                s.temporary_bytes += size(&fp);
                s.temporary.push(fp);
            } else {
                s.not_ours += 1;
            }
        }
        s.folders.push(d);
    }
    s.folders.push(buffer.to_path_buf());
}

pub fn fmt_bytes(b: u64) -> String {
    match b {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.0} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} KB", b.div_ceil(1024)),
    }
}

/// The confirmation text shown before anything is removed.
pub fn describe(s: &Scan) -> String {
    let mut t = String::new();
    if !s.recordings.is_empty() {
        t += &format!("Recordings CAPTURE made ({} files, {}) will be moved to the Recycle Bin from:\n", s.recordings.len(), fmt_bytes(s.recording_bytes));
        for r in &s.roots {
            t += &format!("    {}\\<game>\\Clips, Sessions, Screenshots\n", r.display());
        }
    } else {
        t += "No recordings made by CAPTURE were found.\n";
    }
    if !s.temporary.is_empty() {
        t += &format!("\nTemporary replay footage ({} files, {}) will be deleted.\n", s.temporary.len(), fmt_bytes(s.temporary_bytes));
    }
    if s.not_ours > 0 {
        t += &format!("\n{} other file(s) in these folders were not made by CAPTURE and will not be touched.\n", s.not_ours);
    }
    for u in &s.unavailable {
        t += &format!("\n{} is not available (drive unplugged?). Nothing there will be touched.\n", u.display());
    }
    t
}

/// Remove what `scan` found. Recordings go to the Recycle Bin (Windows may delete files too
/// large for it), temporary files are deleted, emptied folders are removed. Returns the paths
/// that could not be removed (for example because another program has them open).
pub fn remove(s: &Scan, recordings: bool) -> Vec<(PathBuf, String)> {
    let mut failed = Vec::new();
    for f in &s.temporary {
        if let Err(e) = std::fs::remove_file(f) {
            if f.exists() {
                failed.push((f.clone(), e.to_string()));
            }
        }
    }
    if recordings {
        for chunk in s.recordings.chunks(200) {
            if let Err(e) = recycle(chunk) {
                for f in chunk.iter().filter(|f| f.exists()) {
                    failed.push((f.clone(), e.clone()));
                }
            }
        }
    }
    let mut folders = s.folders.clone();
    folders.sort_by_key(|f| std::cmp::Reverse(f.components().count()));
    for d in folders {
        let _ = std::fs::remove_dir(d); // only succeeds when empty
    }
    failed
}

fn recycle(files: &[PathBuf]) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::*;
    let mut from: Vec<u16> = Vec::new();
    for f in files {
        from.extend(f.as_os_str().to_string_lossy().encode_utf16());
        from.push(0);
    }
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(format!("Recycle Bin error {r}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_with(tag: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(24u32.to_be_bytes());
        v.extend(b"ftypisom\0\0\x02\0isomiso2");
        let moov_len = 8 + tag.len() as u32;
        v.extend(moov_len.to_be_bytes());
        v.extend(b"moov");
        v.extend(tag);
        v.extend(16u32.to_be_bytes());
        v.extend(b"mdat01234567");
        v
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("capture-owned-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_captures_own_files_are_selected() {
        let root = temp("scan");
        let clips = root.join("Game A").join("Clips");
        let shots = root.join("Game A").join("Screenshots");
        let stems = clips.join("2026-11-08_10-00-00_Game-A_Clip_Audio");
        let buf = root.join("Buffer").join("2026-11-08_10-00-00_1234");
        for d in [&clips, &shots, &stems, &buf] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(clips.join("2026-11-08_10-00-00_Game-A_Clip.mp4"), mp4_with(b"..Preview Mix..")).unwrap();
        std::fs::write(clips.join("renamed by me.mp4"), mp4_with(b"..Recorded with CAPTURE..")).unwrap();
        std::fs::write(clips.join("renamed by me.mp4.markers.json"), b"[]").unwrap();
        std::fs::write(clips.join("someone elses.mp4"), mp4_with(b"..Lavf..")).unwrap();
        std::fs::write(clips.join("notes.txt"), b"mine").unwrap();
        std::fs::write(stems.join("manifest.json"), br#"{"capture_stems": 1}"#).unwrap();
        std::fs::write(stems.join("01 Game Audio.wav"), b"RIFF").unwrap();
        std::fs::write(stems.join("my mix.aup3"), b"x").unwrap();
        std::fs::write(shots.join("2026-11-08_10-00-05_Game-A.png"), b"png").unwrap();
        std::fs::write(shots.join("wallpaper.png"), b"png").unwrap();
        std::fs::write(buf.join("seg_000001.ts"), b"ts").unwrap();
        std::fs::write(buf.join("session.json"), b"{}").unwrap();
        // Unrelated folders in the recording root are never entered.
        std::fs::create_dir_all(root.join("Holiday")).unwrap();
        std::fs::write(root.join("Holiday").join("2026-11-08_10-00-00_x.mp4"), mp4_with(b"Preview Mix")).unwrap();

        let s = scan(&root, None);
        let names: Vec<String> = s.recordings.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(s.recordings.len(), 6, "{names:?}");
        for keep in ["someone elses.mp4", "notes.txt", "my mix.aup3", "wallpaper.png"] {
            assert!(!names.contains(&keep.to_string()), "{keep} must be kept");
        }
        assert_eq!(s.temporary.len(), 2);
        assert_eq!(s.not_ours, 4);

        let failed = remove(&Scan { recordings: vec![], ..s }, false);
        assert!(failed.is_empty());
        assert!(!root.join("Buffer").exists(), "emptied buffer folder removed");
        assert!(clips.join("notes.txt").exists() && root.join("Holiday").join("2026-11-08_10-00-00_x.mp4").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_root_is_reported_not_touched() {
        let s = scan(Path::new(r"Q:\Nope\Captures"), None);
        assert_eq!(s.unavailable.len(), 1);
        assert!(describe(&s).contains("not available"));
    }
}
