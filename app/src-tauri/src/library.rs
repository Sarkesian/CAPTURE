//! Filesystem-backed clip list: `<root>\Clips\<Game>\*.mp4`. No private index — the folder is
//! the source of truth, so nothing is lost if CAPTURE's data is deleted.

use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ClipEntry {
    pub path: String,
    pub name: String,
    pub game: String,
    pub bytes: u64,
    pub modified_ms: u64,
}

pub fn list_clips(root: &Path, limit: usize) -> Vec<ClipEntry> {
    let mut out = Vec::new();
    let Ok(games) = std::fs::read_dir(root.join("Clips")) else { return out };
    for g in games.flatten().filter(|e| e.path().is_dir()) {
        let game = g.file_name().to_string_lossy().into_owned();
        for f in std::fs::read_dir(g.path()).into_iter().flatten().flatten() {
            let p = f.path();
            if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4")) {
                continue; // skips in-progress *.mp4.partial
            }
            let Ok(m) = f.metadata() else { continue };
            out.push(ClipEntry {
                path: p.to_string_lossy().into_owned(),
                name: p.file_name().unwrap().to_string_lossy().into_owned(),
                game: game.clone(),
                bytes: m.len(),
                modified_ms: m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0),
            });
        }
    }
    out.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    out.truncate(limit);
    out
}
