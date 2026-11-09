// Capture drive disappears and comes back while buffering (simulated with `subst`, which needs
// no admin rights and touches no hardware): CAPTURE must say so, keep running, recover by itself
// when the drive is back, and still save a clip.
//   node scripts/e2e-unplug.mjs <worker.exe> <backingDir> [driveLetter=X]
import { execFileSync, spawn } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import readline from 'node:readline';
const [worker, backing, letter = 'X'] = process.argv.slice(2);
const drive = `${letter}:`;
const subst = (...a) => { try { execFileSync('subst', a, { stdio: 'ignore' }); } catch {} };
mkdirSync(backing, { recursive: true });
subst(drive, '/d');
subst(drive, backing);
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
try {
  send('configure', { capture_root: `${drive}\\cap`, replay_secs: 20, target: { kind: 'display', index: 0 }, quality: 'balanced', fps: 30, capture_cursor: false,
    audio: { game_audio: false }, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '' }, overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 1e9 });
  send('set_replay', { enabled: true });
  await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
  await sleep(5000);
  subst(drive, '/d');
  console.log('drive removed');
  await sleep(6000);
  let st = state();
  check(st.status === 'needs_attention' && /write|folder|drive/i.test(st.attention ?? ''), `drive gone -> needs attention: "${st.attention}"`);
  subst(drive, backing);
  console.log('drive back');
  const back = events.length;
  const ok = await waitFor(m => events.indexOf(m) >= back && m.event === 'state' && m.data.status === 'buffering' && !m.data.attention, 15000).then(() => true, () => false);
  st = state();
  check(ok && st.status === 'buffering' && !st.attention, `drive back -> buffering again by itself (status ${st.status}, attention ${st.attention})`);
  await sleep(6000);
  const s = send('save_replay');
  const fin = await waitFor(m => (m.event === 'save_finished' || m.event === 'save_failed') && (m.re === s || m.event === 'save_finished'), 40000);
  check(fin.event === 'save_finished', fin.event === 'save_finished' ? `clip saved after recovery: ${fin.data.duration_secs.toFixed(1)} s ${fin.data.path}` : `save failed: ${fin.data.message}`);
} catch (e) { console.log('ERROR', e.message); fail++; }
if (process.env.DEBUG) { let last = ''; for (const m of events.filter(m => m.event === 'state')) { const k = m.data.status + ' ' + (m.data.attention ?? '').slice(0, 60) + ' buf=' + m.data.buffered_secs; if (k.split(' buf')[0] !== last) { console.log('  state:', k); last = k.split(' buf')[0]; } } }
send('shutdown');
await new Promise(r => { proc.on('exit', r); setTimeout(r, 40000); });
subst(drive, '/d');
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
