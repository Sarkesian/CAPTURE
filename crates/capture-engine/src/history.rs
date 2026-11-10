//! Rebuildable media catalog. Files remain authoritative; irreplaceable events also have a
//! durable append-only journal. All deletion and lease registration share the same mutex.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use rusqlite::{params, Connection, OptionalExtension};
use capture_protocol::{HistorySession, HistoryState, Review, TimelineEvent};
use crate::{save, storage};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String { e.to_string() }
pub fn millis() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
static NEXT: AtomicU64 = AtomicU64::new(1);
static STORES: OnceLock<Mutex<HashMap<PathBuf, Weak<Catalog>>>> = OnceLock::new();
pub const WINDOW_SECS: f64 = 30.0;
pub const PREVIEW_BUDGET: u64 = 256 * 1024 * 1024;

pub fn wait_for_saves(timeout:std::time::Duration) {
    let deadline=std::time::Instant::now()+timeout;
    while std::time::Instant::now()<deadline {
        let stores:Vec<_>=STORES.get_or_init(Default::default).lock().unwrap().values().filter_map(Weak::upgrade).collect();
        let active=stores.iter().any(|c|c.db.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='save' AND state='active')",[],|r|r.get::<_,bool>(0)).unwrap_or(true));
        if !active {break;}std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

pub struct Catalog { db: Mutex<Connection>, root: PathBuf, imports: Mutex<Vec<(String, PathBuf)>>, importing:std::sync::atomic::AtomicBool,scan: AtomicU64, reason: Mutex<Option<String>> }
#[derive(Clone, Debug)]
pub struct Media { pub path: PathBuf, pub start: f64, pub end: f64, pub bytes: u64 }
pub struct Lease { catalog: Arc<Catalog>, pub id: String, pub session: String, pub start: f64, pub end: f64, keep: bool }
impl Lease {
    pub fn failed(&mut self, message: &str) {
        self.keep = true;
        let _ = self.catalog.db.lock().unwrap().execute("UPDATE jobs SET state='failed',error=?2 WHERE id=?1", params![self.id,message]);
    }
}
impl Drop for Lease { fn drop(&mut self) { if !self.keep { let _ = self.catalog.db.lock().unwrap().execute("DELETE FROM jobs WHERE id=?1",[&self.id]); } } }

impl Catalog {
    pub fn import_complete(&self)->bool {!self.importing.load(Ordering::Acquire)}
    pub fn open(root: &Path) -> Result<Arc<Self>> {
        std::fs::create_dir_all(root).map_err(err)?;
        let key=PathBuf::from(std::fs::canonicalize(root).map_err(err)?.to_string_lossy().to_ascii_lowercase());
        let mut stores = STORES.get_or_init(Default::default).lock().unwrap();
        if let Some(c) = stores.get(&key).and_then(Weak::upgrade) { return Ok(c); }
        std::fs::create_dir_all(root).map_err(err)?;
        let db = Connection::open(root.join("history.sqlite")).map_err(err)?;
        db.busy_timeout(std::time::Duration::from_secs(5)).map_err(err)?;
        let version:i64=db.query_row("PRAGMA user_version",[],|r|r.get(0)).map_err(err)?;
        if version>3 {return Err("This history was created by a newer Capture Point version; it has not been modified".into());}
        if version==1 {
            db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE previews ADD COLUMN session TEXT; ALTER TABLE previews ADD COLUMN start REAL; ALTER TABLE previews ADD COLUMN end REAL; ALTER TABLE previews ADD COLUMN refs INTEGER NOT NULL DEFAULT 1; COMMIT;").map_err(err)?;
        }
        if version>0 && version<3 {db.execute_batch("ALTER TABLE sessions ADD COLUMN ended_ms INTEGER;").map_err(err)?;}
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,dir TEXT NOT NULL UNIQUE,game TEXT NOT NULL,created_ms INTEGER NOT NULL,ended INTEGER NOT NULL,config TEXT NOT NULL,managed INTEGER NOT NULL DEFAULT 0,ended_ms INTEGER);
          CREATE TABLE IF NOT EXISTS epochs(session TEXT NOT NULL REFERENCES sessions(id),epoch INTEGER NOT NULL,media_start REAL NOT NULL,config TEXT NOT NULL,PRIMARY KEY(session,epoch));
          CREATE TABLE IF NOT EXISTS segments(session TEXT NOT NULL REFERENCES sessions(id),seq INTEGER NOT NULL,path TEXT NOT NULL UNIQUE,start REAL NOT NULL,end REAL,bytes INTEGER NOT NULL,complete INTEGER NOT NULL,PRIMARY KEY(session,seq));
          CREATE INDEX IF NOT EXISTS segment_time ON segments(session,start,end);
          CREATE TABLE IF NOT EXISTS events(session TEXT NOT NULL REFERENCES sessions(id),id TEXT NOT NULL,data TEXT NOT NULL,PRIMARY KEY(session,id));
          CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,session TEXT NOT NULL,start REAL NOT NULL,end REAL NOT NULL,kind TEXT NOT NULL,state TEXT NOT NULL,error TEXT);
          CREATE INDEX IF NOT EXISTS job_ranges ON jobs(session,start,end);
          CREATE TABLE IF NOT EXISTS pins(session TEXT NOT NULL,id TEXT NOT NULL,start REAL NOT NULL,end REAL NOT NULL,PRIMARY KEY(session,id));
          CREATE TABLE IF NOT EXISTS previews(path TEXT PRIMARY KEY,bytes INTEGER NOT NULL,session TEXT,start REAL,end REAL,refs INTEGER NOT NULL DEFAULT 1);
          PRAGMA user_version=3;
          UPDATE sessions SET ended=1;
          UPDATE jobs SET state='failed',error='Recording interrupted; source retained for retry' WHERE kind='save' AND state='active';
          DELETE FROM jobs WHERE kind='playback';
          DELETE FROM pins WHERE id LIKE 'session:%';").map_err(err)?;
        let c = Arc::new(Self { db: Mutex::new(db), root: root.to_path_buf(), imports: Mutex::new(vec![]),importing:std::sync::atomic::AtomicBool::new(false),scan:AtomicU64::new(0),reason:Mutex::new(None) });
        // Stale preview files are ours, never user media. Do not delete them until accounted.
        c.reconcile()?;
        stores.insert(key,Arc::downgrade(&c));
        Ok(c)
    }
    pub fn register(&self, dir: &Path, game: &str, config: &serde_json::Value, managed: bool) -> Result<String> {
        let id = dir.file_name().ok_or("Session has no identity")?.to_string_lossy().to_string();
        let mut db=self.db.lock().unwrap();let tx=db.transaction().map_err(err)?;
        let created=config["started_unix_ms"].as_u64().unwrap_or_else(millis);
        tx.execute("INSERT INTO sessions(id,dir,game,created_ms,ended,config,managed) VALUES(?1,?2,?3,?4,0,?5,?6) ON CONFLICT(id) DO UPDATE SET ended=0,managed=MAX(managed,excluded.managed)",params![id,dir.to_string_lossy(),game,created as i64,config.to_string(),managed]).map_err(err)?;
        tx.execute("INSERT OR IGNORE INTO epochs VALUES(?1,0,0,?2)",params![id,config.to_string()]).map_err(err)?;tx.commit().map_err(err)?;
        Ok(id)
    }
    pub fn segment(&self, session: &str, seq: u64, path: &Path, start: f64, end: Option<f64>, bytes: u64) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT INTO segments VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(session,seq) DO UPDATE SET end=excluded.end,bytes=excluded.bytes,complete=excluded.complete",params![session,seq as i64,path.to_string_lossy(),start,end,bytes as i64,end.is_some()]).map_err(err)?;
        Ok(())
    }
    pub fn end(&self, session: &str) { let _ = self.db.lock().unwrap().execute("UPDATE sessions SET ended=1,ended_ms=COALESCE(ended_ms,?2) WHERE id=?1",params![session,millis() as i64]); }
    pub fn pin(&self, session: &str, id: &str, a: f64, b: f64) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT OR REPLACE INTO pins VALUES(?1,?2,?3,?4)",params![session,id,a,b]).map_err(err)?; Ok(())
    }
    pub fn unpin(&self, session: &str, id: &str) { let _ = self.db.lock().unwrap().execute("DELETE FROM pins WHERE session=?1 AND id=?2",params![session,id]); }
    pub fn event(&self, session: &str, event: &TimelineEvent) -> Result<()> {
        let db = self.db.lock().unwrap();
        let dir: String = db.query_row("SELECT dir FROM sessions WHERE id=?1",[session],|r|r.get(0)).map_err(err)?;
        let line = serde_json::to_string(event).map_err(err)?;
        // Journal first: a database rebuild can replay every event, including tombstones.
        use std::io::{Write,Read,Seek,SeekFrom};
        let mut f = std::fs::OpenOptions::new().create(true).read(true).append(true).open(Path::new(&dir).join("events.jsonl")).map_err(err)?;
        if f.metadata().map_err(err)?.len()>0 {
            f.seek(SeekFrom::End(-1)).map_err(err)?;let mut tail=[0u8];f.read_exact(&mut tail).map_err(err)?;
            if tail[0]!=b'\n' {f.write_all(b"\n").map_err(err)?;}
        }
        writeln!(f,"{line}").and_then(|_|f.sync_all()).map_err(err)?;
        db.execute("INSERT OR REPLACE INTO events VALUES(?1,?2,?3)",params![session,event.id,line]).map_err(err)?;
        Ok(())
    }
    pub fn import_failed(&self,message:&str) {
        *self.reason.lock().unwrap()=Some(format!("Earlier recordings could not be fully indexed: {message}. Original media is kept"));
        self.importing.store(false,Ordering::Release);
    }
    pub fn lease(self: &Arc<Self>, session: &str, start: f64, end: f64, kind: &str) -> Result<Lease> {
        if !start.is_finite() || !end.is_finite() || end <= start { return Err("Choose a valid recording range".into()); }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM segments WHERE session=?1 AND start<?3 AND COALESCE(end,1e99)>?2)",params![session,start,end],|r|r.get(0)).map_err(err)?;
        if !exists { return Err("That footage is no longer available".into()); }
        let id = format!("{}-{}",millis(),NEXT.fetch_add(1,Ordering::Relaxed));
        // Audio for boundary frames can be in adjacent segments. Protect before acknowledging.
        tx.execute("INSERT INTO jobs VALUES(?1,?2,?3,?4,?5,'active',NULL)",params![id,session,start-3.0,end+3.0,kind]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(Lease { catalog: self.clone(),id,session:session.into(),start,end,keep:false })
    }
    pub fn media(&self, session: &str, a: f64, b: f64) -> Result<Vec<Media>> {
        let db = self.db.lock().unwrap();
        let mut q = db.prepare("SELECT path,start,end,bytes FROM segments WHERE session=?1 AND complete=1 AND start<?3 AND end>?2 ORDER BY start").map_err(err)?;
        let rows = q.query_map(params![session,a,b],|r|Ok(Media {path:PathBuf::from(r.get::<_,String>(0)?),start:r.get(1)?,end:r.get(2)?,bytes:r.get::<_,i64>(3)? as u64})).map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>,_>>().map_err(err)
    }
    fn details(&self, session: &str) -> Result<(String, Vec<String>)> {
        let (game,config): (String,String) = self.db.lock().unwrap().query_row("SELECT game,config FROM sessions WHERE id=?1",[session],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        let v: serde_json::Value = serde_json::from_str(&config).map_err(err)?;
        Ok((game,v["audio_tracks"].as_array().map(|a|a.iter().filter_map(|s|s.as_str().map(String::from)).collect()).unwrap_or_default()))
    }
    pub fn save_clip(&self, lease: &Lease, root: &Path, save_id: u64) -> Result<capture_protocol::SavedClip> {
        let (game,tracks) = self.details(&lease.session)?;
        let mut segs = self.media(&lease.session,lease.start-3.0,lease.end+3.0)?;
        let first = segs.iter().rposition(|s|s.start <= lease.start).unwrap_or(0);
        let lead = if first>0 {Some(segs[first-1].path.clone())} else {None};
        segs.drain(..first);
        if segs.is_empty() { return Err("The selected footage is not on disk yet".into()); }
        // Never silently publish a clip omitting missing requested moments.
        if segs[0].start>lease.start+0.1 {return Err("The beginning of that selection is no longer available".into());}
        let mut cursor = lease.start.max(segs[0].start);
        for s in &segs { if s.start > cursor+0.1 && cursor<lease.end { return Err("The selection crosses unavailable footage. Choose one available section.".into()); } cursor=cursor.max(s.end); }
        if cursor+0.1 < lease.end { return Err("The end of the selection is not recorded yet. Try again in a moment.".into()); }
        for s in &segs { if !s.path.is_file() { return Err("A selected recording segment is missing; remaining footage has been kept".into()); } }
        let dir=storage::clips_dir(root,&game); std::fs::create_dir_all(&dir).map_err(err)?;
        let needed:u64=segs.iter().map(|s|s.bytes).sum();
        if storage::free_bytes(&dir).is_some_and(|free|free<needed+32*1024*1024) {return Err("Not enough space to finalize this clip. Its original footage is protected for retry.".into());}
        let path=storage::reserve_unique_path(&dir,&storage::clip_file_name(&storage::local_stamp(),&game,"Clip")).map_err(err)?;
        let list:Vec<_>=segs.iter().map(|s|(s.path.clone(),s.start)).collect();
        let (clip,actual)=save::remux(&list,lead.as_deref(),segs[0].start,lease.start,lease.end,&tracks,&path)?;
        if clip.duration_secs+0.15 < lease.end-actual { return Err("Clip verification found incomplete footage; source retained for retry".into()); }
        let events=self.events(&lease.session)?;
        let markers:Vec<_>=events.iter().filter(|e|e.kind=="marker" && e.t>=actual && e.t<=lease.end).map(|e|serde_json::json!({"t":e.t-actual,"label":e.label})).collect();
        if !markers.is_empty() { std::fs::write(format!("{}.markers.json",path.display()),serde_json::to_vec_pretty(&serde_json::json!({"capture_markers":1,"markers":markers})).map_err(err)?).map_err(err)?; }
        self.event(&lease.session,&TimelineEvent{id:format!("clip:{}",path.display()),kind:"clip".into(),t:actual,end:Some(lease.end),label:path.file_name().unwrap_or_default().to_string_lossy().into_owned(),path:Some(path.to_string_lossy().into_owned()),marker:None})?;
        self.db.lock().unwrap().execute("DELETE FROM jobs WHERE state='failed' AND kind='save' AND session=?1 AND start>=?2 AND end<=?3",params![lease.session,lease.start-3.0,lease.end+3.0]).map_err(err)?;
        Ok(capture_protocol::SavedClip { save_id,path:path.to_string_lossy().into_owned(),requested_secs:lease.end-lease.start,duration_secs:clip.duration_secs,bytes:clip.bytes,video_streams:clip.video_streams,audio_tracks:clip.audio_tracks })
    }
    pub fn preview(self: &Arc<Self>, session: &str, at: f64) -> Result<Review> {
        let (game,tracks)=self.details(session)?;
        let nearby=self.media(session,(at-5.0).max(0.0),at+WINDOW_SECS)?;
        let anchor=nearby.iter().find(|s|s.end>at && s.start<=at).or_else(||nearby.first()).ok_or("No footage at this position")?;
        let start=anchor.start;
        let mut end=anchor.end;
        let mut window_bytes=0;
        for s in nearby.iter().filter(|s|s.start>=start) {
            if s.start>end+0.1 {break;}
            if window_bytes>0 && window_bytes+s.bytes>64*1024*1024 {break;}
            window_bytes+=s.bytes;end=s.end;if end>=start+WINDOW_SECS {break;}
        }
        let _lease=self.lease(session,start,end,"playback")?;
        {
            let db=self.db.lock().unwrap();
            let cached:Option<(String,f64,f64)>=db.query_row("SELECT path,start,end FROM previews WHERE session=?1 AND start<=?2 AND end>?2 AND refs>0 LIMIT 1",params![session,at],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
            if let Some((path,start,end))=cached.filter(|(p,_,_)|Path::new(p).is_file()) {
                db.execute("UPDATE previews SET refs=refs+1 WHERE path=?1",[&path]).map_err(err)?;drop(db);
                return Ok(Review{session:session.into(),path,start,end,game,past:true,markers:self.events(session)?.into_iter().filter_map(|e|e.marker).collect()});
            }
        }
        let media=self.media(session,start-3.0,end+3.0)?;
        let first=media.iter().position(|s|s.start>=start-0.01).ok_or("Preview footage expired")?;
        let lead=first.checked_sub(1).map(|i|media[i].path.as_path());
        let list:Vec<_>=media[first..].iter().map(|s|(s.path.clone(),s.start)).collect();
        let path=self.root.join(format!("preview_{}_{}.mp4",millis(),NEXT.fetch_add(1,Ordering::Relaxed)));
        let needed: u64=media.iter().map(|s|s.bytes).sum();
        if storage::free_bytes(&self.root).is_some_and(|f|f<needed+32*1024*1024){return Err("Not enough free space to open this playback window. Original footage is unchanged.".into());}
        let db=self.db.lock().unwrap();
        let cached:i64=db.query_row("SELECT COALESCE(SUM(bytes),0) FROM previews",[],|r|r.get(0)).map_err(err)?;
        if cached as u64+needed>PREVIEW_BUDGET { return Err("Replay cache is busy. Close another Replay view and try again.".into()); }
        // Reservation counts against budget during remux too.
        db.execute("INSERT INTO previews(path,bytes,session,start,end,refs) VALUES(?1,?2,?3,?4,?5,1)",params![path.to_string_lossy(),needed as i64,session,start,end]).map_err(err)?; drop(db);
        match save::remux(&list,lead,start,start,end,&tracks,&path) {
            Ok((clip,actual))=>{ self.db.lock().unwrap().execute("UPDATE previews SET bytes=?2 WHERE path=?1",params![path.to_string_lossy(),clip.bytes as i64]).map_err(err)?; Ok(Review {session:session.into(),path:path.to_string_lossy().into_owned(),start:actual,end:actual+clip.duration_secs,game,past:true,markers:self.events(session)?.into_iter().filter_map(|e|e.marker).collect()}) },
            Err(e)=>{self.release_preview(&path.to_string_lossy());Err(e)}
        }
    }
    pub fn release_preview(&self,path:&str) {
        let db=self.db.lock().unwrap();
        let ours:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM previews WHERE path=?1)",[path],|r|r.get(0)).unwrap_or(false);
        if ours {
            let _=db.execute("UPDATE previews SET refs=MAX(0,refs-1) WHERE path=?1",[path]);
            let refs:i64=db.query_row("SELECT refs FROM previews WHERE path=?1",[path],|r|r.get(0)).unwrap_or(1);
            if refs==0 && (std::fs::remove_file(path).is_ok() || !Path::new(path).exists()) { let _=db.execute("DELETE FROM previews WHERE path=?1",[path]); }
        }
    }
    pub fn events(&self,session:&str)->Result<Vec<TimelineEvent>> {
        let db=self.db.lock().unwrap(); let mut q=db.prepare("SELECT data FROM events WHERE session=?1").map_err(err)?;
        let rows=q.query_map([session],|r|r.get::<_,String>(0)).map_err(err)?;
        rows.map(|r|serde_json::from_str::<TimelineEvent>(&r.map_err(err)?).map_err(err)).filter(|e|!matches!(e,Ok(v) if v.kind=="removed")).collect()
    }
    pub fn state(&self,target_secs:u32)->Result<HistoryState> {
        let db=self.db.lock().unwrap();
        let mut q=db.prepare("SELECT id,game,created_ms,ended FROM sessions ORDER BY created_ms,id").map_err(err)?;
        let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,bool>(3)?))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
        let bytes:i64=db.query_row("SELECT COALESCE(SUM(bytes),0) FROM segments",[],|r|r.get(0)).map_err(err)?;
        let preview_bytes:i64=db.query_row("SELECT COALESCE(SUM(bytes),0) FROM previews",[],|r|r.get(0)).map_err(err)?; drop(q); drop(db);
        let failed_saves={let db=self.db.lock().unwrap();let mut q=db.prepare("SELECT id,session,start+3,end-3,COALESCE(error,'Save interrupted') FROM jobs WHERE state='failed' AND kind='save' ORDER BY id").map_err(err)?;let rows=q.query_map([],|r|Ok(capture_protocol::FailedHistorySave{id:r.get(0)?,session:r.get(1)?,start:r.get(2)?,end:r.get(3)?,error:r.get(4)?})).map_err(err)?;rows.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?};
        let mut sessions=vec![]; let mut retained_secs=0.0;
        for (id,game,created_ms,ended) in rows {
            let media=self.media(&id,-1e99,1e99)?; let mut intervals:Vec<[f64;2]>=vec![];
            for s in media { if let Some(last)=intervals.last_mut().filter(|l|s.start<=l[1]+0.05) {last[1]=last[1].max(s.end);} else {intervals.push([s.start,s.end]);} }
            retained_secs+=intervals.iter().map(|r|r[1]-r[0]).sum::<f64>();
            let events=self.events(&id)?; if !intervals.is_empty() || !events.is_empty() {sessions.push(HistorySession{id,game,created_ms:created_ms as u64,ended,intervals,events});}
        }
        Ok(HistoryState{failed_saves,sessions,bytes:bytes as u64,preview_bytes:preview_bytes as u64,retained_secs,target_secs,reason:self.reason.lock().unwrap().clone()})
    }
    /// One serialization point for global eviction and lease acquisition. A failed unlink
    /// remains indexed/accounted. Imported legacy media are never evicted automatically.
    pub fn retain(&self,secs:u32,cap:u64,reserve:u64)->Result<Option<String>> {
        let db=self.db.lock().unwrap();
        let mut expired=db.prepare("SELECT path FROM previews WHERE refs=0").map_err(err)?;
        let files=expired.query_map([],|r|r.get::<_,String>(0)).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;drop(expired);
        for p in files {if std::fs::remove_file(&p).is_ok() || !Path::new(&p).exists(){db.execute("DELETE FROM previews WHERE path=?1",[p]).map_err(err)?;}}
        // Marker protection is global, bounded to half the unchanged storage budget.
        loop {
            let held:i64=db.query_row("SELECT COALESCE(SUM(bytes),0) FROM segments s WHERE EXISTS(SELECT 1 FROM pins p WHERE p.session=s.session AND p.id LIKE 'marker:%' AND p.start<COALESCE(s.end,1e99) AND p.end>s.start)",[],|r|r.get(0)).map_err(err)?;
            if held<=cap as i64/2 {break;}
            let pin:Option<(String,String)>=db.query_row("SELECT p.session,p.id FROM pins p JOIN sessions x ON x.id=p.session WHERE p.id LIKE 'marker:%' ORDER BY x.created_ms,p.start LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
            let Some((session,id))=pin else {break;};db.execute("DELETE FROM pins WHERE session=?1 AND id=?2",params![session,id]).map_err(err)?;
        }
        let mut q=db.prepare("SELECT s.session,s.seq,s.path,s.start,s.end,s.bytes,x.managed FROM segments s JOIN sessions x ON x.id=s.session WHERE s.complete=1 ORDER BY x.created_ms,x.id,s.start").map_err(err)?;
        let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,f64>(3)?,r.get::<_,f64>(4)?,r.get::<_,i64>(5)?,r.get::<_,bool>(6)?))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;drop(q);
        let mut duration:f64=rows.iter().map(|r|(r.4-r.3).max(0.0)).sum();
        let mut bytes:i64=db.query_row("SELECT COALESCE(SUM(bytes),0)+(SELECT COALESCE(SUM(bytes),0) FROM previews) FROM segments",[],|r|r.get(0)).map_err(err)?;
        let mut reason=None;
        for (session,seq,path,a,b,size,managed) in rows {
            let low=storage::free_bytes(&self.root).is_some_and(|f|f<reserve);
            if duration<=secs as f64 && bytes<=cap as i64 && !low {break;}
            reason=Some(if low {"Free-space reserve"} else if bytes>cap as i64 {"Storage budget"} else {"History duration"}.into());
            let protected:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE session=?1 AND start<?3 AND end>?2 UNION ALL SELECT 1 FROM pins WHERE session=?1 AND start<?3 AND end>?2)",params![session,a,b],|r|r.get(0)).map_err(err)?;
            if !managed || protected {continue;}
            if std::fs::remove_file(&path).is_ok() || !Path::new(&path).exists() {db.execute("DELETE FROM segments WHERE session=?1 AND seq=?2",params![session,seq]).map_err(err)?; duration-=(b-a).max(0.0);bytes=bytes.saturating_sub(size);}
        }
        if reason.is_some() {*self.reason.lock().unwrap()=reason.clone();}
        Ok(reason)
    }
    fn reconcile(&self)->Result<()> {
        let db=self.db.lock().unwrap();
        let mut q=db.prepare("SELECT path FROM previews").map_err(err)?;
        let previews=q.query_map([],|r|r.get::<_,String>(0)).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;drop(q);
        for p in previews {
            let partial=storage::partial_path(Path::new(&p));let partial_gone=std::fs::remove_file(&partial).is_ok() || !partial.exists();
            if partial_gone && (std::fs::remove_file(&p).is_ok() || !Path::new(&p).exists()){db.execute("DELETE FROM previews WHERE path=?1",[p]).map_err(err)?;}
        }
        let mut imports=vec![];
        for e in std::fs::read_dir(&self.root).map_err(err)?.flatten().filter(|e|e.path().is_dir() && storage::is_session_dir_name(&e.file_name().to_string_lossy())) {
            let dir=e.path(); let id=e.file_name().to_string_lossy().into_owned();
            let manifest:serde_json::Value=std::fs::read(dir.join("session.json")).ok().and_then(|v|serde_json::from_slice(&v).ok()).unwrap_or(serde_json::json!({"game":"Recovered gameplay"}));
            let game=manifest["game"].as_str().unwrap_or("Recovered gameplay");
            let created=manifest["started_unix_ms"].as_u64().or_else(||std::fs::metadata(&dir).ok()?.created().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d|d.as_millis() as u64)).unwrap_or_else(millis);
            db.execute("INSERT OR IGNORE INTO sessions(id,dir,game,created_ms,ended,config,managed) VALUES(?1,?2,?3,?4,1,?5,0)",params![id,dir.to_string_lossy(),game,created as i64,manifest.to_string()]).map_err(err)?;
            db.execute("INSERT OR IGNORE INTO epochs VALUES(?1,0,0,?2)",params![id,manifest.to_string()]).map_err(err)?;
            if let Ok(lines)=std::fs::read_to_string(dir.join("events.jsonl")) {for line in lines.lines(){if let Ok(ev)=serde_json::from_str::<TimelineEvent>(line){db.execute("INSERT OR REPLACE INTO events VALUES(?1,?2,?3)",params![id,ev.id,line]).map_err(err)?;}}}
            if let Ok(markers)=std::fs::read(dir.join("markers.json")) {if let Ok(ms)=serde_json::from_slice::<Vec<capture_protocol::Marker>>(&markers) {for m in ms {let ev=TimelineEvent{id:format!("marker:{}",m.id),kind:"marker".into(),t:m.t,end:None,label:m.label.clone(),path:None,marker:Some(m)};db.execute("INSERT OR IGNORE INTO events VALUES(?1,?2,?3)",params![id,ev.id,serde_json::to_string(&ev).map_err(err)?]).map_err(err)?;}}}
            for f in std::fs::read_dir(&dir).map_err(err)?.flatten() {
                let p=f.path();let name=f.file_name().to_string_lossy().into_owned();
                if name.starts_with("seg_") && name.ends_with(".ts") {
                    let known:Option<bool>=db.query_row("SELECT complete FROM segments WHERE path=?1",[p.to_string_lossy()],|r|r.get(0)).optional().map_err(err)?;
                    if known!=Some(true) {imports.push((id.clone(),p));}
                }
            }
        }
        self.importing.store(!imports.is_empty(),Ordering::Release);*self.imports.lock().unwrap()=imports; Ok(())
    }
    /// Recovery is incremental; opening Replay never probes the entire history.
    pub fn import_step(&self)->Result<()> {
        for _ in 0..4 {
            let Some((id,p))=self.imports.lock().unwrap().pop() else {break;};
            if let (Some(start),Some(end))=(save::first_keyframe_secs(&p),save::last_video_secs(&p)) {
                let db=self.db.lock().unwrap();
                let config:String=db.query_row("SELECT config FROM sessions WHERE id=?1",[&id],|r|r.get(0)).map_err(err)?;
                let mut config:serde_json::Value=serde_json::from_str(&config).map_err(err)?;
                if config.get("audio_tracks").is_none() {
                    if let Some(names)=save::recovered_audio_names(&p){config["audio_tracks"]=serde_json::to_value(names).map_err(err)?;db.execute("UPDATE sessions SET config=?2 WHERE id=?1",params![id,config.to_string()]).map_err(err)?;db.execute("UPDATE epochs SET config=?2 WHERE session=?1",params![id,config.to_string()]).map_err(err)?;}
                }
                drop(db);
                let seq=p.file_stem().and_then(|s|s.to_str()).and_then(|s|s.strip_prefix("seg_")).and_then(|s|s.parse().ok()).unwrap_or(0);
                self.segment(&id,seq,&p,start,Some(end+1.0/60.0),std::fs::metadata(&p).map(|m|m.len()).unwrap_or(0))?;
            }
        }
        if self.imports.lock().unwrap().is_empty(){self.importing.store(false,Ordering::Release);}
        Ok(())
    }
    pub fn refresh_files(&self)->Result<()> {
        let db=self.db.lock().unwrap();let cursor=self.scan.load(Ordering::Relaxed) as i64;
        let mut q=db.prepare("SELECT rowid,path FROM segments WHERE rowid>?1 AND complete=1 ORDER BY rowid LIMIT 64").map_err(err)?;
        let rows=q.query_map([cursor],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;drop(q);
        if rows.is_empty(){self.scan.store(0,Ordering::Relaxed);}
        for (id,p) in rows {self.scan.store(id as u64,Ordering::Relaxed);if let Err(e)=std::fs::metadata(&p){if e.kind()==std::io::ErrorKind::NotFound {db.execute("DELETE FROM segments WHERE rowid=?1",[id]).map_err(err)?;}}}
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
}
