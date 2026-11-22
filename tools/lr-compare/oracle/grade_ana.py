"""grade_ana.py SET [LCDIR] [filter]: colour-grid items as OkLab shifts from the renderer's own `base`
(per patch), summarised for the neutral patches by level (OkLab L of the base: ΔL, Δa, Δb, the shift's
hue and size) and over all valid patches (mean shift by base-L band). Camera Raw, then LightCraft."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr
from ok import oklab_pp

S = sys.argv[1]
L = sys.argv[2] if len(sys.argv) > 2 else None
f = sys.argv[3] if len(sys.argv) > 3 else ""
meta = json.load(open(f"{S}/meta.json"))
R = meta["rects"]
cells = np.array(meta["cells"])
valid = np.array(meta["valid"])
sat = (cells.max(1) - cells.min(1)) / np.maximum(cells.max(1), 1e-12)
neutral = np.zeros(len(cells), bool); neutral[::360] = True; neutral &= valid


def patches(kind, n):
    if kind == "acr":
        p = f"{S}/acr/{n}.tif"
        if not os.path.exists(p):
            return None
        a = acr(S, n)
    else:
        p = f"{S}/{kind}/{n}.f32"
        if not os.path.exists(p):
            return None
        b = np.fromfile(p, "<f4"); w, h = b[:2].view("<u4"); a = b[2:].reshape(h, w, 3).astype(np.float64)
    return oklab_pp(np.array([a[y0:y1, x0:x1].reshape(-1, 3).mean(0) for y0, x0, y1, x1 in R]))


cache = {}


def get(kind, n):
    if (kind, n) not in cache:
        cache[(kind, n)] = patches(kind, n)
    return cache[(kind, n)]


def report(kind, n):
    b, x = get(kind, "grid__base"), get(kind, n)
    if b is None or x is None:
        return None
    d = x - b
    nl = np.where(neutral)[0]
    nl = nl[np.argsort(b[nl, 0])]
    row = " ".join(f"{b[i, 0]:.2f}:{d[i, 0]:+.3f}/{np.hypot(d[i, 1], d[i, 2]):.3f}@{np.degrees(np.arctan2(d[i, 2], d[i, 1])) % 360:3.0f}" for i in nl)
    return d, row


for n in sorted(meta["items"]):
    if not n.startswith("grid__") or n == "grid__base" or f not in n:
        continue
    for kind in ["acr"] + ([L] if L else []):
        r = report(kind, n)
        if r is None:
            continue
        print(f"{n[6:]:>14s} {kind:>4s} {r[1]}")
