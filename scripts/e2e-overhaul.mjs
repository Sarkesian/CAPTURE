// Recording-history checks against the real capture-worker, on disposable prerecorded fixtures
// (never starts a capture, never touches the installed app or the user's recordings).
//
//   node scripts/e2e-overhaul.mjs [path\to\capture-worker.exe]
//
// Covers the verification list of the Capture overhaul: one timeline per session with real
// gaps, frame thumbnails that match the footage, rapid seeking without cache errors, clips
// that are the original encoded footage (also across internal segments), no duplicate saves,
// restart persistence, and deleting temporary footage without touching saved media.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';

const repo = path.resolve(import.meta.dirname, '..');
const workerExe = process.argv[2] ?? path.join(repo, 'target/debug/capture-worker.exe');
const ffmpeg = path.join(repo, 'third_party/ffmpeg/bin/ffmpeg.exe');
const ffprobe = path.join(repo, 'third_party/ffmpeg/bin/ffprobe.exe');
const root = path.join(repo, 'test-output', `overhaul-e2e-${Date.now()}`);
const buffer = path.join(root, 'Background Recording');
fs.mkdirSync(buffer, { recursive: true });
const run = (exe, args, binary = false) => {
  const r = spawnSync(exe, args, { encoding: binary ? 'buffer' : 'utf8', maxBuffer: 64 * 1024 * 1024, windowsHide: true });
  assert.equal(r.status, 0, String(r.stderr));
  return r.stdout;
};
const probe = p => JSON.parse(run(ffprobe, ['-v', 'error', '-show_format', '-show_streams', '-of', 'json', p]));

// One evening with "Fixture": 40 s of play, the game changes resolution and records 6 s more
// (a second internal recording 41 s into the session), then it is played again 30 minutes later.
const T0 = Date.UTC(2026, 10, 10, 20, 0, 0);
const recordings = [
  { dir: '2026-11-10_20-00-00_1', at: 0, secs: 40, size: '320x180', pattern: 'testsrc2' },
  { dir: '2026-11-10_20-00-41_1', at: 41, secs: 6, size: '160x90', pattern: 'smptebars' },
  { dir: '2026-11-10_20-30-41_1', at: 1841, secs: 10, size: '320x180', pattern: 'testsrc' },
];
for (const r of recordings) {
  const dir = path.join(buffer, r.dir);
  fs.mkdirSync(dir);
  const [width, height] = r.size.split('x').map(Number);
  fs.writeFileSync(path.join(dir, 'session.json'), JSON.stringify({ capture_session: 1, started_unix_ms: T0 + r.at * 1000, game: 'Fixture', audio_tracks: ['Preview Mix'], video: { encoder: 'h264', width, height, fps: 30 }, segment_container: 'mpegts' }));
  fs.writeFileSync(path.join(dir, 'ended.json'), '{}');
  run(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', `${r.pattern}=size=${r.size}:rate=30`, '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000', '-t', String(r.secs),
    '-vf', 'scale=out_color_matrix=bt709:out_range=tv,format=yuv420p', '-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv',
    '-c:v', 'h264_nvenc', '-preset', 'p1', '-g', '30', '-bf', '0', '-c:a', 'aac', '-b:a', '192k', '-muxdelay', '0', '-muxpreload', '0', '-f', 'segment', '-segment_time', '2', '-segment_format', 'mpegts', path.join(dir, 'seg_%06d.ts')]);
}
fs.writeFileSync(path.join(buffer, recordings[1].dir, 'markers.json'), JSON.stringify([{ id: 1, t: 2, at: '20:00:43', label: 'After the switch', footage: 'buffered', note: null, saved_in: [] }]));

const messages = [];
let serial = 0, stderr = '';
const launch = () => {
  const p = spawn(workerExe, [], { env: { ...process.env, PATH: path.dirname(ffmpeg) + ';' + process.env.PATH, CAPTURE_IN_GAME_OVERLAY: '0' }, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
  p.stderr.on('data', d => stderr += d);
  readline.createInterface({ input: p.stdout }).on('line', s => messages.push(JSON.parse(s)));
  return p;
};
let worker = launch();
const send = (cmd, args) => { const id = ++serial; worker.stdin.write(JSON.stringify({ v: 1, id, cmd, ...(args === undefined ? {} : { args }) }) + '\n'); return id; };
const sleep = ms => new Promise(r => setTimeout(r, ms));
async function wait(pred, timeout = 20000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const m = messages.find(pred); if (m) return m; await sleep(5); }
  throw Error('timeout\n' + stderr);
}
const reply = (id, ...events) => wait(m => m.re === id && events.includes(m.event));
const config = { capture_root: root, replay_secs: 120, history_secs: 7200, target: { kind: 'none' }, quality: 'high', fps: 60, capture_cursor: false, audio: { game_audio: false }, save_hotkey: '', hotkeys: { marker: '', overlay: '', session: '', screenshot: '', mic_mute: '' }, overlay: false, buffer_cap_bytes: 20e9, min_free_bytes: 1e9 };
async function history() {
  let state;
  for (let n = 0; n < 200; n++) {
    state = (await reply(send('history'), 'history_ready')).data;
    if (state.sessions.length && !state.reason?.includes('Recovering')) break;
    await sleep(50);
  }
  return state;
}
const results = { root, checks: [] };
const pass = (name, detail) => { results.checks.push({ name, ...detail }); console.log('PASS', name, detail ? JSON.stringify(detail) : ''); };
const near = (a, b, tolerance) => Math.abs(a - b) <= tolerance;

try {
  await wait(m => m.event === 'hello');
  send('configure', config);

  // --- Timeline overview ---------------------------------------------------------------------
  const state = await history();
  assert.equal(state.sessions.length, 2, 'two gameplay sessions, not three unrelated recordings');
  const [first, second] = state.sessions;
  assert.equal(first.parts.length, 2);
  assert.equal(first.created_ms, T0);
  assert.equal(first.intervals.length, 2);
  assert.ok(near(first.intervals[0][0], 0, .05) && near(first.intervals[0][1], 40, .1), JSON.stringify(first.intervals));
  assert.ok(near(first.intervals[1][0], 41, .05) && near(first.intervals[1][1], 47, .1), JSON.stringify(first.intervals));
  assert.ok(first.intervals[1][0] - first.intervals[0][1] > .8, 'the second in which nothing was recorded is a visible gap');
  assert.deepEqual([first.parts[0].width, first.parts[1].width], [320, 160]);
  assert.ok(near(second.intervals[0][1] - second.intervals[0][0], 10, .1));
  const marker = first.events.find(e => e.kind === 'marker');
  assert.ok(near(marker.t, 43, .01), 'a marker in the second recording sits at its session time');
  pass('timeline overview', { sessions: 2, parts: first.parts.length, intervals: first.intervals.map(i => i.map(n => +n.toFixed(2))), marker_t: marker.t });

  // --- Hover thumbnails ----------------------------------------------------------------------
  const thumb = async (session, at, exact, viewer = 'test') => (await reply(send('history_thumb', { session, at, exact, viewer }), 'history_thumb')).data;
  const gray = (input, extra = []) => run(ffmpeg, ['-v', 'error', '-i', input, ...extra, '-frames:v', '1', ...(extra.length ? [] : ['-vf', 'scale=64:36,format=rgb24,format=gray']), '-f', 'rawvideo', 'pipe:1'], true);
  const distance = (a, b) => { assert.equal(a.length, 64 * 36, 'thumbnail decoded'); assert.equal(b.length, 64 * 36, 'source frame decoded'); let n = 0; for (let i = 0; i < a.length; i++) n += Math.abs(a[i] - b[i]); return n / a.length; };
  const jpegGray = data => { const file = path.join(root, `thumb-${data.t.toFixed(3)}.jpg`); fs.writeFileSync(file, Buffer.from(data.image.split(',')[1], 'base64')); return gray(file); };
  // The source frame at session time t, straight from the segment with ffmpeg (independent of CAPTURE).
  // The source frame at session time t, straight from the segment with ffmpeg (independent of
  // CAPTURE), picked by frame number so the reference itself is frame-exact.
  const sourceGray = (rec, t) => {
    const local = t - rec.at - first.intervals[0][0], index = Math.floor(local / 2 + 1e-6);
    return gray(path.join(buffer, rec.dir, `seg_${String(index).padStart(6, '0')}.ts`), ['-vf', `trim=start_frame=${Math.round((local - index * 2) * 30)},scale=64:36,format=rgb24,format=gray`]);
  };
  let started = performance.now();
  const key = await thumb(first.id, 10.5, false);
  results.thumb_keyframe_ms = +(performance.now() - started).toFixed(1);
  assert.ok(key.image?.startsWith('data:image/jpeg;base64,'), JSON.stringify(key).slice(0, 200));
  assert.ok(key.t <= 10.5 && key.t >= 9.4, `keyframe at or just before the cursor (${key.t})`);
  started = performance.now();
  const exact = await thumb(first.id, 10.5, true);
  results.thumb_exact_ms = +(performance.now() - started).toFixed(1);
  assert.ok(near(exact.t, 10.5, 1 / 30 + .001), `the very frame (${exact.t})`);
  const right = distance(jpegGray(exact), sourceGray(recordings[0], exact.t));
  const wrong = distance(jpegGray(exact), sourceGray(recordings[0], exact.t - 1.3));
  assert.ok(right < 6 && wrong > right * 2.5, `picture matches its position (difference ${right.toFixed(2)} at the position, ${wrong.toFixed(2)} 1.3 s away)`);
  const afterSwitch = await thumb(first.id, 43.2, true);
  assert.ok(near(afterSwitch.t, 43.2, 1 / 30 + .001));
  assert.ok(distance(jpegGray(afterSwitch), sourceGray(recordings[1], afterSwitch.t)) < 6, 'thumbnails follow the session across its internal recordings');
  const inGap = await thumb(first.id, 40.5, false);
  assert.ok(!inGap.image && inGap.error, 'no picture where nothing was recorded');
  // A burst (fast cursor movement): every request is answered, outdated ones as superseded,
  // and the last one is the picture for the last position.
  const burst = Array.from({ length: 40 }, (_, i) => send('history_thumb', { session: first.id, at: 1 + i * .9, exact: false, viewer: 'hover' }));
  const answers = await Promise.all(burst.map(id => reply(id, 'history_thumb')));
  assert.ok(answers.at(-1).data.image && near(answers.at(-1).data.t, 36.1, 1.05));
  assert.ok(answers.every(a => a.data.image || a.data.error === 'superseded'));
  started = performance.now();
  await thumb(first.id, 10.5, false);
  results.thumb_cached_ms = +(performance.now() - started).toFixed(1);
  pass('hover thumbnails', { keyframe_ms: results.thumb_keyframe_ms, exact_ms: results.thumb_exact_ms, cached_ms: results.thumb_cached_ms, match: +right.toFixed(2), mismatch: +wrong.toFixed(2), superseded_in_burst: answers.filter(a => !a.data.image).length });

  // --- Seeking -------------------------------------------------------------------------------
  const review = async (session, at, viewer = 'main', prefetch = false) => reply(send('history_review', { session, at, viewer, prefetch }), 'review_ready', 'review_failed');
  started = performance.now();
  const opened = await review(first.id, 12.3);
  results.open_ms = +(performance.now() - started).toFixed(1);
  assert.equal(opened.event, 'review_ready', JSON.stringify(opened));
  assert.ok(opened.data.start <= 12.3 && opened.data.end > 12.3 && opened.data.session === first.id);
  const window = probe(opened.data.path);
  assert.ok(near(Number(window.format.duration), opened.data.end - opened.data.start, .1), 'window times agree with the media in it');
  // Rapid scrubbing from two views at once, with prefetches in between.
  const ids = [];
  let seed = 7;
  const random = () => (seed = seed * 16807 % 2147483647) / 2147483647;
  for (let i = 0; i < 120; i++) {
    const viewer = i % 3 === 2 ? 'overlay' : 'main';
    const at = i % 5 === 4 ? 41 + random() * 5.5 : random() * 39.5;
    ids.push([send('history_review', { session: first.id, at, viewer, prefetch: i % 7 === 6 }), at, viewer]);
    if (i % 10 === 9) await sleep(15);
  }
  const replies = await Promise.all(ids.map(([id]) => reply(id, 'review_ready', 'review_failed')));
  const failures = replies.filter(r => r.event === 'review_failed' && !r.data.superseded);
  assert.deepEqual(failures, [], 'no seek fails; outdated ones are superseded');
  assert.ok(!JSON.stringify(replies).includes('busy'));
  for (const viewer of ['main', 'overlay']) {
    const index = ids.findLastIndex(x => x[2] === viewer);
    assert.equal(replies[index].event, 'review_ready', `the newest request of ${viewer} is always served`);
    assert.ok(replies[index].data.start <= ids[index][1] + .001 && replies[index].data.end > ids[index][1]);
  }
  // Reopening is served from the cache; a seek into the gap is refused in plain words.
  started = performance.now();
  const again = await review(first.id, 12.3);
  results.reopen_ms = +(performance.now() - started).toFixed(1);
  assert.equal(again.event, 'review_ready');
  const gap = await review(first.id, 40.5);
  assert.equal(gap.event, 'review_failed');
  assert.match(gap.data.message, /No footage/);
  const afterSeeks = (await reply(send('history'), 'history_ready')).data;
  assert.ok(afterSeeks.preview_bytes > 0);
  pass('seeking', { open_ms: results.open_ms, reopen_ms: results.reopen_ms, requests: ids.length, served: replies.filter(r => r.event === 'review_ready').length, superseded: replies.filter(r => r.data?.superseded).length, cache_bytes: afterSeeks.preview_bytes });

  // --- Clipping ------------------------------------------------------------------------------
  const save = async (session, start, end, aligned = false) => {
    const id = send('history_save', { session, start, end, aligned });
    return reply(id, 'save_finished', 'save_failed');
  };
  const clipsDir = path.join(root, 'Fixture', 'Clips');
  // Across three internal segments (segments are 2 s long).
  const across = await save(first.id, 7, 13);
  assert.equal(across.event, 'save_finished', JSON.stringify(across));
  assert.ok(across.data.duration_secs >= 6 && across.data.duration_secs <= 7.1, `keyframe-aligned start, exact end (${across.data.duration_secs})`);
  run(ffmpeg, ['-v', 'error', '-i', across.data.path, '-f', 'null', '-']);
  const sliceHashes = p => {
    const b = run(ffmpeg, ['-v', 'error', '-i', p, '-map', '0:v', '-c:v', 'copy', '-bsf:v', 'h264_mp4toannexb', '-f', 'h264', 'pipe:1'], true), starts = [];
    for (let i = 0; i < b.length - 3; i++) if (b[i] === 0 && b[i + 1] === 0) { const n = b[i + 2] === 1 ? 3 : b[i + 2] === 0 && b[i + 3] === 1 ? 4 : 0; if (n) { starts.push([i, i + n]); i += n - 1; } }
    return starts.map(([, a], i) => b.subarray(a, starts[i + 1]?.[0] ?? b.length)).filter(n => [1, 5].includes(n[0] & 31)).map(n => crypto.createHash('sha256').update(n).digest('hex'));
  };
  const sourceDir = path.join(buffer, recordings[0].dir);
  const original = fs.readdirSync(sourceDir).filter(n => n.endsWith('.ts')).sort().flatMap(n => sliceHashes(path.join(sourceDir, n)));
  const saved = sliceHashes(across.data.path), origin = original.indexOf(saved[0]);
  assert.ok(origin >= 0 && origin <= 7 * 30, 'the clip begins on an original keyframe at or before the selection');
  assert.deepEqual(saved, original.slice(origin, origin + saved.length), 'every frame is the original encoded frame, in order, across segment boundaries');
  assert.ok(near(saved.length / 30, across.data.duration_secs, .05));
  // Frame-accurate range from the viewer: start on a keyframe, end on a frame boundary.
  const frames = (await reply(send('history_frames', { session: first.id, at: 21 }), 'history_frames')).data.frames;
  const a = frames.findLast(f => f.key && f.start <= 21.0001).start, b = frames.find(f => f.end >= 23.5).end;
  const exactClip = await save(first.id, a, b, true);
  assert.equal(exactClip.event, 'save_finished', JSON.stringify(exactClip));
  assert.ok(near(exactClip.data.duration_secs, b - a, .005), `saved exactly the selection (${exactClip.data.duration_secs} for ${b - a})`);
  // In the second internal recording (session time is translated to its own media time).
  const later = await save(first.id, 42, 45);
  assert.equal(later.event, 'save_finished', JSON.stringify(later));
  assert.equal(probe(later.data.path).streams.find(s => s.codec_type === 'video').width, 160);
  // A selection may not silently skip footage that does not exist.
  const crossing = await save(first.id, 38, 44);
  assert.equal(crossing.event, 'save_failed');
  assert.match(crossing.data.message, /not recorded/);
  // A segment that cannot be read fails the save in plain sight, keeps the footage, and the
  // same selection saves once the file is readable again.
  const damaged = path.join(sourceDir, 'seg_000015.ts'), backup = damaged + '.fixture-backup';
  fs.copyFileSync(damaged, backup);
  fs.writeFileSync(damaged, 'deliberately corrupt disposable fixture');
  const failed = await save(first.id, 29, 33);
  assert.equal(failed.event, 'save_failed');
  assert.ok(fs.existsSync(path.join(sourceDir, 'seg_000014.ts')) && fs.existsSync(damaged), 'source footage is kept');
  const withFailure = (await reply(send('history'), 'history_ready')).data;
  assert.ok(withFailure.failed_saves.some(j => j.session === first.id && near(j.start, 29, .01)), 'the failed save is offered for retry at its session time');
  fs.copyFileSync(backup, damaged);
  fs.unlinkSync(backup);
  const retried = await save(first.id, 29, 33);
  assert.equal(retried.event, 'save_finished', JSON.stringify(retried));
  fs.unlinkSync(retried.data.path);
  pass('clipping', { across_segments_secs: across.data.duration_secs, original_frames_verified: saved.length, exact_secs: exactClip.data.duration_secs, selected_secs: +(b - a).toFixed(4), gap_refused: crossing.data.message });

  // --- Library correctness -------------------------------------------------------------------
  const before = fs.readdirSync(clipsDir).filter(n => n.endsWith('.mp4')).length;
  const duplicate = await save(first.id, 7, 13);
  assert.equal(duplicate.event, 'save_finished');
  assert.equal(duplicate.data.existing, true);
  assert.equal(duplicate.data.path, across.data.path, 'the same footage points at the file already saved');
  assert.equal(fs.readdirSync(clipsDir).filter(n => n.endsWith('.mp4')).length, before, 'no second file');
  const listed = (await reply(send('history'), 'history_ready')).data.sessions[0];
  const clipEvents = listed.events.filter(e => e.kind === 'clip');
  assert.equal(clipEvents.length, 3, 'each saved clip is on the timeline once');
  assert.ok(clipEvents.some(e => near(e.end, 45, .01) && e.t >= 41), 'clips sit at their session time');
  pass('library correctness', { files: before, duplicate_refused: true, clips_on_timeline: clipEvents.length });

  // --- Restart, then deleting temporary footage ----------------------------------------------
  send('shutdown');
  await new Promise(r => worker.on('exit', r));
  messages.length = 0;
  worker = launch();
  await wait(m => m.event === 'hello');
  send('configure', config);
  const restored = await history();
  assert.equal(restored.sessions.length, 2);
  assert.equal(restored.sessions[0].id, first.id);
  assert.deepEqual(restored.sessions[0].intervals, listed.intervals);
  assert.equal(restored.sessions[0].events.filter(e => e.kind === 'clip').length, 3);
  assert.equal(restored.preview_bytes, 0, 'playback windows of the last run are cleaned up');
  assert.equal(fs.readdirSync(buffer).filter(n => n.startsWith('preview_')).length, 0);
  send('history_delete', { session: second.id });
  await sleep(300);
  const afterDelete = (await reply(send('history'), 'history_ready')).data;
  assert.equal(afterDelete.sessions.length, 1);
  assert.ok(!fs.existsSync(path.join(buffer, recordings[2].dir)), 'its temporary footage is gone');
  assert.ok([across, exactClip, later].every(c => fs.existsSync(c.data.path)), 'saved clips are untouched');
  pass('restart and delete', { sessions_after_restart: restored.sessions.length, sessions_after_delete: afterDelete.sessions.length });

  send('shutdown');
  await new Promise(r => worker.on('exit', r));
  results.passed = true;
  fs.writeFileSync(path.join(root, 'results.json'), JSON.stringify(results, null, 2));
  console.log('ALL PASSED', root);
} catch (e) {
  console.error(stderr.split('\n').slice(-15).join('\n'));
  throw e;
} finally {
  if (worker.exitCode === null) { worker.stdin.end(); await new Promise(r => { worker.once('exit', r); setTimeout(() => { worker.kill(); r(); }, 5000).unref(); }); }
}
