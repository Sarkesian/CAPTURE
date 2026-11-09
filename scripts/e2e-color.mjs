// Colour accuracy of the recording: a window with known colour patches is recorded with the
// automatic (GPU) encoder and the software encoder; the decoded colours must match within a
// small tolerance (8-bit 4:2:0 H.264, measured in the middle of each patch).
//
//   node scripts/e2e-color.mjs <worker.exe> <captureRoot>
import { spawn, execFileSync } from 'node:child_process';
import readline from 'node:readline';
import path from 'node:path';

const [worker, root] = process.argv.slice(2);
const bin = path.join(import.meta.dirname, '..', 'third_party', 'ffmpeg', 'bin');
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; return ok; };
// Four vertical patches, left to right.
const patches = [[230, 120, 30], [40, 170, 160], [128, 128, 128], [200, 60, 170]];

const title = `CAPTURE-color-test-${process.pid}`;
const form = spawn('powershell', ['-NoProfile', '-Command', [
  'Add-Type -AssemblyName System.Windows.Forms',
  '$f = New-Object Windows.Forms.Form', `$f.Text = '${title}'`, '$f.ClientSize = New-Object Drawing.Size(800, 400)',
  ...patches.map(([r, g, b], i) => `$p${i} = New-Object Windows.Forms.Panel; $p${i}.SetBounds(${i * 200}, 0, 200, 400); $p${i}.BackColor = [Drawing.Color]::FromArgb(${r}, ${g}, ${b}); $f.Controls.Add($p${i})`),
  '[Windows.Forms.Application]::Run($f)'].join('; ')], { stdio: 'ignore' });
await sleep(3000);

async function record(encoder) {
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
  const w = (await waitFor(m => m.re === tid && m.event === 'targets')).data.windows.find(w => w.title === title && w.pid === form.pid);
  if (!w) throw new Error('test window not found');
  send('configure', {
    capture_root: root, replay_secs: 10, target: { kind: 'window', hwnd: w.hwnd, pid: w.pid, exe: w.exe, title: w.title },
    quality: 'high', fps: 30, capture_cursor: false, audio: { game_audio: false }, save_hotkey: 'Alt+F10',
    buffer_cap_bytes: 4e9, min_free_bytes: 5e9, encoder,
  });
  send('set_replay', { enabled: true });
  const st = await waitFor(m => m.event === 'state' && m.data.status === 'buffering', 15000);
  await sleep(4000);
  const r = send('save_replay');
  const started = await waitFor(m => m.re === r && (m.event === 'save_started' || m.event === 'save_failed'));
  const fin = await waitFor(m => (m.event === 'save_finished' || m.event === 'save_failed') && m.data.save_id === started.data.save_id, 40000);
  const name = [...events].reverse().find(m => m.event === 'state')?.data.encoder?.name;
  send('shutdown');
  await new Promise(r => proc.on('exit', r));
  return { fin, name };
}

for (const encoder of ['auto', 'software']) {
  const { fin, name } = await record(encoder);
  if (!check(fin.event === 'save_finished', `${encoder}: clip saved (${name})`)) continue;
  const probe = JSON.parse(execFileSync(path.join(bin, 'ffprobe.exe'), ['-v', 'error', '-select_streams', 'v', '-show_entries', 'stream=width,height,color_space,color_primaries,color_transfer,color_range', '-of', 'json', fin.data.path], { encoding: 'utf8' })).streams[0];
  const raw = execFileSync(path.join(bin, 'ffmpeg.exe'), ['-v', 'error', '-sseof', '-1', '-i', fin.data.path, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'], { maxBuffer: 1 << 28 });
  const px = (x, y) => { const i = (y * probe.width + x) * 3; return [raw[i], raw[i + 1], raw[i + 2]]; };
  const worst = patches.map((want, i) => {
    const got = px(Math.floor(probe.width * (i * 0.25 + 0.125)), Math.floor(probe.height * 0.5));
    return { want, got, err: Math.max(...want.map((v, k) => Math.abs(v - got[k]))) };
  });
  const maxErr = Math.max(...worst.map(w => w.err));
  check(maxErr <= 6, `${name}: colours within 6/255 (worst ${maxErr}); tags ${probe.color_space}/${probe.color_primaries}/${probe.color_transfer}/${probe.color_range}`);
  for (const w of worst) console.log(`      want ${w.want}  got ${w.got}`);
}
form.kill();
try { execFileSync('taskkill', ['/PID', String(form.pid), '/F'], { stdio: 'ignore' }); } catch {}
console.log(fail ? `${fail} FAILED` : 'all passed');
process.exit(fail ? 1 : 0);
