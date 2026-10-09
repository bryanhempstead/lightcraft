import json, sys, numpy as np
from hslfit import fit, adjust
from common import lab
from cmp import de2000
S = "g1"; m = json.load(open(f"{S}/meta.json")); v = np.array(m["valid"]); s = np.array(m["cells"])
base = np.load(f"{S}/p_acr/grid__base.npy"); okb = v & (base.max(1) < .99) & (np.log2(s.max(1)) > -7.5)
axes = tuple(sys.argv[1]) if len(sys.argv) > 1 else ("h", "l", "c")
tables = {}
BANDS = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"]
ops = [f"{k}{b}" for k in ["Hue", "Saturation", "Luminance"] for b in BANDS] + ["Saturation", "Vibrance"]
res = []
for o in ops:
    for val in [-100, -50, 50, 100]:
        n = f"grid__{o}{val:+d}"; a = np.load(f"{S}/p_acr/{n}.npy"); ok = okb & (a.max(1) < .99)
        T = fit(base, a, ok, axes); tables[(o, val)] = T
        pred = adjust(base[ok], [(T, axes)]); d = de2000(lab(pred), lab(a[ok])); d0 = de2000(lab(base[ok]), lab(a[ok]))
        res.append((n, d0.mean(), d.mean(), np.percentile(d, 95)))
for r in res: print(f"{r[0]:32s} effect {r[1]:5.2f} model {r[2]:5.2f} p95 {r[3]:5.2f}")
print("mean model", np.mean([r[2] for r in res]).round(3))
np.save(f"hsltab_{''.join(axes)}.npy", {f"{o}{v:+d}": T for (o, v), T in tables.items()}, allow_pickle=True)
