//! Screenshots: the current captured frame (already cropped to the game) saved as a PNG in
//! `<Game>\Screenshots\`. Uses the live capture texture, so no second capture is started.

use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use ffmpeg_sys_next as ff;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use crate::av::check;
use crate::gpu::Gpu;
use crate::{storage, wgc};

pub fn screenshots_dir(root: &Path, game: &str) -> PathBuf {
    storage::screenshots_dir(root, game)
}

/// Copy `latest` to the CPU and write it as PNG. Returns the file path.
pub fn save(gpu: &Gpu, capture: &wgc::Capture, root: &Path, game: &str) -> Result<PathBuf, String> {
    let (w, h) = (capture.width, capture.height);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        gpu.device.CreateTexture2D(&desc, None, Some(&mut staging)).map_err(|e| e.to_string())?;
        let staging = staging.unwrap();
        {
            let latest = capture.latest.lock().unwrap();
            if !latest.has_frame {
                return Err("No picture has been captured yet.".into());
            }
            gpu.context.CopyResource(&staging, &latest.texture);
        }
        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
        gpu.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).map_err(|e| e.to_string())?;
        for y in 0..h as usize {
            let src = std::slice::from_raw_parts((map.pData as *const u8).add(y * map.RowPitch as usize), w as usize * 4);
            pixels[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(src);
        }
        gpu.context.Unmap(&staging, 0);
    }
    let dir = screenshots_dir(root, game);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let name = format!("{}_{}.png", storage::local_stamp(), storage::file_token(game));
    let path = storage::reserve_unique_path(&dir, &name).map_err(|e| e.to_string())?;
    let png = unsafe { encode_png(&pixels, w, h)? };
    let partial = storage::partial_path(&path);
    std::fs::write(&partial, png).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

unsafe fn encode_png(bgra: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let codec = ff::avcodec_find_encoder(ff::AVCodecID::AV_CODEC_ID_PNG);
    if codec.is_null() {
        return Err("PNG encoder missing".into());
    }
    let mut ctx = ff::avcodec_alloc_context3(codec);
    (*ctx).width = w as i32;
    (*ctx).height = h as i32;
    (*ctx).pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_RGB24;
    (*ctx).time_base = ff::AVRational { num: 1, den: 1 };
    let r = (|| -> Result<Vec<u8>, String> {
        check(ff::avcodec_open2(ctx, codec, null_mut()), "open PNG encoder")?;
        let mut f = ff::av_frame_alloc();
        (*f).format = ff::AVPixelFormat::AV_PIX_FMT_RGB24 as i32;
        (*f).width = w as i32;
        (*f).height = h as i32;
        let res = (|| -> Result<Vec<u8>, String> {
            check(ff::av_frame_get_buffer(f, 0), "png frame")?;
            let sws = ff::sws_getContext(
                w as i32,
                h as i32,
                ff::AVPixelFormat::AV_PIX_FMT_BGRA,
                w as i32,
                h as i32,
                ff::AVPixelFormat::AV_PIX_FMT_RGB24,
                ff::SwsFlags::SWS_POINT as i32,
                null_mut(),
                null_mut(),
                null(),
            );
            let src = [bgra.as_ptr(), null(), null(), null()];
            let stride = [(w * 4) as i32, 0, 0, 0];
            ff::sws_scale(sws, src.as_ptr(), stride.as_ptr(), 0, h as i32, (*f).data.as_ptr(), (*f).linesize.as_ptr());
            ff::sws_freeContext(sws);
            check(ff::avcodec_send_frame(ctx, f), "encode png")?;
            let mut pkt = ff::av_packet_alloc();
            let out = check(ff::avcodec_receive_packet(ctx, pkt), "png packet").map(|_| std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize).to_vec());
            ff::av_packet_free(&mut pkt);
            out
        })();
        ff::av_frame_free(&mut f);
        res
    })();
    ff::avcodec_free_context(&mut ctx);
    r
}
