// Live check of the recording overhaul: the real capture-worker records this PC's display for
// about a minute into a disposable folder (never the installed app or its recordings).
//
//   node scripts/e2e-live-overhaul.mjs [path\to\capture-worker.exe] [display index]
//
// What it proves on real captured footage:
// - recording is not interrupted by browsing, playback windows, thumbnails or clip saves;
// - Instant Replay gives the configured length when that much exists and explains a shorter clip;
// - a pipeline restart (what a resolution change causes) continues the same session, and
//   Instant Replay still reaches back over it instead of saving a few-second fragment;
// - saved clips decode completely and have the promised length.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import assert from 'node:assert/strict';

const repo = path.resolve(import.meta.dirname, '..');
const workerExe = process.argv[2] ?? path.join(repo, 'target/debug/capture-worker.exe');
const display = Number(process.argv[3] ?? 0);
const ffmpeg = path.join(repo, 'third_party/ffmpeg/bin/ffmpeg.exe');
const ffprobe = path.join(repo, 'third_party/ffmpeg/bin/ffprobe.exe');
const root = path.join(repo, 'test-output', `overhaul-live-${Date.now()}`);
fs.mkdirSync(root, { recursive: true });
const REPLAY = 20;
const messages = [];
let serial = 0, stderr = '';
const worker = spawn(workerExe, [], { env: { ...process.env, PATH: path.dirname(ffmpeg) + ';' + process.env.PATH, CAPTURE_IN_GAME_OVERLAY: '0' }, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
worker.stderr.on('data', d => stderr += d);
readline.createInterface({ input: worker.stdout }).on('line', s => messages.push(JSON.parse(s)));
const send = (cmd, args) => { const id = ++serial; worker.stdin.write(JSON.stringify({ v: 1, id, cmd, ...(args === undefined ? {} : { args }) }) + '\n'); return id; };
const sleep = ms => new Promise(r => setTimeout(r, ms));
async function wait(pred, timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const m = messages.findLast(pred); if (m) return m; await sleep(10); }
  throw Error('timeout\n' + stderr.split('\n').slice(-12).join('\n'));
}
const reply = (id, ...events) => wait(m => m.re === id && events.includes(m.event));
const state = () => messages.findLast(m => m.event === 'state')?.data;
const history = async () => (await reply(send('history'), 'history_ready')).data;
const config = fps => ({ capture_root: root, replay_secs: REPLAY, history_secs: 7200, target: { kind: 'display', index: display }, quality: 'balanced', fps, capture_cursor: true, audio: { game_audio: false }, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '', mic_mute: '' }, overlay: false, buffer_cap_bytes: 8e9, min_free_bytes: 2e9 });
const replay = async () => {
  const id = send('save_replay');
  const started = await reply(id, 'save_started', 'save_failed');
  if (started.event === 'save_failed') return { failed: started.data.message };
  const done = await wait(m => (m.event === 'save_finished' || m.event === 'save_failed') && m.data.save_id === started.data.save_id, 40000);
  return { note: started.data.note ?? null, clip: done.event === 'save_finished' ? done.data : null, failed: done.event === 'save_failed' ? done.data.message : null };
};
const results = { root, checks: [] };
const pass = (name, detail) => { results.checks.push({ name, ...detail }); console.log('PASS', name, detail ? JSON.stringify(detail) : ''); };
const duration = file => Number(JSON.parse(spawnSync(ffprobe, ['-v', 'error', '-show_format', '-of', 'json', file], { encoding: 'utf8' }).stdout).format.duration);
const decodes = file => spawnSync(ffmpeg, ['-v', 'error', '-i', file, '-f', 'null', '-'], { encoding: 'utf8' });

try {
  await wait(m => m.event === 'hello');
  send('configure', config(60));
  send('set_replay', { enabled: true });
  await wait(m => m.event === 'state' && m.data.status === 'buffering', 20000);
  const began = Date.now();
  const encoder = state().encoder;

  // --- A clip asked for before 20 s exist explains itself ------------------------------------
  await wait(m => m.event === 'state' && m.data.buffered_secs >= 6);
  const young = await replay();
  assert.ok(young.clip, young.failed);
  assert.match(young.note ?? '', /all that has been recorded/);
  assert.ok(young.clip.duration_secs >= 5.5 && young.clip.duration_secs < REPLAY, `short clip of what exists (${young.clip.duration_secs})`);
  pass('short replay is explained', { secs: +young.clip.duration_secs.toFixed(2), note: young.note });

  // --- Reviewing while recording ---------------------------------------------------------------
  let session = (await history()).sessions.at(-1);
  const before = state().frames_encoded;
  let reviews = 0, thumbs = 0, worstReview = 0, worstThumb = 0;
  for (let n = 0; n < 24; n++) {
    session = (await history()).sessions.at(-1);
    const [a, b] = session.intervals.at(-1);
    const at = a + (b - a - .5) * ((n * 37) % 100) / 100;
    let t = performance.now();
    const review = await reply(send('history_review', { session: session.id, at, viewer: n % 2 ? 'overlay' : 'main', prefetch: false }), 'review_ready', 'review_failed');
    assert.equal(review.event, 'review_ready', JSON.stringify(review.data));
    assert.ok(review.data.start <= at + .001 && review.data.end > at, 'the window holds the asked-for moment');
    worstReview = Math.max(worstReview, performance.now() - t); reviews++;
    t = performance.now();
    const thumb = await reply(send('history_thumb', { session: session.id, at, exact: n % 3 === 0, viewer: 'main' }), 'history_thumb');
    assert.ok(thumb.data.image, JSON.stringify(thumb.data).slice(0, 200));
    assert.ok(thumb.data.t <= at + .02 && at - thumb.data.t < 1.1, `thumbnail time ${thumb.data.t} for ${at}`);
    worstThumb = Math.max(worstThumb, performance.now() - t); thumbs++;
    await sleep(350);
  }
  const during = state();
  assert.ok(during.frames_encoded - before > 400, 'frames kept being encoded while footage was reviewed');
  assert.equal(during.status, 'buffering');
  session = (await history()).sessions.at(-1);
  assert.equal(session.intervals.length, 1, 'one uninterrupted stretch: ' + JSON.stringify(session.intervals));
  assert.ok(!session.ended);
  pass('recording continues while reviewing', { reviews, thumbs, worst_review_ms: Math.round(worstReview), worst_thumb_ms: Math.round(worstThumb), frames_encoded_meanwhile: during.frames_encoded - before, frames_dropped: during.frames_dropped, recorded_secs: +session.intervals[0][1].toFixed(1) });

  // --- Clipping from the viewer while recording ----------------------------------------------
  const frames = (await reply(send('history_frames', { session: session.id, at: 8 }), 'history_frames')).data.frames;
  const tail = (await reply(send('history_frames', { session: session.id, at: 12.4 }), 'history_frames')).data.frames;
  const a = frames.findLast(f => f.key && f.start <= 8.0001).start, b = tail.find(f => f.end >= 12.4).end;
  const trimId = send('history_save', { session: session.id, start: a, end: b, aligned: true });
  const trim = await reply(trimId, 'save_finished', 'save_failed');
  assert.equal(trim.event, 'save_finished', JSON.stringify(trim.data));
  assert.ok(Math.abs(duration(trim.data.path) - (b - a)) < .03, `clip is the selection (${duration(trim.data.path)} for ${b - a})`);
  assert.equal(decodes(trim.data.path).status, 0);
  pass('clip from the viewer while recording', { selected_secs: +(b - a).toFixed(3), saved_secs: +duration(trim.data.path).toFixed(3) });

  // --- Full-length replay ----------------------------------------------------------------------
  await sleep(Math.max(0, (REPLAY + 4) * 1000 - (Date.now() - began)));
  const full = await replay();
  assert.ok(full.clip, full.failed);
  assert.equal(full.note, null);
  assert.ok(full.clip.duration_secs >= REPLAY && full.clip.duration_secs <= REPLAY + 2.2, `the configured ${REPLAY} s (${full.clip.duration_secs})`);
  assert.equal(decodes(full.clip.path).status, 0);
  pass('instant replay honours its length', { asked: REPLAY, secs: +full.clip.duration_secs.toFixed(2), bytes: full.clip.bytes });

  // --- The capture restarts (as it does when a game changes resolution) ---------------------
  send('configure', config(30));
  await wait(m => m.event === 'state' && m.data.status === 'buffering' && m.data.encoder?.fps === 30, 20000);
  await sleep(4500);
  const ready = state().buffered_secs;
  const after = await replay();
  assert.ok(after.clip, after.failed);
  assert.ok(after.clip.duration_secs >= 9, `reaches back over the restart instead of saving the few seconds since (${after.clip.duration_secs})`);
  assert.match(after.note ?? '', /interrupted/);
  assert.ok(ready >= 9, `the app shows what Save Replay will really save (${ready})`);
  assert.equal(decodes(after.clip.path).status, 0);
  const joined = (await history()).sessions;
  assert.equal(joined.length, 1, 'still one session');
  assert.equal(joined[0].parts.length, 2);
  assert.equal(joined[0].intervals.length, 2);
  const break_secs = joined[0].intervals[1][0] - joined[0].intervals[0][1];
  assert.ok(break_secs > 0 && break_secs < 15, `the restart is a short visible gap (${break_secs})`);
  pass('a restart continues the session', { parts: 2, gap_secs: +break_secs.toFixed(2), replay_secs: +after.clip.duration_secs.toFixed(2), shown_ready_secs: ready, note: after.note });

  // --- Recording off: history stays, nothing is saved by accident ----------------------------
  send('set_replay', { enabled: false });
  await wait(m => m.event === 'state' && m.data.status === 'ready', 20000);
  const off = await replay();
  assert.match(off.failed ?? '', /off/);
  const kept = (await history()).sessions[0];
  assert.ok(kept.ended && kept.intervals.length === 2);
  const clips = fs.readdirSync(path.join(root, 'Desktop', 'Clips')).filter(n => n.endsWith('.mp4'));
  assert.equal(clips.length, 4, 'exactly the four clips asked for: ' + clips.join(', '));
  pass('recording off', { sessions: 1, clips: clips.length, history_secs: +kept.intervals.reduce((n, [x, y]) => n + y - x, 0).toFixed(1) });

  send('shutdown');
  await new Promise(r => worker.on('exit', r));
  results.encoder = encoder;
  results.passed = true;
  fs.writeFileSync(path.join(root, 'results.json'), JSON.stringify(results, null, 2));
  console.log('ALL PASSED', root, JSON.stringify({ encoder: encoder?.name, size: `${encoder?.width}x${encoder?.height}` }));
} catch (e) {
  console.error(stderr.split('\n').slice(-15).join('\n'));
  throw e;
} finally {
  if (worker.exitCode === null) { worker.stdin.end(); await new Promise(r => { worker.once('exit', r); setTimeout(() => { worker.kill(); r(); }, 8000).unref(); }); }
}
