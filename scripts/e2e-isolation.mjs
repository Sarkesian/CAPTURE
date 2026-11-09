// Audio source isolation + automatic editor audio files, with separately identifiable signals:
//   game  = ffplay.exe window playing a 440 Hz tone (target: app ffplay.exe)
//   voice = a copy of ffplay named voicechat.exe playing 880 Hz (voice chat app)
//   mic   = Windows default input; All Desktop Audio on; editor_audio_files on.
// Checks: track names, which tone each track contains, that Preview Mix holds each source once
// (not doubled by All Desktop Audio), automatic WAVs after the save, and that a microphone
// delivering pure digital silence is reported in plain language.
//   node scripts/e2e-isolation.mjs <worker.exe> <captureRoot> <ffmpegBinDir>
import { spawn, spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, statSync } from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
const [worker, root, bin] = process.argv.slice(2);
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

const ffplay = path.join(bin, 'ffplay.exe');
const voiceExe = path.join(path.dirname(worker), 'voicechat.exe');
if (!existsSync(voiceExe)) copyFileSync(ffplay, voiceExe);
const game = spawn(ffplay, ['-loglevel', 'quiet', '-window_title', 'IsolationGame', '-f', 'lavfi',
  'testsrc2=size=640x360:rate=30[out0];sine=frequency=440:sample_rate=48000,volume=0.1[out1]'], { stdio: 'ignore' });
const voice = spawn(voiceExe, ['-nodisp', '-loglevel', 'quiet', '-f', 'lavfi', 'sine=frequency=880:sample_rate=48000,volume=0.1'], { stdio: 'ignore', env: { ...process.env, PATH: `${bin};${process.env.PATH}` } });
await sleep(2500);

const cfg = { capture_root: root, replay_secs: 12, target: { kind: 'app', exe: 'ffplay.exe' }, quality: 'balanced', fps: 30, capture_cursor: false,
  audio: { game_audio: true, mic_enabled: true, mic_device: null, mic_muted: false, voice_enabled: true, voice_app: 'voicechat.exe', other_enabled: true, volumes: { game: 1, mic: 1, voice: 1 } },
  save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '' }, overlay: false, buffer_cap_bytes: 4e9, min_free_bytes: 5e9, editor_audio_files: true };
send('configure', cfg);
send('set_replay', { enabled: true });
await waitFor(m => m.event === 'state' && m.data.status === 'buffering');
await sleep(11000);
send('get_state');
await sleep(300);
const st = state();
console.log('tracks:\n  ' + st.tracks.map(t => `${t.name} [${t.mode}] peak ${t.peak}${t.error ? ' :: ' + t.error : ''}`).join('\n  '));
check(st.tracks.map(t => t.name).join(',') === 'Preview Mix,Game Audio,Microphone,Voice Chat,All Desktop Audio', 'track names and order');
const mic = st.tracks.find(t => t.name === 'Microphone');
if (mic.signal_seen) console.log('NOTE  the microphone delivered real samples (no silence warning expected)');
else check(/complete silence|muted|0%/.test(mic.error ?? ''), `silent microphone explained: "${mic.error}"`);

const s = send('save_replay');
const fin = await waitFor(m => m.event === 'save_finished' || (m.event === 'save_failed' && m.re === s), 30000);
check(fin.event === 'save_finished', `saved ${fin.data.path}`);
const stems = await waitFor(m => m.event === 'stems_done' && m.data.path === fin.data.path, 60000).catch(() => null);
check(!!stems && !stems.data.error && stems.data.files.length === 4, `editor audio files written automatically: ${stems?.data.files.map(f => path.basename(f)).join(', ')}`);
game.kill(); voice.kill();
send('shutdown');
await new Promise(r => proc.on('exit', r));

// Tone energy per track (dB, mean volume after a narrow band-pass).
const ffmpeg = path.join(bin, 'ffmpeg.exe');
const band = (input, map, f) => {
  const r = spawnSync(ffmpeg, ['-hide_banner', '-nostats', '-i', input, ...(map ? ['-map', map] : []), '-ss', '2', '-af', `bandpass=f=${f}:width_type=q:w=20,volumedetect`, '-f', 'null', '-'], { encoding: 'utf8' });
  const m = /mean_volume: (-?[\d.]+) dB/.exec(r.stderr);
  return m ? Number(m[1]) : -999;
};
const names = fin.data.audio_tracks;
const lv = {};
for (const [i, n] of names.entries()) lv[n] = { a440: band(fin.data.path, `0:a:${i}`, 440), a880: band(fin.data.path, `0:a:${i}`, 880) };
console.log('tone levels (dB):', JSON.stringify(lv));
// Present = clearly above the band-pass leakage floor of the loudest tone in the clip (absolute
// levels depend on the Windows volume, which loopback capture follows).
const loudest = Math.max(...Object.values(lv).flatMap(v => [v.a440, v.a880]));
const has = (n, f) => lv[n][`a${f}`] > loudest - 15;
check(has('Game Audio', 440) && !has('Game Audio', 880), 'Game Audio = game tone only');
check(has('Voice Chat', 880) && !has('Voice Chat', 440), 'Voice Chat = voice tone only');
check(has('All Desktop Audio', 440) && has('All Desktop Audio', 880), 'All Desktop Audio = everything (labelled as such)');
check(Math.abs(lv['Preview Mix'].a440 - lv['Game Audio'].a440) < 1.5, `Preview Mix holds the game once (mix ${lv['Preview Mix'].a440} vs track ${lv['Game Audio'].a440} dB; doubled would be +6)`);
check(Math.abs(lv['Preview Mix'].a880 - lv['Voice Chat'].a880) < 1.5, `Preview Mix holds voice chat once (mix ${lv['Preview Mix'].a880} vs track ${lv['Voice Chat'].a880} dB)`);
if (stems) {
  const g = stems.data.files.find(f => f.endsWith('GameAudio.wav'));
  const wav = band(g, null, 440);
  check(Math.abs(wav - lv['Game Audio'].a440) < 1.0, `GameAudio.wav matches the embedded track (${wav} vs ${lv['Game Audio'].a440} dB)`);
  const dur = spawnSync(path.join(bin, 'ffprobe.exe'), ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', g], { encoding: 'utf8' }).stdout.trim();
  check(Math.abs(Number(dur) - fin.data.duration_secs) < 0.1, `WAV length ${dur} s = clip ${fin.data.duration_secs.toFixed(2)} s`);
  console.log('disk: clip', (statSync(fin.data.path).size / 1e6).toFixed(1), 'MB; WAVs', (stems.data.files.reduce((a, f) => a + statSync(f).size, 0) / 1e6).toFixed(1), 'MB');
}
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
