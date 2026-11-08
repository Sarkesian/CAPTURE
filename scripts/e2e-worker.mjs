// End-to-end harness for capture-worker (no UI). Drives the real worker over its stdio protocol.
//
//   node scripts/e2e-worker.mjs <worker.exe> <captureRoot> <target> [replaySecs]
//     target: "display:0" | "window:<exe name substring>"
//
// Scenario: young-buffer save at ~3 s, two overlapping saves 300 ms apart after the buffer
// is full, then a clean shutdown. Prints every saved clip and exits non-zero on any failure.
import { spawn } from 'node:child_process';
import readline from 'node:readline';

const [worker, root, targetArg, replayArg] = process.argv.slice(2);
const replay = Number(replayArg ?? 10);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const rl = readline.createInterface({ input: proc.stdout });
let id = 0;
const waiters = [];
const events = [];
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
let target;
if (targetArg.startsWith('display:')) target = { kind: 'display', index: Number(targetArg.slice(8)) };
else {
  const w = targets.windows.find(w => w.exe.toLowerCase().includes(targetArg.slice(7).toLowerCase()));
  if (!w) throw new Error('window not found; have: ' + targets.windows.map(w => w.exe).join(', '));
  target = { kind: 'window', hwnd: w.hwnd, pid: w.pid, exe: w.exe, title: w.title };
  console.log('window target:', w.app_name, `${w.width}x${w.height}`);
}
send('configure', {
  capture_root: root, replay_secs: replay, target, quality: 'balanced', fps: 60, capture_cursor: true,
  audio: { game_audio: true }, save_hotkey: 'Alt+F10', buffer_cap_bytes: 4e9, min_free_bytes: 5e9,
});
const t0 = Date.now();
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering', 15000);
console.log(`buffering after ${Date.now() - t0} ms`);
await sleep(Math.max(0, 3000 - (Date.now() - t0)));
const saves = [];
const save = () => { const r = send('save_replay'); saves.push(r); return r; };
save(); // young buffer
await sleep(replay * 1000 + 4000);
save(); await sleep(300); save(); // overlapping
const done = [];
for (const r of saves) {
  const started = await waitFor(m => m.re === r && (m.event === 'save_started' || m.event === 'save_failed'));
  if (started.event === 'save_failed') { console.log('SAVE FAILED', started.data.message); process.exitCode = 1; continue; }
  const sid = started.data.save_id;
  const fin = await waitFor(m => (m.event === 'save_finished' && m.data.save_id === sid) || (m.event === 'save_failed' && m.data.save_id === sid), 40000);
  done.push(fin);
  if (fin.event === 'save_failed') { console.log('SAVE FAILED', fin.data.message); process.exitCode = 1; }
  else console.log(`saved #${sid}: ${fin.data.duration_secs.toFixed(2)}s (requested ${fin.data.requested_secs}) ${(fin.data.bytes / 1e6).toFixed(1)} MB tracks=${JSON.stringify(fin.data.audio_tracks)}\n   ${fin.data.path}`);
}
const last = [...events].reverse().find(m => m.event === 'state').data;
console.log('state:', JSON.stringify({ status: last.status, attention: last.attention, encoder: last.encoder?.name, rejected: last.encoder?.rejected, fps: last.capture_fps, buffered: last.buffered_secs, bufMB: Math.round(last.buffer_bytes / 1e6), hotkey: last.hotkey, tracks: last.tracks }));
send('shutdown');
await new Promise(r => proc.on('exit', r));
console.log('worker exited cleanly');
