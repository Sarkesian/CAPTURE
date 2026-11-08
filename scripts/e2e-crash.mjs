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
await sleep(12000);
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
b.send('shutdown'); await new Promise(r => b.proc.on('exit', r));
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
