import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ClipEntry, DisplayTarget, EngineMessage, EngineState, SavedClip, Settings, WindowTarget } from './types';

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
  const b = await invoke<{ settings: Settings; data_dir: string; last_state: EngineState | null; engine_down: string | null }>('bootstrap');
  app.settings = b.settings;
  app.dataDir = b.data_dir;
  app.engine = b.last_state;
  app.engineDown = b.engine_down;
  await listen<EngineMessage>('engine', e => onEngine(e.payload));
  await listen<Settings>('settings', e => { app.settings = e.payload; });
  await listen<string>('engine_down', e => { app.engineDown = e.payload; });
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
  }
}

export async function updateSettings(patch: Partial<Settings>) {
  if (!app.settings) return;
  const next = await call<Settings>('update_settings', { settings: { ...app.settings, ...patch } });
  if (next) app.settings = next;
}

export const setReplay = (enabled: boolean) => call('set_replay', { enabled }).then(() => { if (app.settings) app.settings.replay_enabled = enabled; });
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
