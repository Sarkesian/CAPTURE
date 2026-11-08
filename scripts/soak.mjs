// Long-run soak: window-capture a looping flash+beep pattern for N minutes with a 2-minute
// replay, save every 5 minutes, sample worker memory/CPU and buffer size every 30 s.
//   node scripts/soak.mjs <worker.exe> <captureRoot> <ffplay.exe> <pattern.mp4> <minutes> <log.json>
import { spawn, execFileSync } from 'node:child_process';
import readline from 'node:readline';
import { writeFileSync } from 'node:fs';
const [worker, root, ffplay, pattern, minutesArg, logPath] = process.argv.slice(2);
const minutes = Number(minutesArg ?? 25);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const player = spawn(ffplay, ['-window_title', 'SOAK', '-loglevel', 'quiet', '-loop', '0', pattern], { stdio: 'ignore' });
await sleep(2500);
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const events = [];
readline.createInterface({ input: proc.stdout }).on('line', l => { const m = JSON.parse(l); if (m.event !== 'state') events.push(m); else last = m.data; });
let last = null, id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
send('configure', { capture_root: root, replay_secs: 120, target: { kind: 'app', exe: 'ffplay.exe' }, quality: 'high', fps: 60, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: '', overlay: false, buffer_cap_bytes: 20e9, min_free_bytes: 10e9 });
send('set_replay', { enabled: true });
const samples = [];
const t0 = Date.now();
let nextSave = 5 * 60e3;
while (Date.now() - t0 < minutes * 60e3) {
  await sleep(30e3);
  const ps = execFileSync('powershell', ['-NoProfile', '-Command', `$p = Get-Process -Id ${proc.pid}; "{0} {1} {2}" -f $p.WorkingSet64, $p.PrivateMemorySize64, $p.TotalProcessorTime.TotalSeconds`]).toString().trim().split(' ').map(Number);
  const s = { t_min: +((Date.now() - t0) / 60e3).toFixed(2), ws_mb: Math.round(ps[0] / 1e6), private_mb: Math.round(ps[1] / 1e6), cpu_s: ps[2],
    status: last?.status, buffered: last?.buffered_secs, buffer_mb: Math.round((last?.buffer_bytes ?? 0) / 1e6), fps_in: last?.capture_fps };
  samples.push(s);
  console.log(JSON.stringify(s));
  if (Date.now() - t0 >= nextSave) { send('save_replay'); nextSave += 5 * 60e3; }
}
send('save_replay');
await sleep(12000);
const saves = events.filter(m => m.event === 'save_finished').map(m => ({ path: m.data.path, secs: m.data.duration_secs, mb: Math.round(m.data.bytes / 1e6) }));
const fails = events.filter(m => m.event === 'save_failed').map(m => m.data.message);
send('shutdown');
await new Promise(r => proc.on('exit', r));
player.kill();
writeFileSync(logPath, JSON.stringify({ minutes, samples, saves, fails }, null, 2));
console.log('SAVES', JSON.stringify(saves), 'FAILS', JSON.stringify(fails));
