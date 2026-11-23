"""r13cla.py SET LCDIR [SETTING]: Clarity's regional move on `make_r13.py cla` (flat two-level layouts, round 13).
Camera Raw vs LightCraft: the change of output log2 Y (SETTING − base) along a profile across each layout's boundary
(top/bottom layouts: down the middle column; left/right: along the middle row; windows / hole: from the centre
outwards), at distances from the boundary in px."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab, Y
from cmp import de2000

S, L = sys.argv[1], sys.argv[2]
ST = sys.argv[3] if len(sys.argv) > 3 else "Cla+100"
meta = json.load(open(f"{S}/meta.json"))["items"]


def lc(n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); return b[2:].reshape(h, w, 3).astype(np.float64)


ly = lambda img: np.log2(np.maximum(Y(img), 1e-7))
for g in sorted({n.split("__")[0] for n in meta}):
    n, b = f"{g}__{ST}", f"{g}__base"
    if not all(os.path.exists(p) for p in (f"{S}/acr/{n}.tif", f"{S}/acr/{b}.tif", f"{S}/{L}/{n}.f32", f"{S}/{L}/{b}.f32")):
        continue
    A, A0, B, B0 = acr(S, n), acr(S, b), lc(n), lc(b)
    de = de2000(lab(A.reshape(-1, 3)), lab(B.reshape(-1, 3))).mean()
    ca, cb = ly(A) - ly(A0), ly(B) - ly(B0)
    h, w = ca.shape
    if g.startswith("top"):
        y0 = int(0.4 * h)
        pa, pb = ca[:, w // 2 - 50:w // 2 + 50].mean(1), cb[:, w // 2 - 50:w // 2 + 50].mean(1)
        pts = [(y0 - d, d) for d in (270, 150, 80, 40, 15, 3)] + [(y0 + d, -d) for d in (0, 3, 15, 40, 80, 150, 250, 400)]
        pts = [(p, d) for p, d in pts if 0 <= p < h]
    elif g.startswith("left"):
        x0 = w // 2
        pa, pb = ca[h // 2 - 50:h // 2 + 50].mean(0), cb[h // 2 - 50:h // 2 + 50].mean(0)
        pts = [(x0 - d, d) for d in (500, 300, 150, 80, 40, 15, 3)] + [(x0 + d, -d) for d in (0, 3, 15, 40, 80, 150, 300, 500)]
        pts = [(p, d) for p, d in pts if 0 <= p < w]
    else:
        pa, pb = ca[h // 2 - 3:h // 2 + 3].mean(0), cb[h // 2 - 3:h // 2 + 3].mean(0)
        pts = [(w // 2 + d, d) for d in (0, 20, 50, 90, 100, 110, 150, 200, 300, 500) if w // 2 + d < w]
    print(f"{g:12s} dE {de:5.2f}  " + " ".join(f"{d:+d}:{pa[p]:+.2f}/{pb[p]:+.2f}" for p, d in pts))
