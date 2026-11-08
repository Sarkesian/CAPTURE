//! Filesystem-backed library: `<root>\Clips|Sessions|Screenshots\<Game>\*`. No private index:
//! the folders are the source of truth, so nothing is lost if CAPTURE's data is deleted.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ClipEntry {
    pub path: String,
    pub name: String,
    pub game: String,
    pub bytes: u64,
    pub modified_ms: u64,
    /// "clip" | "session" | "screenshot"
    pub kind: String,
    /// `<clip>_Audio\` (editor audio files) exists next to it.
    pub has_stems: bool,
    /// `<clip>.markers.json` exists next to it.
    pub has_markers: bool,
}

pub fn kind_dir(kind: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match kind {
        "clips" => Some(("Clips", "mp4", "clip")),
        "sessions" => Some(("Sessions", "mp4", "session")),
        "screenshots" => Some(("Screenshots", "png", "screenshot")),
        _ => None,
    }
}

pub fn list(root: &Path, kind: &str, limit: usize) -> Vec<ClipEntry> {
    let mut out = Vec::new();
    let Some((dir, ext, k)) = kind_dir(kind) else { return out };
    let Ok(games) = std::fs::read_dir(root.join(dir)) else { return out };
    for g in games.flatten().filter(|e| e.path().is_dir()) {
        let game = g.file_name().to_string_lossy().into_owned();
        for f in std::fs::read_dir(g.path()).into_iter().flatten().flatten() {
            let p = f.path();
            if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
                continue; // skips in-progress *.partial files and sidecars
            }
            let Ok(m) = f.metadata() else { continue };
            out.push(ClipEntry {
                has_stems: stems_dir(&p).is_dir(),
                has_markers: sidecar(&p).is_file(),
                path: p.to_string_lossy().into_owned(),
                name: p.file_name().unwrap().to_string_lossy().into_owned(),
                game: game.clone(),
                bytes: m.len(),
                modified_ms: m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0),
                kind: k.into(),
            });
        }
    }
    out.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    out.truncate(limit);
    out
}

pub fn stems_dir(clip: &Path) -> PathBuf {
    let stem = clip.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    clip.with_file_name(format!("{stem}_Audio"))
}

pub fn sidecar(clip: &Path) -> PathBuf {
    let mut s = clip.as_os_str().to_owned();
    s.push(".markers.json");
    PathBuf::from(s)
}

/// Rename a file in place (same folder, same extension) together with its sidecar and its
/// editor-audio folder. The new name is made file-system safe; existing files are never
/// overwritten.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let clean: String = new_name
        .trim()
        .trim_end_matches(&format!(".{ext}"))
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 32 { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let clean = clean.trim_end_matches(['.', ' ']).to_string();
    if clean.is_empty() {
        return Err("Enter a name.".into());
    }
    let target = path.with_file_name(format!("{clean}.{ext}"));
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err(format!("\"{clean}.{ext}\" already exists in this folder."));
    }
    std::fs::rename(path, &target).map_err(|e| format!("Could not rename: {e}"))?;
    if sidecar(path).is_file() {
        let _ = std::fs::rename(sidecar(path), sidecar(&target));
    }
    if stems_dir(path).is_dir() && !stems_dir(&target).exists() {
        let _ = std::fs::rename(stems_dir(path), stems_dir(&target));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_moves_sidecars_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("capture-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a_Audio")).unwrap();
        std::fs::write(dir.join("a.mp4"), b"1").unwrap();
        std::fs::write(dir.join("a.mp4.markers.json"), b"{}").unwrap();
        std::fs::write(dir.join("taken.mp4"), b"1").unwrap();
        assert!(rename(&dir.join("a.mp4"), "taken").is_err());
        let t = rename(&dir.join("a.mp4"), "Boss: fight?").unwrap();
        assert_eq!(t.file_name().unwrap(), "Boss fight.mp4");
        assert!(dir.join("Boss fight.mp4.markers.json").is_file());
        assert!(dir.join("Boss fight_Audio").is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
