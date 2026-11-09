// Auto mode against whatever game is running: detection reason, frames, tracks, a saved clip.
//   node scripts/e2e-auto-game.mjs <worker.exe> <captureRoot> <waitSecs>
import { spawn } from 'node:child_process';
import readline from 'node:readline';
const [worker, root, waitArg] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const events = [];
readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
let id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
const state = () => [...events].reverse().find(m => m.event === 'state')?.data;
send('configure', { capture_root: root, replay_secs: 20, target: { kind: 'auto', games: [], ignore: [] }, quality: 'high', fps: 60, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: '', overlay: true, buffer_cap_bytes: 4e9, min_free_bytes: 5e9 });
send('set_replay', { enabled: true });
await sleep(Number(waitArg ?? 25) * 1000);
const s = state();
console.log(JSON.stringify({ status: s.status, attention: s.attention, target: s.target_name, exe: s.target_exe, detected_by: s.detected_by,
  encoder: s.encoder && `${s.encoder.name} ${s.encoder.width}x${s.encoder.height}`, fps_in: s.capture_fps, frames: s.frames_captured,
  tracks: s.tracks.map(t => `${t.name}:${t.mode}:peak=${t.peak}:signal=${t.signal_seen}`) }));
send('save_replay');
const u = Date.now() + 30000;
let fin;
while (Date.now() < u && !(fin = events.find(m => m.event === 'save_finished' || m.event === 'save_failed'))) await sleep(200);
console.log('SAVE', JSON.stringify(fin?.data));
send('shutdown');
await new Promise(r => proc.on('exit', r));
