// Game frame-time impact of CAPTURE, measured without touching the game: Windows' own ETW
// events (Microsoft-Windows-DXGI Present, the same source PresentMon uses) recorded with the
// built-in logman/tracerpt. No download, no injection, no elevation (needs the "Performance Log
// Users" group or admin). Nothing in the game or its settings is changed.
//
// Phases: A = capture off, B = CAPTURE buffering the game (Auto detection, the engine as the app
// runs it), A2 = off again (shows drift). Per phase: FPS, frame-time percentiles, 1 % low, stutters
// (> 2x median), game and worker CPU, GPU and NVENC utilisation.
//
//   node scripts/frametime.mjs <worker.exe> <captureRoot> <gameExeName> <phaseSecs> <outDir> [quality=high]
import { execFileSync, spawn } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';

const [worker, root, exe, secsArg = '60', out = 'test-output/ft', quality = 'high'] = process.argv.slice(2);
const secs = Number(secsArg);
mkdirSync(out, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));
const ps = cmd => execFileSync('powershell', ['-NoProfile', '-Command', cmd], { encoding: 'utf8' }).trim();
const sh = (f, a) => { try { return execFileSync(f, a, { encoding: 'utf8' }); } catch (e) { return String(e.stdout ?? ''); } };
const DXGI = '{CA11C036-0102-4A2D-A6AD-F03CFED5D3C9}';
const gamePid = Number(ps(`(Get-Process ${exe.replace(/\.exe$/i, '')} | Sort-Object WorkingSet64 -Descending | Select-Object -First 1).Id`));
if (!gamePid) throw new Error(`${exe} is not running`);
const cores = Number(ps('(Get-CimInstance Win32_Processor | Measure-Object NumberOfLogicalProcessors -Sum).Sum'));
const cpuMs = pid => Number(ps(`(Get-Process -Id ${pid}).TotalProcessorTime.TotalMilliseconds`));

async function phase(name, workerPid) {
  const etl = path.join(out, `${name}.etl`), csv = path.join(out, `${name}.csv`);
  sh('logman', ['stop', 'capture-ft', '-ets']);
  sh('logman', ['create', 'trace', 'capture-ft', '-ets', '-o', etl, '-p', DXGI, '0xffffffffffffffff', '4', '-bs', '1024', '-nb', '64', '256']);
  const g0 = cpuMs(gamePid), w0 = workerPid ? cpuMs(workerPid) : 0, t0 = Date.now();
  const gpu = [];
  for (let t = 0; t < secs; t += 2) {
    await sleep(2000);
    const [enc, g] = sh('nvidia-smi', ['--query-gpu=utilization.encoder,utilization.gpu', '--format=csv,noheader,nounits']).split(',').map(Number);
    gpu.push({ enc, g });
  }
  const wall = Date.now() - t0, g1 = cpuMs(gamePid), w1 = workerPid ? cpuMs(workerPid) : 0;
  sh('logman', ['stop', 'capture-ft', '-ets']);
  sh('tracerpt', [etl, '-o', csv, '-of', 'CSV', '-y']);
  // Present_Start (event 42) of the game process: time between presents = frame time.
  const pidHex = '0x' + gamePid.toString(16).toUpperCase().padStart(8, '0');
  const ts = [];
  for (const line of readFileSync(csv, 'utf8').split('\n')) {
    if (!line.includes('Microsoft-Windows-DXGI')) continue;
    const f = line.split(',');
    if (f[2].trim() === '42' && f[9].trim() === pidHex) ts.push(Number(f[16].trim()));
  }
  ts.sort((a, b) => a - b);
  const ft = ts.slice(1).map((t, i) => (t - ts[i]) / 1e4); // ms
  const s = [...ft].sort((a, b) => a - b), q = p => s[Math.min(s.length - 1, Math.floor(p * s.length))];
  const mean = ft.reduce((a, b) => a + b, 0) / ft.length;
  const worst1 = s.slice(Math.floor(s.length * 0.99));
  const r = {
    phase: name, frames: ts.length, fps: ft.length / ((ts.at(-1) - ts[0]) / 1e7),
    mean_ms: mean, p50_ms: q(0.5), p95_ms: q(0.95), p99_ms: q(0.99), max_ms: s.at(-1),
    low1_fps: 1000 / (worst1.reduce((a, b) => a + b, 0) / worst1.length),
    stutters: ft.filter(x => x > 2 * q(0.5)).length,
    game_cpu_pct: ((g1 - g0) / wall) * 100 / cores, worker_cpu_pct: workerPid ? ((w1 - w0) / wall) * 100 / cores : 0,
    gpu_pct: gpu.reduce((a, b) => a + b.g, 0) / gpu.length, nvenc_pct: gpu.reduce((a, b) => a + b.enc, 0) / gpu.length,
  };
  writeFileSync(path.join(out, `${name}.frametimes.json`), JSON.stringify(ft));
  return r;
}

const results = [];
results.push(await phase('A_off', 0));

const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const events = [];
readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
let id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
const state = () => [...events].reverse().find(m => m.event === 'state')?.data;
send('configure', { capture_root: root, replay_secs: 120, target: { kind: 'auto', games: [exe], ignore: [] }, quality, fps: 60, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '', mic_mute: '' }, overlay: false,
  buffer_cap_bytes: 20e9, min_free_bytes: 10e9 });
send('set_replay', { enabled: true });
for (let i = 0; i < 300 && state()?.status !== 'buffering'; i++) await sleep(100);
const st0 = state();
if (st0?.status !== 'buffering' || st0.target_exe?.toLowerCase() !== exe.toLowerCase()) throw new Error(`not buffering the game: ${JSON.stringify(st0)}`);
await sleep(5000); // warm-up: encoder and first segments
const rec = state();
results.push({ ...(await phase('B_capture', proc.pid)), capture: `${rec.encoder?.name} ${rec.encoder?.width}x${rec.encoder?.height}@${rec.encoder?.fps}, capture ${rec.capture_fps} fps, tracks ${rec.tracks.map(t => t.name).join('+')}` });
send('shutdown');
await new Promise(r => proc.on('exit', r));
await sleep(3000);
results.push(await phase('A2_off', 0));

const f = (x, d = 2) => (Number.isFinite(x) ? x.toFixed(d) : 'n/a');
console.log(`${exe} (pid ${gamePid}), ${secs} s per phase, ${cores} logical CPUs`);
console.log('phase      frames  fps     mean   p50    p95    p99    max     1%low  stutters gameCPU workerCPU GPU   NVENC');
for (const r of results) console.log([r.phase.padEnd(10), String(r.frames).padStart(6), f(r.fps).padStart(7), f(r.mean_ms).padStart(6), f(r.p50_ms).padStart(6), f(r.p95_ms).padStart(6), f(r.p99_ms).padStart(6), f(r.max_ms).padStart(7), f(r.low1_fps, 1).padStart(6), String(r.stutters).padStart(8), (f(r.game_cpu_pct) + '%').padStart(7), (f(r.worker_cpu_pct) + '%').padStart(9), (f(r.gpu_pct, 0) + '%').padStart(5), (f(r.nvenc_pct, 0) + '%').padStart(6)].join(' '));
console.log('capture:', results[1].capture);
writeFileSync(path.join(out, 'results.json'), JSON.stringify(results, null, 2));
