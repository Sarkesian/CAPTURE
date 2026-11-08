// Records a short multi-track clip (system audio + mic + voice app) and exports editor stems.
//   node scripts/e2e-stems.mjs <worker.exe> <captureRoot> <ffplay.exe>
import { spawn } from 'node:child_process';
import readline from 'node:readline';
const [worker, root, ffplay] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const events = [];
readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
let id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
const waitFor = async (pred, ms = 30000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = events.find(pred); if (h) return h; await sleep(100); } throw new Error('timeout'); };
const voice = spawn(ffplay, ['-nodisp', '-autoexit', '-loglevel', 'quiet', '-f', 'lavfi', 'sine=frequency=880:sample_rate=48000:duration=30,volume=0.3'], { stdio: 'ignore' });
await sleep(1000);
send('configure', { capture_root: root, replay_secs: 8, target: { kind: 'display', index: 0 }, quality: 'balanced', fps: 60, capture_cursor: false,
  audio: { game_audio: true, mic_enabled: true, voice_enabled: true, voice_app: 'ffplay.exe' }, save_hotkey: '', overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 5e9 });
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(10000);
send('save_replay');
const fin = await waitFor(m => m.event === 'save_finished');
console.log('clip', fin.data.duration_secs.toFixed(2), 's', fin.data.audio_tracks.join(', '));
const r = send('export_stems', { path: fin.data.path });
const st = await waitFor(m => m.re === r && m.event === 'stems_done', 60000);
console.log('stems', JSON.stringify(st.data));
console.log('CLIP=' + fin.data.path);
voice.kill();
send('shutdown');
await new Promise(r => proc.on('exit', r));
