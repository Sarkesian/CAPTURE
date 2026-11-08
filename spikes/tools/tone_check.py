"""Measure per-file energy at given frequencies (dBFS) to prove streams are distinct, plus
pairwise correlation to prove files are not duplicates. Usage: tone_check.py f1,f2 file..."""
import sys, wave, numpy as np

def load(p):
    """Minimal RIFF reader: handles 16-bit PCM and 32-bit float (incl. WAVE_FORMAT_EXTENSIBLE)."""
    b = open(p, "rb").read()
    pos, fmt, data = 12, None, None
    while pos + 8 <= len(b):
        cid, size = b[pos:pos+4], int.from_bytes(b[pos+4:pos+8], "little")
        if cid == b"fmt ": fmt = b[pos+8:pos+8+size]
        if cid == b"data": data = b[pos+8:pos+8+size]
        pos += 8 + size + (size & 1)
    ch = int.from_bytes(fmt[2:4], "little"); sr = int.from_bytes(fmt[4:8], "little")
    bits = int.from_bytes(fmt[14:16], "little")
    if bits == 24:
        u = np.frombuffer(data[: len(data) // 3 * 3], dtype=np.uint8).reshape(-1, 3).astype(np.int32)
        v = u[:, 0] | (u[:, 1] << 8) | (u[:, 2] << 16)
        a = np.where(v >= 1 << 23, v - (1 << 24), v).astype(np.float64) / (1 << 23)
    else:
        a = np.frombuffer(data, dtype=np.float32 if bits == 32 else np.int16).astype(np.float64)
        if bits == 16: a /= 32768
    return a.reshape(-1, ch).mean(axis=1), sr

freqs = [float(f) for f in sys.argv[1].split(",")]
sig = {}
for p in sys.argv[2:]:
    x, sr = load(p)
    sig[p] = x
    spec = np.abs(np.fft.rfft(x * np.hanning(len(x)))) / (len(x) / 4)
    bins = np.fft.rfftfreq(len(x), 1 / sr)
    out = []
    for f in freqs:
        i = np.argmin(abs(bins - f))
        out.append(f"{int(f)}Hz={20*np.log10(spec[max(i-3,0):i+4].max()+1e-12):6.1f}dB")
    rms = np.sqrt((x**2).mean())
    print(f"{p.split('/')[-1].split(chr(92))[-1]:<18} dur={len(x)/sr:6.2f}s rms={20*np.log10(rms+1e-12):6.1f}dBFS  " + "  ".join(out))
keys = list(sig)
for i in range(len(keys)):
    for j in range(i + 1, len(keys)):
        a, b = sig[keys[i]], sig[keys[j]]
        n = min(len(a), len(b))
        if a[:n].std() == 0 or b[:n].std() == 0: continue
        print(f"corr {keys[i].split(chr(92))[-1].split('/')[-1]} vs {keys[j].split(chr(92))[-1].split('/')[-1]}: {np.corrcoef(a[:n], b[:n])[0,1]:+.3f}")
