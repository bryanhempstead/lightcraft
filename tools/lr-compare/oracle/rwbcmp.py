import sys, os, json, numpy as np
from common import lab, acr, lc
from cmp import de2000
S = sys.argv[1]; meta = json.load(open(f"{S}/meta.json"))["items"]
def small(a, n=48):
    h, w, _ = a.shape; f = max(h, w) // n; return a[:h // f * f, :w // f * f].reshape(h // f, f, w // f, f, 3).mean((1, 3))
for name in sorted(meta):
    if not os.path.exists(f"{S}/acr/{name}.tif") or not os.path.exists(f"{S}/lc/{name}.f32"): continue
    A, B = small(acr(S, name)), small(lc(S, name))
    if A.shape != B.shape:
        h, w = min(A.shape[0], B.shape[0]), min(A.shape[1], B.shape[1]); A, B = A[:h, :w], B[:h, :w]
    la, lb = lab(A), lab(B); d = de2000(la, lb)
    print(f"{name:28s} dE {d.mean():5.2f} acr {la.reshape(-1,3).mean(0).round(1)} lc {lb.reshape(-1,3).mean(0).round(1)}")
