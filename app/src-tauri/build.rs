fn main() {
    // Dev builds find the pinned FFmpeg DLLs in third_party without copying them around.
    // Release/portable builds ship the DLLs in Runtime\ next to capture-worker.exe instead.
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let bin = manifest.join("..").join("..").join("third_party").join("ffmpeg").join("bin");
        println!("cargo:rustc-env=CAPTURE_DEV_FFMPEG_BIN={}", bin.display());
    } else {
        println!("cargo:rustc-env=CAPTURE_DEV_FFMPEG_BIN=");
    }
    // The product version lives in tauri.conf.json (one place for the app, the package
    // manifest and the installer).
    let conf = std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json");
    let v: serde_json::Value = serde_json::from_str(&conf).expect("tauri.conf.json is JSON");
    println!("cargo:rustc-env=CAPTURE_VERSION={}", v["version"].as_str().expect("version"));
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=../dist");
    tauri_build::build()
}
