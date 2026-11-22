"""r12ana.py SET LCDIR [filter]: Highlights / Shadows on Bryan's raws (make_r12.py), Camera Raw vs a LightCraft render
folder: per item mean ΔE2000 and mean ΔL* (LightCraft − Camera Raw) in Camera Raw's L* bands 0-25 / 25-50 / 50-75 / 75-100,
and per photo the H/S *effect* (`own` − `own-noHS`, `HS0` − `base`) in ΔL* per band, Camera Raw against LightCraft."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000

S, L = sys.argv[1], sys.argv[2]
f = sys.argv[3] if len(sys.argv) > 3 else ""
meta = json.load(open(f"{S}/meta.json"))["items"]
BANDS = [(0, 25), (25, 50), (50, 75), (75, 101)]


def load(n):
    if not os.path.exists(f"{S}/{L}/{n}.f32") or not os.path.exists(f"{S}/acr/{n}.tif"):
        return None
    a = acr(S, n)
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); b = b[2:].reshape(h, w, 3).astype(np.float64)
    if a.shape != b.shape:
        return None
    a, b = a[::2, ::2].reshape(-1, 3), b[::2, ::2].reshape(-1, 3)
    return lab(a), lab(b)


cache = {}


def get(n):
    if n not in cache:
        cache[n] = load(n)
    return cache[n]


def bands(la, d):
    return [d[(la >= lo) & (la < hi)].mean() if ((la >= lo) & (la < hi)).sum() > 200 else np.nan for lo, hi in BANDS]


fmt = lambda v: " ".join("  .  " if np.isnan(x) else f"{x:+5.1f}" for x in v)
tot = {}
photos = sorted({v["photo"] for v in meta.values() if v.get("photo")})
for p in photos:
    for n in sorted(k for k in meta if meta[k].get("photo") == p):
        if f not in n:
            continue
        r = get(n)
        if r is None:
            continue
        a, b = r
        d = de2000(a, b)
        kind = n.split("__")[1]
        tot.setdefault(kind, []).append(d.mean())
        print(f"{n:28s} dE {d.mean():5.2f}  dL by band {fmt(bands(a[:, 0], b[:, 0] - a[:, 0]))}")
    kinds = sorted(k.split("__")[1] for k in meta if meta[k].get("photo") == p)
    pairs = [("own", "own-noHS"), ("full", "full-noHS"), ("tone", "tone-noHS")] + [(k, "base") for k in kinds if k not in ("base", "own", "own-noHS", "full", "full-noHS", "tone", "tone-noHS")]
    for x, y in pairs:
        if f and f not in f"{p}__{x}":
            continue
        ra, rb = get(f"{p}__{x}"), get(f"{p}__{y}")
        if ra is None or rb is None:
            continue
        ecr = ra[0][:, 0] - rb[0][:, 0]
        elc = ra[1][:, 0] - rb[1][:, 0]
        print(f"  effect {x:10s} CR {fmt(bands(rb[0][:, 0], ecr))}   LC {fmt(bands(rb[0][:, 0], elc))}   err {np.abs(elc - ecr).mean():4.2f}")
print()
for k, v in sorted(tot.items()):
    print(f"mean {k:14s} {np.mean(v):5.2f} ({len(v)})")
