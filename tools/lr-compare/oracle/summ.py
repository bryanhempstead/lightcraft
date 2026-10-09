import sys, json, numpy as np
from common import *
S = sys.argv[1]; img = sys.argv[2] if len(sys.argv) > 2 else None
meta = json.load(open(f"{S}/meta.json"))["items"]
rows = []
for n, m in meta.items():
    if img and not m['image'].startswith(img): continue
    a, b = lab(acr(S, n)), lab(lc(S, n))
    if a.shape != b.shape: b = b[:a.shape[0], :a.shape[1]]
    d = b - a
    de = np.sqrt((d ** 2).sum(-1))
    rows.append((n, np.abs(d[..., 0]).mean(), d[..., 0].mean(), np.abs(d[..., 1:]).mean(), de.mean(), np.percentile(de, 95)))
for r in rows: print(f"{r[0]:40s} |dL| {r[1]:5.2f} dL {r[2]:+5.2f} |dab| {r[3]:5.2f} dE76 {r[4]:5.2f} p95 {r[5]:5.2f}")
print("mean", np.mean([r[4] for r in rows]).round(2))
