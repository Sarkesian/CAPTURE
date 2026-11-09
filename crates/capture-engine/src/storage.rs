//! Capture-root layout, safe names and disk-space queries.
//!
//! ```text
//! <root>\Background Recording\<session>\seg_000001.ts       disposable ring (cap enforced)
//! <root>\<Game Name>\Clips\YYYY-MM-DD_HH-MM-SS_<Game>_Clip.mp4  permanent, never auto-deleted
//! <root>\<Game Name>\Sessions\...
//! <root>\<Game Name>\Screenshots\...
//! ```
//!
//! Each game folder is the parent of everything recorded for that game. Folder names that
//! would collide with CAPTURE's own top-level folders (`Background Recording`, and `Buffer`,
//! its name before 0.6.1) get a trailing `_`.

use std::path::{Path, PathBuf};

/// The folder that holds the background recording (the footage kept ready for Save Replay and
/// Replay): named for what it is, so someone who finds it in their recordings knows.
pub const BACKGROUND_DIR: &str = "Background Recording";
/// Its name before 0.6.1.
pub const LEGACY_BACKGROUND_DIR: &str = "Buffer";

pub fn buffer_dir(root: &Path) -> PathBuf {
    adopt_legacy(root);
    root.join(BACKGROUND_DIR)
}

/// The background-recording folder for a configuration: inside `buffer_root` if set, else
/// inside the recordings folder.
pub fn buffer_dir_for(root: &Path, buffer_root: Option<&str>) -> PathBuf {
    match buffer_root.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => buffer_dir(Path::new(b)),
        None => buffer_dir(root),
    }
}

/// A `Buffer` folder left by an earlier version becomes `Background Recording`, with whatever
/// footage is still in it (so "leftover footage" from before the update can still be saved).
/// Only when it is recognizably CAPTURE's: nothing in it but session folders. A folder a
/// person happens to have named `Buffer` is left alone.
fn adopt_legacy(parent: &Path) {
    let (old, new) = (parent.join(LEGACY_BACKGROUND_DIR), parent.join(BACKGROUND_DIR));
    if new.exists() || !old.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&old) else { return };
    let ours = entries.flatten().all(|e| e.path().is_dir() && is_session_dir_name(&e.file_name().to_string_lossy()));
    if ours {
        let _ = std::fs::rename(&old, &new);
    }
}

/// Starts with CAPTURE's `YYYY-MM-DD_HH-MM-SS_` stamp followed by at least one character, as
/// every recording, stem folder and buffer session CAPTURE creates does.
pub fn is_stamped(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() > 20
        && b[..19].iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 13 | 16 => *c == b'-',
            10 => *c == b'_',
            _ => c.is_ascii_digit(),
        })
        && b[19] == b'_'
}

/// True for a buffer session folder name CAPTURE creates: `YYYY-MM-DD_HH-MM-SS_<pid>`.
/// The buffer folder may sit inside a broad, user-chosen folder, so cleanup only ever touches
/// sub-folders with exactly this shape.
pub fn is_session_dir_name(name: &str) -> bool {
    is_stamped(name) && name.as_bytes()[20..].iter().all(u8::is_ascii_digit)
}

/// Written into a session folder when its recording ended normally: the footage is kept for
/// Replay until the next game is recorded (a folder without it is from a crash or power loss).
pub const ENDED_FILE: &str = "ended.json";

/// Files CAPTURE writes inside a buffer session folder.
pub fn is_session_file(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.starts_with("seg_") && n.ends_with(".ts"))
        || (n.starts_with("review_") && (n.ends_with(".mp4") || n.ends_with(".mp4.partial")))
        || matches!(n.as_str(), "session.json" | "markers.json" | "markers.json.tmp" | ENDED_FILE)
}

/// Delete CAPTURE's own files from a buffer session folder, then the folder if that left it
/// empty. Never recursive: anything else in the folder is kept (and so is the folder).
/// Returns the bytes freed.
pub fn remove_session_dir(dir: &Path) -> u64 {
    let mut freed = 0;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(m) = e.metadata() else { continue };
        if m.is_file() && is_session_file(&name) && std::fs::remove_file(e.path()).is_ok() {
            freed += m.len();
        }
    }
    let _ = std::fs::remove_dir(dir); // only succeeds when empty
    freed
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
                // Only CAPTURE's own recordings (all start with its timestamp): a broad capture
                // folder may hold an unrelated `Clips\<something>` folder of the user's.
                if !is_stamped(&f.file_name().to_string_lossy()) {
                    continue;
                }
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
    // Old thumbnail cache: only its `<clip stem>_<size>_<mtime>.jpg` files, never recursively.
    let data = root.join("Data");
    let thumbs = data.join("thumbs");
    for f in std::fs::read_dir(&thumbs).into_iter().flatten().flatten() {
        let name = f.file_name().to_string_lossy().into_owned();
        if is_stamped(&name) && name.to_ascii_lowercase().ends_with(".jpg") && f.path().is_file() {
            let _ = std::fs::remove_file(f.path());
        }
    }
    let _ = std::fs::remove_dir(&thumbs);
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
        || upper == "BUFFER"
        || upper == BACKGROUND_DIR.to_ascii_uppercase();
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
        assert_eq!(folder_name("Background Recording"), "Background Recording_");
        assert_eq!(folder_name("..."), "Unknown");
        assert_eq!(folder_name(""), "Unknown");
        assert_eq!(folder_name(&"x".repeat(200)).len(), 80);
    }

    #[test]
    fn an_old_buffer_folder_is_adopted_only_when_it_is_captures() {
        let root = std::env::temp_dir().join(format!("capture-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // CAPTURE's own: session folders only.
        let a = root.join("a");
        std::fs::create_dir_all(a.join("Buffer").join("2026-11-08_10-00-00_1234")).unwrap();
        assert_eq!(buffer_dir(&a), a.join("Background Recording"));
        assert!(a.join("Background Recording").join("2026-11-08_10-00-00_1234").is_dir());
        assert!(!a.join("Buffer").exists());
        // Someone's own folder that happens to be called Buffer: untouched.
        let b = root.join("b");
        std::fs::create_dir_all(b.join("Buffer")).unwrap();
        std::fs::write(b.join("Buffer").join("notes.txt"), b"mine").unwrap();
        assert_eq!(buffer_dir(&b), b.join("Background Recording"));
        assert!(b.join("Buffer").join("notes.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
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
        let (clip, stems, shot, dup) = (
            "2026-01-01_10-00-00_Game-A_Clip.mp4",
            "2026-01-01_10-00-00_Game-A_Clip_Audio",
            "2026-01-01_10-00-01_Game-A.png",
            "2026-01-01_10-00-02_Game-B_Session.mp4",
        );
        std::fs::create_dir_all(root.join("Clips").join("Game A").join(stems)).unwrap();
        std::fs::write(root.join("Clips").join("Game A").join(clip), b"1").unwrap();
        std::fs::create_dir_all(root.join("Screenshots").join("Game A")).unwrap();
        std::fs::write(root.join("Screenshots").join("Game A").join(shot), b"1").unwrap();
        // A user's own file in a same-named folder is not CAPTURE's and is not moved.
        std::fs::write(root.join("Clips").join("Game A").join("holiday.mp4"), b"user").unwrap();
        // A name already taken in the new place is left alone.
        std::fs::create_dir_all(root.join("Sessions").join("Game B")).unwrap();
        std::fs::write(root.join("Sessions").join("Game B").join(dup), b"old").unwrap();
        std::fs::create_dir_all(root.join("Game B").join("Sessions")).unwrap();
        std::fs::write(root.join("Game B").join("Sessions").join(dup), b"new").unwrap();
        std::fs::create_dir_all(root.join("Data").join("thumbs")).unwrap();
        std::fs::write(root.join("Data").join("thumbs").join("2026-01-01_00-00-00_G_Clip_1_2.jpg"), b"1").unwrap();
        // ...and a user's own file there keeps the folder.
        std::fs::write(root.join("Data").join("thumbs").join("cat.jpg"), b"user").unwrap();

        assert_eq!(migrate_layout(&root), 3);
        assert!(root.join("Game A").join("Clips").join(clip).is_file());
        assert!(root.join("Game A").join("Clips").join(stems).is_dir());
        assert!(root.join("Game A").join("Screenshots").join(shot).is_file());
        assert!(!root.join("Screenshots").exists());
        assert!(root.join("Clips").join("Game A").join("holiday.mp4").is_file(), "user file untouched");
        assert_eq!(std::fs::read(root.join("Game B").join("Sessions").join(dup)).unwrap(), b"new");
        assert!(root.join("Sessions").join("Game B").join(dup).is_file(), "never overwritten");
        assert!(root.join("Data").join("thumbs").join("cat.jpg").is_file(), "user file untouched");
        assert!(!root.join("Data").join("thumbs").join("2026-01-01_00-00-00_G_Clip_1_2.jpg").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn session_cleanup_only_touches_capture_files() {
        assert!(is_session_dir_name("2026-11-08_14-32-15_1234"));
        assert!(!is_session_dir_name("2026-11-08_14-32-15_"));
        assert!(!is_session_dir_name("My Projects"));
        assert!(!is_session_dir_name("2026-11-08_14-32-15_notes"));
        let dir = std::env::temp_dir().join(format!("capture-sess-{}", std::process::id())).join("2026-11-08_14-32-15_1");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["seg_000001.ts", "session.json", "markers.json", "review_001.mp4"] {
            std::fs::write(dir.join(f), b"12345").unwrap();
        }
        std::fs::write(dir.join("mine.txt"), b"keep").unwrap();
        assert_eq!(remove_session_dir(&dir), 20);
        assert!(dir.join("mine.txt").is_file() && dir.join("sub").is_dir(), "unrelated content kept");
        assert!(!dir.join("seg_000001.ts").exists());
        std::fs::remove_file(dir.join("mine.txt")).unwrap();
        std::fs::remove_dir(dir.join("sub")).unwrap();
        remove_session_dir(&dir);
        assert!(!dir.exists(), "empty folder removed");
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
