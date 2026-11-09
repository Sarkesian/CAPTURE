//! Clip duration for the clips lists (reads the container header only; writes nothing).

use std::path::Path;
use std::ptr::{null, null_mut};

use ffmpeg_sys_next as ff;

use crate::av::{self, check};

struct Fmt(*mut ff::AVFormatContext);
impl Drop for Fmt {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.0) };
    }
}

pub fn duration_secs(clip: &Path) -> Result<f64, String> {
    if !clip.is_file() {
        return Err("Clip not found".into());
    }
    unsafe {
        let mut ic = null_mut();
        check(ff::avformat_open_input(&mut ic, av::cstr(&clip.to_string_lossy()).as_ptr(), null(), null_mut()), "open clip")?;
        let ic = Fmt(ic);
        check(ff::avformat_find_stream_info(ic.0, null_mut()), "read clip")?;
        Ok((*ic.0).duration as f64 / ff::AV_TIME_BASE as f64)
    }
}
