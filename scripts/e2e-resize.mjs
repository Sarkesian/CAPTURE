// The recorded window changes size mid-recording (a game going from a small window or splash
// screen to its real resolution). The whole picture must stay in the recording, scaled into
// the recording size: nothing cut off, the replay buffer continuous across the change.
//
//   node scripts/e2e-resize.mjs <worker.exe> <captureRoot>
//
// The target is a window this test creates: blue, with a red square pinned to its bottom-right
// corner. It grows from 640x400 to 1400x900 after 5 s. In the saved clip's last frame the
// bottom-right area must be red (it is cut off when frames are only copied at the old size).
import { spawn, execFileSync } from 'node:child_process';
import readline from 'node:readline';
import path from 'node:path';

const [worker, root] = process.argv.slice(2);
const ffmpeg = path.join(import.meta.dirname, '..', 'third_party', 'ffmpeg', 'bin', 'ffmpeg.exe');
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };

const title = `CAPTURE-resize-test-${process.pid}`;
const form = spawn('powershell', ['-NoProfile', '-Command', [
  'Add-Type -AssemblyName System.Windows.Forms',
  '$f = New-Object Windows.Forms.Form', `$f.Text = '${title}'`, '$f.Width = 640; $f.Height = 400', "$f.BackColor = 'Blue'",
  '$p = New-Object Windows.Forms.Panel; $p.Width = 120; $p.Height = 120; $p.BackColor = [Drawing.Color]::Red',
  '$p.Anchor = [Windows.Forms.AnchorStyles]::Bottom -bor [Windows.Forms.AnchorStyles]::Right',
  '$p.Left = $f.ClientSize.Width - 120; $p.Top = $f.ClientSize.Height - 120; $f.Controls.Add($p)',
  '$t = New-Object Windows.Forms.Timer; $t.Interval = 5000; $t.Add_Tick({ $f.Width = 1400; $f.Height = 900; $t.Stop() }); $t.Start()',
  '[Windows.Forms.Application]::Run($f)'].join('; ')], { stdio: 'ignore' });
await sleep(3000);

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
if (!w) { console.log('test window not found'); form.kill(); proc.kill(); process.exit(2); }
send('configure', {
  capture_root: root, replay_secs: 30, target: { kind: 'window', hwnd: w.hwnd, pid: w.pid, exe: w.exe, title: w.title },
  quality: 'balanced', fps: 30, capture_cursor: false, audio: { game_audio: false }, save_hotkey: 'Alt+F10', buffer_cap_bytes: 4e9, min_free_bytes: 5e9,
});
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering', 15000);
await sleep(9000); // the window grows at ~5 s after it opened
const r = send('save_replay');
const started = await waitFor(m => m.re === r && (m.event === 'save_started' || m.event === 'save_failed'));
const fin = started.event === 'save_started'
  ? await waitFor(m => (m.event === 'save_finished' || m.event === 'save_failed') && m.data.save_id === started.data.save_id, 40000)
  : started;
check(fin.event === 'save_finished', `clip spans the size change: ${fin.data.path ?? fin.data.message}`);
if (fin.event === 'save_finished') {
  const probe = JSON.parse(execFileSync(path.join(path.dirname(ffmpeg), 'ffprobe.exe'), ['-v', 'error', '-select_streams', 'v', '-show_entries', 'stream=width,height', '-of', 'json', fin.data.path], { encoding: 'utf8' })).streams[0];
  // Last frame as raw RGB; sample the bottom-right and the centre.
  const raw = execFileSync(ffmpeg, ['-v', 'error', '-sseof', '-0.5', '-i', fin.data.path, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'], { maxBuffer: 1 << 28 });
  const px = (x, y) => { const i = (y * probe.width + x) * 3; return [raw[i], raw[i + 1], raw[i + 2]]; };
  const near = (fx, fy) => px(Math.floor(probe.width * fx), Math.floor(probe.height * fy));
  const isRed = ([r, g, b]) => r > 180 && g < 80 && b < 80;
  const isBlue = ([r, g, b]) => b > 180 && r < 80 && g < 80;
  const corner = near(0.96, 0.94), centre = near(0.5, 0.5);
  check(isBlue(centre), `centre of the last frame is the window (blue): ${centre}`);
  check(isRed(corner), `bottom-right of the last frame shows the red corner (whole window kept): ${corner}`);
  console.log(`recording ${probe.width}x${probe.height}, ${fin.data.duration_secs.toFixed(2)} s`);
  const v = spawn(process.execPath, [path.join(import.meta.dirname, 'validate-media.mjs'), fin.data.path], { stdio: 'inherit' });
  check(await new Promise(r => v.on('exit', r)) === 0, 'clip decodes cleanly');
}
send('shutdown');
await new Promise(r => proc.on('exit', r));
form.kill();
try { execFileSync('taskkill', ['/PID', String(form.pid), '/F'], { stdio: 'ignore' }); } catch {}
console.log(fail ? `${fail} FAILED` : 'all passed');
process.exit(fail ? 1 : 0);
