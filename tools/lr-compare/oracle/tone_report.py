import sys, json, numpy as np
from common import *
S = sys.argv[1]; meta = json.load(open(f"{S}/meta.json"))["items"]
img = sys.argv[2] if len(sys.argv) > 2 else "ramp"
ev = np.linspace(-14, 0.5, 1024)
cols = [np.argmin(abs(ev - e)) for e in [-11, -9, -7, -6, -5, -4, -3, -2, -1, -0.5, 0]]
print("setting      " + " ".join(f"{ev[c]:6.1f}" for c in cols) + "   (L* ACR / ΔL* LightCraft−ACR)")
for name, m in meta.items():
    if m["image"] != img: continue
    a, b = lab(acr(S, name)), lab(lc(S, name))
    y = a.shape[0] // 2
    if img.startswith("probe"): y = a.shape[0] // 2
    la = a[y, :, 0]; lb = b[y, :, 0]
    print(f"{name.split('__')[1]:12s} " + " ".join(f"{la[c]:6.1f}" for c in cols))
    print(f"{'':12s} " + " ".join(f"{lb[c]-la[c]:+6.1f}" for c in cols))
