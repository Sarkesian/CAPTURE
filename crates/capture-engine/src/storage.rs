//! Capture-root layout, safe names and disk-space queries.
//!
//! ```text
//! <root>\Buffer\<session>\seg_000001.ts                     disposable ring (cap enforced)
//! <root>\<Game Name>\Clips\YYYY-MM-DD_HH-MM-SS_<Game>_Clip.mp4  permanent, never auto-deleted
//! <root>\<Game Name>\Sessions\...
//! <root>\<Game Name>\Screenshots\...
//! ```
//!
//! Each game folder is the parent of everything recorded for that game. Folder names that
//! would collide with CAPTURE's own top-level folders (`Buffer`) get a trailing `_`.

use std::path::{Path, PathBuf};

pub fn buffer_dir(root: &Path) -> PathBuf {
    root.join("Buffer")
}

/// The buffer folder for a configuration: `<buffer_root>\Buffer` if set, else `<root>\Buffer`.
pub fn buffer_dir_for(root: &Path, buffer_root: Option<&str>) -> PathBuf {
    match buffer_root.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => Path::new(b).join("Buffer"),
        None => buffer_dir(root),
    }
}

/// Sub-folders inside each game folder, in display order.
pub const KINDS: [&str; 3] = ["Clips", "Sessions", "Screenshots"];

/// `<root>\<Game>` — the parent folder for all of a game's recordings.
pub fn game_dir(root: &Path, game: &str) -> PathBuf {
    root.join(folder_name(game))
}

pub fn clips_dir(root: &Path, game: &str) -> PathBuf {
    game_dir(root, game).join("Clips")
}

pub fn sessions_dir(root: &Path, game: &str) -> PathBuf {
    game_dir(root, game).join("Sessions")
}

pub fn screenshots_dir(root: &Path, game: &str) -> PathBuf {
    game_dir(root, game).join("Screenshots")
}

/// One-time move from the old layout (`<root>\Clips|Sessions|Screenshots\<Game>\*`) to the
/// game-first layout (`<root>\<Game>\Clips|Sessions|Screenshots\*`), and removal of the old
/// `Data	humbs` cache. Files are renamed (same volume), never overwritten: a name already
/// taken in the new folder stays where it is. Old folders are removed only once empty.
/// Returns the number of entries moved.
pub fn migrate_layout(root: &Path) -> usize {
    let mut moved = 0;
    for kind in KINDS {
        let old_kind = root.join(kind);
        let Ok(games) = std::fs::read_dir(&old_kind) else { continue };
        for g in games.flatten().filter(|e| e.path().is_dir()) {
            let game = g.file_name().to_string_lossy().into_owned();
            let dest = game_dir(root, &game).join(kind);
            for f in std::fs::read_dir(g.path()).into_iter().flatten().flatten() {
                let target = dest.join(f.file_name());
                if target.exists() || std::fs::create_dir_all(&dest).is_err() {
                    continue;
                }
                if std::fs::rename(f.path(), &target).is_ok() {
                    moved += 1;
                }
            }
            let _ = std::fs::remove_dir(g.path()); // only succeeds when empty
        }
        let _ = std::fs::remove_dir(&old_kind);
    }
    let data = root.join("Data");
    let _ = std::fs::remove_dir_all(data.join("thumbs"));
    let _ = std::fs::remove_dir(&data);
    moved
}

/// Folder-safe, human-readable name: keeps spaces and most punctuation, strips characters
/// Windows forbids, trailing dots/spaces and reserved device names. Never empty.
pub fn folder_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if (c as u32) < 32 => ' ',
            c => c,
        })
        .collect();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_end_matches(['.', ' ']).to_string();
    let s: String = s.chars().take(80).collect();
    let s = s.trim_end_matches(['.', ' ']).to_string();
    let upper = s.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|r| upper == *r)
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit())
        || upper == "BUFFER";
    if s.is_empty() {
        "Unknown".into()
    } else if reserved {
        format!("{s}_")
    } else {
        s
    }
}

/// File-name token: folder-safe name with spaces turned into dashes.
pub fn file_token(name: &str) -> String {
    folder_name(name).replace(' ', "-")
}

/// Local wall-clock timestamp `YYYY-MM-DD_HH-MM-SS`.
pub fn local_stamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

pub fn clip_file_name(stamp: &str, game: &str, kind: &str) -> String {
    format!("{stamp}_{}_{kind}.mp4", file_token(game))
}

/// Atomically reserve a unique clip path: creates `<name>.partial` with create-new semantics so
/// two saves finishing in the same second can never pick the same file. Returns the final path.
pub fn reserve_unique_path(dir: &Path, file_name: &str) -> std::io::Result<PathBuf> {
    loop {
        let p = unique_path(dir, file_name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(partial_path(&p)) {
            Ok(_) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// First path among `name`, `name (2)`, `name (3)`… that exists neither as the final file nor
/// as an in-progress `.partial` file.
pub fn unique_path(dir: &Path, file_name: &str) -> PathBuf {
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (file_name.to_string(), String::new()),
    };
    for n in 1.. {
        let candidate = if n == 1 { format!("{stem}{ext}") } else { format!("{stem} ({n}){ext}") };
        let p = dir.join(&candidate);
        if !p.exists() && !partial_path(&p).exists() {
            return p;
        }
    }
    unreachable!()
}

pub fn partial_path(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_owned();
    s.push(".partial");
    PathBuf::from(s)
}

/// Free bytes available to the caller on the volume holding `path` (walks up to an existing
/// ancestor so it works before the capture root exists).
pub fn free_bytes(path: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(p.as_os_str()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => total += dir_size(&e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_safe_and_readable() {
        assert_eq!(folder_name("The Witcher 3: Wild Hunt"), "The Witcher 3 Wild Hunt");
        assert_eq!(folder_name("  a/b\\c?.  "), "a b c");
        assert_eq!(folder_name("CON"), "CON_");
        assert_eq!(folder_name("com1"), "com1_");
        assert_eq!(folder_name("Company"), "Company");
        assert_eq!(folder_name("Buffer"), "Buffer_");
        assert_eq!(folder_name("..."), "Unknown");
        assert_eq!(folder_name(""), "Unknown");
        assert_eq!(folder_name(&"x".repeat(200)).len(), 80);
    }

    #[test]
    fn clip_names_follow_contract() {
        assert_eq!(
            clip_file_name("2026-11-08_14-32-15", "Cyberpunk 2077", "Clip"),
            "2026-11-08_14-32-15_Cyberpunk-2077_Clip.mp4"
        );
    }

    #[test]
    fn game_folder_is_the_parent() {
        let r = Path::new(r"G:\Captures");
        assert_eq!(clips_dir(r, "The Witcher 3"), Path::new(r"G:\Captures\The Witcher 3\Clips"));
        assert_eq!(sessions_dir(r, "The Witcher 3"), Path::new(r"G:\Captures\The Witcher 3\Sessions"));
        assert_eq!(screenshots_dir(r, "The Witcher 3"), Path::new(r"G:\Captures\The Witcher 3\Screenshots"));
    }

    #[test]
    fn migrate_moves_old_layout_and_drops_thumbs() {
        let root = std::env::temp_dir().join(format!("capture-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Clips").join("Game A").join("x_Audio")).unwrap();
        std::fs::write(root.join("Clips").join("Game A").join("x.mp4"), b"1").unwrap();
        std::fs::create_dir_all(root.join("Screenshots").join("Game A")).unwrap();
        std::fs::write(root.join("Screenshots").join("Game A").join("s.png"), b"1").unwrap();
        // A name already taken in the new place is left alone.
        std::fs::create_dir_all(root.join("Sessions").join("Game B")).unwrap();
        std::fs::write(root.join("Sessions").join("Game B").join("dup.mp4"), b"old").unwrap();
        std::fs::create_dir_all(root.join("Game B").join("Sessions")).unwrap();
        std::fs::write(root.join("Game B").join("Sessions").join("dup.mp4"), b"new").unwrap();
        std::fs::create_dir_all(root.join("Data").join("thumbs")).unwrap();
        std::fs::write(root.join("Data").join("thumbs").join("t.jpg"), b"1").unwrap();

        assert_eq!(migrate_layout(&root), 3);
        assert!(root.join("Game A").join("Clips").join("x.mp4").is_file());
        assert!(root.join("Game A").join("Clips").join("x_Audio").is_dir());
        assert!(root.join("Game A").join("Screenshots").join("s.png").is_file());
        assert!(!root.join("Clips").exists() && !root.join("Screenshots").exists());
        assert_eq!(std::fs::read(root.join("Game B").join("Sessions").join("dup.mp4")).unwrap(), b"new");
        assert!(root.join("Sessions").join("Game B").join("dup.mp4").is_file(), "never overwritten");
        assert!(!root.join("Data").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unique_path_skips_existing_and_partial() {
        let dir = std::env::temp_dir().join(format!("capture-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = unique_path(&dir, "x.mp4");
        assert_eq!(a.file_name().unwrap(), "x.mp4");
        std::fs::write(&a, b"1").unwrap();
        std::fs::write(partial_path(&dir.join("x (2).mp4")), b"1").unwrap();
        assert_eq!(unique_path(&dir, "x.mp4").file_name().unwrap(), "x (3).mp4");
        let r1 = reserve_unique_path(&dir, "y.mp4").unwrap();
        let r2 = reserve_unique_path(&dir, "y.mp4").unwrap();
        assert_ne!(r1, r2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
