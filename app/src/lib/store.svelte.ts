import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppInfo, AudioApp, AudioDevice, ClipEntry, PathIssue, DisplayTarget, EngineMessage, EngineState, Review, SavedClip, Settings, WindowTarget } from './types';

interface Toast { id: number; text: string; kind: 'ok' | 'error' | 'info' }

export const app = $state({
  settings: null as Settings | null,
  engine: null as EngineState | null,
  engineDown: null as string | null,
  windows: [] as WindowTarget[],
  displays: [] as DisplayTarget[],
  lastClip: null as SavedClip | null,
  clips: [] as ClipEntry[],
  toasts: [] as Toast[],
  dataDir: '',
  /** This copy of CAPTURE: version, portable or installed, its folders. */
  info: null as AppInfo | null,
  /** Saved folders that are not reachable on this PC right now (setting kept). */
  pathIssues: [] as PathIssue[],
  review: null as Review | null,
  reviewLoading: false,
  reviewError: null as string | null,
  clipInfo: {} as Record<string, { duration: number | null }>,
  tab: 'home' as 'home' | 'replay' | 'library' | 'settings',
  inputs: [] as AudioDevice[],
  outputs: [] as AudioDevice[],
  apps: [] as AudioApp[],
  micTest: null as { phase: string; peak: number; message: string | null } | null,
  /** Bumped whenever files in the capture folder change (saves, stems, screenshots). */
  libraryVersion: 0,
  stemsBusy: {} as Record<string, boolean>,
});

let toastId = 0;
export function toast(text: string, kind: Toast['kind'] = 'info', ms = 5000) {
  const id = ++toastId;
  app.toasts.push({ id, text, kind });
  setTimeout(() => { app.toasts = app.toasts.filter(t => t.id !== id); }, ms);
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    toast(String(e), 'error', 8000);
    return undefined;
  }
}

export async function init() {
  const b = await invoke<{ settings: Settings; data_dir: string; last_state: EngineState | null; engine_down: string | null; app: AppInfo; path_issues: PathIssue[]; notices: string[] }>('bootstrap');
  app.settings = b.settings;
  app.dataDir = b.data_dir;
  app.engine = b.last_state;
  app.engineDown = b.engine_down;
  app.info = b.app;
  app.pathIssues = b.path_issues;
  for (const n of b.notices) toast(n, n.startsWith('CAPTURE was updated') ? 'ok' : 'error', 12000);
  await listen<EngineMessage>('engine', e => onEngine(e.payload));
  await listen<Settings>('settings', e => { app.settings = e.payload; checkPaths(); });
  await listen<string>('engine_down', e => { app.engineDown = e.payload; });
  await listen<string>('goto', e => { app.tab = e.payload as typeof app.tab; refreshClips(); });
  refreshTargets();
  refreshClips();
}

function onEngine(m: EngineMessage) {
  switch (m.event) {
    case 'hello':
      app.engineDown = null;
      break;
    case 'state':
      app.engine = m.data;
      app.engineDown = null;
      break;
    case 'targets':
      app.windows = m.data.windows.sort((a, b) => a.app_name.localeCompare(b.app_name));
      app.displays = m.data.displays;
      break;
    case 'save_started':
      toast('Saving…', 'info', 2500);
      break;
    case 'save_finished':
      app.lastClip = m.data;
      app.libraryVersion++;
      toast(
        m.data.duration_secs + 1 < m.data.requested_secs
          ? `Saved ${fmtDuration(m.data.duration_secs)} — only that much was buffered (you asked for ${fmtDuration(m.data.requested_secs)})`
          : `Saved ${fmtDuration(m.data.duration_secs)}`,
        'ok',
      );
      refreshClips();
      break;
    case 'save_failed':
      toast(`Not saved: ${m.data.message}`, 'error', 9000);
      break;
    case 'error':
      toast(m.data.message, 'info', 7000);
      break;
    case 'marker_added':
      toast(`Marker added at ${m.data.at}`, 'ok', 2500);
      break;
    case 'review_ready':
      app.review = m.data;
      app.reviewLoading = false;
      app.reviewError = null;
      break;
    case 'review_failed':
      app.reviewLoading = false;
      app.reviewError = m.data.message;
      break;
    case 'audio_devices':
      app.inputs = m.data.inputs;
      app.outputs = m.data.outputs;
      app.apps = m.data.apps;
      break;
    case 'stems_done':
      app.stemsBusy[m.data.path] = false;
      if (m.data.error) toast(`Editor audio files not made: ${m.data.error}`, 'error', 8000);
      else toast(`Editor audio files saved (${m.data.files.length} WAV) next to the clip`, 'ok');
      app.libraryVersion++;
      break;
    case 'recovered':
      if (m.data.clips.length) toast(`Recovered ${m.data.clips.length} clip${m.data.clips.length > 1 ? 's' : ''} into your Clips folder`, 'ok', 6000);
      if (m.data.failed.length) toast(`Some footage could not be recovered: ${m.data.failed.join('; ')}`, 'error', 10000);
      app.libraryVersion++;
      break;
    case 'screenshot_saved':
      toast('Screenshot saved', 'ok', 2500);
      app.libraryVersion++;
      break;
    case 'mic_test':
      app.micTest = m.data;
      break;
    case 'clip_info':
      app.clipInfo[m.data.path] = { duration: m.data.duration_secs };
      break;
  }
}

export async function checkPaths() {
  app.pathIssues = (await call<PathIssue[]>('check_paths')) ?? [];
}

export const openAppFolder = (which: 'data' | 'logs' | 'app' | 'resources') => call('open_app_folder', { which });

export async function updateSettings(patch: Partial<Settings>) {
  if (!app.settings) return;
  // Only the changed fields: the backend merges them into its current settings, so a change
  // made meanwhile elsewhere (mute hotkey, tray, the other window) is never undone.
  const next = await call<Settings>('update_settings', { patch });
  if (next) { app.settings = next; checkPaths(); }
}

/** Send any engine command; the reply arrives as an engine event with `re` = returned id. */
export function engine(cmd: string, args?: Record<string, unknown>) {
  return call<number>('engine_command', { command: args === undefined ? { cmd } : { cmd, args } });
}
export const addMarker = () => engine('add_marker', {});
export const toggleSession = () => engine(app.engine?.session_secs != null ? 'stop_session' : 'start_session');
export function prepareReview() {
  app.reviewLoading = true;
  app.reviewError = null;
  return engine('prepare_review');
}
export const releaseReview = () => { app.review = null; return engine('release_review'); };
export const saveRange = (start: number, end: number) => engine('save_range', { start, end });
export function requestClipInfo(path: string) {
  if (!(path in app.clipInfo)) {
    app.clipInfo[path] = { duration: null };
    engine('clip_info', { path });
  }
}
export async function deleteClip(path: string) {
  const ok = await call('delete_clip', { path });
  if (ok !== undefined) { toast('Moved to the Recycle Bin', 'ok', 3000); refreshClips(); }
}
export const showMain = (tab?: string) => call('show_main', { tab });
export const hideOverlay = () => call('hide_overlay');

// The backend's `settings` event carries the saved value; on failure `call` shows the error and
// the toggle keeps showing what is really saved.
export const setReplay = (enabled: boolean) => call('set_replay', { enabled });
export const saveReplay = () => call('save_replay');
export const refreshTargets = () => call('list_targets');
export const clearTempCache = () => call('clear_temp_cache');
export const openPath = (path: string) => call('open_path', { path });
export const revealPath = (path: string) => call('reveal_path', { path });
export const openCaptureFolder = () => call('open_capture_folder');
export async function copyPath(path: string) {
  await call('copy_text', { text: path });
  toast('Path copied', 'ok', 2000);
}
export async function refreshClips() {
  app.clips = (await call<ClipEntry[]>('list_clips')) ?? [];
}

export function fmtDuration(s: number): string {
  const t = Math.round(s);
  if (t < 60) return `${t}s`;
  const m = Math.floor(t / 60), r = t % 60;
  return r ? `${m}m ${String(r).padStart(2, '0')}s` : `${m} min`;
}

export function fmtBytes(b: number): string {
  if (b >= 1e9) return `${(b / 1e9).toFixed(1)} GB`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(b / 1e3))} KB`;
}
