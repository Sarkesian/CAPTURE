// Low-disk behaviour: raise the free-space reserve above the actual free space mid-recording
// (with a session running), then lower it again. Also records with the software encoder.
//   node scripts/e2e-lowdisk.mjs <worker.exe> <captureRoot>
import { spawn } from 'node:child_process';
import readline from 'node:readline';
import { existsSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
const [worker, root] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const events = [];
readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
let id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
const waitFor = async (pred, ms = 30000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = events.find(pred); if (h) return h; await sleep(100); } throw new Error('timeout'); };
const state = () => [...events].reverse().find(m => m.event === 'state')?.data;
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };
const files = sub => { const d = path.join(root, sub); return existsSync(d) ? readdirSync(d, { recursive: true }).filter(f => String(f).endsWith('.mp4')).map(f => path.join(d, String(f))) : []; };

const base = { capture_root: root, replay_secs: 20, target: { kind: 'display', index: 0 }, quality: 'balanced', fps: 60, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: '', overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 5e9 };

// Part 1: software encoder records a valid clip.
send('configure', { ...base, encoder: 'software' });
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering', 30000);
const enc = state().encoder;
check(enc && enc.name === 'h264_mf' && !enc.hardware, `software encoder in use: ${enc?.name} hardware=${enc?.hardware} (${enc?.rejected?.join('; ')})`);
await sleep(8000);
send('save_replay');
const sw = await waitFor(m => m.event === 'save_finished' || m.event === 'save_failed', 30000);
check(sw.event === 'save_finished' && sw.data.duration_secs > 5, `software-encoded clip saved: ${sw.data.duration_secs?.toFixed(2)} s`);
const swPath = sw.data.path;

// Part 2: low disk while a session is running.
send('configure', { ...base });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering' && m.data.encoder?.hardware, 30000);
await sleep(3000);
send('start_session', {});
await sleep(5000);
const free = state().free_bytes;
const kept = statSync(swPath).size;
send('configure', { ...base, min_free_bytes: free + 50e9 }); // reserve now larger than the drive's free space
const paused = await waitFor(m => m.event === 'state' && m.data.status === 'needs_attention' && /Low disk space/.test(m.data.attention ?? ''), 15000).then(m => m.data.attention, () => null);
check(!!paused, `buffering paused with: "${paused}"`);
const sess = await waitFor(m => m.event === 'save_finished' && m.data.path.includes('Sessions'), 20000).then(m => m.data, () => null);
check(!!sess, `running session was finalized safely: ${sess?.path ?? 'none'} (${sess?.duration_secs?.toFixed(1)} s)`);
check(existsSync(swPath) && statSync(swPath).size === kept, 'previously saved clip untouched');
send('configure', { ...base });
const resumed = await waitFor(m => m.event === 'state' && m.data.status === 'buffering' && events.indexOf(m) > events.length - 50, 20000).then(() => true, () => false);
check(resumed, 'buffering resumed when space was available again');
send('shutdown');
await new Promise(r => proc.on('exit', r));
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
