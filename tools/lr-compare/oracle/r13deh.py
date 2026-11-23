"""r13deh.py SET LCDIR [SETTING…]: negative Dehaze's spatial part on `make_r13.py deh` (round 13). Per scene, Camera Raw
vs LightCraft at each SETTING (default Deh-25 Deh-50 Deh-100) against the scene's base render, read back to scene
level through Camera Raw's own base (each scene's flat / grating levels): the mean change of log2 Y and the contrast
factor (std ratio) for gratings / checkers / flats, and the change's profile across the edge and the lines; plus
ΔE2000 of the render."""
import sys, os, json, re, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab, Y
from cmp import de2000

S, L = sys.argv[1], sys.argv[2]
SETS = sys.argv[3:] or ["Deh-25", "Deh-50", "Deh-100"]
meta = json.load(open(f"{S}/meta.json"))["items"]


def lc(n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); return b[2:].reshape(h, w, 3).astype(np.float64)


def ly(img):
    return np.log2(np.maximum(Y(img), 1e-7))


scenes = sorted({n.split("__")[0] for n in meta}, key=lambda g: (g[0], int(re.sub(r"\D.*", "", g[1:]) or 0) if g[1:2].isdigit() else 0, g))
for st in SETS:
    print(f"== {st}: per scene  CR dmean / LC dmean   CR k / LC k   dE")
    des = []
    for g in scenes:
        n, b = f"{g}__{st}", f"{g}__base"
        if not all(os.path.exists(p) for p in (f"{S}/acr/{n}.tif", f"{S}/acr/{b}.tif", f"{S}/{L}/{n}.f32", f"{S}/{L}/{b}.f32")):
            continue
        A, A0, B, B0 = acr(S, n), acr(S, b), lc(n), lc(b)
        de = de2000(lab(A.reshape(-1, 3)), lab(B.reshape(-1, 3))).mean()
        des.append(de)
        h, w = A.shape[:2]
        if g in ("edge", "lineb", "lined"):
            # column profile of the change (rows averaged), Camera Raw and LightCraft
            ca = (ly(A) - ly(A0))[h // 4:3 * h // 4].mean(0)
            cb = (ly(B) - ly(B0))[h // 4:3 * h // 4].mean(0)
            if g == "edge":
                xs = [w // 2 - d for d in (200, 80, 40, 20, 10, 5, 2, 1)] + [w // 2 + d for d in (0, 1, 4, 9, 19, 39, 79, 199)]
            else:
                xs = [o + 128 + 256 * i for i in range(4) for o in (-20, -3, 0, (1, 3, 9, 27)[i] // 2, (1, 3, 9, 27)[i] + 3)]
            print(f"  {g:14s} dE {de:5.2f}  x: " + " ".join(f"{x}:{ca[x]:+.2f}/{cb[x]:+.2f}" for x in xs))
            continue
        c = (slice(h // 4, 3 * h // 4), slice(w // 8, 7 * w // 8))
        la, la0, lb, lb0 = ly(A[c]), ly(A0[c]), ly(B[c]), ly(B0[c])
        ka = la.std() / max(la0.std(), 1e-6)
        kb = lb.std() / max(lb0.std(), 1e-6)
        print(f"  {g:14s} {la.mean() - la0.mean():+6.3f} / {lb.mean() - lb0.mean():+6.3f}   {ka:5.2f} / {kb:5.2f}   {de:5.2f}")
    print(f"  mean dE {np.mean(des):.3f} ({len(des)})")
