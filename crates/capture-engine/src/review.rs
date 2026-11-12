//! Background work for Replay views: opening playback windows and making hover thumbnails.
//!
//! One worker each, so nothing here ever runs on the engine thread or competes with itself
//! for the disk. Requests are per viewer ("main" window, "overlay"): a viewer that asks again
//! before its earlier request was started replaces it (scrubbing produces a burst of requests
//! of which only the newest matters), and what was replaced is answered as superseded so no
//! caller waits forever. A window being prefetched for continuous playback never delays a seek.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use capture_protocol::{Event, Message};

use crate::history::Catalog;
use crate::Emit;

pub struct ReviewJob {
    pub re: Option<u64>,
    pub catalog: Arc<Catalog>,
    pub session: String,
    pub at: f64,
    pub viewer: String,
    pub prefetch: bool,
}

pub struct ThumbJob {
    pub re: Option<u64>,
    pub catalog: Arc<Catalog>,
    pub session: String,
    pub at: f64,
    pub viewer: String,
    pub exact: bool,
}

#[derive(Default)]
struct Queue {
    reviews: VecDeque<ReviewJob>,
    thumbs: VecDeque<ThumbJob>,
    stop: bool,
}

struct Inner {
    queue: Mutex<Queue>,
    wake: Condvar,
    emit: Emit,
}

pub struct Jobs {
    inner: Arc<Inner>,
}

fn superseded_review(emit: &Emit, job: &ReviewJob) {
    emit(Message::new(job.re, Event::ReviewFailed { message: "A newer position was requested".into(), superseded: true }));
}

fn superseded_thumb(emit: &Emit, job: ThumbJob) {
    emit(Message::new(job.re, Event::HistoryThumb { session: job.session, at: job.at, t: None, image: None, error: Some("superseded".into()) }));
}

impl Jobs {
    pub fn start(emit: Emit) -> Self {
        let inner = Arc::new(Inner { queue: Mutex::new(Queue::default()), wake: Condvar::new(), emit });
        for thumbs in [false, true] {
            let inner = inner.clone();
            let _ = std::thread::Builder::new().name(if thumbs { "replay-thumbs" } else { "replay-windows" }.into()).spawn(move || run(inner, thumbs));
        }
        Self { inner }
    }

    pub fn review(&self, job: ReviewJob) {
        let mut q = self.inner.queue.lock().unwrap();
        // A seek replaces the viewer's pending seek and its pending prefetch (which was for the
        // place it is leaving); a prefetch only replaces a pending prefetch.
        let (mut kept, mut dropped) = (VecDeque::new(), vec![]);
        for old in q.reviews.drain(..) {
            if old.viewer == job.viewer && (old.prefetch || !job.prefetch) {
                dropped.push(old);
            } else {
                kept.push_back(old);
            }
        }
        q.reviews = kept;
        q.reviews.push_back(job);
        drop(q);
        dropped.iter().for_each(|old| superseded_review(&self.inner.emit, old));
        self.inner.wake.notify_all();
    }

    pub fn thumb(&self, job: ThumbJob) {
        let mut q = self.inner.queue.lock().unwrap();
        let (mut kept, mut dropped) = (VecDeque::new(), vec![]);
        for old in q.thumbs.drain(..) {
            if old.viewer == job.viewer {
                dropped.push(old);
            } else {
                kept.push_back(old);
            }
        }
        q.thumbs = kept;
        q.thumbs.push_back(job);
        drop(q);
        dropped.into_iter().for_each(|old| superseded_thumb(&self.inner.emit, old));
        self.inner.wake.notify_all();
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        self.inner.queue.lock().unwrap().stop = true;
        self.inner.wake.notify_all();
    }
}

fn run(inner: Arc<Inner>, thumbs: bool) {
    loop {
        let mut q = inner.queue.lock().unwrap();
        loop {
            if q.stop {
                return;
            }
            if if thumbs { !q.thumbs.is_empty() } else { !q.reviews.is_empty() } {
                break;
            }
            q = inner.wake.wait(q).unwrap();
        }
        if thumbs {
            let job = q.thumbs.pop_front().unwrap();
            drop(q);
            let event = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.catalog.thumb(&job.session, job.at, job.exact))).unwrap_or_else(|_| Err("Preview picture failed".into())) {
                Ok((t, image)) => Event::HistoryThumb { session: job.session, at: job.at, t: Some(t), image: Some(image.to_string()), error: None },
                Err(error) => Event::HistoryThumb { session: job.session, at: job.at, t: None, image: None, error: Some(error) },
            };
            (inner.emit)(Message::new(job.re, event));
        } else {
            // Seeks before prefetches.
            let index = q.reviews.iter().position(|j| !j.prefetch).unwrap_or(0);
            let job = q.reviews.remove(index).unwrap();
            drop(q);
            let event = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.catalog.preview(&job.session, job.at, !job.prefetch))).unwrap_or_else(|_| Err("This moment could not be opened".into())) {
                Ok(review) => Event::ReviewReady(review),
                Err(message) => Event::ReviewFailed { message, superseded: false },
            };
            (inner.emit)(Message::new(job.re, event));
        }
    }
}
