"""Max short-window level at given frequencies anywhere in a WAV (finds brief blips like the
marker chime that a whole-file FFT averages away). Usage: blip_peak.py f1,f2 file..."""
import sys, numpy as np
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
for p in sys.argv[2:]:
    x, sr = load(p)
    n, hop = 2048, 256
    win = np.hanning(n)
    best = {f: -240.0 for f in freqs}
    for i in range(0, max(1, len(x) - n), hop):
        spec = np.abs(np.fft.rfft(x[i:i+n] * win)) / (n / 4)
        bins = np.fft.rfftfreq(n, 1 / sr)
        for f in freqs:
            k = np.argmin(abs(bins - f))
            best[f] = max(best[f], 20 * np.log10(spec[max(k-2,0):k+3].max() + 1e-12))
    print(f"{p.replace(chr(92),'/').split('/')[-1]:<16} " + "  ".join(f"{int(f)}Hz peak={best[f]:6.1f}dB" for f in freqs))
