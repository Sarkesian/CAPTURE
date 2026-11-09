// Worker cost while buffering a reproducible source: an ffplay test pattern window that changes
// every frame (worst case for capture + encode). Samples the worker's own CPU and memory every
// 5 s, the capture rate the engine reports, NVENC/GPU utilisation (machine-wide), then saves a
// clip and checks its frame count against its duration (dropped/duplicated frames).
//   node scripts/perf-sample.mjs <worker.exe> <captureRoot> <ffmpegBinDir> <WxH> <fps> <seconds>
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import path from 'node:path';
import readline from 'node:readline';
const [worker, root, bin, size = '1920x1080', fpsArg = '60', secsArg = '120'] = process.argv.slice(2);
const fps = Number(fpsArg), secs = Number(secsArg);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const ps = cmd => execFileSync('powershell', ['-NoProfile', '-Command', cmd], { encoding: 'utf8' }).trim();
const src = spawn(path.join(bin, 'ffplay.exe'), ['-loglevel', 'quiet', '-window_title', 'PerfPattern', '-noborder', '-f', 'lavfi', `testsrc2=size=${size}:rate=${fps}`], { stdio: 'ignore' });
await sleep(2500);
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const events = [];
readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
let id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
const waitFor = async (pred, ms = 30000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = events.find(pred); if (h) return h; await sleep(100); } throw new Error('timeout'); };
const state = () => [...events].reverse().find(m => m.event === 'state')?.data;
send('configure', { capture_root: root, replay_secs: 60, target: { kind: 'app', exe: 'ffplay.exe' }, quality: 'high', fps, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '' }, overlay: false, buffer_cap_bytes: 8e9, min_free_bytes: 5e9 });
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(5000);
const sample = () => ps(`$p = Get-Process -Id ${proc.pid}; "$($p.TotalProcessorTime.TotalMilliseconds) $($p.WorkingSet64) $($p.PrivateMemorySize64)"`).split(' ').map(Number);
const gpu = () => ps('nvidia-smi --query-gpu=utilization.encoder,utilization.gpu --format=csv,noheader,nounits').split(',').map(Number);
const cores = Number(ps('(Get-CimInstance Win32_Processor | Measure-Object NumberOfLogicalProcessors -Sum).Sum'));
let [t0] = sample(), w0 = Date.now();
const rows = [];
for (let t = 0; t < secs; t += 5) {
  await sleep(5000);
  const [cpu, ws, priv] = sample(), now = Date.now();
  const st = state(), [enc, g] = gpu();
  rows.push({ cpuPct: ((cpu - t0) / (now - w0)) * 100 / cores, wsMB: ws / 1e6, privMB: priv / 1e6, captureFps: st.capture_fps, enc, gpu: g, status: st.status });
  t0 = cpu; w0 = now;
}
const s = send('save_replay');
const fin = await waitFor(m => m.event === 'save_finished' || (m.event === 'save_failed' && m.re === s), 60000);
const st = state();
send('shutdown');
await new Promise(r => proc.on('exit', r));
src.kill();
const avg = k => rows.reduce((a, r) => a + r[k], 0) / rows.length;
const max = k => Math.max(...rows.map(r => r[k]));
console.log(`source ${size}@${fps}, ${secs}s, encoder ${st.encoder?.name ?? '?'} ${st.encoder ? `${st.encoder.width}x${st.encoder.height}@${st.encoder.fps}` : ''}, ${cores} logical CPUs`);
console.log(`worker CPU avg ${avg('cpuPct').toFixed(2)} % of machine (max ${max('cpuPct').toFixed(2)})`);
console.log(`worker memory: working set ${rows[0].wsMB.toFixed(0)} -> ${rows.at(-1).wsMB.toFixed(0)} MB (max ${max('wsMB').toFixed(0)}), private ${rows[0].privMB.toFixed(0)} -> ${rows.at(-1).privMB.toFixed(0)} MB`);
console.log(`capture rate from Windows: avg ${avg('captureFps').toFixed(1)} fps (min ${Math.min(...rows.map(r => r.captureFps)).toFixed(1)})`);
console.log(`machine-wide NVENC ${avg('enc').toFixed(0)} % / GPU ${avg('gpu').toFixed(0)} % (includes anything else running)`);
if (fin.event === 'save_finished') {
  const p = spawnSync(path.join(bin, 'ffprobe.exe'), ['-v', 'error', '-select_streams', 'v:0', '-count_packets', '-show_entries', 'stream=nb_read_packets,r_frame_rate:format=duration', '-of', 'json', fin.data.path], { encoding: 'utf8' });
  const j = JSON.parse(p.stdout), n = Number(j.streams[0].nb_read_packets), d = Number(j.format.duration);
  console.log(`clip ${d.toFixed(2)} s, ${n} video frames = ${(n / d).toFixed(2)} fps (target ${fps}); expected ${Math.round(d * fps)} frames`);
} else console.log('save failed', fin.data.message);
console.log('rows', JSON.stringify(rows.map(r => [r.cpuPct.toFixed(2), r.wsMB.toFixed(0), r.captureFps, r.enc, r.gpu])));
