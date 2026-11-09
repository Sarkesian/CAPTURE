// Real DX11 recording test. Drives the test game's own stdio controls, then independently
// decodes saved footage. Usage: node scripts/e2e-resolution.mjs <worker> <root> [--exclusive]
import { spawn, execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import readline from 'node:readline';
import path from 'node:path';

const [worker, root] = process.argv.slice(2);
mkdirSync(root, { recursive: true });
const bin = path.resolve(import.meta.dirname, '../third_party/ffmpeg/bin');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const events = [], phases = [], results = [];
let id = 0, failures = 0;
const check = (ok, label) => { results.push({ ok, label }); console.log(`${ok ? 'PASS' : 'FAIL'} ${label}`); if (!ok) failures++; };
const game = spawn(path.resolve(import.meta.dirname, '../target/release/hook-testapp.exe'), ['--resolution-test', '--flip', '--secs', '240', '--title', `Resolution-${process.pid}`], { stdio: ['pipe', 'pipe', 'inherit'] });
readline.createInterface({ input: game.stdout }).on('line', l => { console.log(`GAME ${l}`); phases.push(l); });
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'pipe'] });
let diagnostics = '';
proc.stderr.on('data', b => { diagnostics += b; });
readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
const until = async (fn, ms = 30000) => { const end = Date.now() + ms; while (Date.now() < end) { const value = fn(); if (value) return value; await sleep(50); } throw Error('Timed out'); };
const wait = pred => until(() => events.find(pred));
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args === undefined ? {} : { args }) }) + '\n'); return id; };
const change = async command => {
  const n = phases.length;
  game.stdin.write(command + '\n');
  await until(() => phases.slice(n).find(l => l === `phase ${command}`));
  await sleep(2500);
};
const save = async command => {
  const re = send(command);
  const start = await wait(m => m.re === re && /save_started|save_failed/.test(m.event));
  if (start.event === 'save_failed') throw Error(start.data.message);
  const end = await wait(m => /save_finished|save_failed/.test(m.event) && m.data.save_id === start.data.save_id);
  if (end.event === 'save_failed') throw Error(end.data.message);
  return end.data;
};
function inspect(file, sw, sh, label) {
  const info = JSON.parse(execFileSync(path.join(bin, 'ffprobe.exe'), ['-v', 'error', '-show_streams', '-of', 'json', file], { encoding: 'utf8' })).streams.find(s => s.codec_type === 'video');
  const { width: w, height: h } = info;
  const raw = execFileSync(path.join(bin, 'ffmpeg.exe'), ['-v', 'error', '-sseof', '-0.3', '-i', file, '-frames:v', '1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'], { maxBuffer: 64 << 20 });
  const scale = Math.min(w/sw, h/sh), dw = sw*scale, dh = sh*scale, x = (w-dw)/2, y = (h-dh)/2;
  const pixel = (fx, fy) => { const i = (Math.floor(y+dh*fy)*w + Math.floor(x+dw*fx))*3; return [...raw.subarray(i, i+3)]; };
  const corners = [[.07,.07],[.93,.07],[.07,.93],[.93,.93]].map(([x,y]) => pixel(x,y));
  check(corners.every(([r,g,b]) => r > 180 && g < 80 && b < 80), `${label}: all four corners retained ${JSON.stringify(corners)} (${w}x${h})`);
  const centre = pixel(.25,.5);
  check(centre[2] > 180 && centre[0] < 80 && centre[1] < 80, `${label}: correct picture and aspect ratio`);
  const motion = execFileSync(path.join(bin, 'ffmpeg.exe'), ['-v', 'error', '-sseof', '-1', '-i', file, '-vf', 'crop=iw/3:ih/3:iw/3:ih/3,scale=8:8,fps=10', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'], { maxBuffer: 1 << 20 });
  const pictures = new Set();
  for (let i = 0; i + 192 <= motion.length; i += 192) pictures.add(motion.subarray(i, i + 192).toString('hex'));
  check(pictures.size >= 3, `${label}: picture keeps updating (${pictures.size} distinct samples)`);
  const absolutePixel = (x, y) => { const i = (Math.floor(y)*w + Math.floor(x))*3; return [...raw.subarray(i, i+3)]; };
  if (x > 5) check(absolutePixel(1, h/2).every(c => c < 20), `${label}: side bars contain no stale image`);
  if (y > 5) check(absolutePixel(w/2, 1).every(c => c < 20), `${label}: top/bottom bars contain no stale image`);
}
try {
  await until(() => phases.find(l => l.startsWith('ready')));
  await change('window 640 360');
  await wait(m => m.event === 'hello');
  const re = send('list_targets');
  const target = (await wait(m => m.re === re && m.event === 'targets')).data.windows.find(w => w.pid === game.pid);
  if (!target) throw Error('DX11 test game not found');
  send('configure', { capture_root: root, replay_secs: 3, target: { kind: 'window', hwnd: target.hwnd, pid: target.pid, exe: target.exe, title: target.title }, quality: 'balanced', fps: 30, capture_cursor: false, audio: { game_audio: false }, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '' }, overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 1e9 });
  send('set_replay', { enabled: true });
  await wait(m => m.event === 'state' && m.data.status === 'buffering');
  send('start_session', {});
  const cases = ['window 640 360', 'window 1280 720', 'window 800 600', 'window 1280 540', 'borderless 960 540'];
  if (process.argv.includes('--exclusive')) cases.push('exclusive 1280 720');
  cases.push('window 640 360');
  for (const command of cases) {
    await change(command);
    const clip = await save('save_replay');
    const [,w,h] = command.split(' ');
    inspect(clip.path, +w, +h, command);
  }
  await change('minimize');
  await change('restore');
  inspect((await save('save_replay')).path, 640, 360, 'minimize/restore');
  const reReview = send('prepare_review');
  const review = await wait(m => m.re === reReview && /review_ready|review_failed/.test(m.event));
  check(review.event === 'review_ready', 'review preview prepared across all changes');
  if (review.event === 'review_ready') inspect(review.data.path, 640, 360, 'review preview');
  const session = await save('stop_session');
  check(session.duration_secs > 30, `full session preserved: ${session.duration_secs.toFixed(2)}s`);
  const validation = spawnSync(process.execPath, [path.join(import.meta.dirname, 'validate-media.mjs'), root], { encoding: 'utf8' });
  console.log(validation.stdout);
  check(validation.status === 0, 'all clips and full session decode with continuous timestamps');
  const black = spawnSync(path.join(bin, 'ffmpeg.exe'), ['-hide_banner', '-nostats', '-i', session.path, '-vf', 'blackdetect=d=0.02:pix_th=0.1', '-an', '-f', 'null', '-'], { encoding: 'utf8' });
  check(black.status === 0 && !black.stderr.includes('black_start:'), 'full session has no black frames during transitions');
  check(!events.some(m => m.event === 'state' && m.data.attention), 'no capture or encoder error reported');
} catch (e) { check(false, e.stack); }
finally {
  send('shutdown');
  await Promise.race([new Promise(r => proc.once('exit', r)), sleep(5000)]);
  proc.kill(); game.kill();
  writeFileSync(path.join(root, 'resolution-results.json'), JSON.stringify({ results, phases, events }, null, 2));
  writeFileSync(path.join(root, 'worker.log'), diagnostics);
}
process.exitCode = failures ? 1 : 0;
