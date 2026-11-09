// Recorded audio level must equal what the output device actually plays, on any device.
//
// audio_spike renders a quiet 1 kHz tone (-40 dBFS, barely audible) as a genuine 7.1 stream on
// chosen speakers: front pair, centre, side left, back right. It runs as the child of a small
// window, which the worker records as the game (game audio includes child processes). Meanwhile
// audio_spike captures that app's audio in the output device's own layout, with no conversion:
// what each speaker channel of this PC receives from it. The recording's left/right peaks must
// equal the standard stereo downmix of that (front L/R as is, centre and surrounds at -3 dB,
// LFE out) within 1 dB. On a stereo device that means "as loud as it plays"; on a 5.1/7.1
// device it also checks the channel mapping on the real Windows path. (Windows' own
// surround-to-stereo conversion, used before 0.5.3, was ~14 dB too quiet on 7.1 devices.)
//
//   node scripts/e2e-audio-level.mjs <worker.exe> <captureRoot> <audio_spike.exe>
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import readline from 'node:readline';
import path from 'node:path';

const [worker, root, spike] = process.argv.slice(2);
const ffmpeg = path.join(import.meta.dirname, '..', 'third_party', 'ffmpeg', 'bin', 'ffmpeg.exe');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const db = x => 20 * Math.log10(Math.max(x, 1e-9));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };

// 7.1 speaker masks to play the tone on.
const signals = [
  { name: 'stereo (front pair)', speakers: '0x3' },
  { name: 'centre only', speakers: '0x4' },
  { name: 'side left only', speakers: '0x200' },
  { name: 'back right only', speakers: '0x20' },
];

// Standard downmix gains per speaker bit (same table as the engine).
const K = Math.SQRT1_2;
function gains(bit) {
  if (bit === 0x1 || bit === 0x40) return [1, 0];
  if (bit === 0x2 || bit === 0x80) return [0, 1];
  if (bit === 0x4) return [K, K];
  if (bit === 0x8) return [0, 0];
  if ([0x10, 0x200, 0x1000, 0x8000].includes(bit)) return [K, 0];
  if ([0x20, 0x400, 0x4000, 0x20000].includes(bit)) return [0, K];
  return [0.5, 0.5];
}
function expected({ channels, mask, peaks }) {
  if (channels <= 2) return [peaks[0], peaks[1] ?? peaks[0]];
  const bits = [];
  for (let b = 0; b < 32 && bits.length < channels; b++) if (mask & (1 << b)) bits.push(1 << b);
  let l = 0, r = 0;
  peaks.forEach((p, i) => { const [gl, gr] = gains(bits[i] ?? 0); l += p * gl; r += p * gr; });
  return [l, r];
}

async function run(sig) {
  const title = `CAPTURE-level-${process.pid}`;
  // A window whose process starts the player as its child, so recording the window's app records it.
  const player = spawn('powershell', ['-NoProfile', '-Command', [
    'Add-Type -AssemblyName System.Windows.Forms',
    `$f = New-Object Windows.Forms.Form; $f.Text = '${title}'; $f.Width = 480; $f.Height = 270`,
    `$f.Add_Shown({ Start-Process -NoNewWindow -FilePath '${spike}' -ArgumentList 'play','40','${sig.speakers}','0.01' })`,
    '$t = New-Object Windows.Forms.Timer; $t.Interval = 42000; $t.Add_Tick({ $f.Close() }); $t.Start()',
    '[Windows.Forms.Application]::Run($f)'].join('; ')], { stdio: 'ignore' });
  await sleep(3500);
  const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'ignore'] });
  const ev = [];
  readline.createInterface({ input: proc.stdout }).on('line', l => ev.push(JSON.parse(l)));
  let id = 0;
  const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args ? { args } : {}) }) + '\n'); return id; };
  const waitFor = async (p, ms = 60000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = ev.find(p); if (h) return h; await sleep(100); } throw new Error('timeout'); };
  try {
    await waitFor(m => m.event === 'hello');
    const t = send('list_targets');
    const w = (await waitFor(m => m.re === t && m.event === 'targets')).data.windows.find(w => w.title === title);
    if (!w) throw new Error('test window not found');
    send('configure', { capture_root: root, replay_secs: 8, target: { kind: 'window', hwnd: w.hwnd, pid: w.pid, exe: w.exe, title: w.title }, quality: 'balanced', fps: 30, capture_cursor: false, audio: { game_audio: true }, save_hotkey: 'Alt+F10', buffer_cap_bytes: 2e9, min_free_bytes: 5e9 });
    send('set_replay', { enabled: true });
    await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
    // What the speakers receive from the player (only it: other apps may be playing), in the
    // device's own channel layout, measured while the recording runs.
    const device = JSON.parse(execFileSync(spike, ['native', '4', String(w.pid)], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim().split('\n').at(-1));
    await sleep(6000);
    const r = send('save_replay');
    const st = await waitFor(m => m.re === r && /save_(started|failed)/.test(m.event));
    const done = await waitFor(m => /save_(finished|failed)/.test(m.event) && m.data.save_id === st.data.save_id);
    if (done.event === 'save_failed') throw new Error(done.data.message);
    const [el, er] = expected(device);
    for (const [i, name] of done.data.audio_tracks.entries()) {
      const o = spawnSync(ffmpeg, ['-hide_banner', '-nostats', '-i', done.data.path, '-map', `0:a:${i}`, '-ss', '1', '-af', 'astats=measure_perchannel=Peak_level:measure_overall=none', '-f', 'null', '-'], { encoding: 'utf8' }).stderr;
      const got = [...String(o).matchAll(/Peak level dB: (-?[\d.]+|-inf)/g)].map(m => Number(m[1] === '-inf' ? -200 : m[1]));
      const side = (want, have, ch) => {
        if (want < 1e-5) return check(have < db(el + er) - 30, `${sig.name} / ${name} ${ch}: silent as on the device (${have.toFixed(1)} dB)`);
        check(Math.abs(have - db(want)) <= 1, `${sig.name} / ${name} ${ch}: ${have.toFixed(1)} dB, device downmix ${db(want).toFixed(1)} dB`);
      };
      side(el, got[0], 'L');
      side(er, got[1], 'R');
    }
    console.log(`      device: ${device.channels} ch, mask 0x${device.mask.toString(16)}, peaks ${device.peaks.map(p => db(p).toFixed(1)).join(' ')}`);
  } finally {
    send('shutdown');
    await Promise.race([new Promise(r => proc.on('exit', r)), sleep(5000)]);
    proc.kill();
    spawnSync('taskkill', ['/T', '/F', '/PID', String(player.pid)]);
  }
}

for (const s of signals) await run(s);
console.log(fail ? `${fail} FAILED` : 'all passed');
process.exitCode = fail ? 1 : 0;
