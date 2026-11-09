// Independent check of recordings with FFmpeg's own tools (not CAPTURE's status messages).
//
//   node scripts/validate-media.mjs <file-or-folder> [...] [--expect-tracks N] [--min-secs S]
//
// For every .mp4: full decode of every stream with warnings shown (corrupt packets,
// non-monotonic timestamps), video size / frame rate / frame count, every audio track's start,
// duration, sample rate and channels, and the start/end offset between video and each audio
// track. Exits 1 if any file fails.
import { execFileSync, spawnSync } from 'node:child_process';
import { readdirSync, statSync } from 'node:fs';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const bin = path.join(root, 'third_party', 'ffmpeg', 'bin');
const args = process.argv.slice(2);
const opt = (name, d) => { const i = args.indexOf(name); if (i < 0) return d; const v = Number(args[i + 1]); args.splice(i, 2); return v; };
const expectTracks = opt('--expect-tracks', null);
const minSecs = opt('--min-secs', 0);
const files = args.flatMap(a => statSync(a).isDirectory()
  ? readdirSync(a, { recursive: true }).map(f => path.join(a, f)).filter(f => /\.mp4$/i.test(f))
  : [a]);
if (!files.length) { console.log('no .mp4 files given'); process.exit(2); }

let failed = 0;
for (const f of files) {
  const problems = [];
  const info = JSON.parse(execFileSync(path.join(bin, 'ffprobe.exe'),
    ['-v', 'error', '-show_streams', '-show_format', '-count_frames', '-of', 'json', f], { encoding: 'utf8', maxBuffer: 64 << 20 }));
  const v = info.streams.filter(s => s.codec_type === 'video');
  const a = info.streams.filter(s => s.codec_type === 'audio');
  const dur = Number(info.format.duration);
  if (v.length !== 1) problems.push(`${v.length} video streams`);
  if (expectTracks != null && a.length !== expectTracks) problems.push(`${a.length} audio tracks, expected ${expectTracks}`);
  if (!(dur >= minSecs)) problems.push(`duration ${dur}s < ${minSecs}s`);
  const vs = v[0] ?? {};
  const vStart = Number(vs.start_time), vDur = Number(vs.duration);
  const [n, d] = String(vs.avg_frame_rate ?? '0/1').split('/').map(Number);
  const fps = d ? n / d : 0;
  const frames = Number(vs.nb_read_frames);
  const expectedFrames = vDur * fps;
  if (fps && Math.abs(frames - expectedFrames) > Math.max(2, expectedFrames * 0.01)) problems.push(`video frames ${frames} vs ${expectedFrames.toFixed(0)} expected from duration`);
  const tracks = a.map(s => {
    const start = Number(s.start_time), sdur = Number(s.duration);
    const off = (start - vStart) * 1000, endOff = ((start + sdur) - (vStart + vDur)) * 1000;
    // Audio frames are ~21 ms (AAC 1024 @ 48 kHz) and video frames ~17 ms: allow ~2 frames.
    if (Math.abs(off) > 50) problems.push(`${s.tags?.handler_name}: starts ${off.toFixed(0)} ms from video`);
    if (Math.abs(endOff) > 250) problems.push(`${s.tags?.handler_name}: ends ${endOff.toFixed(0)} ms from video`);
    return `${s.tags?.handler_name ?? '?'} ${s.sample_rate}Hz/${s.channels}ch start${off >= 0 ? '+' : ''}${off.toFixed(0)}ms end${endOff >= 0 ? '+' : ''}${endOff.toFixed(0)}ms`;
  });
  // Full decode: any error or timestamp warning is a problem.
  const dec = spawnSync(path.join(bin, 'ffmpeg.exe'), ['-hide_banner', '-nostats', '-v', 'warning', '-i', f, '-map', '0', '-f', 'null', '-'], { encoding: 'utf8', maxBuffer: 64 << 20 });
  const warn = (dec.stderr || '').split(/\r?\n/).filter(l => l.trim());
  if (dec.status !== 0) problems.push(`decode exit ${dec.status}`);
  if (warn.length) problems.push(`decoder: ${warn.slice(0, 3).join(' | ')}${warn.length > 3 ? ` (+${warn.length - 3})` : ''}`);
  const ok = problems.length === 0;
  if (!ok) failed++;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${path.basename(f)}  ${dur.toFixed(2)}s  ${vs.width}x${vs.height}@${fps.toFixed(2)} ${frames}f  [${tracks.join('; ')}]`);
  for (const p of problems) console.log(`      ${p}`);
}
console.log(`${files.length - failed}/${files.length} valid`);
process.exit(failed ? 1 : 0);
