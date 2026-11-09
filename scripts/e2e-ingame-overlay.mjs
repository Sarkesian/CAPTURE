// In-game overlay hook against a Direct3D 11 test "game" (hook-testapp), windowed or in true
// exclusive fullscreen (where no ordinary window can be shown over a game).
//
// The worker records the test app with notices on. Checks:
//   - the "Recording" notice is drawn by the hook: it is on the real screen (desktop
//     duplication, which also sees exclusive fullscreen) at the top-right corner,
//   - the game keeps running and presenting the whole time,
//   - after the session ends the hook goes dormant (it stays loaded: unloading it is not safe
//     while other overlays hook the same functions), and a second session wakes it again.
//
//   node scripts/e2e-ingame-overlay.mjs <worker.exe> <captureRoot> <hook-testapp.exe> [--fullscreen] [--flip]
//   node scripts/e2e-ingame-overlay.mjs <worker.exe> <captureRoot> attach:<game.exe> [--fullscreen]
//     (a real game you started yourself; it is left running)
import { spawn, spawnSync } from 'node:child_process';
import readline from 'node:readline';
import path from 'node:path';
import { readFileSync, mkdirSync } from 'node:fs';

const [worker, root, testapp, ...flags] = process.argv.slice(2);
const ffmpeg = path.join(import.meta.dirname, '..', 'third_party', 'ffmpeg', 'bin', 'ffmpeg.exe');
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };
mkdirSync(root, { recursive: true });
// Every helper runs hidden: a console window popping up would take focus from a fullscreen game
// (which then minimizes).
const ps = cmd => spawnSync('powershell', ['-NoProfile', '-Command', cmd], { encoding: 'utf8', windowsHide: true }).stdout.trim();

// Screen grab through desktop duplication: one frame of the primary display as raw RGB.
function grab(file) {
  const r = spawnSync(ffmpeg, ['-hide_banner', '-y', '-f', 'lavfi', '-i', 'ddagrab=output_idx=0:framerate=10,hwdownload,format=bgra,showinfo', '-frames:v', '1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', file], { encoding: 'utf8', windowsHide: true });
  const [, w, h] = r.stderr.match(/ s:(\d+)x(\d+)/) ?? [0, 0, 0];
  return { buf: readFileSync(file), w: +w, h: +h };
}
// Desktop duplication can hand back a stale frame around fullscreen transitions: a notice
// counts as on screen if any of three grabs, a quarter second apart, shows it.
async function shownOnScreen(file) {
  for (let i = 0; i < 6; i++) {
    if (noticeAt(grab(file))) return true;
    await sleep(250);
  }
  return false;
}
// One frame of a clip at `t` seconds as raw RGB.
function frameOf(clip, t, file) {
  const r = spawnSync(ffmpeg, ['-hide_banner', '-y', '-ss', String(t), '-i', clip, '-frames:v', '1', '-vf', 'showinfo', '-pix_fmt', 'rgb24', '-f', 'rawvideo', file], { encoding: 'utf8', windowsHide: true });
  const [, w, h] = r.stderr.match(/ s:(\d+)x(\d+)/) ?? [0, 0, 0];
  return { buf: readFileSync(file), w: +w, h: +h };
}
// The notice is a flat dark panel at the frame's top-right, over the test app's flat cycling
// colour. Shown = three points inside the panel match each other and clearly differ from the
// background (colour-independent, so it also holds for HDR desktop grabs).
function noticeAt(img) {
  const { buf, w, h } = img;
  if (!w) return false;
  const k = h / 1080;
  const px = (x, y) => { const i = (Math.round(y) * w + Math.round(x)) * 3; return [buf[i], buf[i + 1], buf[i + 2]]; };
  const d = (a, b) => Math.abs(a[0] - b[0]) + Math.abs(a[1] - b[1]) + Math.abs(a[2] - b[2]);
  const bg = px(w / 2, h / 2);
  const pts = [[w - 24 * k - 30 * k, 24 * k + 55 * k], [w - 24 * k - 200 * k, 24 * k + 5 * k], [w - 24 * k - 340 * k, 24 * k + 60 * k]].map(([x, y]) => px(x, y));
  return pts.every(p => d(p, bg) > 40 && d(p, pts[0]) < 15);
}

const attachTo = testapp.startsWith('attach:') ? testapp.slice(7).toLowerCase() : null;
const app = attachTo
  ? { pid: Number(ps(`(Get-Process ${attachTo.replace(/\.exe$/, '')} | Select-Object -First 1).Id`)), get exitCode() { return ps(`Get-Process -Id ${this.pid} -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id`) ? null : 0; }, kill() {} }
  : spawn(testapp, ['--secs', '120', '--title', `CAPTURE-hook-test-${process.pid}`, ...flags], { stdio: ['ignore', 'pipe', 'inherit'] });
if (!app.pid) throw new Error(`${attachTo} is not running`);
if (attachTo) {
  // Bring the game to the front (a fullscreen game sits minimized while anything else has focus).
  ps(`Add-Type -Name F -Namespace W -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c); [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);'; $h = (Get-Process -Id ${app.pid}).MainWindowHandle; [W.F]::keybd_event(0x12,0,0,[UIntPtr]::Zero); [W.F]::keybd_event(0x12,0,2,[UIntPtr]::Zero); [W.F]::ShowWindow($h, 9) | Out-Null; [W.F]::SetForegroundWindow($h) | Out-Null`);
  await sleep(3000);
}
await sleep(attachTo ? 0 : 3000);
const fullscreen = flags.includes('--fullscreen');

// One CAPTURE session on the game: the worker loads the hook (or wakes it), a save shows a
// notice, the notice must appear in the game's frames.
async function session(label) {
  const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
  let log = '';
  proc.stderr.on('data', d => { log += d; });
  const ev = [];
  readline.createInterface({ input: proc.stdout }).on('line', l => ev.push(JSON.parse(l)));
  let id = 0;
  const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args ? { args } : {}) }) + '\n'); return id; };
  const waitFor = async (p, ms = 30000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = ev.find(p); if (h) return h; await sleep(100); } throw new Error('timeout'); };

  await waitFor(m => m.event === 'hello');
  const t = send('list_targets');
  const w = (await waitFor(m => m.re === t && m.event === 'targets')).data.windows.find(w => w.pid === app.pid);
  if (!w) throw new Error('game window not found (minimized?)');
  send('configure', { capture_root: root, replay_secs: 10, target: { kind: 'window', hwnd: w.hwnd, pid: w.pid, exe: w.exe, title: w.title }, quality: 'balanced', fps: 60, capture_cursor: false, audio: { game_audio: false }, save_hotkey: 'Alt+F10', buffer_cap_bytes: 2e9, min_free_bytes: 5e9 });
  send('set_replay', { enabled: true });
  await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
  if (fullscreen) {
    // "Recording …" is published before the hook draws its first frame; it is handed over to the
    // hook, so it shows over exclusive fullscreen too.
    await sleep(1200);
    check(await shownOnScreen(path.join(root, `${label}-open.raw`)), `${label}: opening "Recording" notice shown in the game`);
  }
  await sleep(6000);
  check(/in-game overlay: loaded/.test(log), `${label}: hook in the game (${(log.match(/in-game overlay: .*/) ?? ['no log'])[0]})`);
  const r = send('save_replay');
  const st = await waitFor(m => m.re === r && /save_(started|failed)/.test(m.event));
  await sleep(700);
  if (fullscreen) {
    // One notice per save ("Clip saved"), shown the moment save is pressed; then it fades.
    check(await shownOnScreen(path.join(root, `${label}-screen.raw`)), `${label}: "Clip saved" notice drawn in the game, on screen`);
    await sleep(4000);
    check(!noticeAt(grab(path.join(root, `${label}-screen2.raw`))), `${label}: notice gone after it faded`);
    await waitFor(m => /save_(finished|failed)/.test(m.event) && m.data.save_id === st.data.save_id, 60000);
  }
  if (!fullscreen) {
    // One or the other, never both: while the hook draws the notice in the game, the notice
    // window (top-right of the screen) stays hidden.
    check(!noticeAt(grab(path.join(root, `${label}-desk.raw`))), `${label}: no second notice in the notice window`);
  }
  await waitFor(m => /save_(finished|failed)/.test(m.event) && m.data.save_id === st.data.save_id, 60000);
  if (!fullscreen) {
    // The notice is drawn into the game's own frames, which the recording captures. A replay
    // ends when save is pressed, so the first save's notice is in the next clip: save again.
    await sleep(2000);
    const r2 = send('save_replay');
    const st2 = await waitFor(m => m.re === r2 && /save_(started|failed)/.test(m.event));
    const d2 = await waitFor(m => /save_(finished|failed)/.test(m.event) && m.data.save_id === st2.data.save_id, 60000);
    const n = Math.floor(d2.data.duration_secs * 4);
    let withN = 0;
    for (let i = 0; i < n; i++) if (noticeAt(frameOf(d2.data.path, i / 4, path.join(root, 'f.raw')))) withN++;
    check(withN >= 4 && withN < n, `${label}: notice in the game's frames: ${withN} of ${n} sampled frames show it`);
  }
  send('shutdown');
  await new Promise(r => proc.on('exit', r));
  console.log('      ' + (log.match(/in-game overlay: state.*/) ?? ['(no hook summary)'])[0]);
  await sleep(1500);
  check(app.exitCode === null, `${label}: game still running after the session ended`);
}

// The hook stays in the game (dormant) after a session; the next session wakes it.
await session('session 1');
await session('session 2');
await sleep(500);
app.kill();
console.log(fail ? `${fail} FAILED` : 'all passed');
process.exitCode = fail ? 1 : 0;
