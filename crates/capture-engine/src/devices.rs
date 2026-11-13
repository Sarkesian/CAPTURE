//! Audio devices, apps that are playing sound, and process lookup by executable name.

use capture_protocol::{AudioApp, AudioDevice};
use windows::core::Interface;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Diagnostics::ToolHelp::*;

use crate::targets;

pub fn name_of(dev: &IMMDevice) -> String {
    unsafe {
        dev.OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "Unknown device".into())
    }
}

/// Windows' own mute switch and level (0..1) for an endpoint, as shown in Sound settings.
/// CAPTURE only reads these; it never changes them.
pub fn endpoint_level(dev: &IMMDevice) -> Option<(bool, f32)> {
    unsafe {
        let v: windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).ok()?;
        Some((v.GetMute().ok()?.as_bool(), v.GetMasterVolumeLevelScalar().ok()?))
    }
}

/// Plain-language reason a microphone records silence because of Windows settings, if any.
pub fn mic_level_problem(name: &str, muted: bool, volume: f32) -> Option<String> {
    if muted {
        Some(format!("Windows has {name} muted, so it records silence. Unmute it in Windows Sound settings › Input (CAPTURE never changes it)."))
    } else if volume <= 0.001 {
        Some(format!("{name} is set to 0% in Windows Sound settings › Input, so it records silence."))
    } else {
        None
    }
}

fn id_of(dev: &IMMDevice) -> String {
    unsafe { dev.GetId().ok().and_then(|p| p.to_string().ok()).unwrap_or_default() }
}

/// Virtual/loopback devices are labelled so they are never mistaken for a real microphone.
pub fn is_virtual(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    ["steam streaming", "virtual", "vb-audio", "voicemeeter", "cable output", "cable input", "nvidia broadcast", "obs-camera"]
        .iter()
        .any(|k| l.contains(k))
}

/// Active endpoints for `flow` (eCapture = microphones, eRender = outputs).
pub fn list(flow: EDataFlow) -> Vec<AudioDevice> {
    unsafe {
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return vec![] };
        let default = en.GetDefaultAudioEndpoint(flow, eConsole).map(|d| id_of(&d)).unwrap_or_default();
        let Ok(coll) = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else { return vec![] };
        let mut out = Vec::new();
        for i in 0..coll.GetCount().unwrap_or(0) {
            if let Ok(d) = coll.Item(i) {
                let (id, name) = (id_of(&d), name_of(&d));
                let (muted, volume) = endpoint_level(&d).map_or((false, None), |(m, v)| (m, Some(v)));
                out.push(AudioDevice { is_default: id == default, is_virtual: is_virtual(&name), id, name, muted, volume });
            }
        }
        out.sort_by_key(|d| (!d.is_default, d.is_virtual, d.name.clone()));
        out
    }
}

/// Endpoint id of the current Windows default device for `flow`.
pub fn default_id(flow: EDataFlow) -> Option<String> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        en.GetDefaultAudioEndpoint(flow, eConsole).ok().map(|d| id_of(&d))
    }
}

/// (pid, parent pid, exe name) for every process.
fn processes() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                out.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..len])));
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// The top process of an app (e.g. the main Discord.exe whose children render the audio), so
/// process-tree loopback covers every helper process. `None` if the app isn't running.
pub fn root_process(exe: &str) -> Option<u32> {
    pick_root(&processes(), exe)
}

pub fn pick_root(procs: &[(u32, u32, String)], exe: &str) -> Option<u32> {
    let mine: Vec<&(u32, u32, String)> = procs.iter().filter(|p| p.2.eq_ignore_ascii_case(exe)).collect();
    mine.iter()
        .find(|p| !mine.iter().any(|q| q.0 == p.1))
        .or(mine.first())
        .map(|p| p.0)
}

/// Apps that currently have an audio session on any output device (for the voice-chat
/// picker). CAPTURE itself and the Windows system-sounds session are excluded.
pub fn audio_apps() -> Vec<AudioApp> {
    let me = std::process::id();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    unsafe {
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return out };
        let Ok(coll) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        for i in 0..coll.GetCount().unwrap_or(0) {
            let Ok(dev) = coll.Item(i) else { continue };
            let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(sessions) = mgr.GetSessionEnumerator() else { continue };
            for j in 0..sessions.GetCount().unwrap_or(0) {
                let Some(ctl2) = sessions.GetSession(j).ok().and_then(|c| c.cast::<IAudioSessionControl2>().ok()) else { continue };
                let pid = ctl2.GetProcessId().unwrap_or(0);
                if pid == 0 || pid == me {
                    continue;
                }
                let path = targets::process_path(pid).unwrap_or_default();
                let exe = path.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                if exe.is_empty() || exe.eq_ignore_ascii_case("capture-worker.exe") || !seen.insert(exe.to_ascii_lowercase()) {
                    continue;
                }
                let name = targets::app_name(&path, &exe, "");
                out.push(AudioApp { exe, name });
            }
        }
    }
    out.sort_by_key(|a| a.name.to_ascii_lowercase());
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn root_is_the_process_whose_parent_is_not_the_same_app() {
        let p = vec![
            (10, 1, "explorer.exe".to_string()),
            (20, 10, "Discord.exe".to_string()),
            (21, 20, "Discord.exe".to_string()),
            (22, 20, "Discord.exe".to_string()),
        ];
        assert_eq!(super::pick_root(&p, "discord.exe"), Some(20));
        assert_eq!(super::pick_root(&p, "Teams.exe"), None);
    }

    #[test]
    fn virtual_devices_are_flagged() {
        assert!(super::is_virtual("Microphone (Steam Streaming Microphone)"));
        assert!(!super::is_virtual("Microphone (G06-BT)"));
    }
}
