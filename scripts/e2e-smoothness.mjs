// Recording smoothness against a real, running game: does every frame the game shows reach the
// recording, at an even cadence? Runs the worker with CAPTURE_TRACE, buffers the game window,
// saves a clip, then reports from the trace and the saved file:
//   - capture delivery: frames Windows handed us per second, and gaps between them
//   - encoder: on-time ticks, dropped ticks, lock waits, NVENC send/drain times
//   - clip: distinct pictures per second and runs of repeated frames (freezes)
// The game's own present rate is the reference (it must be running and moving).
//
//   node scripts/e2e-smoothness.mjs <worker.exe> <captureRoot> <exe substring> [secs=30] [quality=high]
import { spawn, execFileSync } from 'node:child_process';
import { readFileSync, mkdirSync } from 'node:fs';
import readline from 'node:readline';
import path from 'node:path';

const [worker, root, exeSub, secsArg = '30', quality = 'high'] = process.argv.slice(2);
const secs = Number(secsArg);
const ffprobe = path.join(import.meta.dirname, '..', 'third_party', 'ffmpeg', 'bin', 'ffprobe.exe');
const sleep = ms => new Promise(r => setTimeout(r, ms));
mkdirSync(root, { recursive: true });
const tracePath = path.join(root, `trace-${Date.now()}.csv`);

const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'], env: { ...process.env, CAPTURE_TRACE: tracePath } });
const rl = readline.createInterface({ input: proc.stdout });
let id = 0;
const events = [], waiters = [];
rl.on('line', line => {
  const m = JSON.parse(line);
  events.push(m);
  for (const w of [...waiters]) if (w.pred(m)) { waiters.splice(waiters.indexOf(w), 1); w.resolve(m); }
});
const send = (cmd, args) => { const r = { v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }; proc.stdin.write(JSON.stringify(r) + '\n'); return r.id; };
const waitFor = (pred, ms = 30000) => new Promise((resolve, reject) => {
  const hit = events.find(pred); if (hit) return resolve(hit);
  waiters.push({ pred, resolve });
  setTimeout(() => reject(new Error('timeout waiting for event')), ms);
});

await waitFor(m => m.event === 'hello');
const tid = send('list_targets');
const targets = (await waitFor(m => m.re === tid && m.event === 'targets')).data;
const w = targets.windows.find(w => w.exe.toLowerCase().includes(exeSub.toLowerCase()));
if (!w) throw new Error('window not found; have: ' + targets.windows.map(w => w.exe).join(', '));
console.log('window:', w.app_name, `${w.width}x${w.height}`);
send('configure', {
  capture_root: root, replay_secs: secs, target: { kind: 'window', hwnd: w.hwnd, pid: w.pid, exe: w.exe, title: w.title },
  quality, fps: 60, capture_cursor: true, audio: { game_audio: true }, save_hotkey: 'Alt+F10', buffer_cap_bytes: 8e9, min_free_bytes: 5e9,
});
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering', 15000);
console.log(`buffering ${secs} s...`);
await sleep((secs + 3) * 1000);
const sr = send('save_replay');
const started = await waitFor(m => m.re === sr && (m.event === 'save_started' || m.event === 'save_failed'));
if (started.event === 'save_failed') throw new Error(started.data.message);
const done = await waitFor(m => (m.event === 'save_finished' || m.event === 'save_failed') && m.data.save_id === started.data.save_id, 120000);
if (done.event === 'save_failed') throw new Error(done.data.message);
const clip = done.data.path;
send('set_replay', { enabled: false });
await sleep(1500);
send('shutdown');
await new Promise(r => proc.on('exit', r));

// ---- analysis ----
const pct = (a, p) => { const s = [...a].sort((x, y) => x - y); return s.length ? s[Math.min(s.length - 1, Math.floor(p * s.length))] : 0; };
const ms = v => (v / 1e4).toFixed(1);
const rows = readFileSync(tracePath, 'utf8').trim().split('\n').slice(1).map(l => l.split(',')).map(([k, t, a, b, c]) => ({ k, t: +t, a: +a, b: +b, c: +c }));
const cap = rows.filter(r => r.k === 'cap'), enc = rows.filter(r => r.k === 'enc'), encd = rows.filter(r => r.k === 'encd'), drop = rows.filter(r => r.k === 'drop');
const tEnd = rows.at(-1).t, tStart = tEnd - secs * 1e7;
const inWin = r => r.t >= tStart;
const capW = cap.filter(inWin);
const gaps = capW.slice(1).map((r, i) => r.t - capW[i].t);
const shownGaps = capW.slice(1).map((r, i) => r.a - capW[i].a);
console.log('\n== capture delivery (last', secs, 's) ==');
console.log(`frames/s ${(capW.length / secs).toFixed(1)}  arrival gap p50 ${ms(pct(gaps, .5))} p99 ${ms(pct(gaps, .99))} max ${ms(Math.max(...gaps))} ms; >50 ms gaps: ${gaps.filter(g => g > 5e5).length}`);
console.log(`DWM time gap p50 ${ms(pct(shownGaps, .5))} p99 ${ms(pct(shownGaps, .99))} max ${ms(Math.max(...shownGaps))} ms; callback lock wait p99 ${ms(pct(capW.map(r => r.b), .99))} max ${ms(Math.max(...capW.map(r => r.b)))} ms`);
const encW = enc.filter(inWin), encdW = encd.filter(inWin);
console.log('== encoder ==');
console.log(`ticks ${encW.length} (${(encW.length / secs).toFixed(1)}/s), dropped ticks ${drop.filter(inWin).reduce((s, r) => s + r.b, 0)}`);
console.log(`lock wait p99 ${ms(pct(encW.map(r => r.b), .99))} max ${ms(Math.max(...encW.map(r => r.b)))} ms; encode p50 ${ms(pct(encW.map(r => r.c), .5))} p99 ${ms(pct(encW.map(r => r.c), .99))} max ${ms(Math.max(...encW.map(r => r.c)))} ms`);
console.log(`send p99 ${ms(pct(encdW.map(r => r.b), .99))} max ${ms(Math.max(...encdW.map(r => r.b)))}; drain p99 ${ms(pct(encdW.map(r => r.c), .99))} max ${ms(Math.max(...encdW.map(r => r.c)))} ms`);

const pk = execFileSync(ffprobe, ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'packet=pts_time,size', '-of', 'csv=p=0', clip], { encoding: 'utf8', maxBuffer: 1 << 28 })
  .trim().split('\n').map(l => l.split(',')).map(([t, s]) => ({ t: +t, s: +s })).sort((a, b) => a.t - b.t);
// A repeated picture encodes to a tiny P-frame; ratio to the clip's median frame size marks it.
const med = pct(pk.map(p => p.s), .5);
const dup = pk.map(p => p.s < Math.max(1500, med * 0.02));
let run = 0, runs = [];
dup.forEach(d => { if (d) run++; else { if (run) runs.push(run); run = 0; } });
const holes = pk.slice(1).filter((p, i) => p.t - pk[i].t > 1.5 / 60).length;
const dur = pk.at(-1).t - pk[0].t;
console.log('== clip ==', clip);
console.log(`frames ${pk.length} over ${dur.toFixed(1)} s (${(pk.length / dur).toFixed(1)}/s), timestamp holes ${holes}`);
console.log(`distinct pictures/s ${(dup.filter(d => !d).length / dur).toFixed(1)}; repeated-frame runs >=3: ${runs.filter(r => r >= 3).length}, longest ${Math.max(0, ...runs)} frames`);
console.log('trace:', tracePath);
