//! Rebuildable media catalog. Files remain authoritative; irreplaceable events also have a
//! durable append-only journal. All deletion and lease registration share the same mutex.
//!
//! Two levels of identity:
//! - A **recording** (`sessions` row, one folder of segments) is what one capture pipeline
//!   wrote: fixed game, resolution and audio layout, media time 0 at its start. A new one starts
//!   whenever the game is relaunched or changes resolution.
//! - A **session** (`grp`) is what the player sees: consecutive recordings of the same game
//!   with only short breaks between them, on one wall-clock timeline (seconds since the
//!   session began). Breaks appear as gaps. Everything the UI sends or receives is in session
//!   time; the functions ending in `_at` translate to a recording and its media time.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use capture_protocol::{FrameTiming, HistoryPart, HistorySession, HistoryState, Review, SavedClip, TimelineEvent};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{save, storage, thumb};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
static NEXT: AtomicU64 = AtomicU64::new(1);
static STORES: OnceLock<Mutex<HashMap<PathBuf, Weak<Catalog>>>> = OnceLock::new();

/// Playback windows: a seek opens a short one (ready fast), playback continues in long ones.
pub const QUICK_SECS: f64 = 4.0;
pub const QUICK_BYTES: u64 = 32 * 1024 * 1024;
pub const WINDOW_SECS: f64 = 30.0;
pub const WINDOW_BYTES: u64 = 96 * 1024 * 1024;
/// Soft size of the playback cache. Opening a window never fails because of it: the least
/// recently used windows are dropped instead (the newest `PREVIEW_KEEP` always stay, since one
/// of them is on screen and another is the one about to play).
pub const PREVIEW_BUDGET: u64 = 512 * 1024 * 1024;
const PREVIEW_KEEP: usize = 6;
const PREVIEW_IDLE_MS: u64 = 10 * 60 * 1000;
/// How long a released window survives housekeeping, so a player still reading it is safe.
const PREVIEW_RELEASE_GRACE_MS: u64 = 60 * 1000;
/// A break shorter than this between two recordings of the same game continues the session.
pub const SESSION_GAP_MS: i64 = 15 * 60 * 1000;
const THUMB_WIDTH: i32 = 320;
const THUMB_CACHE_BYTES: usize = 12 * 1024 * 1024;

pub fn wait_for_saves(timeout: std::time::Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let stores: Vec<_> = STORES.get_or_init(Default::default).lock().unwrap().values().filter_map(Weak::upgrade).collect();
        let active = stores.iter().any(|c| {
            c.db.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='save' AND state='active')", [], |r| r.get::<_, bool>(0)).unwrap_or(true)
        });
        if !active {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

type ThumbKey = (String, u64, i64, bool);

#[derive(Default)]
struct ThumbCache {
    items: std::collections::VecDeque<(ThumbKey, f64, Arc<str>)>,
    bytes: usize,
}

pub struct Catalog {
    db: Mutex<Connection>,
    root: PathBuf,
    imports: Mutex<Vec<(String, PathBuf)>>,
    importing: std::sync::atomic::AtomicBool,
    scan: AtomicU64,
    reason: Mutex<Option<String>>,
    /// Bumped by every change the published state depends on.
    rev: AtomicU64,
    state_cache: Mutex<Option<(u64, u32, HistoryState)>>,
    /// One playback window is built at a time (they share one disk and one cache budget).
    build: Mutex<()>,
    clock: AtomicU64,
    thumbs: Mutex<ThumbCache>,
    pruned: Mutex<std::time::Instant>,
}

#[derive(Clone, Debug)]
pub struct Media {
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
    pub bytes: u64,
    pub seq: u64,
}

/// Footage held for a save or a playback window. `session` is a recording id and the times are
/// its media times.
pub struct Lease {
    catalog: Arc<Catalog>,
    pub id: String,
    pub session: String,
    pub start: f64,
    pub end: f64,
    keep: bool,
}
impl Lease {
    pub fn failed(&mut self, message: &str) {
        self.keep = true;
        let _ = self.catalog.db.lock().unwrap().execute("UPDATE jobs SET state='failed',error=?2 WHERE id=?1", params![self.id, message]);
        self.catalog.touch();
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.keep {
            let _ = self.catalog.db.lock().unwrap().execute("DELETE FROM jobs WHERE id=?1", [&self.id]);
        }
    }
}

/// What an Instant Replay save will contain: the recording, its media range, and (when it is
/// shorter than asked for) a plain-language reason.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayPick {
    pub session: String,
    pub start: f64,
    pub end: f64,
    pub note: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClipKind {
    Clip,
    /// A full recording the player started and stopped (`<Game>\Sessions`).
    Session,
}

struct Part {
    id: String,
    offset: f64,
}

fn merge(intervals: &mut Vec<[f64; 2]>, a: f64, b: f64, tolerance: f64) {
    match intervals.last_mut().filter(|l| a <= l[1] + tolerance) {
        Some(last) => last[1] = last[1].max(b),
        None => intervals.push([a, b]),
    }
}

fn fmt_len(secs: f64) -> String {
    crate::fmt_secs(secs.max(1.0))
}

impl Catalog {
    pub fn import_complete(&self) -> bool {
        !self.importing.load(Ordering::Acquire)
    }

    fn touch(&self) {
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    /// A strictly increasing "last used" stamp (milliseconds, never repeating).
    fn stamp(&self) -> u64 {
        let now = millis();
        self.clock.fetch_max(now, Ordering::SeqCst);
        self.clock.fetch_add(1, Ordering::SeqCst)
    }

    pub fn open(root: &Path) -> Result<Arc<Self>> {
        std::fs::create_dir_all(root).map_err(err)?;
        let key = PathBuf::from(std::fs::canonicalize(root).map_err(err)?.to_string_lossy().to_ascii_lowercase());
        let mut stores = STORES.get_or_init(Default::default).lock().unwrap();
        if let Some(c) = stores.get(&key).and_then(Weak::upgrade) {
            return Ok(c);
        }
        let db = Connection::open(root.join("history.sqlite")).map_err(err)?;
        db.busy_timeout(std::time::Duration::from_secs(5)).map_err(err)?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(err)?;
        if version > 4 {
            return Err("This history was created by a newer CAPTURE version; it has not been modified".into());
        }
        if version == 1 {
            db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE previews ADD COLUMN session TEXT; ALTER TABLE previews ADD COLUMN start REAL; ALTER TABLE previews ADD COLUMN end REAL; ALTER TABLE previews ADD COLUMN refs INTEGER NOT NULL DEFAULT 1; COMMIT;").map_err(err)?;
        }
        if version > 0 && version < 3 {
            db.execute_batch("ALTER TABLE sessions ADD COLUMN ended_ms INTEGER;").map_err(err)?;
        }
        if version > 0 && version < 4 {
            db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE sessions ADD COLUMN grp TEXT; ALTER TABLE sessions ADD COLUMN grp_ms INTEGER; ALTER TABLE previews ADD COLUMN used INTEGER NOT NULL DEFAULT 0; COMMIT;").map_err(err)?;
        }
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,dir TEXT NOT NULL UNIQUE,game TEXT NOT NULL,created_ms INTEGER NOT NULL,ended INTEGER NOT NULL,config TEXT NOT NULL,managed INTEGER NOT NULL DEFAULT 0,ended_ms INTEGER,grp TEXT,grp_ms INTEGER);
          CREATE TABLE IF NOT EXISTS epochs(session TEXT NOT NULL REFERENCES sessions(id),epoch INTEGER NOT NULL,media_start REAL NOT NULL,config TEXT NOT NULL,PRIMARY KEY(session,epoch));
          CREATE TABLE IF NOT EXISTS segments(session TEXT NOT NULL REFERENCES sessions(id),seq INTEGER NOT NULL,path TEXT NOT NULL UNIQUE,start REAL NOT NULL,end REAL,bytes INTEGER NOT NULL,complete INTEGER NOT NULL,PRIMARY KEY(session,seq));
          CREATE INDEX IF NOT EXISTS segment_time ON segments(session,start,end);
          CREATE TABLE IF NOT EXISTS events(session TEXT NOT NULL REFERENCES sessions(id),id TEXT NOT NULL,data TEXT NOT NULL,PRIMARY KEY(session,id));
          CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,session TEXT NOT NULL,start REAL NOT NULL,end REAL NOT NULL,kind TEXT NOT NULL,state TEXT NOT NULL,error TEXT);
          CREATE INDEX IF NOT EXISTS job_ranges ON jobs(session,start,end);
          CREATE TABLE IF NOT EXISTS pins(session TEXT NOT NULL,id TEXT NOT NULL,start REAL NOT NULL,end REAL NOT NULL,PRIMARY KEY(session,id));
          CREATE TABLE IF NOT EXISTS previews(path TEXT PRIMARY KEY,bytes INTEGER NOT NULL,session TEXT,start REAL,end REAL,refs INTEGER NOT NULL DEFAULT 1,used INTEGER NOT NULL DEFAULT 0);
          CREATE INDEX IF NOT EXISTS session_group ON sessions(grp,created_ms);
          PRAGMA user_version=4;
          UPDATE sessions SET ended=1;
          UPDATE jobs SET state='failed',error='Recording interrupted; source retained for retry' WHERE kind='save' AND state='active';
          DELETE FROM jobs WHERE kind='playback';
          DELETE FROM pins WHERE id LIKE 'session:%';",
        )
        .map_err(err)?;
        let c = Arc::new(Self {
            db: Mutex::new(db),
            root: root.to_path_buf(),
            imports: Mutex::new(vec![]),
            importing: std::sync::atomic::AtomicBool::new(false),
            scan: AtomicU64::new(0),
            reason: Mutex::new(None),
            rev: AtomicU64::new(1),
            state_cache: Mutex::new(None),
            build: Mutex::new(()),
            clock: AtomicU64::new(millis()),
            thumbs: Mutex::new(ThumbCache::default()),
            pruned: Mutex::new(std::time::Instant::now()),
        });
        // Stale preview files are ours, never user media. Do not delete them until accounted.
        c.reconcile()?;
        stores.insert(key, Arc::downgrade(&c));
        Ok(c)
    }

    /// Give every recording without a session one: the session of the recording before it when
    /// that is the same game and ended less than `SESSION_GAP_MS` earlier, otherwise its own.
    fn assign_groups(db: &Connection) -> Result<()> {
        let rows = {
            let mut q = db
                .prepare("SELECT id,game,created_ms,ended_ms,grp,grp_ms,(SELECT MAX(end) FROM segments g WHERE g.session=s.id) FROM sessions s ORDER BY created_ms,id")
                .map_err(err)?;
            let rows = q
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<i64>>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<i64>>(5)?, r.get::<_, Option<f64>>(6)?)))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            rows
        };
        // (game, wall-clock end, session id, session start)
        let mut previous: Option<(String, i64, String, i64)> = None;
        for (id, game, created, ended_ms, grp, grp_ms, last) in rows {
            let end = ended_ms.unwrap_or(created + (last.unwrap_or(0.0) * 1000.0) as i64).max(created);
            let (grp, grp_ms) = match (grp, grp_ms) {
                (Some(g), Some(ms)) => (g, ms),
                _ => {
                    let joined = previous.as_ref().filter(|p| p.0 == game && created - p.1 <= SESSION_GAP_MS && created >= p.3).map(|p| (p.2.clone(), p.3));
                    let (g, ms) = joined.unwrap_or((id.clone(), created));
                    db.execute("UPDATE sessions SET grp=?2,grp_ms=?3 WHERE id=?1", params![id, g, ms]).map_err(err)?;
                    (g, ms)
                }
            };
            previous = Some((game, end, grp, grp_ms));
        }
        Ok(())
    }

    pub fn register(&self, dir: &Path, game: &str, config: &serde_json::Value, managed: bool) -> Result<String> {
        let id = dir.file_name().ok_or("Session has no identity")?.to_string_lossy().to_string();
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        let created = config["started_unix_ms"].as_u64().unwrap_or_else(millis);
        tx.execute(
            "INSERT INTO sessions(id,dir,game,created_ms,ended,config,managed) VALUES(?1,?2,?3,?4,0,?5,?6) ON CONFLICT(id) DO UPDATE SET ended=0,managed=MAX(managed,excluded.managed)",
            params![id, dir.to_string_lossy(), game, created as i64, config.to_string(), managed],
        )
        .map_err(err)?;
        tx.execute("INSERT OR IGNORE INTO epochs VALUES(?1,0,0,?2)", params![id, config.to_string()]).map_err(err)?;
        Self::assign_groups(&tx)?;
        tx.commit().map_err(err)?;
        self.touch();
        Ok(id)
    }

    pub fn segment(&self, session: &str, seq: u64, path: &Path, start: f64, end: Option<f64>, bytes: u64) -> Result<()> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO segments VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(session,seq) DO UPDATE SET end=excluded.end,bytes=excluded.bytes,complete=excluded.complete",
                params![session, seq as i64, path.to_string_lossy(), start, end, bytes as i64, end.is_some()],
            )
            .map_err(err)?;
        if end.is_some() {
            self.touch();
        }
        Ok(())
    }

    pub fn end(&self, session: &str) {
        let _ = self.db.lock().unwrap().execute("UPDATE sessions SET ended=1,ended_ms=COALESCE(ended_ms,?2) WHERE id=?1", params![session, millis() as i64]);
        self.touch();
    }

    pub fn pin(&self, session: &str, id: &str, a: f64, b: f64) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT OR REPLACE INTO pins VALUES(?1,?2,?3,?4)", params![session, id, a, b]).map_err(err)?;
        Ok(())
    }

    pub fn unpin(&self, session: &str, id: &str) {
        let _ = self.db.lock().unwrap().execute("DELETE FROM pins WHERE session=?1 AND id=?2", params![session, id]);
    }

    pub fn event(&self, session: &str, event: &TimelineEvent) -> Result<()> {
        let db = self.db.lock().unwrap();
        let dir: String = db.query_row("SELECT dir FROM sessions WHERE id=?1", [session], |r| r.get(0)).map_err(err)?;
        let line = serde_json::to_string(event).map_err(err)?;
        // Journal first: a database rebuild can replay every event, including tombstones.
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new().create(true).read(true).append(true).open(Path::new(&dir).join("events.jsonl")).map_err(err)?;
        if f.metadata().map_err(err)?.len() > 0 {
            f.seek(SeekFrom::End(-1)).map_err(err)?;
            let mut tail = [0u8];
            f.read_exact(&mut tail).map_err(err)?;
            if tail[0] != b'\n' {
                f.write_all(b"\n").map_err(err)?;
            }
        }
        writeln!(f, "{line}").and_then(|_| f.sync_all()).map_err(err)?;
        db.execute("INSERT OR REPLACE INTO events VALUES(?1,?2,?3)", params![session, event.id, line]).map_err(err)?;
        self.touch();
        Ok(())
    }

    pub fn import_failed(&self, message: &str) {
        *self.reason.lock().unwrap() = Some(format!("Earlier recordings could not be fully indexed: {message}. Original media is kept"));
        self.importing.store(false, Ordering::Release);
        self.touch();
    }

    pub fn lease(self: &Arc<Self>, session: &str, start: f64, end: f64, kind: &str) -> Result<Lease> {
        if !start.is_finite() || !end.is_finite() || end <= start {
            return Err("Choose a valid recording range".into());
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM segments WHERE session=?1 AND start<?3 AND COALESCE(end,1e99)>?2)", params![session, start, end], |r| r.get(0)).map_err(err)?;
        if !exists {
            return Err("That footage is no longer available".into());
        }
        let id = format!("{}-{}", millis(), NEXT.fetch_add(1, Ordering::Relaxed));
        // Audio for boundary frames can be in adjacent segments. Protect before acknowledging.
        tx.execute("INSERT INTO jobs VALUES(?1,?2,?3,?4,?5,'active',NULL)", params![id, session, start - 3.0, end + 3.0, kind]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(Lease { catalog: self.clone(), id, session: session.into(), start, end, keep: false })
    }

    pub fn media(&self, session: &str, a: f64, b: f64) -> Result<Vec<Media>> {
        let db = self.db.lock().unwrap();
        Self::media_in(&db, session, a, b)
    }

    fn media_in(db: &Connection, session: &str, a: f64, b: f64) -> Result<Vec<Media>> {
        let mut q = db.prepare_cached("SELECT path,start,end,bytes,seq FROM segments WHERE session=?1 AND complete=1 AND start<?3 AND end>?2 ORDER BY start").map_err(err)?;
        let rows = q
            .query_map(params![session, a, b], |r| Ok(Media { path: PathBuf::from(r.get::<_, String>(0)?), start: r.get(1)?, end: r.get(2)?, bytes: r.get::<_, i64>(3)? as u64, seq: r.get::<_, i64>(4)? as u64 }))
            .map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(err)
    }

    fn details(&self, session: &str) -> Result<(String, Vec<String>)> {
        let (game, config): (String, String) = self.db.lock().unwrap().query_row("SELECT game,config FROM sessions WHERE id=?1", [session], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
        let v: serde_json::Value = serde_json::from_str(&config).map_err(err)?;
        Ok((game, v["audio_tracks"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()))
    }

    // ---- Session time <-> recording media time -------------------------------------------

    /// The recordings of a session in order, with the session time of their media time 0.
    /// Also accepts a recording id (its session is used).
    fn parts(db: &Connection, session: &str) -> Result<Vec<Part>> {
        let grp: Option<String> = db.query_row("SELECT grp FROM sessions WHERE grp=?1 OR id=?1 ORDER BY grp=?1 DESC LIMIT 1", [session], |r| r.get(0)).optional().map_err(err)?.flatten();
        let grp = grp.ok_or("That recording is no longer available")?;
        let mut q = db.prepare_cached("SELECT id,created_ms,grp_ms FROM sessions WHERE grp=?1 ORDER BY created_ms,id").map_err(err)?;
        let rows = q.query_map([grp], |r| Ok(Part { id: r.get(0)?, offset: (r.get::<_, i64>(1)? - r.get::<_, i64>(2)?) as f64 / 1000.0 })).map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(err)
    }

    /// The recording that holds session time `t`, and its offset (`media = t - offset`).
    pub fn locate(&self, session: &str, t: f64) -> Result<(String, f64)> {
        let db = self.db.lock().unwrap();
        let parts = Self::parts(&db, session)?;
        let part = parts.iter().rev().find(|p| p.offset <= t + 0.000_5).or(parts.first()).ok_or("That recording is no longer available")?;
        Ok((part.id.clone(), part.offset))
    }

    pub fn lease_at(self: &Arc<Self>, session: &str, start: f64, end: f64, kind: &str) -> Result<Lease> {
        let (recording, offset) = self.locate(session, start)?;
        self.lease(&recording, start - offset, end - offset, kind)
    }

    pub fn frame_timing_at(&self, session: &str, at: f64) -> Result<Vec<FrameTiming>> {
        let (recording, offset) = self.locate(session, at)?;
        let mut frames = self.frame_timing(&recording, at - offset)?;
        for f in &mut frames {
            f.start += offset;
            f.end += offset;
        }
        Ok(frames)
    }

    pub fn frame_timing(&self, session: &str, at: f64) -> Result<Vec<FrameTiming>> {
        if !at.is_finite() {
            return Err("Choose a time on the recording".into());
        }
        let segments = self.media(session, at - 4.0, at + 4.0)?;
        let index = segments.iter().rposition(|segment| segment.start <= at).unwrap_or(0);
        let mut frames = Vec::new();
        for segment in segments.iter().skip(index.saturating_sub(1)).take(if index == 0 { 2 } else { 3 }) {
            frames.extend(save::frame_timing(&segment.path, segment.start)?);
        }
        frames.sort_by(|a, b| a.start.total_cmp(&b.start));
        frames.dedup_by(|a, b| (a.start - b.start).abs() < 0.000_001);
        if frames.is_empty() {
            return Err("Frame timing is unavailable here".into());
        }
        Ok(frames)
    }

    // ---- Saving ---------------------------------------------------------------------------

    /// Has footage up to media time `end` reached the disk, together with the audio that
    /// arrives slightly later (it lands in the following segment)?
    pub fn ready_through(&self, session: &str, end: f64) -> bool {
        self.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM segments WHERE session=?1 AND complete=1 AND start>=?2) OR (SELECT ended FROM sessions WHERE id=?1)=1 AND NOT EXISTS(SELECT 1 FROM segments WHERE session=?1 AND complete=0)",
                params![session, end],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(true)
    }

    /// A clip already saved from exactly this footage (same end, same start once aligned to its
    /// keyframe) whose file still exists. Saving it again would only create a duplicate.
    pub fn existing_clip(&self, session: &str, start: f64, end: f64) -> Option<SavedClip> {
        let events = self.events(session).ok()?;
        let hit = events.iter().rev().find(|e| {
            e.kind == "clip" && e.end.is_some_and(|b| (b - end).abs() < 0.05) && e.t <= start + 0.05 && start - e.t < 2.5 && e.path.as_deref().is_some_and(|p| Path::new(p).is_file())
        })?;
        let path = hit.path.clone()?;
        Some(SavedClip {
            save_id: 0,
            requested_secs: end - start,
            duration_secs: end - hit.t,
            bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            video_streams: 1,
            audio_tracks: vec![],
            existing: true,
            path,
        })
    }

    /// Which footage "save the last `secs`" means for the recording `session` at its media time
    /// `end`. Normally the last `secs` of that recording. When the recording is younger than
    /// that, the session's earlier recordings are considered too (a game that changes
    /// resolution or restarts begins a new recording): a clip is one continuous piece at one
    /// resolution, so the longest piece inside the replay window is saved — preferring the most
    /// recent one when it is at least half as long — and `note` says why it is shorter.
    pub fn replay_pick(&self, session: &str, end: f64, secs: f64) -> Result<ReplayPick> {
        let db = self.db.lock().unwrap();
        let parts = Self::parts(&db, session)?;
        let current = parts.iter().find(|p| p.id == session).ok_or("That recording is no longer available")?;
        let (w1, w0) = (current.offset + end, current.offset + end - secs);
        // (recording, start, end) in session time, clipped to the replay window.
        let mut pieces: Vec<(usize, f64, f64)> = vec![];
        for (i, part) in parts.iter().enumerate().filter(|(_, p)| p.offset < w1) {
            let mut q = db.prepare_cached("SELECT start,end FROM segments WHERE session=?1 AND start<?3 AND COALESCE(end,1e99)>?2 ORDER BY start").map_err(err)?;
            let rows = q.query_map(params![part.id, w0 - part.offset, w1 - part.offset], |r| Ok((r.get::<_, f64>(0)?, r.get::<_, Option<f64>>(1)?))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
            let mut intervals: Vec<[f64; 2]> = vec![];
            for (a, b) in rows {
                // The segment still being written reaches "now" (only the live recording has one).
                let b = b.unwrap_or(if part.id == session { end } else { a });
                merge(&mut intervals, a, b, 0.1);
            }
            for [a, b] in intervals {
                let (a, b) = ((a + part.offset).max(w0), (b + part.offset).min(w1));
                if b - a > 0.2 {
                    pieces.push((i, a, b));
                }
            }
        }
        let longest = pieces.iter().map(|p| p.2 - p.1).fold(0.0, f64::max);
        let Some(&(i, a, b)) = pieces.iter().rev().find(|p| p.2 - p.1 >= longest * 0.5) else {
            return Err("Nothing has been recorded yet. Wait a moment and try again.".into());
        };
        let length = b - a;
        let note = if length >= secs - 2.5 {
            None
        } else if pieces.len() == 1 {
            Some(format!("{} saved: that is all that has been recorded so far", fmt_len(length)))
        } else {
            Some(format!("{} saved: the recording was interrupted there (the game restarted or changed resolution)", fmt_len(length)))
        };
        Ok(ReplayPick { session: parts[i].id.clone(), start: a - parts[i].offset, end: b - parts[i].offset, note })
    }

    /// Stream-copy the leased footage into a normal MP4 in the capture folder. Returns the clip
    /// and the media time of its first frame.
    pub fn save_clip(&self, lease: &Lease, root: &Path, save_id: u64, aligned: bool, kind: ClipKind) -> Result<(SavedClip, f64)> {
        let (game, tracks) = self.details(&lease.session)?;
        let mut segs = self.media(&lease.session, lease.start - 3.0, lease.end + 3.0)?;
        let first = segs.iter().rposition(|s| s.start <= lease.start + if aligned { 0.000_001 } else { 0.0 }).unwrap_or(0);
        let lead = if first > 0 { Some(segs[first - 1].path.clone()) } else { None };
        segs.drain(..first);
        if segs.is_empty() {
            return Err("The selected footage is not on disk yet".into());
        }
        // Never silently publish a clip omitting missing requested moments.
        if segs[0].start > lease.start + 0.1 {
            return Err("The beginning of that selection is no longer available".into());
        }
        let mut cursor = lease.start.max(segs[0].start);
        for s in &segs {
            if s.start > cursor + 0.1 && cursor < lease.end {
                return Err("The selection crosses unavailable footage. Choose one available section.".into());
            }
            cursor = cursor.max(s.end);
        }
        if cursor + 0.1 < lease.end {
            return Err("The end of the selection is not recorded yet. Try again in a moment.".into());
        }
        for s in &segs {
            if !s.path.is_file() {
                return Err("A selected recording segment is missing; remaining footage has been kept".into());
            }
        }
        let (dir, label) = match kind {
            ClipKind::Clip => (storage::clips_dir(root, &game), "Clip"),
            ClipKind::Session => (storage::sessions_dir(root, &game), "Session"),
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create the folder {}: {e}", dir.display()))?;
        let needed: u64 = segs.iter().map(|s| s.bytes).sum();
        if storage::free_bytes(&dir).is_some_and(|free| free < needed + 32 * 1024 * 1024) {
            return Err("Not enough space to finalize this clip. Its original footage is protected for retry.".into());
        }
        let path = storage::reserve_unique_path(&dir, &storage::clip_file_name(&storage::local_stamp(), &game, label)).map_err(err)?;
        let list: Vec<_> = segs.iter().map(|s| (s.path.clone(), s.start)).collect();
        let (clip, actual) = if aligned {
            save::remux_aligned(&list, lead.as_deref(), lease.start..lease.end, &tracks, &path)?
        } else {
            save::remux(&list, lead.as_deref(), segs[0].start, lease.start, lease.end, &tracks, &path)?
        };
        if clip.duration_secs + 0.15 < lease.end - actual {
            let _ = std::fs::remove_file(&path);
            return Err("Clip verification found incomplete footage; source retained for retry".into());
        }
        let events = self.events(&lease.session)?;
        let markers: Vec<_> = events.iter().filter(|e| e.kind == "marker" && e.t >= actual && e.t <= lease.end).map(|e| serde_json::json!({"t":((e.t-actual)*100.0).round()/100.0,"label":e.label,"at":e.marker.as_ref().map(|m|m.at.clone())})).collect();
        if !markers.is_empty() {
            let doc = serde_json::json!({"capture_markers":1,"clip":path.file_name().map(|n|n.to_string_lossy().into_owned()),"markers":markers});
            std::fs::write(format!("{}.markers.json", path.display()), serde_json::to_vec_pretty(&doc).map_err(err)?).map_err(err)?;
        }
        self.event(
            &lease.session,
            &TimelineEvent { id: format!("clip:{}", path.display()), kind: "clip".into(), t: actual, end: Some(lease.end), label: path.file_name().unwrap_or_default().to_string_lossy().into_owned(), path: Some(path.to_string_lossy().into_owned()), marker: None },
        )?;
        self.db.lock().unwrap().execute("DELETE FROM jobs WHERE state='failed' AND kind='save' AND session=?1 AND start>=?2 AND end<=?3", params![lease.session, lease.start - 3.0, lease.end + 3.0]).map_err(err)?;
        let saved = SavedClip {
            save_id,
            path: path.to_string_lossy().into_owned(),
            requested_secs: lease.end - lease.start,
            duration_secs: clip.duration_secs,
            bytes: clip.bytes,
            video_streams: clip.video_streams,
            audio_tracks: clip.audio_tracks,
            existing: false,
        };
        Ok((saved, actual))
    }

    // ---- Playback windows -----------------------------------------------------------------

    /// A playable window of the session at session time `at` (stream copy into a temporary
    /// MP4, reused while it is in the cache). `quick`: a short window that is ready fast, for
    /// seeking and scrubbing; otherwise a long one for continuous playback.
    pub fn preview(self: &Arc<Self>, session: &str, at: f64, quick: bool) -> Result<Review> {
        let (recording, offset) = self.locate(session, at)?;
        let m = at - offset;
        let review = |path: String, start: f64, end: f64, game: String| Review { session: session.into(), path, start: start + offset, end: end + offset, game, past: true, markers: vec![] };
        let (game, tracks) = self.details(&recording)?;
        if let Some((path, start, end)) = self.cached_window(&recording, m)? {
            return Ok(review(path, start, end, game));
        }
        // One build at a time; whoever waited may find the window it wanted already built.
        let _building = self.build.lock().unwrap();
        if let Some((path, start, end)) = self.cached_window(&recording, m)? {
            return Ok(review(path, start, end, game));
        }
        let (secs, bytes) = if quick { (QUICK_SECS, QUICK_BYTES) } else { (WINDOW_SECS, WINDOW_BYTES) };
        let nearby = self.media(&recording, m - 0.05, m + secs)?;
        let anchor = nearby.iter().find(|s| s.end > m + 0.000_5 && s.start <= m + 0.05).ok_or("No footage at this position")?;
        let start = anchor.start;
        let mut end = anchor.end;
        let mut window_bytes = 0;
        for s in nearby.iter().filter(|s| s.start >= start) {
            if s.start > end + 0.1 {
                break; // a gap: the window ends at the last continuous moment
            }
            if window_bytes > 0 && window_bytes + s.bytes > bytes {
                break;
            }
            window_bytes += s.bytes;
            end = s.end;
            if end >= m + secs {
                break;
            }
        }
        // The source stays put while it is copied.
        let _lease = self.lease(&recording, start, end, "playback")?;
        let media = self.media(&recording, start - 3.0, end + 3.0)?;
        let first = media.iter().position(|s| s.start >= start - 0.01).ok_or("Preview footage expired")?;
        let lead = first.checked_sub(1).map(|i| media[i].path.as_path());
        let list: Vec<_> = media[first..].iter().map(|s| (s.path.clone(), s.start)).collect();
        let needed: u64 = media.iter().map(|s| s.bytes).sum();
        let path = self.root.join(format!("preview_{}_{}.mp4", millis(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let name = path.to_string_lossy().into_owned();
        {
            let db = self.db.lock().unwrap();
            self.evict_previews(&db, needed)?;
            if storage::free_bytes(&self.root).is_some_and(|f| f < needed + 32 * 1024 * 1024) {
                return Err("Not enough free disk space to open this moment. Your recordings are unchanged.".into());
            }
            db.execute("INSERT INTO previews(path,bytes,session,start,end,refs,used) VALUES(?1,?2,?3,?4,?5,0,?6)", params![name, needed as i64, recording, start, end, self.stamp() as i64]).map_err(err)?;
        }
        match save::remux_preview(&list, lead, start, end, &tracks, &path) {
            Ok((clip, actual)) => {
                let end = actual + clip.duration_secs;
                self.db.lock().unwrap().execute("UPDATE previews SET bytes=?2,start=?3,end=?4 WHERE path=?1", params![name, clip.bytes as i64, actual, end]).map_err(err)?;
                Ok(review(name, actual, end, game))
            }
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                let _ = self.db.lock().unwrap().execute("DELETE FROM previews WHERE path=?1", [&name]);
                Err(e)
            }
        }
    }

    /// The cached window holding media time `m` with the most footage after it.
    fn cached_window(&self, recording: &str, m: f64) -> Result<Option<(String, f64, f64)>> {
        let db = self.db.lock().unwrap();
        let hit: Option<(String, f64, f64)> = db
            .query_row("SELECT path,start,end FROM previews WHERE session=?1 AND start<=?2+0.0005 AND end>?2+0.05 ORDER BY end DESC LIMIT 1", params![recording, m], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()
            .map_err(err)?;
        match hit {
            Some((path, start, end)) if Path::new(&path).is_file() => {
                db.execute("UPDATE previews SET used=?2 WHERE path=?1", params![path, self.stamp() as i64]).map_err(err)?;
                Ok(Some((path, start, end)))
            }
            Some((path, ..)) => {
                db.execute("DELETE FROM previews WHERE path=?1", [path]).map_err(err)?;
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// Make room for `needed` more bytes: least recently used windows go first; the newest
    /// `PREVIEW_KEEP` stay. A file a player still has open cannot be deleted and is skipped
    /// (it goes on a later pass). Never an error: the budget is a target, not a gate.
    fn evict_previews(&self, db: &Connection, needed: u64) -> Result<()> {
        let rows = {
            let mut q = db.prepare_cached("SELECT path,bytes FROM previews ORDER BY used DESC").map_err(err)?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
            rows
        };
        let mut total: u64 = rows.iter().map(|r| r.1).sum();
        for (path, bytes) in rows.iter().skip(PREVIEW_KEEP).rev() {
            if total + needed <= PREVIEW_BUDGET {
                break;
            }
            if std::fs::remove_file(path).is_ok() || !Path::new(path).exists() {
                db.execute("DELETE FROM previews WHERE path=?1", [path]).map_err(err)?;
                total = total.saturating_sub(*bytes);
            }
        }
        Ok(())
    }

    /// A viewer is done with a window. Windows stay cached for the next seek; this only makes
    /// the window the first to go when room is needed. It is not made idle at once: a player
    /// that is still reading it (it does not keep the file open between reads) must not have it
    /// deleted underneath it by the next housekeeping pass.
    pub fn release_preview(&self, path: &str) {
        let grace = self.clock.load(Ordering::SeqCst).max(millis()).saturating_sub(PREVIEW_IDLE_MS - PREVIEW_RELEASE_GRACE_MS);
        let _ = self.db.lock().unwrap().execute("UPDATE previews SET used=MIN(used,?2) WHERE path=?1", params![path, grace as i64]);
    }

    // ---- Thumbnails -----------------------------------------------------------------------

    /// A small JPEG (data URL) of the session at session time `at`, with the session time of
    /// the frame shown. Served from a bounded in-memory cache when it was made before.
    pub fn thumb(&self, session: &str, at: f64, exact: bool) -> Result<(f64, Arc<str>)> {
        let (recording, offset) = self.locate(session, at)?;
        let m = at - offset;
        let segment = {
            let db = self.db.lock().unwrap();
            db.query_row("SELECT path,start,seq FROM segments WHERE session=?1 AND complete=1 AND start<=?2+0.0005 AND end>?2 ORDER BY start DESC LIMIT 1", params![recording, m], |r| {
                Ok((PathBuf::from(r.get::<_, String>(0)?), r.get::<_, f64>(1)?, r.get::<_, i64>(2)? as u64))
            })
            .optional()
            .map_err(err)?
        };
        let (path, start, seq) = segment.ok_or("No footage at this position")?;
        // Keyframe pictures are shared by every request in the same second of the segment.
        let bucket = if exact { ((m - start) * 120.0).round() as i64 } else { (m - start).floor() as i64 };
        let key: ThumbKey = (recording, seq, bucket, exact);
        {
            let mut cache = self.thumbs.lock().unwrap();
            if let Some(i) = cache.items.iter().position(|(k, ..)| *k == key) {
                let hit = cache.items.remove(i).unwrap();
                let found = (hit.1 + offset, hit.2.clone());
                cache.items.push_back(hit);
                return Ok(found);
            }
        }
        let (t, jpeg) = thumb::frame_jpeg(&path, start, m, exact, THUMB_WIDTH)?;
        let url: Arc<str> = thumb::data_url(&jpeg).into();
        let mut cache = self.thumbs.lock().unwrap();
        cache.bytes += url.len();
        cache.items.push_back((key, t, url.clone()));
        while cache.bytes > THUMB_CACHE_BYTES {
            let Some(old) = cache.items.pop_front() else { break };
            cache.bytes -= old.2.len();
        }
        Ok((t + offset, url))
    }

    /// For the live recording's markers: where its remaining footage begins, and which marker
    /// pins still hold footage.
    pub fn marker_facts(&self, session: &str) -> (Option<f64>, Vec<String>) {
        let db = self.db.lock().unwrap();
        let oldest = db.query_row("SELECT MIN(start) FROM segments WHERE session=?1", [session], |r| r.get::<_, Option<f64>>(0)).ok().flatten();
        let pins = db.prepare_cached("SELECT id FROM pins WHERE session=?1").and_then(|mut q| q.query_map([session], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()).unwrap_or_default();
        (oldest, pins)
    }

    // ---- Published state ------------------------------------------------------------------

    pub fn events(&self, session: &str) -> Result<Vec<TimelineEvent>> {
        let db = self.db.lock().unwrap();
        let mut q = db.prepare_cached("SELECT data FROM events WHERE session=?1").map_err(err)?;
        let rows = q.query_map([session], |r| r.get::<_, String>(0)).map_err(err)?;
        rows.map(|r| serde_json::from_str::<TimelineEvent>(&r.map_err(err)?).map_err(err)).filter(|e| !matches!(e,Ok(v) if v.kind=="removed")).collect()
    }

    /// An event id as published (`<recording>/<id>`) back to its recording and own id.
    pub fn event_ref(public: &str) -> Option<(&str, &str)> {
        public.split_once('/')
    }

    pub fn state(&self, target_secs: u32) -> Result<HistoryState> {
        let rev = self.rev.load(Ordering::Relaxed);
        let cached = self.state_cache.lock().unwrap().as_ref().filter(|(r, t, _)| *r == rev && *t == target_secs).map(|c| c.2.clone());
        if let Some(mut state) = cached {
            // The playback cache changes without touching the history itself.
            let bytes: i64 = self.db.lock().unwrap().query_row("SELECT COALESCE(SUM(bytes),0) FROM previews", [], |r| r.get(0)).map_err(err)?;
            state.preview_bytes = bytes as u64;
            return Ok(state);
        }
        let state = self.build_state(target_secs)?;
        *self.state_cache.lock().unwrap() = Some((rev, target_secs, state.clone()));
        Ok(state)
    }

    fn build_state(&self, target_secs: u32) -> Result<HistoryState> {
        let db = self.db.lock().unwrap();
        struct Row {
            id: String,
            game: String,
            created: i64,
            ended: bool,
            grp: String,
            grp_ms: i64,
            width: u32,
            height: u32,
        }
        let rows = {
            let mut q = db.prepare_cached("SELECT id,game,created_ms,ended,COALESCE(grp,id),COALESCE(grp_ms,created_ms),config FROM sessions ORDER BY COALESCE(grp_ms,created_ms),created_ms,id").map_err(err)?;
            let rows = q
                .query_map([], |r| {
                    let config: serde_json::Value = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
                    Ok(Row { id: r.get(0)?, game: r.get(1)?, created: r.get(2)?, ended: r.get(3)?, grp: r.get(4)?, grp_ms: r.get(5)?, width: config["video"]["width"].as_u64().unwrap_or(0) as u32, height: config["video"]["height"].as_u64().unwrap_or(0) as u32 })
                })
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            rows
        };
        // Footage per recording: merged intervals (media time) and bytes, in one pass.
        let mut footage: HashMap<String, (Vec<[f64; 2]>, u64)> = HashMap::new();
        {
            let mut q = db.prepare_cached("SELECT session,start,end,bytes FROM segments WHERE complete=1 ORDER BY session,start").map_err(err)?;
            let mut rows = q.query([]).map_err(err)?;
            while let Some(r) = rows.next().map_err(err)? {
                let entry = footage.entry(r.get::<_, String>(0).map_err(err)?).or_default();
                merge(&mut entry.0, r.get(1).map_err(err)?, r.get(2).map_err(err)?, 0.05);
                entry.1 += r.get::<_, i64>(3).map_err(err)? as u64;
            }
        }
        let mut events: HashMap<String, Vec<TimelineEvent>> = HashMap::new();
        {
            let mut q = db.prepare_cached("SELECT session,data FROM events").map_err(err)?;
            let mut rows = q.query([]).map_err(err)?;
            while let Some(r) = rows.next().map_err(err)? {
                if let Ok(ev) = serde_json::from_str::<TimelineEvent>(&r.get::<_, String>(1).map_err(err)?) {
                    // A clip the player has since deleted or moved is no longer "saved from here".
                    let gone = ev.kind == "clip" && ev.path.as_deref().is_some_and(|p| !Path::new(p).is_file());
                    if ev.kind != "removed" && !gone {
                        events.entry(r.get(0).map_err(err)?).or_default().push(ev);
                    }
                }
            }
        }
        let bytes: i64 = db.query_row("SELECT COALESCE(SUM(bytes),0) FROM segments", [], |r| r.get(0)).map_err(err)?;
        let preview_bytes: i64 = db.query_row("SELECT COALESCE(SUM(bytes),0) FROM previews", [], |r| r.get(0)).map_err(err)?;
        let offsets: HashMap<&str, (&str, f64)> = rows.iter().map(|r| (r.id.as_str(), (r.grp.as_str(), (r.created - r.grp_ms) as f64 / 1000.0))).collect();
        let failed_saves = {
            let mut q = db.prepare_cached("SELECT id,session,start+3,end-3,COALESCE(error,'Save interrupted') FROM jobs WHERE state='failed' AND kind='save' ORDER BY id").map_err(err)?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?, r.get::<_, String>(4)?))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
            rows.into_iter()
                .filter_map(|(id, session, start, end, error)| {
                    let (grp, offset) = offsets.get(session.as_str())?;
                    Some(capture_protocol::FailedHistorySave { id, session: grp.to_string(), start: start + offset, end: end + offset, error })
                })
                .collect()
        };
        let mut sessions: Vec<HistorySession> = vec![];
        let mut retained_secs = 0.0;
        for row in &rows {
            let offset = (row.created - row.grp_ms) as f64 / 1000.0;
            let (intervals, size) = footage.remove(&row.id).unwrap_or_default();
            let evs = events.remove(&row.id).unwrap_or_default();
            if intervals.is_empty() && evs.is_empty() {
                continue;
            }
            if sessions.last().is_none_or(|s| s.id != row.grp) {
                sessions.push(HistorySession { id: row.grp.clone(), game: row.game.clone(), created_ms: row.grp_ms as u64, ended: true, intervals: vec![], events: vec![], parts: vec![], bytes: 0 });
            }
            let session = sessions.last_mut().unwrap();
            session.ended &= row.ended;
            session.bytes += size;
            retained_secs += intervals.iter().map(|r| r[1] - r[0]).sum::<f64>();
            let end = intervals.last().map_or(offset, |l| l[1] + offset);
            // Recordings never share an interval: a clip is one recording at one resolution.
            session.intervals.extend(intervals.iter().map(|[a, b]| [a + offset, b + offset]));
            session.parts.push(HistoryPart { id: row.id.clone(), offset, end, width: row.width, height: row.height });
            session.events.extend(evs.into_iter().map(|mut e| {
                e.id = format!("{}/{}", row.id, e.id);
                e.t += offset;
                e.end = e.end.map(|t| t + offset);
                if let Some(m) = &mut e.marker {
                    m.t += offset;
                }
                e
            }));
        }
        Ok(HistoryState { failed_saves, sessions, bytes: bytes as u64, preview_bytes: preview_bytes as u64, retained_secs, target_secs, reason: self.reason.lock().unwrap().clone() })
    }

    // ---- Retention ------------------------------------------------------------------------

    /// One serialization point for global eviction and lease acquisition. Keeps at most `secs`
    /// of footage in at most `cap` bytes, with `reserve` bytes free on the disk, by removing
    /// the oldest unprotected footage CAPTURE recorded. A failed unlink remains
    /// indexed/accounted. Imported legacy media is never evicted automatically and does not
    /// count against the limits; the playback cache has its own budget.
    pub fn retain(&self, secs: u32, cap: u64, reserve: u64) -> Result<Option<String>> {
        let cap = cap.min(i64::MAX as u64) as i64;
        let db = self.db.lock().unwrap();
        // Playback windows nobody has used for a while.
        let idle = {
            let mut q = db.prepare_cached("SELECT path FROM previews WHERE used<?1").map_err(err)?;
            let rows = q.query_map([self.clock.load(Ordering::SeqCst).saturating_sub(PREVIEW_IDLE_MS) as i64], |r| r.get::<_, String>(0)).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
            rows
        };
        self.clock.fetch_max(millis(), Ordering::SeqCst);
        for p in idle {
            if std::fs::remove_file(&p).is_ok() || !Path::new(&p).exists() {
                db.execute("DELETE FROM previews WHERE path=?1", [p]).map_err(err)?;
            }
        }
        // Marker protection is global, bounded to half the unchanged storage budget.
        loop {
            let held: i64 = db
                .query_row("SELECT COALESCE(SUM(bytes),0) FROM segments s WHERE EXISTS(SELECT 1 FROM pins p WHERE p.session=s.session AND p.id LIKE 'marker:%' AND p.start<COALESCE(s.end,1e99) AND p.end>s.start)", [], |r| r.get(0))
                .map_err(err)?;
            if held <= cap / 2 {
                break;
            }
            let pin: Option<(String, String)> = db.query_row("SELECT p.session,p.id FROM pins p JOIN sessions x ON x.id=p.session WHERE p.id LIKE 'marker:%' ORDER BY x.created_ms,p.start LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(err)?;
            let Some((session, id)) = pin else { break };
            db.execute("DELETE FROM pins WHERE session=?1 AND id=?2", params![session, id]).map_err(err)?;
        }
        let (mut duration, mut bytes): (f64, i64) = db
            .query_row("SELECT COALESCE(SUM(MAX(s.end-s.start,0)),0),COALESCE(SUM(s.bytes),0) FROM segments s JOIN sessions x ON x.id=s.session WHERE s.complete=1 AND x.managed=1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?;
        let mut reason = None;
        let mut removed = false;
        let mut after: (i64, String, f64) = (i64::MIN, String::new(), f64::MIN);
        'evict: loop {
            let low = storage::free_bytes(&self.root).is_some_and(|f| f < reserve);
            if duration <= secs as f64 && bytes <= cap && !low {
                break;
            }
            // Oldest first, a page at a time: the common case (nothing to do) reads no rows.
            let page = {
                let mut q = db
                    .prepare_cached(
                        "SELECT s.session,s.seq,s.path,s.start,s.end,s.bytes,x.created_ms,
                           EXISTS(SELECT 1 FROM jobs j WHERE j.session=s.session AND j.start<s.end AND j.end>s.start UNION ALL SELECT 1 FROM pins p WHERE p.session=s.session AND p.start<s.end AND p.end>s.start)
                         FROM segments s JOIN sessions x ON x.id=s.session
                         WHERE s.complete=1 AND x.managed=1 AND (x.created_ms,x.id,s.start)>(?1,?2,?3)
                         ORDER BY x.created_ms,x.id,s.start LIMIT 256",
                    )
                    .map_err(err)?;
                let rows = q
                    .query_map(params![after.0, after.1, after.2], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, f64>(3)?, r.get::<_, f64>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?, r.get::<_, bool>(7)?)))
                    .map_err(err)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(err)?;
                rows
            };
            if page.is_empty() {
                break;
            }
            for (session, seq, path, a, b, size, created, protected) in page {
                let low = storage::free_bytes(&self.root).is_some_and(|f| f < reserve);
                if duration <= secs as f64 && bytes <= cap && !low {
                    break 'evict;
                }
                reason = Some(if low { "Free-space reserve" } else if bytes > cap { "Storage budget" } else { "History duration" }.to_string());
                after = (created, session.clone(), a);
                if protected {
                    continue;
                }
                if std::fs::remove_file(&path).is_ok() || !Path::new(&path).exists() {
                    db.execute("DELETE FROM segments WHERE session=?1 AND seq=?2", params![session, seq]).map_err(err)?;
                    duration -= (b - a).max(0.0);
                    bytes = bytes.saturating_sub(size);
                    removed = true;
                }
            }
        }
        if self.pruned.lock().unwrap().elapsed().as_secs() >= 60 {
            *self.pruned.lock().unwrap() = std::time::Instant::now();
            removed |= Self::prune(&db)?;
        }
        if removed {
            self.touch();
        }
        if reason.is_some() && *self.reason.lock().unwrap() != reason {
            *self.reason.lock().unwrap() = reason.clone();
            self.touch();
        }
        Ok(reason)
    }

    /// Forget recordings whose footage has all gone (finished a while ago, nothing held for a
    /// save): their rows, their markers (which point at nothing now) and their empty folders.
    /// Saved clips, recordings and screenshots are separate files and are not touched.
    fn prune(db: &Connection) -> Result<bool> {
        let gone = {
            let mut q = db
                .prepare_cached(
                    "SELECT id,dir FROM sessions s WHERE ended=1 AND managed=1 AND COALESCE(ended_ms,created_ms)<?1
                       AND NOT EXISTS(SELECT 1 FROM segments g WHERE g.session=s.id)
                       AND NOT EXISTS(SELECT 1 FROM jobs j WHERE j.session=s.id)
                       AND NOT EXISTS(SELECT 1 FROM pins p WHERE p.session=s.id)",
                )
                .map_err(err)?;
            let rows = q.query_map([millis() as i64 - 10 * 60 * 1000], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
            rows
        };
        for (id, dir) in &gone {
            Self::forget(db, id, Path::new(dir))?;
        }
        Ok(!gone.is_empty())
    }

    fn forget(db: &Connection, id: &str, dir: &Path) -> Result<()> {
        if dir.file_name().is_some_and(|n| storage::is_session_dir_name(&n.to_string_lossy())) {
            storage::remove_session_dir(dir);
        }
        db.execute("DELETE FROM events WHERE session=?1", [id]).map_err(err)?;
        db.execute("DELETE FROM epochs WHERE session=?1", [id]).map_err(err)?;
        db.execute("DELETE FROM pins WHERE session=?1", [id]).map_err(err)?;
        db.execute("DELETE FROM sessions WHERE id=?1", [id]).map_err(err)?;
        Ok(())
    }

    /// The player deletes one session's temporary footage. A recording still being written, or
    /// footage a save is using, is left alone (and reported). Returns the bytes freed.
    pub fn delete_session(&self, session: &str) -> Result<u64> {
        let db = self.db.lock().unwrap();
        let rows = {
            let mut q = db.prepare_cached("SELECT id,dir,ended FROM sessions WHERE grp=?1 OR (grp IS NULL AND id=?1) ORDER BY created_ms").map_err(err)?;
            let rows = q.query_map([session], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
            rows
        };
        if rows.is_empty() {
            return Err("That recording is no longer available".into());
        }
        let (mut freed, mut kept) = (0u64, None);
        for (id, dir, ended) in rows {
            if !ended {
                kept = Some("The part still being recorded was kept.");
                continue;
            }
            if db.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE session=?1 AND kind='save' AND state='active')", [&id], |r| r.get::<_, bool>(0)).map_err(err)? {
                kept = Some("Footage a clip is still being saved from was kept.");
                continue;
            }
            let files = {
                let mut q = db.prepare_cached("SELECT seq,path,bytes FROM segments WHERE session=?1").map_err(err)?;
                let rows = q.query_map([&id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
                rows
            };
            let mut left = false;
            for (seq, path, bytes) in files {
                if std::fs::remove_file(&path).is_ok() || !Path::new(&path).exists() {
                    db.execute("DELETE FROM segments WHERE session=?1 AND seq=?2", params![id, seq]).map_err(err)?;
                    freed += bytes as u64;
                } else {
                    left = true;
                }
            }
            if left {
                kept = Some("Some footage is in use and was kept.");
            } else {
                db.execute("DELETE FROM jobs WHERE session=?1", [&id]).map_err(err)?;
                Self::forget(&db, &id, Path::new(&dir))?;
            }
        }
        drop(db);
        self.touch();
        match kept {
            Some(message) if freed == 0 => Err(message.into()),
            _ => Ok(freed),
        }
    }

    /// Background history off: delete the footage of every finished session CAPTURE recorded.
    /// Left alone: the session being recorded, recovered footage from older versions, and
    /// anything a save is using or failed on. Returns the bytes freed.
    pub fn drop_finished(&self) -> u64 {
        let sessions: Vec<String> = {
            let db = self.db.lock().unwrap();
            let found = db
                .prepare_cached(
                    "SELECT COALESCE(grp,id) g FROM sessions GROUP BY g
                     HAVING MIN(ended)=1 AND MIN(managed)=1
                       AND NOT EXISTS(SELECT 1 FROM jobs j JOIN sessions o ON o.id=j.session WHERE COALESCE(o.grp,o.id)=g)",
                )
                .and_then(|mut q| q.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>())
                .unwrap_or_default();
            found
        };
        sessions.iter().map(|s| self.delete_session(s).unwrap_or(0)).sum()
    }

    fn reconcile(&self) -> Result<()> {
        let db = self.db.lock().unwrap();
        let mut q = db.prepare("SELECT path FROM previews").map_err(err)?;
        let previews = q.query_map([], |r| r.get::<_, String>(0)).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
        drop(q);
        for p in previews {
            let partial = storage::partial_path(Path::new(&p));
            let partial_gone = std::fs::remove_file(&partial).is_ok() || !partial.exists();
            if partial_gone && (std::fs::remove_file(&p).is_ok() || !Path::new(&p).exists()) {
                db.execute("DELETE FROM previews WHERE path=?1", [p]).map_err(err)?;
            }
        }
        let mut imports = vec![];
        for e in std::fs::read_dir(&self.root).map_err(err)?.flatten().filter(|e| e.path().is_dir() && storage::is_session_dir_name(&e.file_name().to_string_lossy())) {
            let dir = e.path();
            let id = e.file_name().to_string_lossy().into_owned();
            let known: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)", [&id], |r| r.get(0)).map_err(err)?;
            // A folder with no footage and no row is a leftover of an already forgotten recording.
            let has_segments = std::fs::read_dir(&dir).map_err(err)?.flatten().any(|f| {
                let n = f.file_name().to_string_lossy().into_owned();
                n.starts_with("seg_") && n.ends_with(".ts")
            });
            if !known && !has_segments {
                storage::remove_session_dir(&dir);
                continue;
            }
            let manifest: serde_json::Value = std::fs::read(dir.join("session.json")).ok().and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or(serde_json::json!({"game":"Recovered gameplay"}));
            let game = manifest["game"].as_str().unwrap_or("Recovered gameplay");
            let created = manifest["started_unix_ms"].as_u64().or_else(|| std::fs::metadata(&dir).ok()?.created().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)).unwrap_or_else(millis);
            db.execute("INSERT OR IGNORE INTO sessions(id,dir,game,created_ms,ended,config,managed) VALUES(?1,?2,?3,?4,1,?5,0)", params![id, dir.to_string_lossy(), game, created as i64, manifest.to_string()]).map_err(err)?;
            db.execute("INSERT OR IGNORE INTO epochs VALUES(?1,0,0,?2)", params![id, manifest.to_string()]).map_err(err)?;
            if let Ok(lines) = std::fs::read_to_string(dir.join("events.jsonl")) {
                for line in lines.lines() {
                    if let Ok(ev) = serde_json::from_str::<TimelineEvent>(line) {
                        db.execute("INSERT OR REPLACE INTO events VALUES(?1,?2,?3)", params![id, ev.id, line]).map_err(err)?;
                    }
                }
            }
            if let Ok(markers) = std::fs::read(dir.join("markers.json")) {
                if let Ok(ms) = serde_json::from_slice::<Vec<capture_protocol::Marker>>(&markers) {
                    for m in ms {
                        let ev = TimelineEvent { id: format!("marker:{}", m.id), kind: "marker".into(), t: m.t, end: None, label: m.label.clone(), path: None, marker: Some(m) };
                        db.execute("INSERT OR IGNORE INTO events VALUES(?1,?2,?3)", params![id, ev.id, serde_json::to_string(&ev).map_err(err)?]).map_err(err)?;
                    }
                }
            }
            for f in std::fs::read_dir(&dir).map_err(err)?.flatten() {
                let p = f.path();
                let name = f.file_name().to_string_lossy().into_owned();
                if name.starts_with("seg_") && name.ends_with(".ts") {
                    let known: Option<bool> = db.query_row("SELECT complete FROM segments WHERE path=?1", [p.to_string_lossy()], |r| r.get(0)).optional().map_err(err)?;
                    if known != Some(true) {
                        imports.push((id.clone(), p));
                    }
                }
            }
        }
        Self::assign_groups(&db)?;
        self.importing.store(!imports.is_empty(), Ordering::Release);
        *self.imports.lock().unwrap() = imports;
        Ok(())
    }

    /// Recovery is incremental; opening Replay never probes the entire history. A segment that
    /// was still being written when the app or the PC stopped is measured and indexed up to its
    /// last whole frame; one with no readable picture is dropped from the index (file kept).
    pub fn import_step(&self) -> Result<()> {
        for _ in 0..4 {
            let Some((id, p)) = self.imports.lock().unwrap().pop() else { break };
            if let (Some(start), Some(end)) = (save::first_keyframe_secs(&p), save::last_video_secs(&p)) {
                let db = self.db.lock().unwrap();
                let config: String = db.query_row("SELECT config FROM sessions WHERE id=?1", [&id], |r| r.get(0)).map_err(err)?;
                let mut config: serde_json::Value = serde_json::from_str(&config).map_err(err)?;
                if config.get("audio_tracks").is_none() {
                    if let Some(names) = save::recovered_audio_names(&p) {
                        config["audio_tracks"] = serde_json::to_value(names).map_err(err)?;
                        db.execute("UPDATE sessions SET config=?2 WHERE id=?1", params![id, config.to_string()]).map_err(err)?;
                        db.execute("UPDATE epochs SET config=?2 WHERE session=?1", params![id, config.to_string()]).map_err(err)?;
                    }
                }
                // Segments keep absolute timestamps only relative to their own first keyframe;
                // a segment already indexed (interrupted while open) keeps its known start.
                let known: Option<f64> = db.query_row("SELECT start FROM segments WHERE path=?1", [p.to_string_lossy()], |r| r.get(0)).optional().map_err(err)?;
                drop(db);
                let seq = p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.strip_prefix("seg_")).and_then(|s| s.parse().ok()).unwrap_or(0);
                let begin = known.unwrap_or(start);
                self.segment(&id, seq, &p, begin, Some(begin + (end - start) + 1.0 / 60.0), std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0))?;
            } else {
                let _ = self.db.lock().unwrap().execute("DELETE FROM segments WHERE path=?1 AND complete=0", [p.to_string_lossy()]);
            }
        }
        if self.imports.lock().unwrap().is_empty() {
            self.importing.store(false, Ordering::Release);
            self.touch();
        }
        Ok(())
    }

    pub fn refresh_files(&self) -> Result<()> {
        let db = self.db.lock().unwrap();
        let cursor = self.scan.load(Ordering::Relaxed) as i64;
        let mut q = db.prepare_cached("SELECT rowid,path FROM segments WHERE rowid>?1 AND complete=1 ORDER BY rowid LIMIT 64").map_err(err)?;
        let rows = q.query_map([cursor], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
        drop(q);
        if rows.is_empty() {
            self.scan.store(0, Ordering::Relaxed);
        }
        let mut changed = false;
        for (id, p) in rows {
            self.scan.store(id as u64, Ordering::Relaxed);
            if let Err(e) = std::fs::metadata(&p) {
                if e.kind() == std::io::ErrorKind::NotFound {
                    db.execute("DELETE FROM segments WHERE rowid=?1", [id]).map_err(err)?;
                    changed = true;
                }
            }
        }
        if changed {
            self.touch();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(Arc<Catalog>,String,PathBuf) {
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-output/indexed-history").join(format!("{}-{}",millis(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir_all(&root).unwrap();let c=Catalog::open(&root).unwrap();
        let dir=root.join("2026-11-09_10-00-00_1");std::fs::create_dir(&dir).unwrap();
        let id=c.register(&dir,"Fixture",&serde_json::json!({"audio_tracks":[],"video":{"width":1920,"height":1080,"fps":60}}),true).unwrap();
        (c,id,dir)
    }
    fn segment(c:&Catalog,id:&str,dir:&Path,seq:u64,start:f64) ->PathBuf {
        let p=dir.join(format!("seg_{seq:06}.ts"));std::fs::write(&p,[0u8;100]).unwrap();c.segment(id,seq,&p,start,Some(start+2.0),100).unwrap();p
    }
    #[test] fn duration_is_cumulative_and_replay_setting_is_independent(){
        let (c,id,dir)=fixture();for n in 0..10{segment(&c,&id,&dir,n,n as f64*2.0);}
        let dir2=dir.parent().unwrap().join("2026-11-09_23-00-00_2");std::fs::create_dir(&dir2).unwrap();let id2=c.register(&dir2,"Other game",&serde_json::json!({}),true).unwrap();for n in 0..10{segment(&c,&id2,&dir2,n,n as f64*2.0);}
        assert_eq!(c.state(14400).unwrap().retained_secs,40.0);c.retain(14400,100000,0).unwrap();assert_eq!(c.media(&id,0.0,20.0).unwrap().len(),10);c.retain(30,100000,0).unwrap();assert_eq!(c.state(30).unwrap().retained_secs,30.0);
    }
    #[test] fn lease_is_atomic_against_cleanup_and_failed_save_survives_restart(){
        let(c,id,dir)=fixture();let p=segment(&c,&id,&dir,0,0.0);let mut lease=c.lease(&id,0.0,2.0,"save").unwrap();c.retain(0,0,0).unwrap();assert!(p.exists());lease.failed("fixture failure");drop(lease);drop(c);
        let reopened=Catalog::open(dir.parent().unwrap()).unwrap();reopened.retain(0,0,0).unwrap();assert!(p.exists());
    }
    #[test] fn successful_lease_release_allows_eviction_but_permanent_media_survives(){
        let(c,id,dir)=fixture();let p=segment(&c,&id,&dir,0,0.0);let permanent=dir.parent().unwrap().join("permanent.mp4");std::fs::write(&permanent,"permanent fixture").unwrap();let lease=c.lease(&id,0.0,2.0,"save").unwrap();drop(lease);c.retain(0,0,0).unwrap();assert!(!p.exists());assert!(permanent.exists());
    }
    #[test] fn events_and_gaps_survive_restart(){
        let(c,id,dir)=fixture();segment(&c,&id,&dir,0,0.0);segment(&c,&id,&dir,1,10.0);
        for kind in ["marker","screenshot"]{c.event(&id,&TimelineEvent{id:kind.into(),kind:kind.into(),t:1.25,end:None,label:"remember".into(),path:Some("permanent.png".into()),marker:None}).unwrap();}drop(c);
        let c=Catalog::open(dir.parent().unwrap()).unwrap();let state=c.state(14400).unwrap();assert_eq!(state.sessions[0].intervals,vec![[0.0,2.0],[10.0,12.0]]);assert_eq!(state.sessions[0].events.len(),2);assert_eq!(state.sessions[0].events[0].t,1.25);
    }
    #[test] fn missing_segments_are_not_shown_as_continuous(){let(c,id,dir)=fixture();let p=segment(&c,&id,&dir,0,0.0);segment(&c,&id,&dir,1,2.0);std::fs::remove_file(&p).unwrap();c.refresh_files().unwrap();assert_eq!(c.state(14400).unwrap().sessions[0].intervals,vec![[2.0,4.0]]);}
    #[test] fn old_user_media_is_never_migration_cleanup(){let(c,id,dir)=fixture();let p=segment(&c,&id,&dir,0,0.0);c.db.lock().unwrap().execute("UPDATE sessions SET managed=0",[]).unwrap();c.retain(0,0,0).unwrap();assert!(p.exists());}
    #[test] fn six_hour_catalog_selects_only_nearby_segments(){
        let(c,id,dir)=fixture();let started=std::time::Instant::now();{let mut db=c.db.lock().unwrap();let tx=db.transaction().unwrap();for n in 0..10800 {tx.execute("INSERT INTO segments VALUES(?1,?2,?3,?4,?5,1,1)",params![id,n,dir.join(format!("seg_{n:06}.ts")).to_string_lossy(),n as f64*2.0,(n+1) as f64*2.0]).unwrap();}tx.commit().unwrap();}
        let q=std::time::Instant::now();let media=c.media(&id,18000.0,18030.0).unwrap();assert_eq!(media.len(),15);assert_eq!(media[0].start,18000.0);println!("synthetic 10800-segment catalog: insert {:?}, 30-second lookup {:?}",started.elapsed(),q.elapsed());
    }
    #[test] fn journal_rebuild_preserves_irreplaceable_events(){let(c,id,dir)=fixture();c.event(&id,&TimelineEvent{id:"shot".into(),kind:"screenshot".into(),t:1.5,end:None,label:"frame".into(),path:Some("image.png".into()),marker:None}).unwrap();c.db.lock().unwrap().execute("DELETE FROM events",[]).unwrap();drop(c);let c=Catalog::open(dir.parent().unwrap()).unwrap();assert_eq!(c.events(&id).unwrap()[0].t,1.5);}
    #[test] fn failed_deletion_stays_accounted(){let(c,id,dir)=fixture();let p=dir.join("seg_000001.ts");std::fs::create_dir(&p).unwrap();c.segment(&id,1,&p,0.0,Some(2.0),100).unwrap();c.retain(0,0,0).unwrap();assert_eq!(c.state(0).unwrap().bytes,100);}
    fn recording(c:&Catalog,root:&Path,name:&str,game:&str,started_ms:u64)->(String,PathBuf){
        let dir=root.join(name);std::fs::create_dir(&dir).unwrap();
        let id=c.register(&dir,game,&serde_json::json!({"started_unix_ms":started_ms,"audio_tracks":[],"video":{"width":1920,"height":1080,"fps":60}}),true).unwrap();(id,dir)
    }
    fn ended_at(c:&Catalog,id:&str,ms:u64){c.end(id);c.db.lock().unwrap().execute("UPDATE sessions SET ended_ms=?2 WHERE id=?1",params![id,ms as i64]).unwrap();}
    #[test] fn one_session_spans_relaunches_and_resolution_changes_with_real_gaps(){
        let(c,first,dir)=fixture();let root=dir.parent().unwrap().to_path_buf();let t0=1_800_000_000_000u64;
        let(a,da)=recording(&c,&root,"2026-11-10_20-00-00_7","Game",t0);for n in 0..5{segment(&c,&a,&da,n,n as f64*2.0);}ended_at(&c,&a,t0+10_000);
        // The game changes resolution: a new recording begins 12 s into the session.
        let(b,db)=recording(&c,&root,"2026-11-10_20-00-12_7","Game",t0+12_000);for n in 0..3{segment(&c,&b,&db,n,n as f64*2.0);}ended_at(&c,&b,t0+18_000);
        // Played again an hour later: a new session. Another game: its own session.
        let(d,dd)=recording(&c,&root,"2026-11-10_21-00-00_7","Game",t0+3_600_000);segment(&c,&d,&dd,0,0.0);
        let(e,de)=recording(&c,&root,"2026-11-10_21-00-05_7","Other",t0+3_605_000);segment(&c,&e,&de,0,0.0);
        let state=c.state(7200).unwrap();let s:Vec<_>=state.sessions.iter().filter(|s|s.id!=first).collect();
        assert_eq!(s.len(),3);assert_eq!(s[0].id,a);assert_eq!(s[0].parts.len(),2);
        assert_eq!(s[0].intervals,vec![[0.0,10.0],[12.0,18.0]],"a visible gap where nothing was recorded");
        assert_eq!(c.locate(&a,13.0).unwrap(),(b.clone(),12.0));assert_eq!(c.locate(&a,9.0).unwrap(),(a.clone(),0.0));
        // A marker in the second recording sits at its place on the session timeline.
        c.event(&b,&TimelineEvent{id:"marker:1".into(),kind:"marker".into(),t:1.5,end:None,label:"x".into(),path:None,marker:None}).unwrap();
        let state=c.state(7200).unwrap();let ev=&state.sessions.iter().find(|s|s.id==a).unwrap().events[0];assert_eq!(ev.t,13.5);assert_eq!(Catalog::event_ref(&ev.id),Some((b.as_str(),"marker:1")));
        assert_eq!(s[1].id,d);assert_eq!(s[2].game,"Other");
    }
    #[test] fn instant_replay_uses_the_session_not_just_the_newest_fragment(){
        let(c,_,dir)=fixture();let root=dir.parent().unwrap().to_path_buf();let t0=1_800_000_000_000u64;
        // 5 minutes of play, then the game switches resolution on exit and records 5 more seconds.
        let(a,da)=recording(&c,&root,"2026-11-11_00-24-16_7","Game",t0);for n in 0..150{segment(&c,&a,&da,n,n as f64*2.0);}ended_at(&c,&a,t0+300_000);
        let(b,db)=recording(&c,&root,"2026-11-11_00-29-17_7","Game",t0+301_000);for n in 0..3{segment(&c,&b,&db,n,n as f64*2.0);}
        let pick=c.replay_pick(&b,5.4,120.0).unwrap();
        assert_eq!(pick.session,a,"the two minutes of gameplay, not the 5-second fragment");
        assert!((pick.end-300.0).abs()<0.01 && (pick.end-pick.start-113.6).abs()<0.01,"{pick:?}");assert!(pick.note.unwrap().contains("interrupted"));
        // Plenty of history in the live recording: exactly what was asked for, no note.
        let pick=c.replay_pick(&a,300.0,120.0).unwrap();assert_eq!((pick.session.as_str(),pick.start,pick.end,pick.note),(a.as_str(),180.0,300.0,None));
        // A recording that has only just begun explains its short clip.
        let(d,dd)=recording(&c,&root,"2026-11-11_09-00-00_7","Game",t0+40_000_000);for n in 0..10{segment(&c,&d,&dd,n,n as f64*2.0);}
        let pick=c.replay_pick(&d,20.0,120.0).unwrap();assert_eq!((pick.start,pick.end),(0.0,20.0));assert!(pick.note.unwrap().contains("all that has been recorded"));
    }
    #[test] fn the_same_footage_is_never_saved_twice(){
        let(c,id,dir)=fixture();segment(&c,&id,&dir,0,0.0);let clip=dir.parent().unwrap().join("saved.mp4");std::fs::write(&clip,"clip").unwrap();
        c.event(&id,&TimelineEvent{id:"clip:saved".into(),kind:"clip".into(),t:0.0,end:Some(5.435),label:"saved.mp4".into(),path:Some(clip.to_string_lossy().into_owned()),marker:None}).unwrap();
        let again=c.existing_clip(&id,0.0,5.435).unwrap();assert!(again.existing);assert_eq!(again.path,clip.to_string_lossy());
        assert!(c.existing_clip(&id,0.0,4.0).is_none(),"a different range is a different clip");
        std::fs::remove_file(&clip).unwrap();assert!(c.existing_clip(&id,0.0,5.435).is_none(),"a deleted clip can be saved again");
    }
    #[test] fn playback_cache_makes_room_instead_of_refusing(){
        let(c,id,dir)=fixture();let root=dir.parent().unwrap();let db=c.db.lock().unwrap();
        for n in 0..12u64 {let p=root.join(format!("preview_{n}.mp4"));std::fs::write(&p,"x").unwrap();db.execute("INSERT INTO previews(path,bytes,session,start,end,refs,used) VALUES(?1,?2,?3,?4,?5,0,?6)",params![p.to_string_lossy(),(100*1024*1024) as i64,id,n as f64*10.0,n as f64*10.0+10.0,(n+1) as i64]).unwrap();}
        c.evict_previews(&db,100*1024*1024).unwrap();
        let left:Vec<i64>=db.prepare("SELECT used FROM previews ORDER BY used").unwrap().query_map([],|r|r.get(0)).unwrap().map(|r|r.unwrap()).collect();
        assert_eq!(left,vec![7,8,9,10,11,12],"the six newest windows stay; older ones made room");assert!(!root.join("preview_0.mp4").exists() && root.join("preview_11.mp4").exists());
    }
    #[test] fn deleting_a_session_removes_only_its_temporary_footage(){
        let(c,id,dir)=fixture();let p=segment(&c,&id,&dir,0,0.0);let clip=dir.parent().unwrap().join("kept.mp4");std::fs::write(&clip,"clip").unwrap();
        assert!(c.delete_session(&id).is_err() && p.exists(),"a recording still being written is kept");
        c.end(&id);assert_eq!(c.delete_session(&id).unwrap(),100);assert!(!p.exists() && clip.exists() && !dir.exists());assert!(c.state(7200).unwrap().sessions.is_empty());
    }
    #[test] fn history_off_drops_finished_sessions_but_not_live_held_or_legacy_footage(){
        let(c,live,dir)=fixture();let root=dir.parent().unwrap().to_path_buf();let keep=segment(&c,&live,&dir,0,0.0);
        let(a,da)=recording(&c,&root,"2026-11-10_08-00-00_7","Done",1_800_000_000_000);let gone=segment(&c,&a,&da,0,0.0);c.end(&a);
        let(b,db)=recording(&c,&root,"2026-11-10_09-00-00_7","Saving",1_800_003_600_000);let held=segment(&c,&b,&db,0,0.0);c.end(&b);let lease=c.lease(&b,0.0,2.0,"save").unwrap();
        let(l,dl)=recording(&c,&root,"2026-11-10_10-00-00_7","Legacy",1_800_007_200_000);let legacy=segment(&c,&l,&dl,0,0.0);c.end(&l);c.db.lock().unwrap().execute("UPDATE sessions SET managed=0 WHERE id=?1",[&l]).unwrap();
        assert_eq!(c.drop_finished(),100);assert!(!gone.exists() && !da.exists() && keep.exists() && held.exists() && legacy.exists());
        drop(lease);assert_eq!(c.drop_finished(),100);assert!(!held.exists() && keep.exists() && legacy.exists());
    }
    #[test] fn emptied_recordings_are_forgotten_and_legacy_footage_does_not_use_the_budget(){
        let(c,id,dir)=fixture();let root=dir.parent().unwrap().to_path_buf();segment(&c,&id,&dir,0,0.0);c.db.lock().unwrap().execute("UPDATE sessions SET managed=0",[]).unwrap();
        let(b,db)=recording(&c,&root,"2026-11-10_08-00-00_7","Game",1_800_000_000_000);for n in 0..5{segment(&c,&b,&db,n,n as f64*2.0);}c.end(&b);
        c.retain(10,u64::MAX,0).unwrap();assert_eq!(c.media(&b,0.0,99.0).unwrap().len(),5,"10 s of managed history fit; the legacy 2 s are not counted");
        c.retain(0,u64::MAX,0).unwrap();c.db.lock().unwrap().execute("UPDATE sessions SET ended_ms=1",[]).unwrap();
        assert!(Catalog::prune(&c.db.lock().unwrap()).unwrap());assert!(!db.exists());
        let ids:Vec<String>=c.state(7200).unwrap().sessions.into_iter().map(|s|s.id).collect();assert_eq!(ids,vec![id]);
    }
    #[test] fn long_histories_stay_cheap_to_maintain_and_publish(){
        let(c,id,dir)=fixture();{let mut db=c.db.lock().unwrap();let tx=db.transaction().unwrap();for n in 0..10800 {tx.execute("INSERT INTO segments VALUES(?1,?2,?3,?4,?5,1,1)",params![id,n,dir.join(format!("seg_{n:06}.ts")).to_string_lossy(),n as f64*2.0,(n+1) as f64*2.0]).unwrap();}tx.commit().unwrap();}
        c.touch();
        let t=std::time::Instant::now();for _ in 0..50{c.retain(21600,u64::MAX,0).unwrap();}let retain=t.elapsed()/50;
        let t=std::time::Instant::now();let state=c.state(21600).unwrap();let first=t.elapsed();let t=std::time::Instant::now();c.state(21600).unwrap();let cached=t.elapsed();
        assert_eq!(state.sessions[0].intervals,vec![[0.0,21600.0]]);
        println!("six-hour catalog (10800 segments): retention pass {retain:?}, state {first:?}, unchanged state {cached:?}");
        assert!(retain.as_millis()<50 && cached.as_micros()<5000);
    }
    /// Optional: `CAPTURE_CATALOG_COPY` names a folder holding a *copy* of a real `history.sqlite`.
    /// Opens it (migrating the copy), and prints the sessions the player would see. Only reads
    /// state: nothing is retained, evicted or deleted.
    #[test] #[ignore] fn a_copy_of_a_real_catalog_migrates_and_groups(){
        let Ok(dir)=std::env::var("CAPTURE_CATALOG_COPY") else {return;};
        let c=Catalog::open(Path::new(&dir)).unwrap();let t=std::time::Instant::now();let state=c.state(7200).unwrap();
        println!("migrated copy: {} sessions, {:.1} min kept, {:.1} GB, state built in {:?}",state.sessions.len(),state.retained_secs/60.0,state.bytes as f64/1e9,t.elapsed());
        for s in &state.sessions {println!("  {} | {} | {} part(s) | {} stretch(es) | {:.0} s | {} event(s)",s.id,s.game,s.parts.len(),s.intervals.len(),s.intervals.iter().map(|i|i[1]-i[0]).sum::<f64>(),s.events.len());}
        assert_eq!(c.db.lock().unwrap().query_row("PRAGMA user_version",[],|r|r.get::<_,i64>(0)).unwrap(),4);
    }
}
