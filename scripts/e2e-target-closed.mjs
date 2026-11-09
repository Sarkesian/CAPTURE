// The recorded window closes, then the user saves: the clip must end where the window closed,
// arrive quickly (the last buffer segment is finalized, not left open), contain no audio-only
// tail, and decode cleanly.
//
//   node scripts/e2e-target-closed.mjs <worker.exe> <captureRoot>
//
// Uses a window this test creates as the target (explicit window target, not Auto detection).
import { spawn, execFileSync } from 'node:child_process';
import readline from 'node:readline';
import path from 'node:path';

const [worker, root] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };

// A window this test owns: a PowerShell WinForms window with a unique title. (Never an app the
// user may have open: Windows 11 Notepad, for example, hosts all its windows in one process.)
const title = `CAPTURE-test-window-${process.pid}`;
const np = spawn('powershell', ['-NoProfile', '-Command',
  `Add-Type -AssemblyName System.Windows.Forms; $f = New-Object Windows.Forms.Form; $f.Text = '${title}'; $f.Width = 800; $f.Height = 600; $f.BackColor = 'DarkCyan'; [Windows.Forms.Application]::Run($f)`],
  { stdio: 'ignore' });
await sleep(4000);
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const rl = readline.createInterface({ input: proc.stdout });
let id = 0;
const events = [], waiters = [];
rl.on('line', line => { const m = JSON.parse(line); events.push(m); for (const w of [...waiters]) if (w.pred(m)) { waiters.splice(waiters.indexOf(w), 1); w.resolve(m); } });
const send = (cmd, args) => { const r = { v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }; proc.stdin.write(JSON.stringify(r) + '\n'); return r.id; };
const waitFor = (pred, ms = 30000) => new Promise((resolve, reject) => {
  const hit = events.find(pred); if (hit) return resolve(hit);
  waiters.push({ pred, resolve }); setTimeout(() => reject(new Error('timeout')), ms);
});

await waitFor(m => m.event === 'hello');
const tid = send('list_targets');
const targets = (await waitFor(m => m.re === tid && m.event === 'targets')).data;
const w = targets.windows.find(w => w.title === title && w.pid === np.pid);
if (!w) { console.log('test window not found'); np.kill(); process.exit(2); }
send('configure', {
  capture_root: root, replay_secs: 30, target: { kind: 'window', hwnd: w.hwnd, pid: w.pid, exe: w.exe, title: w.title },
  quality: 'balanced', fps: 60, capture_cursor: true, audio: { game_audio: true, other: true }, save_hotkey: 'Alt+F10',
  buffer_cap_bytes: 4e9, min_free_bytes: 5e9,
});
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering', 15000);
await sleep(8000);
const closedAt = Date.now();
execFileSync('taskkill', ['/PID', String(w.pid), '/F'], { stdio: 'ignore' });
await sleep(4000); // the user saves a few seconds after the game closed
const save = async () => {
  const r = send('save_replay');
  const t0 = Date.now();
  const started = await waitFor(m => m.re === r && (m.event === 'save_started' || m.event === 'save_failed'));
  if (started.event !== 'save_started') return { started, took: 0 };
  const fin = await waitFor(m => (m.event === 'save_finished' || m.event === 'save_failed') && m.data.save_id === started.data.save_id, 40000);
  return { started, fin, took: (Date.now() - t0) / 1000 };
};
const r = send('save_replay');
const t0 = Date.now();
const started = await waitFor(m => m.re === r && (m.event === 'save_started' || m.event === 'save_failed'));
check(started.event === 'save_started', `save accepted after the window closed (${started.event})`);
const fin = await waitFor(m => (m.event === 'save_finished' || m.event === 'save_failed') && m.data.save_id === started.data.save_id, 40000);
const took = (Date.now() - t0) / 1000;
check(fin.event === 'save_finished', `clip saved: ${fin.data.path ?? fin.data.message}`);
check(took < 5, `save finished in ${took.toFixed(1)} s (no wait for a segment that never closes)`);
if (fin.event === 'save_finished') {
  const dur = fin.data.duration_secs;
  check(dur > 6 && dur < 12, `clip is the ~8 s recorded before the close, not padded to the save time: ${dur.toFixed(2)} s`);
  const v = spawn(process.execPath, [path.join(import.meta.dirname, 'validate-media.mjs'), fin.data.path], { stdio: 'inherit' });
  check(await new Promise(r => v.on('exit', r)) === 0, 'clip decodes cleanly, audio ends with the video');
}
// A second save after the close still has the footage (the buffer is kept until a new game).
const again = await save();
check(again.fin?.event === 'save_finished' && again.fin.data.duration_secs > 6, `second save after the close also works (${again.fin?.data.duration_secs?.toFixed(2) ?? again.started.data.message})`);
const st = [...events].reverse().find(m => m.event === 'state').data;
const bufMB = st.buffer_bytes / 1e6;
await sleep(3000);
const st2 = [...events].reverse().find(m => m.event === 'state').data;
check(st2.buffer_bytes / 1e6 - bufMB < 0.5, `buffer stops growing after the close (${bufMB.toFixed(1)} -> ${(st2.buffer_bytes / 1e6).toFixed(1)} MB)`);
console.log(`closed ${((t0 - closedAt) / 1000).toFixed(1)} s before saving; status ${st2.status}: ${st2.attention ?? ''}`);
send('shutdown');
await new Promise(r => proc.on('exit', r));
console.log(fail ? `${fail} FAILED` : 'all passed');
process.exit(fail ? 1 : 0);
