import sys, json, numpy as np
from common import lab
from cmp import load
from inv import Inv
S = "g1"; I = Inv(S); m = json.load(open(f"{S}/meta.json")); s = np.array(m["cells"]); v = np.array(m["valid"])
base = load(S, "acr", "grid__base")
def hsv(x):
    mx, mn = x.max(1), x.min(1); sat = np.where(mx > 0, (mx - mn) / np.maximum(mx, 1e-12), 0)
    r, g, b = x.T; d = np.maximum(mx - mn, 1e-12)
    h = np.where(mx == r, (g - b) / d, np.where(mx == g, 2 + (b - r) / d, 4 + (r - g) / d)) / 6 % 1
    return h, sat, mx
def scene_op(name):
    out = load(S, "acr", name); ok = v & (out.max(1) < 0.995) & (base.max(1) < 0.995) & (out.min(1) > 1e-5)
    return ok, I(lab(out[ok]))
if __name__ == "__main__":
    name = sys.argv[1]; ok, sp = scene_op(name); s0 = s[ok]
    M = np.linalg.lstsq(s0, sp, rcond=None)[0].T
    e = lab(s0 @ M.T) - lab(sp); print("3x3 fit resid (scene Lab)", np.sqrt((e ** 2).sum(1)).mean().round(2)); print(np.round(M, 4))
    h0, sa0, v0 = hsv(s0); h1, sa1, v1 = hsv(sp)
    ev = np.log2(v0); dh = ((h1 - h0 + .5) % 1 - .5) * 360
    for hb in range(0, 36, 3):
        sel = (np.abs(((h0 * 36 - hb + 18) % 36) - 18) < .5) & (sa0 > .3)
        if sel.sum() == 0: continue
        print(f"hue {hb*10:3d}: dh {np.round(dh[sel][::4],1)} satr {np.round((sa1/np.maximum(sa0,1e-6))[sel][::4],2)} vr {np.round((v1/v0)[sel][::4],2)}")
