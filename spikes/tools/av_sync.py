"""A/V sync of a recorded flash+beep pattern (white flash and 1 kHz beep start together every
second). Usage: av_sync.py <clip.mp4> <audio.wav> <ffmpeg.exe>
Prints each flash/beep pair's offset (positive = audio late) and early/late averages (drift)."""
import subprocess, sys, numpy as np
clip, wav, ffmpeg = sys.argv[1:4]
src = open(__file__.replace("av_sync.py", "tone_check.py"), encoding="utf-8").read()
exec(src[src.index("def load(p):"):src.index("freqs = [float")])
W, H = 32, 18
raw = subprocess.run([ffmpeg, "-v", "error", "-i", clip, "-map", "0:v:0", "-vf", f"scale={W}:{H},format=gray", "-f", "rawvideo", "-"], capture_output=True).stdout
fr = np.frombuffer(raw, np.uint8).reshape(-1, W * H).mean(axis=1)
fps = 60.0
vt = [i / fps for i in range(1, len(fr)) if fr[i] > 128 and fr[i - 1] <= 128]
a, sr = load(wav)
env = np.abs(a)
hot = env > max(1e-5, 0.3 * np.percentile(env, 99.9))
at = []
i = 0
while i < len(hot):
    if hot[i] and (i < int(0.3 * sr) or not hot[max(0, i - int(0.3 * sr)):i].any()):
        at.append(i / sr); i += int(0.3 * sr)
    else:
        i += 1
pairs = []
for v in vt:
    near = min(at, key=lambda x: abs(x - v)) if at else None
    if near is not None and abs(near - v) < 0.4:
        pairs.append((v, (near - v) * 1000))
if not pairs:
    print(f"no pairs (flashes={len(vt)}, beeps={len(at)})"); sys.exit(1)
offs = np.array([p[1] for p in pairs])
k = max(1, len(offs) // 4)
print(f"pairs={len(pairs)} mean={offs.mean():+.1f} ms  min={offs.min():+.1f}  max={offs.max():+.1f}  first quarter={offs[:k].mean():+.1f}  last quarter={offs[-k:].mean():+.1f}  (frame = {1000/fps:.1f} ms)")
