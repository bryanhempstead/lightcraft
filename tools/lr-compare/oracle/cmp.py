"""cmp.py SET [filter]: per-setting ΔE2000 LightCraft vs Camera Raw on chart patches"""
import sys, os, json, numpy as np
from common import lab
def de2000(a, b):
    L1, a1, b1 = a[..., 0], a[..., 1], a[..., 2]; L2, a2, b2 = b[..., 0], b[..., 1], b[..., 2]
    C1, C2 = np.hypot(a1, b1), np.hypot(a2, b2); Cm = (C1 + C2) / 2
    G = 0.5 * (1 - np.sqrt(Cm ** 7 / (Cm ** 7 + 25 ** 7))); a1p, a2p = a1 * (1 + G), a2 * (1 + G)
    C1p, C2p = np.hypot(a1p, b1), np.hypot(a2p, b2); h1, h2 = np.degrees(np.arctan2(b1, a1p)) % 360, np.degrees(np.arctan2(b2, a2p)) % 360
    dL, dC = L2 - L1, C2p - C1p; dh = h2 - h1; dh = np.where(dh > 180, dh - 360, np.where(dh < -180, dh + 360, dh)); dh = np.where(C1p * C2p == 0, 0, dh)
    dH = 2 * np.sqrt(C1p * C2p) * np.sin(np.radians(dh / 2)); Lm, Cpm = (L1 + L2) / 2, (C1p + C2p) / 2
    hs = h1 + h2; hm = np.where(C1p * C2p == 0, hs, np.where(np.abs(h1 - h2) <= 180, hs / 2, np.where(hs < 360, (hs + 360) / 2, (hs - 360) / 2)))
    T = 1 - .17 * np.cos(np.radians(hm - 30)) + .24 * np.cos(np.radians(2 * hm)) + .32 * np.cos(np.radians(3 * hm + 6)) - .2 * np.cos(np.radians(4 * hm - 63))
    SL = 1 + .015 * (Lm - 50) ** 2 / np.sqrt(20 + (Lm - 50) ** 2); SC = 1 + .045 * Cpm; SH = 1 + .015 * Cpm * T
    RT = -2 * np.sqrt(Cpm ** 7 / (Cpm ** 7 + 25 ** 7)) * np.sin(np.radians(60 * np.exp(-((hm - 275) / 25) ** 2)))
    return np.sqrt((dL / SL) ** 2 + (dC / SC) ** 2 + (dH / SH) ** 2 + RT * (dC / SC) * (dH / SH))
def load(S, k, n): return np.load(f"{S}/p_{k}/{n}.npy")
if __name__ == "__main__":
    S = sys.argv[1]; f = sys.argv[2] if len(sys.argv) > 2 else ""
    names = sorted(x[:-4] for x in os.listdir(f"{S}/p_acr") if f in x and os.path.exists(f"{S}/p_lc/{x}"))
    tot = []
    for n in names:
        a, b = lab(load(S, "acr", n)), lab(load(S, "lc", n)); d = de2000(a, b); tot.append(d.mean())
        dl = (b - a)[:, 0].mean(); print(f"{n:40s} dE {d.mean():5.2f} p95 {np.percentile(d, 95):5.2f} max {d.max():5.2f} dL {dl:+5.2f}")
    print("mean", round(float(np.mean(tot)), 2))
