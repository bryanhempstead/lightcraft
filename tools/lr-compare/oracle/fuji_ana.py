"""fuji_ana.py SET LCDIR: per photo of make_fuji.py, LightCraft vs Camera Raw: mean ΔL* by tone and the
exposure (EV) LightCraft is off by — the median log2 luminance ratio over unclipped mid tones
(Camera Raw L* 25–75)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab, Y

S, L = sys.argv[1], sys.argv[2]
meta = json.load(open(f"{S}/meta.json"))
for n in sorted(meta["items"]):
    p = f"{S}/{L}/{n}.f32"
    if not (os.path.exists(p) and os.path.exists(f"{S}/acr/{n}.tif")):
        continue
    a = acr(S, n)
    b = np.fromfile(p, "<f4"); w, h = b[:2].view("<u4"); b = b[2:].reshape(h, w, 3).astype(np.float64)
    if a.shape != b.shape:
        print(n, "size", a.shape, b.shape)
        continue
    la, lb = lab(a.reshape(-1, 3)), lab(b.reshape(-1, 3))
    mid = (la[:, 0] > 25) & (la[:, 0] < 75)
    ev = np.median(np.log2(np.maximum(Y(b.reshape(-1, 3))[mid], 1e-9) / np.maximum(Y(a.reshape(-1, 3))[mid], 1e-9)))
    d = lb[:, 0] - la[:, 0]
    m = meta["items"][n]
    print(f"{n:22s} {m['model']:6s} dL* {d.mean():+.2f} (dark {d[la[:, 0] < 30].mean():+.2f} mid {d[mid].mean():+.2f} light {d[la[:, 0] >= 70].mean():+.2f})  "
          f"mid-tone exposure {ev:+.3f} EV")
