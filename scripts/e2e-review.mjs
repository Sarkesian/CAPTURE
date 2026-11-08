// Worker-level test: review snapshot + exact range save + precise replay length.
//   node scripts/e2e-review.mjs <worker.exe> <captureRoot>
import { spawn } from 'node:child_process';
import readline from 'node:readline';
const [worker, root] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const events = [];
readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
let id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
const waitFor = async (pred, ms = 30000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = events.find(pred); if (h) return h; await sleep(100); } throw new Error('timeout'); };
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };

send('configure', { capture_root: root, replay_secs: 6, target: { kind: 'display', index: 0 }, quality: 'balanced', fps: 60, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '' }, overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 5e9 });
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(15000);
const t = Date.now();
const r = send('prepare_review');
const rv = await waitFor(m => m.re === r && (m.event === 'review_ready' || m.event === 'review_failed'));
check(rv.event === 'review_ready', `review snapshot ready in ${Date.now() - t} ms: ${JSON.stringify(rv.data)}`);
const s = send('save_range', { start: rv.data.start + 3, end: rv.data.start + 8 });
const st = await waitFor(m => m.re === s && m.event === 'save_started');
const fin = await waitFor(m => m.event === 'save_finished' && m.data.save_id === st.data.save_id);
check(fin.data.duration_secs >= 4.9 && fin.data.duration_secs <= 6.1, `range 3..8 s saved as ${fin.data.duration_secs.toFixed(2)} s (5 s + up to 1 s keyframe)`);
const s2 = send('save_replay');
const st2 = await waitFor(m => m.re === s2 && m.event === 'save_started');
const fin2 = await waitFor(m => m.event === 'save_finished' && m.data.save_id === st2.data.save_id);
check(fin2.data.duration_secs >= 5.9 && fin2.data.duration_secs <= 7.1, `6 s replay saved as ${fin2.data.duration_secs.toFixed(2)} s (was up to +2 s before)`);
console.log('review file:', rv.data.path);
send('release_review');
await sleep(500);
send('shutdown');
await new Promise(r => proc.on('exit', r));
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
