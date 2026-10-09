"""gcmp.py SET [filter] [lcdir]: ΔE2000 on valid, unclipped grid patches"""
import sys, os, json, numpy as np
from common import lab
from cmp import de2000
S = sys.argv[1]; f = sys.argv[2] if len(sys.argv) > 2 else ""; K = sys.argv[3] if len(sys.argv) > 3 else "p_lc"
m = json.load(open(f"{S}/meta.json")); v = np.array(m["valid"]); s = np.array(m["cells"]); sat = 1 - s.min(1) / np.maximum(s.max(1), 1e-12)
tot = []
for n in sorted(x[:-4] for x in os.listdir(f"{S}/p_acr") if f in x):
    if not os.path.exists(f"{S}/{K}/{n}.npy"): continue
    a, b = np.load(f"{S}/p_acr/{n}.npy"), np.load(f"{S}/{K}/{n}.npy"); ok = v & (a.max(1) < .99) & (np.log2(s.max(1)) > -7.5)
    d = de2000(lab(a), lab(b)); tot.append(d[ok].mean())
    print(f"{n:34s} dE {d[ok].mean():5.2f} p95 {np.percentile(d[ok], 95):5.2f}  sat<0.7 {d[ok & (sat < .7)].mean():5.2f}  dL {(lab(b) - lab(a))[ok, 0].mean():+5.2f}")
print("mean", round(float(np.mean(tot)), 2))
