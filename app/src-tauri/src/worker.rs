//! Spawns and supervises `capture-worker.exe`. Messages from the worker are forwarded to the
//! webview as `engine` events and to `on_message` for tray/notification handling.

use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command as Proc, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture_protocol::{Command, Message, Request};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct Worker {
    /// `<app>\Runtime\capture-worker.exe`; its FFmpeg DLLs sit beside it.
    exe: PathBuf,
    runtime: PathBuf,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    next_id: AtomicU64,
    pub quitting: AtomicBool,
    restarts: Mutex<Vec<Instant>>,
}

pub type OnMessage = Arc<dyn Fn(Message) + Send + Sync>;
pub type OnRestart = Arc<dyn Fn() + Send + Sync>;
pub type OnDown = Arc<dyn Fn(String) + Send + Sync>;

impl Worker {
    pub fn new(exe: PathBuf, runtime: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            exe,
            runtime,
            stdin: Mutex::new(None),
            child: Mutex::new(None),
            next_id: AtomicU64::new(1),
            quitting: AtomicBool::new(false),
            restarts: Mutex::new(Vec::new()),
        })
    }

    pub fn start(self: &Arc<Self>, on_msg: OnMessage, on_restart: OnRestart, on_down: OnDown) -> Result<(), String> {
        let exe = &self.exe;
        if !exe.exists() {
            return Err(format!(
                "The capture engine is missing ({}). Extract or reinstall the complete CAPTURE package; your settings are kept.",
                exe.display()
            ));
        }
        let mut path = std::env::var("PATH").unwrap_or_default();
        let dev = env!("CAPTURE_DEV_FFMPEG_BIN");
        if !dev.is_empty() {
            path = format!("{dev};{path}");
        }
        let mut child = Proc::new(exe)
            .current_dir(&self.runtime)
            .env("PATH", path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Could not start the capture engine: {e}"))?;
        let stdout = child.stdout.take().unwrap();
        *self.stdin.lock().unwrap() = child.stdin.take();
        *self.child.lock().unwrap() = Some(child);

        let me = self.clone();
        std::thread::Builder::new()
            .name("worker-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    match serde_json::from_str::<Message>(&line) {
                        Ok(m) if m.v == capture_protocol::PROTOCOL_VERSION => on_msg(m),
                        Ok(m) => eprintln!("worker speaks protocol v{}, UI expects v{}", m.v, capture_protocol::PROTOCOL_VERSION),
                        Err(e) => eprintln!("bad worker line: {e}: {line}"),
                    }
                }
                if let Some(mut c) = me.child.lock().unwrap().take() {
                    let _ = c.wait();
                }
                *me.stdin.lock().unwrap() = None;
                if me.quitting.load(Ordering::SeqCst) {
                    return;
                }
                // Unexpected exit: restart with a cap so a crash loop cannot spin forever.
                let allowed = {
                    let mut r = me.restarts.lock().unwrap();
                    r.retain(|t| t.elapsed() < Duration::from_secs(60));
                    r.push(Instant::now());
                    r.len() <= 3
                };
                if !allowed {
                    on_down("The capture engine stopped repeatedly. Restart CAPTURE; if it keeps happening, check the log.".into());
                    return;
                }
                on_down("The capture engine stopped unexpectedly and is restarting…".into());
                std::thread::sleep(Duration::from_millis(800));
                match me.start(on_msg.clone(), on_restart.clone(), on_down.clone()) {
                    Ok(()) => on_restart(),
                    Err(e) => on_down(e),
                }
            })
            .unwrap();
        Ok(())
    }

    pub fn send(&self, cmd: Command) -> Result<u64, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let line = serde_json::to_string(&Request::new(id, cmd)).unwrap();
        let mut g = self.stdin.lock().unwrap();
        let w = g.as_mut().ok_or("The capture engine is not running.")?;
        writeln!(w, "{line}").and_then(|_| w.flush()).map_err(|e| format!("Capture engine unreachable: {e}"))?;
        Ok(id)
    }

    /// Ask the worker to finish in-flight saves and exit; wait up to `timeout`.
    pub fn shutdown(&self, timeout: Duration) {
        self.quitting.store(true, Ordering::SeqCst);
        let _ = self.send(Command::Shutdown);
        *self.stdin.lock().unwrap() = None; // EOF also means shutdown
        let until = Instant::now() + timeout;
        loop {
            let mut g = self.child.lock().unwrap();
            match g.as_mut().map(|c| c.try_wait()) {
                None | Some(Ok(Some(_))) | Some(Err(_)) => return,
                Some(Ok(None)) if Instant::now() >= until => {
                    if let Some(c) = g.as_mut() {
                        let _ = c.kill();
                    }
                    return;
                }
                _ => {}
            }
            drop(g);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
