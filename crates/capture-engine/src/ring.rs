//! Index of disk-backed replay segments: selection of a replay window, protection of segments
//! that an in-flight save needs, and retention (duration, byte cap, free-space reserve).
//!
//! Pure bookkeeping — no I/O except the deletion callback — so it is fully unit-tested.
//! All times are seconds on the session timeline (QPC-based, 0 = session start).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Segment {
    pub seq: u64,
    pub path: PathBuf,
    /// Timeline time of the segment's first (key)frame.
    pub start: f64,
    /// Timeline time where the next segment begins; `None` while still being written.
    pub end: Option<f64>,
    pub bytes: u64,
    /// Number of in-flight saves that need this segment.
    pub protect: u32,
}

#[derive(Debug, Default)]
pub struct Ring {
    segments: Vec<Segment>,
}

/// Segments chosen for a save plus the effective clip start/end on the timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub seqs: Vec<u64>,
    pub start: f64,
    pub end: f64,
}

impl Ring {
    pub fn push(&mut self, seg: Segment) {
        debug_assert!(self.segments.last().is_none_or(|l| l.seq < seg.seq));
        self.segments.push(seg);
    }

    /// Close the currently open segment.
    pub fn close_last(&mut self, end: f64, bytes: u64) {
        if let Some(s) = self.segments.last_mut() {
            if s.end.is_none() {
                s.end = Some(end);
                s.bytes = bytes;
            }
        }
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn total_bytes(&self) -> u64 {
        self.segments.iter().map(|s| s.bytes).sum()
    }

    /// Seconds of footage available ending at `now`.
    pub fn available_secs(&self, now: f64) -> f64 {
        self.segments.first().map(|s| (now - s.start).max(0.0)).unwrap_or(0.0)
    }

    /// Choose segments covering `[t_end - secs, t_end]`. The clip starts at the start of the
    /// newest segment that begins at or before the requested start (segments begin on a
    /// keyframe), so a clip may be up to one segment longer than requested — never shorter,
    /// unless the buffer itself is younger than `secs`.
    pub fn select(&self, t_end: f64, secs: f64) -> Option<Selection> {
        let want = t_end - secs;
        let usable: Vec<&Segment> = self.segments.iter().filter(|s| s.start < t_end).collect();
        if usable.is_empty() {
            return None;
        }
        let first = usable.iter().rposition(|s| s.start <= want).unwrap_or(0);
        let chosen = &usable[first..];
        Some(Selection { seqs: chosen.iter().map(|s| s.seq).collect(), start: chosen[0].start, end: t_end })
    }

    /// Segments after the selection that may still hold audio packets belonging before `t_end`
    /// (audio arrives slightly later than video). Returns seqs of segments starting within
    /// `margin` after `t_end`.
    pub fn tail_after(&self, t_end: f64, margin: f64) -> Vec<u64> {
        self.segments.iter().filter(|s| s.start >= t_end && s.start < t_end + margin).map(|s| s.seq).collect()
    }

    /// True once a *closed* segment starting at or after `t` exists. That implies the segment
    /// containing `t` is closed too, and that late-arriving audio for times before `t`
    /// (which lands in the following segment) is on disk.
    pub fn is_complete_through(&self, t: f64) -> bool {
        self.segments.iter().any(|s| s.start >= t && s.end.is_some())
    }

    pub fn protect(&mut self, seqs: &[u64]) {
        for s in self.segments.iter_mut().filter(|s| seqs.contains(&s.seq)) {
            s.protect += 1;
        }
    }

    pub fn unprotect(&mut self, seqs: &[u64]) {
        for s in self.segments.iter_mut().filter(|s| seqs.contains(&s.seq)) {
            s.protect = s.protect.saturating_sub(1);
        }
    }

    pub fn paths_for(&self, seqs: &[u64]) -> Vec<(u64, PathBuf, f64)> {
        self.segments.iter().filter(|s| seqs.contains(&s.seq)).map(|s| (s.seq, s.path.clone(), s.start)).collect()
    }

    /// Remove closed, unprotected segments, oldest first, while either
    /// (a) the footage before them is not needed: the segment ends before `now - keep_secs`, or
    /// (b) the ring exceeds `cap_bytes`, or (c) `need_free_bytes` more space is required.
    /// The open (newest) segment is never removed. Returns removed segments for deletion.
    pub fn retain(&mut self, now: f64, keep_secs: f64, cap_bytes: u64, mut need_free_bytes: u64) -> Vec<Segment> {
        let mut removed = Vec::new();
        let mut total = self.total_bytes();
        let mut i = 0;
        while i < self.segments.len() {
            let s = &self.segments[i];
            let closed = s.end.is_some();
            let expired = s.end.is_some_and(|e| e < now - keep_secs);
            let over_cap = total > cap_bytes;
            let need_space = need_free_bytes > 0;
            if closed && s.protect == 0 && (expired || over_cap || need_space) {
                let s = self.segments.remove(i);
                total -= s.bytes;
                need_free_bytes = need_free_bytes.saturating_sub(s.bytes);
                removed.push(s);
                continue;
            }
            if !(expired || over_cap || need_space) {
                break; // oldest-first: nothing newer qualifies either
            }
            i += 1; // protected/open: skip but keep looking at newer ones for cap/space
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(n: u64, len: f64, bytes: u64) -> Ring {
        let mut r = Ring::default();
        for i in 0..n {
            r.push(Segment {
                seq: i,
                path: format!("seg{i}").into(),
                start: i as f64 * len,
                end: if i + 1 < n { Some((i + 1) as f64 * len) } else { None },
                bytes,
                protect: 0,
            });
        }
        r
    }

    #[test]
    fn select_covers_requested_window_from_a_keyframe() {
        let r = ring(10, 2.0, 100); // 0..20 s
        let s = r.select(19.0, 5.0).unwrap(); // want 14..19
        assert_eq!(s.seqs, vec![7, 8, 9]);
        assert_eq!(s.start, 14.0);
        let s = r.select(19.0, 4.5).unwrap(); // want 14.5..19 -> starts at keyframe 14
        assert_eq!(s.start, 14.0);
    }

    #[test]
    fn young_buffer_gives_shorter_clip_from_oldest_segment() {
        let r = ring(3, 2.0, 100); // 0..6 s
        let s = r.select(5.0, 60.0).unwrap();
        assert_eq!(s.seqs, vec![0, 1, 2]);
        assert_eq!(s.end - s.start, 5.0);
        assert!(Ring::default().select(5.0, 60.0).is_none());
    }

    #[test]
    fn retention_never_deletes_protected_or_open_segments() {
        let mut r = ring(10, 2.0, 100);
        r.protect(&[0, 1]);
        let removed = r.retain(20.0, 6.0, u64::MAX, 0);
        // cutoff 14: segs 2..5 end at 6..12 -> removable; seg 6 ends exactly at 14 -> kept;
        // 0,1 protected
        assert_eq!(removed.iter().map(|s| s.seq).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        assert!(r.segments().iter().any(|s| s.seq == 0));
        r.unprotect(&[0, 1]);
        let removed = r.retain(20.0, 6.0, u64::MAX, 0);
        assert_eq!(removed.iter().map(|s| s.seq).collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn byte_cap_and_free_space_trim_oldest_first_but_keep_open_segment() {
        let mut r = ring(10, 2.0, 100);
        let removed = r.retain(20.0, 1000.0, 450, 0);
        assert_eq!(removed.len(), 6); // 1000 -> 400
        assert_eq!(r.total_bytes(), 400);
        let removed = r.retain(20.0, 1000.0, u64::MAX, 10_000);
        assert_eq!(removed.len(), 3); // everything closed goes, open one stays
        assert_eq!(r.segments().len(), 1);
        assert!(r.segments()[0].end.is_none());
    }

    #[test]
    fn overlapping_saves_keep_shared_segments_until_both_finish() {
        let mut r = ring(10, 2.0, 100);
        let a = r.select(16.0, 6.0).unwrap();
        let b = r.select(19.0, 6.0).unwrap();
        r.protect(&a.seqs);
        r.protect(&b.seqs);
        r.unprotect(&a.seqs);
        assert!(r.retain(100.0, 0.0, u64::MAX, 0).iter().all(|s| !b.seqs.contains(&s.seq)));
        r.unprotect(&b.seqs);
        r.retain(100.0, 0.0, u64::MAX, 0);
        assert!(r.segments().iter().all(|s| s.end.is_none()));
    }

    #[test]
    fn completion_requires_a_closed_segment_past_t() {
        let mut r = ring(3, 2.0, 100); // seg2 open from 4
        assert!(!r.is_complete_through(4.5));
        r.close_last(6.0, 100);
        r.push(Segment { seq: 3, path: "s3".into(), start: 6.0, end: None, bytes: 0, protect: 0 });
        assert!(!r.is_complete_through(4.5)); // segment starting after 4.5 is still open
        r.close_last(8.0, 100);
        assert!(r.is_complete_through(4.5));
        assert_eq!(r.tail_after(4.5, 3.0), vec![3]);
    }
}
