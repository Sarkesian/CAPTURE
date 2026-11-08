fn main() {
    // Dev builds find the pinned FFmpeg DLLs in third_party without copying them around.
    // Release/portable builds ship the DLLs next to capture-worker.exe instead.
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let bin = manifest.join("..").join("..").join("third_party").join("ffmpeg").join("bin");
        println!("cargo:rustc-env=CAPTURE_DEV_FFMPEG_BIN={}", bin.display());
    } else {
        println!("cargo:rustc-env=CAPTURE_DEV_FFMPEG_BIN=");
    }
    tauri_build::build()
}
