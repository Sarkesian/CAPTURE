// Worker-level test: full-session recording via the shared ring.
//   node scripts/e2e-session.mjs <worker.exe> <captureRoot>
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
const state = () => [...events].reverse().find(m => m.event === 'state')?.data;
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };
send('configure', { capture_root: root, replay_secs: 5, target: { kind: 'display', index: 0 }, quality: 'balanced', fps: 60, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: 'Alt+F7', screenshot: '' }, overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 5e9 });
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(3000);
send('start_session', {});
await sleep(1200);
check(state().session_secs > 0, `session running (${state().session_secs} s)`);
await sleep(9000); // longer than the 5 s replay length: session must keep it all
const s = send('stop_session', {});
const st = await waitFor(m => m.re === s && m.event === 'save_started');
const fin = await waitFor(m => m.event === 'save_finished' && m.data.save_id === st.data.save_id);
check(fin.data.path.replaceAll('\\', '/').includes('/Desktop/Sessions/') && fin.data.path.endsWith('_Desktop_Session.mp4'), `saved to ${fin.data.path}`);
check(fin.data.duration_secs > 9.5 && fin.data.duration_secs < 12, `session length ${fin.data.duration_secs.toFixed(2)} s (expected ~10.2, longer than the 5 s replay)`);
await sleep(600);
check(state().session_secs == null, 'session state cleared');
send('shutdown');
await new Promise(r => proc.on('exit', r));
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
