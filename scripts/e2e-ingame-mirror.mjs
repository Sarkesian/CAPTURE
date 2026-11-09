// CAPTURE's overlay window shown inside a game through the in-game hook.
//
// A green stand-in "overlay" window is shown behind the game without taking focus (as CAPTURE
// does with its real overlay over a fullscreen game, where nothing behind the game is seen); the worker is told to mirror it into the test
// game. The recording of the game must then show the game darkened with the green panel in
// the middle, live, and, once the mirror is stopped, the plain game again. Input: a click and
// a key sent to the GAME window must reach the stand-in through the hook (it turns red when
// clicked, blue on a key), and its new colour must show in the game.
//
//   node scripts/e2e-ingame-mirror.mjs <worker.exe> <captureRoot> <hook-testapp.exe>
import { spawn, spawnSync } from 'node:child_process';
import readline from 'node:readline';
import path from 'node:path';
import { readFileSync, mkdirSync } from 'node:fs';

const [worker, root, testapp] = process.argv.slice(2);
const ffmpeg = path.join(import.meta.dirname, '..', 'third_party', 'ffmpeg', 'bin', 'ffmpeg.exe');
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };
mkdirSync(root, { recursive: true });

function frameAt(clip, t) {
  const file = path.join(root, 'f.raw');
  const r = spawnSync(ffmpeg, ['-hide_banner', '-y', '-ss', String(t), '-i', clip, '-frames:v', '1', '-vf', 'showinfo', '-pix_fmt', 'rgb24', '-f', 'rawvideo', file], { encoding: 'utf8', windowsHide: true });
  const [, w, h] = r.stderr.match(/ s:(\d+)x(\d+)/) ?? [0, 0, 0];
  const buf = readFileSync(file);
  const px = (x, y) => { const i = (Math.round(y) * +w + Math.round(x)) * 3; return [buf[i], buf[i + 1], buf[i + 2]]; };
  // A little up and left of the middle: our mouse pointer is drawn from the middle down-right.
  return { centre: px(w / 2 - 30, h / 2 - 30), corner: px(w * 0.05, h * 0.9) };
}
const green = p => p[1] > 150 && p[0] < 90 && p[2] < 90;

const title = `CAPTURE-mirror-overlay-${process.pid}`;
// Stand-in overlay: green, behind every other window (covered by the game), not activated.
// (An off-screen window is not painted, so there would be nothing to capture.)
const overlay = spawn('powershell', ['-NoProfile', '-Command', [
  'Add-Type -AssemblyName System.Windows.Forms',
  "Add-Type -Name N -Namespace W -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool ShowWindow(IntPtr h, int c); [DllImport(\"user32.dll\")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);'",
  `$f = New-Object Windows.Forms.Form; $f.Text = '${title}'; $f.FormBorderStyle = 'None'; $f.BackColor = [Drawing.Color]::FromArgb(0, 200, 0)`,
  "$f.Add_MouseDown({ $f.BackColor = [Drawing.Color]::FromArgb(220, 0, 0) }); $f.Add_KeyDown({ $f.BackColor = [Drawing.Color]::FromArgb(0, 0, 230) })",
  "$f.StartPosition = 'Manual'; $f.ShowInTaskbar = $false; $f.Left = 400; $f.Top = 300; $f.Width = 700; $f.Height = 450",
  '[W.N]::SetWindowPos($f.Handle, [IntPtr]1, 0, 0, 0, 0, 0x0013) | Out-Null; [W.N]::ShowWindow($f.Handle, 4) | Out-Null; [W.N]::SetWindowPos($f.Handle, [IntPtr]1, 0, 0, 0, 0, 0x0013) | Out-Null; [Console]::Out.WriteLine([int64]$f.Handle); [Console]::Out.Flush()',
  '$t = New-Object Windows.Forms.Timer; $t.Interval = 90000; $t.Add_Tick({ $f.Close() }); $t.Start()',
  '[Windows.Forms.Application]::Run()'].join('; ')], { stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true });
let overlayHwnd = 0;
overlay.stdout.on('data', d => { const n = Number(String(d).trim()); if (n) overlayHwnd = n; });
await sleep(1500);
const app = spawn(testapp, ['--secs', '90', '--title', `CAPTURE-mirror-game-${process.pid}`], { stdio: 'ignore' });
await sleep(3500);

const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
let log = '';
proc.stderr.on('data', d => { log += d; });
const ev = [];
readline.createInterface({ input: proc.stdout }).on('line', l => ev.push(JSON.parse(l)));
let id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args ? { args } : {}) }) + '\n'); return id; };
const waitFor = async (p, ms = 30000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = ev.findLast(p); if (h) return h; await sleep(100); } throw new Error('timeout'); };

await waitFor(m => m.event === 'hello');
let t = send('list_targets');
const targets = (await waitFor(m => m.re === t && m.event === 'targets')).data;
const game = targets.windows.find(w => w.pid === app.pid);
const ov = { hwnd: overlayHwnd };
if (!ov.hwnd) throw new Error('stand-in overlay window did not start');
if (!game) throw new Error('test game window not found');
send('configure', { capture_root: root, replay_secs: 30, target: { kind: 'window', hwnd: game.hwnd, pid: game.pid, exe: game.exe, title: game.title }, quality: 'balanced', fps: 30, capture_cursor: false, audio: { game_audio: false }, save_hotkey: 'Alt+F10', buffer_cap_bytes: 2e9, min_free_bytes: 5e9 });
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(3000);
send('get_state');
const st = await waitFor(m => m.event === 'state' && m.data.in_game !== undefined);
// (in_game stays false here: a windowed game does not fill its monitor, the overlay window is used.)
void st;

const t0 = Date.now();
send('overlay_mirror', { hwnd: ov.hwnd });
await sleep(1500);
// A click in the middle of the game window, then a key, as Windows would deliver them.
const post = (msg, wp, lp) => spawnSync('powershell', ['-NoProfile', '-Command', `Add-Type -Name P -Namespace W -MemberDefinition '[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);'; [W.P]::PostMessageW([IntPtr]${game.hwnd}, ${msg}, [IntPtr]${wp}, [IntPtr]${lp}) | Out-Null`], { windowsHide: true });
const gw = Number(spawnSync('powershell', ['-NoProfile', '-Command', `Add-Type -Name R -Namespace W -MemberDefinition '[DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r); public struct RECT { public int l, t, r, b; }'; $r = New-Object W.R+RECT; [W.R]::GetClientRect([IntPtr]${game.hwnd}, [ref]$r) | Out-Null; "$($r.r) $($r.b)"`], { encoding: 'utf8', windowsHide: true }).stdout.trim().split(' ')[0]);
const gh = Number(spawnSync('powershell', ['-NoProfile', '-Command', `Add-Type -Name R -Namespace W -MemberDefinition '[DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r); public struct RECT { public int l, t, r, b; }'; $r = New-Object W.R+RECT; [W.R]::GetClientRect([IntPtr]${game.hwnd}, [ref]$r) | Out-Null; "$($r.b)"`], { encoding: 'utf8', windowsHide: true }).stdout.trim());
const lp = (Math.round(gh / 2) << 16) | Math.round(gw / 2);
post(0x0200, 0, lp); post(0x0201, 1, lp); post(0x0202, 0, lp);
await sleep(1200);
const tClick = Date.now();
post(0x0100, 0x42, 0x00300001); post(0x0101, 0x42, 0xC0300001);
await sleep(1200);
const tKey = Date.now();
await sleep(300);
const t1 = Date.now();
send('overlay_mirror', { hwnd: null });
await sleep(3000);
const r = send('save_replay');
const started = await waitFor(m => m.re === r && /save_(started|failed)/.test(m.event));
const done = await waitFor(m => /save_(finished|failed)/.test(m.event) && m.data.save_id === started.data.save_id, 60000);
send('shutdown');
await new Promise(r => proc.on('exit', r));
app.kill();
overlay.kill();
if (/cannot show the overlay/.test(log)) console.log('      ' + log.match(/in-game overlay: cannot.*/)[0]);

// Clip timeline: it ends at the save; the mirror ran from (end - (save - t0)) for ~3 s.
const end = done.data.duration_secs, saveAt = t1 + 3000;
const during = end - (saveAt - t0) / 1000 + 1.0;
const after = end - 1.0;
const before = Math.max(0.3, end - (saveAt - t0) / 1000 - 1.5);
const fb = frameAt(done.data.path, before), fd = frameAt(done.data.path, during), fa = frameAt(done.data.path, after);
check(!green(fb.centre), `before: plain game in the middle (${fb.centre})`);
check(green(fd.centre), `while open: the overlay window's picture in the middle of the game (${fd.centre})`);
// The test game slowly changes colour: compare just before and just after closing.
const closeAt = end - (saveAt - t1) / 1000;
const lastOpen = frameAt(done.data.path, closeAt - 0.4), firstClosed = frameAt(done.data.path, closeAt + 0.6);
const sum = p => p.reduce((a, b) => a + b);
check(sum(lastOpen.corner) < sum(firstClosed.corner) * 0.7, `while open: game darkened around it (${lastOpen.corner} vs ${firstClosed.corner} after closing)`);
check(!green(fa.centre), `after closing: plain game again (${fa.centre})`);
const red = p => p[0] > 150 && p[1] < 90 && p[2] < 90, blue = p => p[2] > 150 && p[0] < 90 && p[1] < 90;
const fc = frameAt(done.data.path, end - (saveAt - tClick) / 1000 - 0.2);
check(red(fc.centre), `a click on the game reached the overlay window (it turned red: ${fc.centre})`);
const fk = frameAt(done.data.path, end - (saveAt - tKey) / 1000 - 0.2);
check(blue(fk.centre), `a key press in the game reached the overlay window (it turned blue: ${fk.centre})`);
console.log(fail ? `${fail} FAILED` : 'all passed');
process.exitCode = fail ? 1 : 0;
