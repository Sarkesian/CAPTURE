// Mirrors crates/capture-protocol (v1) and app/src-tauri/src/settings.rs.

export type Quality = 'balanced' | 'high' | 'maximum' | 'archival';
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
  history_secs: number;
  background_history: boolean;
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

/** This copy of CAPTURE (app/src-tauri/src/main.rs `AppInfo`). */
export interface AppInfo { version: string; installed: boolean; app_dir: string; data_dir: string; logs_dir: string; data_fallback: boolean }
export interface PathIssue { which: 'capture_root' | 'buffer_root'; path: string }
export interface StagedUpdate { version: string; current: string; order: -1 | 0 | 1 }

export interface GameRule { exe: string; replay_secs: number | null; quality: Quality | null; game_audio: boolean | null }

export interface AudioDevice { id: string; name: string; is_default: boolean; is_virtual: boolean; muted?: boolean; volume?: number | null }
export interface AudioApp { exe: string; name: string }
export interface MicLevel { device: string; peak: number; error: string | null }

export interface MarkerConfig { sound: boolean; sound_volume: number; sound_device: string | null; protect: boolean; protect_before_secs: number; protect_after_secs: number }
export type HotkeyAction = 'save_replay' | 'add_marker' | 'toggle_overlay' | 'toggle_session' | 'screenshot' | 'toggle_mic_mute';
export interface SavedMark { kind: 'clip' | 'screenshot'; t: number; end: number | null; name: string }
export interface Marker { id: number; t: number; at: string; label: string; footage: 'buffered' | 'protected' | 'expired'; note: string | null; saved_in: string[] }

export interface EncoderInfo { name: string; hardware: boolean; width: number; height: number; fps: number; rejected: string[] }
export interface TrackState { name: string; mode: string; source: string; peak: number; signal_seen: boolean; error: string | null }
export interface HotkeyState { action: HotkeyAction | null; combo: string; registered: boolean; message: string | null }

export interface EngineState {
  history_buffered_secs: number;
  frames_encoded: number;
  frames_without_source: number;
  frames_dropped: number;
  frames_repeated: number;
  source_frames_unselected: number;
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
  /** Clips and screenshots saved from this background recording (optional: older engines). */
  saved?: SavedMark[];
  /** The game whose background recording is still kept while nothing is being recorded. */
  past_game?: string | null;
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
  /** This footage was already saved: `path` is that earlier file; nothing new was written. */
  existing?: boolean;
}

export interface Review { session: string; path: string; start: number; end: number; game: string; past?: boolean; markers?: Marker[] }
export interface FrameTiming { start: number; end: number; key: boolean }
export interface TimelineEvent { id: string; kind: string; t: number; end: number | null; label: string; path: string | null; marker: Marker | null }
/** One internal recording of a session (a new one starts when the game relaunches or changes resolution). */
export interface HistoryPart { id: string; offset: number; end: number; width: number; height: number }
/** A gameplay session as the player sees it. All times are seconds since `created_ms`; gaps between `intervals` are moments that were not recorded. */
export interface HistorySession { id: string; game: string; created_ms: number; ended: boolean; intervals: [number, number][]; events: TimelineEvent[]; parts: HistoryPart[]; bytes: number }
export interface HistoryState { failed_saves: {id:string;session:string;start:number;end:number;error:string}[];sessions: HistorySession[]; bytes: number; preview_bytes: number; retained_secs: number; target_secs: number; reason: string | null }

export interface ClipEntry { path: string; name: string; game: string; bytes: number; modified_ms: number; kind: 'clip' | 'session' | 'screenshot'; has_stems: boolean; has_markers: boolean }

export type EngineMessage = { v: number; re?: number } & (
  | { event: 'hello'; data: { worker_version: string } }
  | { event: 'state'; data: EngineState }
  | { event: 'targets'; data: { windows: WindowTarget[]; displays: DisplayTarget[] } }
  | { event: 'capture_triggered'; data: { action: 'screenshot' | 'replay' | 'recording_start' | 'recording_stop'; overlay_notice: boolean } }
  | { event: 'save_started'; data: { save_id: number; requested_secs: number; note?: string | null } }
  | { event: 'save_finished'; data: SavedClip }
  | { event: 'save_failed'; data: { save_id: number; message: string } }
  | { event: 'error'; data: { message: string } }
  | { event: 'hotkey_pressed'; data: { action: HotkeyAction } }
  | { event: 'marker_added'; data: Marker }
  | { event: 'review_ready'; data: Review }
  | { event: 'review_failed'; data: { message: string; superseded?: boolean } }
  | { event: 'history_thumb'; data: { session: string; at: number; t: number | null; image: string | null; error: string | null } }
  | { event: 'history_ready'; data: HistoryState }
  | { event: 'history_frames'; data: { frames: FrameTiming[]; error: string | null } }
  | { event: 'clip_info'; data: { path: string; duration_secs: number | null; error: string | null } }
  | { event: 'audio_devices'; data: { inputs: AudioDevice[]; outputs: AudioDevice[]; apps: AudioApp[] } }
  | { event: 'stems_done'; data: { path: string; folder: string | null; files: string[]; error: string | null } }
  | { event: 'screenshot_saved'; data: { path: string } }
  | { event: 'recovered'; data: { clips: string[]; failed: string[] } }
  | { event: 'mic_test'; data: { phase: 'recording' | 'playing' | 'done' | 'failed'; peak: number; message: string | null } }
);
