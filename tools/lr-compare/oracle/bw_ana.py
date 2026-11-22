"""bw_ana.py SET LCDIR: B&W on the colour grid. For `bw` (default mix) the output OkLab L against the
colour's OkLab L in the colour base, and for each `bw<Band>±60` the change of L from `bw`, binned by the
base colour's OkLab hue (30° bins) for chroma > 0.06 — Camera Raw | LightCraft."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr
from ok import oklab_pp

S, LC = sys.argv[1], sys.argv[2]
meta = json.load(open(f"{S}/meta.json"))
R, valid = meta["rects"], np.array(meta["valid"])


def P(kind, n):
    if kind == "acr":
        a = acr(S, n)
    else:
        b = np.fromfile(f"{S}/{kind}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); a = b[2:].reshape(h, w, 3).astype(np.float64)
    return oklab_pp(np.array([a[y0:y1, x0:x1].reshape(-1, 3).mean(0) for y0, x0, y1, x1 in R]))


base = P("acr", "grid__base")
hue = np.degrees(np.arctan2(base[:, 2], base[:, 1])) % 360
chroma = np.hypot(base[:, 1], base[:, 2])
ok = valid & (chroma > 0.06) & (base[:, 0] > 0.25) & (base[:, 0] < 0.9)
bins = np.arange(0, 360, 30)


def binned(v):
    return " ".join(f"{np.mean(v[ok & (hue >= b) & (hue < b + 30)]) if (ok & (hue >= b) & (hue < b + 30)).any() else np.nan:+.3f}" for b in bins)


print("hue bins:  " + " ".join(f"{b:>6d}" for b in bins))
for kind in ("acr", LC):
    bw = P(kind, "grid__bw")
    print(f"bw - colour L  {kind:>4s}: {binned(bw[:, 0] - base[:, 0])}   chroma left {np.abs(bw[:, 1:]).max():.4f}")
for n in sorted(meta["items"]):
    if not n.startswith("grid__bw") or n == "grid__bw":
        continue
    for kind in ("acr", LC):
        d = P(kind, n)[:, 0] - P(kind, "grid__bw")[:, 0]
        print(f"{n[6:]:>14s} {kind:>4s}: {binned(d)}")
for kind in ("acr", LC):
    m = P(kind, "grid__mono")
    print(f"mono - colour L {kind:>4s}: {binned(m[:, 0] - base[:, 0])}  chroma left {np.abs(m[:, 1:]).max():.4f}")
