//! Filesystem-backed library: `<root>\<Game>\Clips|Sessions|Screenshots\*`. No private index:
//! the folders are the source of truth, so nothing is lost if CAPTURE's data is deleted.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

/// Read the installed Steam client's cached art. A missing Steam install, manifest or
/// artwork is normal; Library falls back to a screenshot or its own title card.
pub fn steam_art() -> std::collections::BTreeMap<String, PathBuf> {
    use windows::{core::w, Win32::System::Registry::*};
    let mut result = std::collections::BTreeMap::new();
    let mut buf = [0u16; 2048];
    let mut len = (buf.len() * 2) as u32;
    let value = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Valve\\Steam"), w!("SteamPath"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    if value.0 != 0 { return result; }
    let steam = PathBuf::from(String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())]));
    let Ok(cache) = steam.join("appcache/librarycache").canonicalize() else { return result; };
    let mut libraries = vec![steam.clone()];
    if let Ok(folders) = std::fs::read_to_string(steam.join("steamapps/libraryfolders.vdf")) {
        libraries.extend(folders.lines().filter_map(|line| vdf_field(line, "path")).map(PathBuf::from));
    }
    for root in libraries {
        for entry in std::fs::read_dir(root.join("steamapps")).into_iter().flatten().flatten() {
            let filename = entry.file_name();
            let Some(id) = filename.to_str().and_then(|s| s.strip_prefix("appmanifest_")).and_then(|s| s.strip_suffix(".acf")) else { continue; };
            if id.is_empty() || !id.bytes().all(|c| c.is_ascii_digit()) { continue; }
            let Some(image) = cached_art(&cache, id) else { continue; };
            let Ok(data) = std::fs::read_to_string(entry.path()) else { continue; };
            for label in ["name", "installdir"].into_iter().filter_map(|k| data.lines().find_map(|line| vdf_field(line, k))) {
                let key: String = label.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
                if !key.is_empty() { result.entry(key).or_insert_with(|| image.clone()); }
            }
        }
    }
    result
}

fn vdf_field(line: &str, key: &str) -> Option<String> {
    let line = line.trim_start().strip_prefix('"')?;
    let (field, value) = line.split_once('"')?;
    if field != key { return None; }
    let quoted = value.trim().strip_prefix('"')?.strip_suffix('"')?;
    Some(quoted.replace("\\\\", "\\").replace("\\\"", "\""))
}

fn cached_art(cache: &Path, id: &str) -> Option<PathBuf> {
    let dir = cache.join(id);
    let mut variants = vec![dir.clone()];
    variants.extend(std::fs::read_dir(&dir).into_iter().flatten().flatten()
        .filter(|d| d.file_type().is_ok_and(|t| t.is_dir())).map(|d| d.path()));
    variants.sort();
    let candidates = ["library_header.jpg", "library_hero.jpg"].into_iter()
        .flat_map(|name| variants.iter().map(move |dir| dir.join(name)))
        .chain([cache.join(format!("{id}_header.jpg")), cache.join(format!("{id}_library_hero.jpg"))]);
    candidates.filter_map(|p| p.canonicalize().ok()).find(|p| p.starts_with(cache) && p.is_file())
}

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
    let Ok(games) = std::fs::read_dir(root) else { return out };
    for g in games.flatten().filter(|e| e.path().is_dir()) {
        let game = g.file_name().to_string_lossy().into_owned();
        for f in std::fs::read_dir(g.path().join(dir)).into_iter().flatten().flatten() {
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
    // Windows names are case-insensitive: "clip" -> "Clip" is the same file, not a clash.
    let case_only = target.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy());
    if target.exists() && !case_only {
        return Err(format!("\"{clean}.{ext}\" already exists in this folder."));
    }
    move_no_replace(path, &target).map_err(|e| format!("Could not rename: {e}"))?;
    if sidecar(path).is_file() {
        let _ = std::fs::rename(sidecar(path), sidecar(&target));
    }
    if stems_dir(path).is_dir() && !stems_dir(&target).exists() {
        let _ = std::fs::rename(stems_dir(path), stems_dir(&target));
    }
    Ok(target)
}

/// Rename that fails instead of replacing an existing file (`std::fs::rename` replaces on
/// Windows), so a file that appears between the check and the rename is never overwritten.
fn move_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};
    unsafe { MoveFileExW(&HSTRING::from(from.as_os_str()), &HSTRING::from(to.as_os_str()), MOVE_FILE_FLAGS(0)) }
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
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
        // Case-only rename is allowed (same file on Windows).
        let t = rename(&t, "BOSS FIGHT").unwrap();
        assert!(std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name() == "BOSS FIGHT.mp4"));
        assert!(t.is_file());
        // The no-replace move itself refuses an existing target.
        assert!(move_no_replace(&t, &dir.join("taken.mp4")).is_err());
        assert_eq!(std::fs::read(dir.join("taken.mp4")).unwrap(), b"1");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
