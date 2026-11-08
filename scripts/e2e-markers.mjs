// Worker-level marker test: add a marker (chime plays), check protection state, save a clip
// containing it, and leave the clip's System Audio for an isolation check.
//   node scripts/e2e-markers.mjs <worker.exe> <captureRoot>
import { spawn } from 'node:child_process';
import readline from 'node:readline';
const [worker, root] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const proc = spawn(worker, [], { stdio: ['pipe', 'pipe', 'inherit'] });
const events = [];
readline.createInterface({ input: proc.stdout }).on('line', l => events.push(JSON.parse(l)));
let id = 0;
const send = (cmd, args) => { proc.stdin.write(JSON.stringify({ v: 1, id: ++id, cmd, ...(args !== undefined ? { args } : {}) }) + '\n'); return id; };
const waitFor = async (pred, ms = 20000) => { const u = Date.now() + ms; while (Date.now() < u) { const h = events.find(pred); if (h) return h; await sleep(100); } throw new Error('timeout'); };
const lastState = () => [...events].reverse().find(m => m.event === 'state')?.data;

send('configure', { capture_root: root, replay_secs: 20, target: { kind: 'display', index: 0 }, quality: 'balanced', fps: 60, capture_cursor: false,
  audio: { game_audio: true }, save_hotkey: 'Alt+F10', hotkeys: { marker: 'Alt+F8', overlay: 'Alt+F11', session: '', screenshot: '' },
  markers: { sound: true, sound_volume: 0.6, protect: true, protect_before_secs: 5, protect_after_secs: 3 },
  overlay: true, buffer_cap_bytes: 4e9, min_free_bytes: 5e9 });
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(4000);
const t0 = Date.now();
const r = send('add_marker', {});
const added = await waitFor(m => m.event === 'marker_added');
console.log('marker added:', JSON.stringify(added.data), `(${Date.now() - t0} ms)`);
await sleep(1000);
const st = lastState();
console.log('marker in state:', JSON.stringify(st.markers), 'now', st.now_secs);
console.log('hotkeys:', JSON.stringify(st.hotkeys.map(h => [h.action, h.combo, h.registered, h.message])));
console.log('tracks:', JSON.stringify(st.tracks.map(t => [t.name, t.mode, t.source])));
await sleep(4000);
send('save_replay');
const fin = await waitFor(m => m.event === 'save_finished' || (m.event === 'save_failed' && m.data.save_id > 0), 30000);
console.log('saved:', JSON.stringify(fin.data));
await sleep(600);
console.log('marker after save:', JSON.stringify(lastState().markers));
send('remove_marker', { id: added.data.id });
await sleep(800);
console.log('markers after remove:', JSON.stringify(lastState().markers));
send('shutdown');
await new Promise(r => proc.on('exit', r));
