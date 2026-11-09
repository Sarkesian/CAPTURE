// Live level meters.
//
// Data: the worker sends real peak/RMS levels (~30/s) as `levels` events, only while some
// visible view has subscribed. Rendering: one requestAnimationFrame loop for all meters that
// writes CSS custom properties consumed by `transform: scaleX()` (compositor-only, no layout).
// Ballistics (all on the real levels, nothing synthesised): one-pole smoothing in the dB domain
// with a fast attack and a slower release, so bars glide between the ~30/s updates instead of
// stepping. The RMS bar (loudness) is smoothed more than the peak bar. The peak-hold tick and the
// clip mark use the raw, unsmoothed peak so short overloads are never hidden.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// Time constants (seconds). ~63% of a change after one constant, ~95% after three.
const PEAK_ATTACK = 0.03;
const PEAK_RELEASE = 0.09; // ~95% of a drop within ~270 ms
const RMS_ATTACK = 0.06;
const RMS_RELEASE = 0.12;
const HOLD_FALL_S = 0.6; // peak-hold tick: full-scale fall time once the hold expires
const HOLD_S = 0.9; // peak-hold tick before it falls
const FLOOR_DB = -60;
const STALE_MS = 150; // no fresh level for this long = silence

interface Lvl { peak: number; rms: number; at: number }
interface Meter { el: HTMLElement; key: string; peak: number; rms: number; hold: number; holdAt: number; clipAt: number }

const latest = new Map<string, Lvl>();
const meters = new Set<Meter>();
const client = location.hash === '#overlay' ? 'overlay' : 'main';
let subscribed = false;
let raf = 0;
let last = 0;
let listening = false;
// Diagnostics for the acceptance scripts: level events received and frames drawn.
const stats = { events: 0, frames: 0 };
(window as unknown as { __captureMeters: typeof stats }).__captureMeters = stats;

function ensureListener() {
  if (listening) return;
  listening = true;
  listen<{ event: string; data: { tracks: { name: string; peak: number; rms: number }[]; mic_monitor: { peak: number; rms: number } | null } }>('levels', e => {
    const now = performance.now();
    stats.events++;
    const d = e.payload.data;
    for (const t of d.tracks) latest.set(t.name, { peak: t.peak, rms: t.rms, at: now });
    if (d.mic_monitor) latest.set('mic_monitor', { ...d.mic_monitor, at: now });
    kick();
  });
  document.addEventListener('visibilitychange', sync);
  // A restarted engine has forgotten who wanted levels: subscribe again.
  listen<{ event: string }>('engine', e => {
    if (e.payload.event === 'hello') { subscribed = false; sync(); }
  });
}

function sync() {
  const want = meters.size > 0 && !document.hidden;
  if (want !== subscribed) {
    subscribed = want;
    invoke('engine_command', { command: { cmd: 'meters', args: { client, enabled: want } } }).catch(() => {});
  }
  if (want) kick();
}

function kick() {
  if (raf || meters.size === 0 || document.hidden) return;
  last = performance.now();
  raf = requestAnimationFrame(frame);
}

const MIN_FRAME_MS = 1000 / 60 - 2; // draw at most ~60 times a second, whatever the display rate

const toPos = (x: number) => (x > 0 ? Math.min(1, Math.max(0, (20 * Math.log10(x) - FLOOR_DB) / -FLOOR_DB)) : 0);

function frame(now: number) {
  if (now - last < MIN_FRAME_MS) { raf = requestAnimationFrame(frame); return; }
  const dt = Math.min(0.1, (now - last) / 1000);
  last = now;
  stats.frames++;
  const k = (tau: number) => 1 - Math.exp(-dt / tau);
  const kpa = k(PEAK_ATTACK), kpr = k(PEAK_RELEASE), kra = k(RMS_ATTACK), krr = k(RMS_RELEASE);
  const fall = dt / HOLD_FALL_S;
  const settle = (x: number) => (x < 0.002 ? 0 : x);
  let moving = false;
  for (const m of meters) {
    const l = latest.get(m.key);
    const fresh = l && now - l.at < STALE_MS;
    const tp = fresh ? toPos(l!.peak) : 0;
    const tr = fresh ? toPos(l!.rms) : 0;
    m.peak = settle(m.peak + (tp - m.peak) * (tp > m.peak ? kpa : kpr));
    m.rms = settle(m.rms + (tr - m.rms) * (tr > m.rms ? kra : krr));
    if (tp >= m.hold) { m.hold = tp; m.holdAt = now; }
    else if (now - m.holdAt > HOLD_S * 1000) m.hold = settle(Math.max(m.peak, m.hold - fall));
    if (fresh && l!.peak >= 0.99) m.clipAt = now;
    const s = m.el.style;
    s.setProperty('--rms', m.rms.toFixed(3));
    s.setProperty('--peak', m.peak.toFixed(3));
    s.setProperty('--hold', m.hold.toFixed(3));
    m.el.dataset.hot = m.hold > 0.95 ? '1' : ''; // within ~3 dB of full scale
    const clip = now - m.clipAt < 1500;
    m.el.dataset.clip = clip ? '1' : '';
    if (fresh || m.peak > 0 || m.rms > 0 || m.hold > 0 || clip) moving = true;
  }
  // Idle (no signal, everything at rest): stop drawing until the next level event arrives.
  raf = moving && meters.size > 0 && !document.hidden ? requestAnimationFrame(frame) : 0;
}

/** Svelte action: `use:meter={'Game Audio'}` registers a live meter element. */
export function meter(el: HTMLElement, key: string) {
  ensureListener();
  const m: Meter = { el, key, peak: 0, rms: 0, hold: 0, holdAt: 0, clipAt: -1e9 };
  meters.add(m);
  sync();
  return {
    update(k: string) { m.key = k; },
    destroy() { meters.delete(m); sync(); },
  };
}
