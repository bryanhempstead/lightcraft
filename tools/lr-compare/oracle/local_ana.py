"""local_ana.py SET LCDIR [items…]: masks' local tone sliders (make_fx.py mask) read off the spatial
scene's gradient row (−8 … −0.5 EV): the scene-referred change (EV) that LightCraft's own base would
need to land on Camera Raw's output, by input level — i.e. the local slider's curve as LightCraft's
per-pixel stage applies it (`delta` in log2 luminance, `l1` = log2(Y / 0.18)). `--json=OUT` saves it."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, Y

args = [a for a in sys.argv[1:] if not a.startswith("--")]
S, L = args[0], args[1]
items = args[2:] or ["lCon+50", "lCon-50", "lWhi+50", "lBla+50", "lHig-60", "lSha+60", "lExp+1"]
H, W = 512, 1024
row = slice(H // 2 + 16, H // 2 + 48)
SC = os.environ.get("SCENE", "spatial")
OFF = {"spatial": 0.0, "spatialD": -2.0, "spatialB": 1.0}.get(SC, 0.0)
x = np.linspace(-8, -0.5, W) + OFF
l1 = x - np.log2(0.18)


def load(kind, n):
    if kind == "acr":
        return acr(S, n)
    b = np.fromfile(f"{S}/{kind}/{n}.f32", "<f4"); w, h = b[:2].view("<u4")
    return b[2:].reshape(h, w, 3).astype(np.float64)


def ramp(kind, n):
    return np.log2(np.maximum(Y(load(kind, n)[row].mean(0)), 1e-9))


base_lc = ramp(L, f"{SC}__base")
order = np.argsort(base_lc)
out = {}
knots = np.arange(-5.5, 2.01, 0.5)
print("l1 knots:   " + " ".join(f"{k:+5.1f}" for k in knots))
for it in items:
    n = f"{SC}__{it}"
    res = {}
    for kind in ("acr", L):
        if not (os.path.exists(f"{S}/acr/{n}.tif") and os.path.exists(f"{S}/{L}/{n}.f32")):
            continue
        y = ramp(kind, n)
        # the scene level LightCraft's base renders at this output
        xs = np.interp(y, base_lc[order], l1[order], left=np.nan, right=np.nan)
        d = xs - l1
        res[kind] = np.interp(knots, l1, d)
    if res:
        out[it] = {k: v.tolist() for k, v in res.items()}
        for k, v in res.items():
            print(f"{it:>9s} {k:>4s} " + " ".join(f"{e:+5.2f}" for e in v))
for a in sys.argv[1:]:
    if a.startswith("--json="):
        json.dump({"knots": knots.tolist(), "items": out}, open(a[7:], "w"))
