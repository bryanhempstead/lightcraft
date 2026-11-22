"""r12grat.py SET LCDIR [SETTING]: full-frame sine gratings (make_fx.py fx2's `g<period>[a<amp>b<level>]`), Camera Raw vs
LightCraft at SETTING (default Deh-50) against each one's base render: the change of the mean log2 luminance and the
log contrast ratio (std of log2 Y), per grating, plus ΔE2000 of the setting's render."""
import sys, os, json, re, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab, Y
from cmp import de2000

S, L = sys.argv[1], sys.argv[2]
SET = sys.argv[3] if len(sys.argv) > 3 else "Deh-50"
meta = json.load(open(f"{S}/meta.json"))["items"]


def lc(n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); return b[2:].reshape(h, w, 3).astype(np.float64)


def stats(img):
    h, w = img.shape[:2]
    c = img[h // 4:3 * h // 4, w // 8:7 * w // 8]
    l = np.log2(np.maximum(Y(c.reshape(-1, 3)), 1e-6))
    return l.mean(), l.std()


key = lambda g: (int(re.match(r"g(\d+)", g).group(1)), g)
print(f"{'grating':16s} {'CR dmean':>8s} {'LC dmean':>8s} {'CR k':>6s} {'LC k':>6s} {'dE':>6s}")
des = []
for g in sorted({n.split("__")[0] for n in meta if re.match(r"g\d", n)}, key=key):
    n, b = f"{g}__{SET}", f"{g}__base"
    if not all(os.path.exists(p) for p in (f"{S}/acr/{n}.tif", f"{S}/acr/{b}.tif", f"{S}/{L}/{n}.f32", f"{S}/{L}/{b}.f32")):
        continue
    A, A0, B, B0 = acr(S, n), acr(S, b), lc(n), lc(b)
    (am, asd), (am0, asd0) = stats(A), stats(A0)
    (bm, bsd), (bm0, bsd0) = stats(B), stats(B0)
    d = de2000(lab(A.reshape(-1, 3)), lab(B.reshape(-1, 3))).mean()
    des.append(d)
    print(f"{g:16s} {am - am0:+8.3f} {bm - bm0:+8.3f} {asd / asd0:6.3f} {bsd / bsd0:6.3f} {d:6.2f}")
print(f"mean dE {np.mean(des):.3f} ({len(des)})")
