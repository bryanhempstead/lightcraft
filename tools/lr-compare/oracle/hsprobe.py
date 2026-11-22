"""hsprobe.py SET [LCDIR]: `make_hs.py` results: each slider's change (EV of luminance vs the scene's
base render) of the uniform fields, of the dark / bright regions of the two-level fields and along
their probe ramp (input EV −10..0), for Camera Raw (and LightCraft with LCDIR)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, Y

S = sys.argv[1]; L = sys.argv[2] if len(sys.argv) > 2 else None
meta = json.load(open(f"{S}/meta.json"))


def lc(n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); return b[2:].reshape(h, w, 3).astype(np.float64)


def dev(get, n):
    sc = meta["items"][n]["scene"]
    return np.log2(np.maximum(Y(get(n)), 1e-7) / np.maximum(Y(get(f"{sc}__base")), 1e-7))


gets = [("ACR", lambda n: acr(S, n))] + ([("LC", lc)] if L else [])
PROBE = [-9, -8, -7, -6, -5, -4, -3, -2, -1, -0.3]
for n in sorted(meta["items"]):
    sc = meta["items"][n]["scene"]
    if n.endswith("__base"):
        continue
    for tag, g in gets:
        try:
            d = dev(g, n)
        except (FileNotFoundError, ValueError):
            continue
        if sc.startswith("u"):
            print(f"{n:22s} {tag:3s} uniform dEV {d.mean():+.3f}")
            continue
        H, W = d.shape
        k = float(sc[4:]) if "k" in sc else 0.0
        f = int(sc[1:3]) / 100
        dark = d[20:H // 2 - 30, 10:max(11, int(f * W) - 10)].mean()
        bright = d[20:H // 2 - 30, int(f * W) + 10:W - 10].mean() if f < 0.95 else float("nan")
        row = d[H // 2 - 3:H // 2 + 3].mean(0)
        ev = np.linspace(-10, 0, W)
        pr = " ".join(f"{p:+.0f}:{np.interp(p, ev, row):+.2f}" for p in PROBE)
        print(f"{n:22s} {tag:3s} dark {dark:+.2f} bright {bright:+.2f} | probe {pr}")
