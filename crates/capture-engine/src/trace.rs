//! Opt-in timing trace for diagnosing capture smoothness. Set `CAPTURE_TRACE=<file.csv>` before
//! the worker starts; every captured frame and every encoder tick then appends one CSV line
//! (`kind,t,a,b,c`, times in 100 ns QPC units). Off (one relaxed load per call) otherwise.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};

static SINK: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();

fn sink() -> &'static Option<Mutex<BufWriter<File>>> {
    SINK.get_or_init(|| {
        let path = std::env::var_os("CAPTURE_TRACE")?;
        let f = File::create(path).ok()?;
        let mut w = BufWriter::with_capacity(1 << 20, f);
        let _ = writeln!(w, "kind,t,a,b,c");
        Some(Mutex::new(w))
    })
}

pub fn enabled() -> bool {
    sink().is_some()
}

pub fn ev(kind: &str, t: u64, a: i64, b: i64, c: i64) {
    if let Some(m) = sink() {
        if let Ok(mut w) = m.lock() {
            let _ = writeln!(w, "{kind},{t},{a},{b},{c}");
        }
    }
}

pub fn flush() {
    if let Some(m) = sink() {
        if let Ok(mut w) = m.lock() {
            let _ = w.flush();
        }
    }
}
