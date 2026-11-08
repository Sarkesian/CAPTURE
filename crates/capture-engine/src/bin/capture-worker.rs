//! `capture-worker`: the isolated capture/encoding process.
//!
//! stdin: one `capture_protocol::Request` JSON per line. stdout: one `Message` JSON per line.
//! stdin EOF (UI exited or crashed) finalizes the session cleanly and exits.
//! stderr: human-readable diagnostics only.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use capture_engine::{hotkey::HotkeyService, Engine};
use capture_protocol::*;

enum Input {
    Req(Option<u64>, Command),
    Tick,
}

fn main() {
    unsafe {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        windows::Win32::Media::timeBeginPeriod(1);
        ffmpeg_sys_next::av_log_set_level(ffmpeg_sys_next::AV_LOG_ERROR);
    }

    let out = Arc::new(Mutex::new(std::io::stdout()));
    let emit = {
        let out = out.clone();
        Arc::new(move |m: Message| {
            let line = serde_json::to_string(&m).expect("serializable");
            let mut o = out.lock().unwrap();
            let _ = writeln!(o, "{line}");
            let _ = o.flush();
        })
    };
    emit(Message::new(None, Event::Hello { worker_version: env!("CARGO_PKG_VERSION").into() }));

    let (tx, rx) = mpsc::channel::<Input>();
    let mut engine = Engine::new(emit.clone());

    let hk_tx = tx.clone();
    let hk_emit = emit.clone();
    engine.attach_hotkey(HotkeyService::start(
        "",
        move || {
            let _ = hk_tx.send(Input::Req(None, Command::SaveReplay));
        },
        move |_st| {
            let _ = &hk_emit; // state is published with the next State event
        },
    ));

    {
        let tx = tx.clone();
        let emit = emit.clone();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Request>(&line) {
                    Ok(r) if r.v == PROTOCOL_VERSION => {
                        let _ = tx.send(Input::Req(Some(r.id), r.cmd));
                    }
                    Ok(r) => emit(Message::new(Some(r.id), Event::Error { message: format!("Protocol version {} not supported (worker speaks {PROTOCOL_VERSION})", r.v) })),
                    Err(e) => eprintln!("capture-worker: bad request: {e}"),
                }
            }
            let _ = tx.send(Input::Req(None, Command::Shutdown));
        });
    }
    {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            if tx.send(Input::Tick).is_err() {
                break;
            }
        });
    }

    for input in rx {
        match input {
            Input::Req(re, cmd) => {
                let shutdown = matches!(cmd, Command::Shutdown);
                let keep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.handle(re, cmd))).unwrap_or_else(|_| {
                    emit(Message::new(re, Event::Error { message: "Internal error in the capture engine; see logs.".into() }));
                    true
                });
                if shutdown || !keep {
                    break;
                }
            }
            Input::Tick => {
                engine.tick();
                engine.publish_state(None, false);
            }
        }
    }
    engine.shutdown();
}
