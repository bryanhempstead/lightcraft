"""gen_colortab.py TABLES.npy OUT.bin: pack the measured colour-op tables (hslfit.py, axes h-l-c) for
lightcraft_pipeline::colortab. Layout (little endian): magic b"LCT2", u8 NH, NL, NC, NOPS, NSTOPS;
then per op (Hue x 8 bands, Saturation x 8, Luminance x 8, Saturation, Vibrance) and stop (-100, -50,
+50, +100): u8 first hue bin, u8 hue-bin count, 3 f32 scales, then count x NL x NC x 3 i8 (Δhue rad, ln chroma
ratio, ΔL; value = i8 x scale); hue bins outside the stored run are zero."""
import sys, struct, numpy as np
from hslfit import NH, LC_, CC
T = np.load(sys.argv[1], allow_pickle=True).item()
BANDS = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"]
OPS = [f"{k}{b}" for k in ["Hue", "Saturation", "Luminance"] for b in BANDS] + ["Saturation", "Vibrance"]
STOPS = [-100, -50, 50, 100]
out = bytearray(b"LCT2") + bytes([NH, len(LC_), len(CC), len(OPS), len(STOPS)])
for o in OPS:
    for v in STOPS:
        f = np.maximum(T[f"{o}{v:+d}"], -8.0)  # NH x NL x NC x 3
        sc = np.maximum(np.abs(f).reshape(-1, 3).max(0), 1e-6) / 127
        t = np.clip(np.round(f / sc), -127, 127).astype("i1")
        live = (np.abs(f) > 0.006).reshape(NH, -1).any(1)
        if live.all() or not live.any(): first, cnt = 0, NH if live.any() else 0
        else:
            # the complement of the longest circular run of dead bins
            best, start = 0, 0
            for i in range(NH):
                n = 0
                while n < NH and not live[(i + n) % NH]: n += 1
                if n > best: best, start = n, i
            first, cnt = (start + best) % NH, NH - best
        idx = [(first + i) % NH for i in range(cnt)]
        out += bytes([first, cnt]) + sc.astype('<f4').tobytes() + t[idx].tobytes()
open(sys.argv[2], "wb").write(out); print(len(out), "bytes")
