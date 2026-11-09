// Mirrors crates/capture-protocol (v1) and app/src-tauri/src/settings.rs.

export type Quality = 'balanced' | 'high' | 'maximum';
export type Status = 'ready' | 'starting' | 'waiting' | 'buffering' | 'needs_attention';

export type TargetPref =
  | { kind: 'none' }
  | { kind: 'auto' }
  | { kind: 'app'; exe: string; label: string }
  | { kind: 'display'; index: number; label: string };

export interface Settings {
  version: number;
  capture_root: string;
  replay_enabled: boolean;
  replay_secs: number;
  target: TargetPref;
  quality: Quality;
  fps: number;
  capture_cursor: boolean;
  game_audio: boolean;
  save_hotkey: string;
  buffer_cap_gb: number;
  min_free_gb: number;
  close_to_tray: boolean;
  notify_on_save: boolean;
  games: string[];
  ignored: string[];
  hotkeys: { marker: string; overlay: string; session: string; screenshot: string; mic_mute: string };
  markers: MarkerConfig;
  mic_enabled: boolean;
  mic_device: string | null;
  mic_muted: boolean;
  voice_enabled: boolean;
  voice_app: string | null;
  other_enabled: boolean;
  volumes: { game: number; mic: number; voice: number };
  per_game: GameRule[];
  first_run_done: boolean;
  start_with_windows: boolean;
  encoder: 'auto' | 'software';
  notices: { recording_started: boolean; saved: boolean; marker: boolean; problems: boolean };
  overlay_scale: number;
  buffer_root: string | null;
  editor_audio_files: boolean;
  minimize_to_tray: boolean;
}

export interface GameRule { exe: string; replay_secs: number | null; quality: Quality | null; game_audio: boolean | null }

export interface AudioDevice { id: string; name: string; is_default: boolean; is_virtual: boolean; muted?: boolean; volume?: number | null }
export interface AudioApp { exe: string; name: string }
export interface MicLevel { device: string; peak: number; error: string | null }

export interface MarkerConfig { sound: boolean; sound_volume: number; sound_device: string | null; protect: boolean; protect_before_secs: number; protect_after_secs: number }
export type HotkeyAction = 'save_replay' | 'add_marker' | 'toggle_overlay' | 'toggle_session' | 'screenshot' | 'toggle_mic_mute';
export interface Marker { id: number; t: number; at: string; label: string; footage: 'buffered' | 'protected' | 'expired'; note: string | null; saved_in: string[] }

export interface EncoderInfo { name: string; hardware: boolean; width: number; height: number; fps: number; rejected: string[] }
export interface TrackState { name: string; mode: string; source: string; peak: number; signal_seen: boolean; error: string | null }
export interface HotkeyState { action: HotkeyAction | null; combo: string; registered: boolean; message: string | null }

export interface EngineState {
  status: Status;
  attention: string | null;
  target_name: string | null;
  target_exe: string | null;
  detected_by: string | null;
  encoder: EncoderInfo | null;
  buffered_secs: number;
  buffer_bytes: number;
  capture_fps: number;
  frames_captured: number;
  tracks: TrackState[];
  saves_in_progress: number;
  hotkey: HotkeyState;
  hotkeys: HotkeyState[];
  markers: Marker[];
  now_secs: number;
  session_secs: number | null;
  mic_monitor: MicLevel | null;
  leftover_buffer_bytes: number;
  free_bytes: number | null;
}

export interface WindowTarget { hwnd: number; pid: number; exe: string; title: string; app_name: string; width: number; height: number }
export interface DisplayTarget { index: number; name: string; width: number; height: number; primary: boolean }

export interface SavedClip {
  save_id: number;
  path: string;
  requested_secs: number;
  duration_secs: number;
  bytes: number;
  video_streams: number;
  audio_tracks: string[];
}

export interface Review { path: string; start: number; end: number; game: string }

export interface ClipEntry { path: string; name: string; game: string; bytes: number; modified_ms: number; kind: 'clip' | 'session' | 'screenshot'; has_stems: boolean; has_markers: boolean }

export type EngineMessage = { v: number; re?: number } & (
  | { event: 'hello'; data: { worker_version: string } }
  | { event: 'state'; data: EngineState }
  | { event: 'targets'; data: { windows: WindowTarget[]; displays: DisplayTarget[] } }
  | { event: 'save_started'; data: { save_id: number; requested_secs: number } }
  | { event: 'save_finished'; data: SavedClip }
  | { event: 'save_failed'; data: { save_id: number; message: string } }
  | { event: 'error'; data: { message: string } }
  | { event: 'hotkey_pressed'; data: { action: HotkeyAction } }
  | { event: 'marker_added'; data: Marker }
  | { event: 'review_ready'; data: Review }
  | { event: 'review_failed'; data: { message: string } }
  | { event: 'clip_info'; data: { path: string; thumb: string | null; duration_secs: number | null; error: string | null } }
  | { event: 'audio_devices'; data: { inputs: AudioDevice[]; outputs: AudioDevice[]; apps: AudioApp[] } }
  | { event: 'stems_done'; data: { path: string; folder: string | null; files: string[]; error: string | null } }
  | { event: 'screenshot_saved'; data: { path: string } }
  | { event: 'recovered'; data: { clips: string[]; failed: string[] } }
  | { event: 'mic_test'; data: { phase: 'recording' | 'playing' | 'done' | 'failed'; peak: number; message: string | null } }
);
