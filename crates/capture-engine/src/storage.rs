//! Capture-root layout, safe names and disk-space queries.
//!
//! ```text
//! <root>\Buffer\<session>\seg_000001.ts   disposable ring (cap enforced)
//! <root>\Clips\<Game Name>\YYYY-MM-DD_HH-MM-SS_<Game>_Clip.mp4   permanent, never auto-deleted
//! <root>\Sessions\<Game Name>\...          (Phase 3)
//! <root>\Data\                             small rebuildable settings/index
//! ```

use std::path::{Path, PathBuf};

pub fn buffer_dir(root: &Path) -> PathBuf {
    root.join("Buffer")
}

pub fn clips_dir(root: &Path, game: &str) -> PathBuf {
    root.join("Clips").join(folder_name(game))
}

pub fn sessions_dir(root: &Path, game: &str) -> PathBuf {
    root.join("Sessions").join(folder_name(game))
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
            && upper.as_bytes()[3].is_ascii_digit());
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
