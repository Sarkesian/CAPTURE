// Forced-kill + recovery: record, kill the worker (crash), restart, recover leftovers.
//   node scripts/e2e-crash.mjs <worker.exe> <captureRoot>
import { spawn } from 'node:child_process';
import readline from 'node:readline';
const [worker, root] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };
function start() {
  const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
  const events = [];
  readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
  let id = 0;
  const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
  const waitFor = async (pred, ms = 30000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = events.find(pred); if (h) return h; await sleep(100); } throw new Error('timeout'); };
  return { proc, events, send, waitFor, state: () => [...events].reverse().find(m => m.event === 'state')?.data };
}
const cfg = { capture_root: root, replay_secs: 30, target: { kind: 'display', index: 0 }, quality: 'balanced', fps: 60, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: '', overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 5e9 };
const a = start();
a.send('configure', cfg); a.send('set_replay', { enabled: true });
await a.waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(5000);
const mk = a.send('add_marker', { label: 'before crash' });
const marker = (await a.waitFor(m => m.re === mk && m.event === 'marker_added')).data;
await sleep(7000);
a.send('get_state'); await sleep(300);
const nowAtKill = a.state().now_secs;
a.proc.kill('SIGKILL'); // TerminateProcess: no cleanup at all
await new Promise(r => a.proc.on('exit', r));
const b = start();
b.send('configure', cfg); b.send('get_state');
await sleep(1500);
const left = b.state().leftover_buffer_bytes;
check(left > 1e6, `after the crash, leftover footage is reported (${(left / 1e6).toFixed(1)} MB), nothing deleted`);
const r = b.send('recover_leftovers');
const rec = await b.waitFor(m => m.re === r && m.event === 'recovered', 60000);
check(rec.data.clips.length === 1 && rec.data.failed.length === 0, `recovered: ${JSON.stringify(rec.data)}`);
b.send('get_state'); await sleep(800);
check(b.state().leftover_buffer_bytes === 0, 'leftover removed only after the clip was saved');
console.log('RECOVERED=' + (rec.data.clips[0] ?? ''));
// The marker survives the crash, placed where it was in the recovered clip. The recovered clip
// ends at the kill, so marker position from the end = (now at kill - marker.t), within ~1.5 s.
const side = rec.data.clips[0] + '.markers.json';
const { existsSync, readFileSync } = await import('node:fs');
const { spawnSync } = await import('node:child_process');
if (existsSync(side)) {
  const doc = JSON.parse(readFileSync(side, 'utf8'));
  const dur = Number(spawnSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', rec.data.clips[0]], { encoding: 'utf8' }).stdout.trim());
  const fromEnd = dur - doc.markers[0].t, expected = nowAtKill - marker.t;
  check(doc.markers.length === 1 && doc.markers[0].label === 'before crash' && Math.abs(fromEnd - expected) < 1.5,
    `marker kept in the recovered clip at ${doc.markers[0].t} s of ${dur.toFixed(1)} s (${fromEnd.toFixed(1)} s before the end, expected ~${expected.toFixed(1)})`);
} else check(false, 'marker sidecar written for the recovered clip');
b.send('shutdown'); await new Promise(r => b.proc.on('exit', r));
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
