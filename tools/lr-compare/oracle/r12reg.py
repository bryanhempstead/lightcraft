"""r12reg.py SET LCDIR NAME [REF]: a setting's change of L* (NAME − REF, REF default `<scene>__base`), Camera Raw vs
LightCraft, split into a regional part (64 px block means) and the detail left; per row of blocks the mean regional
change, and the RMS of each part's error. Shows whether a local operator's misses are regional moves or detail."""
import sys, os, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab

S, L, N = sys.argv[1], sys.argv[2], sys.argv[3]
REF = sys.argv[4] if len(sys.argv) > 4 else N.split("__")[0] + "__base"


def lc(n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); return b[2:].reshape(h, w, 3).astype(np.float64)


def block(x, k=64):
    h, w = (x.shape[0] // k) * k, (x.shape[1] // k) * k
    return x[:h, :w].reshape(h // k, k, w // k, k).mean((1, 3))


Lc = lambda img: lab(img)[..., 0]
a, a0, b, b0 = Lc(acr(S, N)), Lc(acr(S, REF)), Lc(lc(N)), Lc(lc(REF))
ca, cb = a - a0, b - b0
ra, rb, base = block(ca), block(cb), block(a0)
print("rows: base L* / CR regional change / LC regional change (block means across the row)")
for i in range(ra.shape[0]):
    print(f"{i:2d} " + " ".join(f"{base[i, j]:3.0f}:{ra[i, j]:+5.1f}/{rb[i, j]:+5.1f}" for j in range(0, ra.shape[1], max(1, ra.shape[1] // 8))))
up = lambda r, shape: np.kron(r, np.ones((64, 64)))[:shape[0] // 64 * 64, :shape[1] // 64 * 64]
h, w = ra.shape[0] * 64, ra.shape[1] * 64
fa, fb = ca[:h, :w] - up(ra, ca.shape), cb[:h, :w] - up(rb, cb.shape)
print(f"regional RMS error {np.sqrt(((ra - rb) ** 2).mean()):.2f} L*, detail RMS error {np.sqrt(((fa - fb) ** 2).mean()):.2f} L*;"
      f" CR regional RMS {np.sqrt((ra ** 2).mean()):.2f}, detail RMS CR {np.sqrt((fa ** 2).mean()):.2f} LC {np.sqrt((fb ** 2).mean()):.2f}")
