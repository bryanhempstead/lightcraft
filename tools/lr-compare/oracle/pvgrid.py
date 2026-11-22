"""pvgrid.py SET NAME: where PV6 (15.4) differs from PV5 (11.0) on the colour grid of `make_pv.py`:
ΔE2000 by grid saturation and level, and the largest patches (hue, saturation, EV, Lab of both)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000
import scenes

S, N = sys.argv[1], (sys.argv[2] if len(sys.argv) > 2 else "base")
m = json.load(open(f"{S}/meta.json"))
R = m["rects"]; valid = np.array(m["valid"])
P = lambda img: np.array([img[y0:y1, x0:x1].reshape(-1, 3).mean(0) for y0, x0, y1, x1 in R])
a, b = P(acr(S, f"grid__{N}__pv11")), P(acr(S, f"grid__{N}__pv15"))
la, lb = lab(a), lab(b)
d = de2000(la, lb)
n = len(scenes.GRID_H) * len(scenes.GRID_S)
ev = np.repeat(scenes.GRID_EV, n)
sat = np.tile(np.repeat(scenes.GRID_S, len(scenes.GRID_H)), len(scenes.GRID_EV))
hue = np.tile(scenes.GRID_H, len(scenes.GRID_S) * len(scenes.GRID_EV)) * 360
print("by saturation:", " ".join(f"{s:.2f}:{d[valid & (sat == s)].mean():.2f}" for s in scenes.GRID_S))
print("by EV:", " ".join(f"{e:.1f}:{d[valid & (ev == e)].mean():.2f}" for e in scenes.GRID_EV))
print("PV6 - PV5 mean dL dC:", (lb - la)[valid, 0].mean(), (np.hypot(lb[:, 1], lb[:, 2]) - np.hypot(la[:, 1], la[:, 2]))[valid].mean())
for i in np.argsort(-np.where(valid, d, 0))[:8]:
    print(f"hue {hue[i]:5.0f} sat {sat[i]:.2f} ev {ev[i]:5.1f}  PV5 {np.round(la[i], 1)}  PV6 {np.round(lb[i], 1)}  dE {d[i]:.1f}")
