//! Session timeline: QPC in 100 ns units (the unit WASAPI reports), 0 = session start.

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub fn now_100ns() -> u64 {
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
        let _ = QueryPerformanceFrequency(&mut f);
    }
    (c as i128 * 10_000_000 / f as i128) as u64
}

pub fn secs_since(t0: u64) -> f64 {
    (now_100ns() as i64 - t0 as i64) as f64 / 1e7
}

pub fn qpc100ns_to_samples(t: u64, t0: u64, rate: u32) -> i64 {
    ((t as i128 - t0 as i128) * rate as i128 / 10_000_000) as i64
}

pub fn timeline_samples(t0: u64, rate: u32) -> i64 {
    qpc100ns_to_samples(now_100ns(), t0, rate)
}

#[cfg(test)]
mod tests {
    #[test]
    fn conversions() {
        assert_eq!(super::qpc100ns_to_samples(10_000_000 + 5, 5, 48_000), 48_000);
        assert_eq!(super::qpc100ns_to_samples(0, 10_000_000, 48_000), -48_000);
    }
}
