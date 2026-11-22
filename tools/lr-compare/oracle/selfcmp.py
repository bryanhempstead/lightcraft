"""selfcmp.py SET KIND A B [A B …] | SET KIND --vs-base FILTER: ΔE2000 between two renders of the same
renderer (KIND = acr or a LightCraft folder) — did a setting change anything, is a local slider in a
full-frame mask the same as the global one?"""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000

S, K = sys.argv[1], sys.argv[2]
meta = json.load(open(f"{S}/meta.json"))


def load(n):
    if K == "acr":
        return acr(S, n)
    b = np.fromfile(f"{S}/{K}/{n}.f32", "<f4"); w, h = b[:2].view("<u4")
    return b[2:].reshape(h, w, 3).astype(np.float64)


if sys.argv[3] == "--vs-base":
    pairs = [(n, n.split("__")[0] + "__base") for n in sorted(meta["items"]) if sys.argv[4] in n and not n.endswith("__base")]
else:
    pairs = list(zip(sys.argv[3::2], sys.argv[4::2]))
for a, b in pairs:
    try:
        d = de2000(lab(load(a).reshape(-1, 3)), lab(load(b).reshape(-1, 3)))
    except FileNotFoundError:
        continue
    print(f"{a:28s} vs {b:24s} dE {d.mean():6.2f} p95 {np.percentile(d, 95):6.2f} max {d.max():6.2f}")
