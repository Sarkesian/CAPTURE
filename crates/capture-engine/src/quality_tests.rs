//! Explicit hardware acceptance fixtures. No installed application or user media is touched.
use crate::{
    gpu::Gpu,
    mux::{SegmentWriter, StreamDesc},
    video::VideoEncoder,
    wgc,
};
use capture_protocol::Quality;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

fn bin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/ffmpeg/bin")
        .join(name)
}
fn folder() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-output")
        .join(format!("quality-{}", crate::history::millis()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn probe(path: &Path) -> serde_json::Value {
    let o = std::process::Command::new(bin("ffprobe.exe"))
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(o.status.success());
    serde_json::from_slice(&o.stdout).unwrap()
}
fn encode(
    mut enc: VideoEncoder,
    source: &wgc::Latest,
    root: &Path,
    label: &str,
    frames: i64,
) -> (PathBuf, f64, f64) {
    let segment = root.join(format!("{label}.ts"));
    let mut writer = SegmentWriter::create(
        &segment,
        &[unsafe { StreamDesc::from_ctx(enc.codec_ctx()) }],
    )
    .unwrap();
    let grabber = enc.grabber();
    let started = Instant::now();
    // Match production's separate clock and encode threads and bounded GPU queue.
    let (tx, rx) = std::sync::mpsc::sync_channel(6);
    let thread = std::thread::spawn(move || {
        for g in rx {
            enc.encode(g, &mut |p| writer.write(p).unwrap()).unwrap();
        }
        enc.flush(&mut |p| writer.write(p).unwrap());
        writer.finish();
    });
    for n in 0..frames {
        tx.send(grabber.grab(source, n).unwrap()).unwrap();
    }
    drop(tx);
    thread.join().unwrap();
    let elapsed = started.elapsed().as_secs_f64();
    let clip = root.join(format!("{label}.mp4"));
    let save = Instant::now();
    crate::save::remux(
        &[(segment, 0.0)],
        None,
        0.0,
        0.0,
        frames as f64 / 60.0,
        &[],
        &clip,
    )
    .unwrap();
    let ms = save.elapsed().as_secs_f64() * 1000.0;
    (clip, elapsed, ms)
}

#[test]
#[ignore = "Explicit NVIDIA synthetic native-resolution throughput test; not gameplay"]
fn native_sdr_hdr_resolution_fps_and_remux() {
    let gpu = Arc::new(Gpu::create().unwrap());
    let root = folder();
    let mut results = vec![];
    for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
        for hdr in [false, true] {
            let label = format!("{}-{w}x{h}", if hdr { "hdr" } else { "sdr" });
            let source = if hdr {
                let mut pixels = vec![0u16; (w * h * 4) as usize];
                for y in 0..h {
                    for x in 0..w {
                        let i = ((y * w + x) * 4) as usize;
                        pixels[i] = crate::hdr::test_half(x as f32 / w as f32 * 12.5);
                        pixels[i + 1] = crate::hdr::test_half(y as f32 / h as f32 * 4.0);
                        pixels[i + 2] = crate::hdr::test_half(0.1);
                        pixels[i + 3] = crate::hdr::test_half(1.0);
                    }
                }
                wgc::synthetic_hdr(&gpu, w, h, &pixels)
            } else {
                let mut pixels = vec![255u8; (w * h * 4) as usize];
                for y in 0..h {
                    for x in 0..w {
                        let i = ((y * w + x) * 4) as usize;
                        pixels[i] = (x % 256) as u8;
                        pixels[i + 1] = (y % 256) as u8;
                        pixels[i + 2] = ((x + y) % 256) as u8;
                    }
                }
                wgc::synthetic_latest(&gpu, w, h, &pixels)
            };
            let enc = if hdr {
                VideoEncoder::open_hdr(gpu.clone(), w, h, 60, Quality::Maximum, false)
            } else {
                VideoEncoder::open(gpu.clone(), w, h, 60, Quality::Maximum, false)
            }
            .unwrap();
            let (clip, elapsed, save_ms) = encode(enc, &source, &root, &label, 180);
            let info = probe(&clip);
            let s = &info["streams"][0];
            assert_eq!(s["width"], w);
            assert_eq!(s["height"], h);
            assert_eq!(s["nb_read_frames"], "180");
            assert_eq!(s["avg_frame_rate"], "60/1");
            assert_eq!(s["pix_fmt"], if hdr { "yuv420p10le" } else { "yuv420p" });
            assert_eq!(s["color_transfer"], if hdr { "smpte2084" } else { "bt709" });
            if hdr {
                assert_eq!(s["codec_tag_string"], "hvc1");
            }
            let decoded = std::process::Command::new(bin("ffmpeg.exe"))
                .args(["-v", "error", "-i"])
                .arg(&clip)
                .args(["-f", "null", "-"])
                .output()
                .unwrap();
            assert!(
                decoded.status.success(),
                "{}",
                String::from_utf8_lossy(&decoded.stderr)
            );
            let result = serde_json::json!({"label":label,"submitted_frames":180,"decoded_frames":180,"encode_seconds":elapsed,"throughput_fps":180.0/elapsed,"remux_ms":save_ms,"bytes":std::fs::metadata(&clip).unwrap().len(),"path":clip});
            println!("{result}");
            results.push(result);
        }
    }
    std::fs::write(
        root.join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
    println!("Results: {}", root.display());
}

#[test]
#[ignore = "Explicit FP16 HDR-to-SDR GPU and software-encoder color proof"]
fn compatible_hdr_tone_mapping_roundtrip() {
    let gpu = Arc::new(Gpu::create().unwrap());
    let root = folder();
    let bands = [
        [0.0, 0.0, 0.0],
        [0.01, 0.01, 0.01],
        [0.5, 0.5, 0.5],
        [2.5375, 2.5375, 2.5375],
        [12.5, 12.5, 12.5],
        [1.0, 0.1, 0.2],
        [0.1, 1.0, 0.2],
        [0.1, 0.2, 1.0],
    ];
    let mut pixels = vec![0u16; 256 * 144 * 4];
    for y in 0..144 {
        for x in 0..256 {
            let i = (y * 256 + x) * 4;
            for c in 0..3 {
                pixels[i + c] = crate::hdr::test_half(bands[x / 32][c]);
            }
            pixels[i + 3] = crate::hdr::test_half(1.0);
        }
    }
    let source = wgc::synthetic_hdr(&gpu, 256, 144, &pixels);
    for software in [false, true] {
        let enc = VideoEncoder::open_tonemapped(
            gpu.clone(),
            256,
            144,
            60,
            Quality::Maximum,
            software,
            203.0,
        )
        .unwrap();
        let (clip, elapsed, ms) = encode(
            enc,
            &source,
            &root,
            if software { "sdr-software" } else { "sdr-gpu" },
            60,
        );
        let info = probe(&clip);
        assert_eq!(info["streams"][0]["codec_name"], "h264");
        assert_eq!(info["streams"][0]["nb_read_frames"], "60");
        assert_eq!(info["streams"][0]["color_transfer"], "bt709");
        let raw = std::process::Command::new(bin("ffmpeg.exe"))
            .args(["-v", "error", "-i"])
            .arg(&clip)
            .args([
                "-frames:v",
                "1",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(raw.status.success());
        let mut max_error = 0.0f64;
        for (n, rgb) in bands.iter().enumerate() {
            let rgb = rgb
                .map(|v| crate::hdr::half_to_float(crate::hdr::test_half(v)) as f64 * 80.0 / 203.0);
            let l = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            let mapped = if l <= 0.75 {
                l
            } else {
                0.75 + 0.25 * (1.0 - (-4.0 * (l - 0.75)).exp())
            };
            let want = rgb.map(|v| {
                let v = (v * mapped / l.max(1e-8)).clamp(0.0, 1.0);
                255.0
                    * if v < 0.0031308 {
                        12.92 * v
                    } else {
                        1.055 * v.powf(1.0 / 2.4) - 0.055
                    }
            });
            let i = (72 * 256 + n * 32 + 16) * 3;
            let got = &raw.stdout[i..i + 3];
            for c in 0..3 {
                max_error = max_error.max((got[c] as f64 - want[c]).abs());
            }
            println!("software={software}, patch={n}, expected={want:?}, actual={got:?}");
        }
        println!("Tone map max error {max_error:0.3}/255; software={software}; encode={elapsed}s remux={ms}ms; {}",clip.display());
        assert!(max_error <= 6.0);
    }
}
