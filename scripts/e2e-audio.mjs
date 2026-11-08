// Multi-source audio test against the real worker:
//   game = FAULTHEART (window), voice chat = a separate ffplay tone (880 Hz), Other Desktop on,
//   microphone = Windows default. Checks track layout, live mute without restart, voice-app
//   restart recovery, and no silent fallback for a missing microphone.
//   node scripts/e2e-audio.mjs <worker.exe> <captureRoot> <ffplay.exe>
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
const state = () => [...events].reverse().find(m => m.event === 'state')?.data;
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };
const tone = () => spawn(ffplay, ['-nodisp', '-autoexit', '-loglevel', 'quiet', '-f', 'lavfi', 'sine=frequency=880:sample_rate=48000:duration=60,volume=0.08'], { stdio: 'ignore' });

const la = send('list_audio');
const devs = (await waitFor(m => m.re === la && m.event === 'audio_devices')).data;
console.log('inputs:', devs.inputs.map(d => `${d.name}${d.is_default ? ' [default]' : ''}${d.is_virtual ? ' [virtual]' : ''}`).join(' | '));
console.log('apps playing sound:', devs.apps.map(a => a.exe).join(', ') || '(none)');

let voice = tone();
await sleep(1500);
const audio = { game_audio: true, mic_enabled: true, mic_device: null, mic_muted: false, voice_enabled: true, voice_app: 'ffplay.exe', other_enabled: true, volumes: { game: 1, mic: 1, voice: 1 } };
const cfg = { capture_root: root, replay_secs: 30, target: { kind: 'app', exe: 'FAULTHEART.exe' }, quality: 'balanced', fps: 60, capture_cursor: false,
  audio, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '' }, overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 5e9 };
send('configure', cfg);
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(4000);
let st = state();
console.log('tracks:', st.tracks.map(t => `${t.name} [${t.mode}] ${t.source}${t.error ? ' ERROR ' + t.error : ''}`).join('\n        '));
check(st.tracks.map(t => t.name).join(',') === 'Preview Mix,Game Audio,Microphone,Voice Chat,Other Desktop', 'five tracks in contract order');
const frames0 = st.frames_captured;

// Live mute: no pipeline restart (frames keep counting, same session).
send('configure', { ...cfg, audio: { ...audio, mic_muted: true } });
await sleep(1500);
st = state();
check(st.frames_captured > frames0 && st.tracks.find(t => t.name === 'Microphone').source.includes('(muted)'), 'mic muted live without restarting the recording');

// Voice app restart: track reports it, then recovers on its own.
voice.kill();
await sleep(3500);
const err = state().tracks.find(t => t.name === 'Voice Chat').error;
check(!!err, `voice app closed -> reported: ${err}`);
voice = tone();
await waitFor(m => m.event === 'state' && m.data.tracks.find(t => t.name === 'Voice Chat')?.error == null, 8000).then(() => check(true, 'voice app restarted -> recording it again'), () => check(false, 'voice app restart recovery'));
await sleep(3000);
const s = send('save_replay');
const fin = await waitFor(m => m.event === 'save_finished' || (m.event === 'save_failed' && m.re === s), 30000);
check(fin.event === 'save_finished' && fin.data.audio_tracks.length === 5, `saved with ${fin.data.audio_tracks?.join(', ')}`);
console.log('CLIP', fin.data.path);
voice.kill();

// A missing microphone must never fall back to another device.
send('configure', { ...cfg, target: { kind: 'display', index: 0 }, audio: { ...audio, voice_enabled: false, other_enabled: false, mic_device: '{0.0.1.00000000}.{00000000-dead-beef-0000-000000000000}' } });
await sleep(4000);
const mic = state().tracks.find(t => t.name === 'Microphone');
check(!!mic?.error && mic.peak === 0, `missing mic -> "${mic?.error}" (no fallback)`);
send('shutdown');
await new Promise(r => proc.on('exit', r));
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
