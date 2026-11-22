"""pvcmp.py SET: Camera Raw's PV5 (11.0) vs PV6 (15.4) render of each setting of `make_pv.py`
(ΔE2000 per pixel, grid on valid patches; spatial on every pixel)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000

S = sys.argv[1]
meta = json.load(open(f"{S}/meta.json"))
names = sorted({n.rsplit("__", 1)[0] for n in meta["items"]})
for n in names:
    a, b = acr(S, n + "__pv11"), acr(S, n + "__pv15")
    d = de2000(lab(a.reshape(-1, 3)), lab(b.reshape(-1, 3)))
    dl = (lab(b) - lab(a))[..., 0]
    print(f"{n:22s} PV6 vs PV5: dE mean {d.mean():5.2f} p95 {np.percentile(d, 95):5.2f} max {d.max():6.2f}  dL {dl.mean():+5.2f}")
