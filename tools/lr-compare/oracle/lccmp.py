"""lccmp.py SET LCDIR [filter]: Camera Raw vs a LightCraft render folder (`oracle_render` .f32) for
every item of SET: mean / p95 ΔE2000 (grid: valid, unclipped patches; other scenes: every pixel)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000

S, L = sys.argv[1], sys.argv[2]
f = sys.argv[3] if len(sys.argv) > 3 else ""
meta = json.load(open(f"{S}/meta.json"))
R = meta.get("rects"); valid = np.array(meta.get("valid", []))
tot = {}
for n in sorted(meta["items"]):
    if f not in n or not os.path.exists(f"{S}/{L}/{n}.f32") or not os.path.exists(f"{S}/acr/{n}.tif"):
        continue
    a = acr(S, n)
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); b = b[2:].reshape(h, w, 3).astype(np.float64)
    if b.shape != a.shape:
        print(n, "size", a.shape, b.shape); continue
    if n.startswith("grid") and R:
        pa = np.array([a[y0:y1, x0:x1].reshape(-1, 3).mean(0) for y0, x0, y1, x1 in R])
        pb = np.array([b[y0:y1, x0:x1].reshape(-1, 3).mean(0) for y0, x0, y1, x1 in R])
        ok = valid & (pa.max(1) < 0.99)
        d = de2000(lab(pa), lab(pb))[ok]
    else:
        d = de2000(lab(a.reshape(-1, 3)), lab(b.reshape(-1, 3)))
    tot.setdefault(n.split("__")[0], []).append(d.mean())
    print(f"{n:30s} dE {d.mean():5.2f} p95 {np.percentile(d, 95):5.2f}")
for k, v in tot.items():
    print(f"mean {k}: {np.mean(v):.3f} ({len(v)})")
