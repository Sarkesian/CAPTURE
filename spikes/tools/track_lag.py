"""Lag between two recordings of the same sound (cross-correlation). Positive = B is late.
Usage: track_lag.py a.wav b.wav [max_ms]"""
import sys, numpy as np
src = open(__file__.replace("track_lag.py", "tone_check.py"), encoding="utf-8").read()
exec(src[src.index("def load(p):"):src.index("freqs = [float")])
a, sr = load(sys.argv[1]); b, _ = load(sys.argv[2])
n = min(len(a), len(b)); a, b = a[:n], b[:n]
maxlag = int(float(sys.argv[3] if len(sys.argv) > 3 else 200) * sr / 1000)
N = 1 << (2 * n - 1).bit_length()
c = np.fft.irfft(np.fft.rfft(b, N) * np.conj(np.fft.rfft(a, N)), N)
c = np.concatenate([c[-maxlag:], c[:maxlag + 1]])
lag = np.argmax(c) - maxlag
k = max(lag, 0); j = max(-lag, 0)
m = min(n - k, n - j)
corr = np.corrcoef(a[j:j + m], b[k:k + m])[0, 1]
print(f"lag = {lag} samples = {lag * 1000 / sr:+.2f} ms ; correlation at that lag = {corr:+.3f}")
