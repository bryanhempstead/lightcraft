"""r12sum.py SET LCDIR [LCDIR…]: Camera Raw vs several LightCraft render folders of a real-raw set side by side: per
setting, mean ΔE2000 over Canon R6 photos and over the others, and the mean |ΔL*| error of the setting's *effect*
(setting − its reference: `base`, or `own-noHS` / `full-noHS` / `tone-noHS`)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000

S, dirs = sys.argv[1], sys.argv[2:]
meta = json.load(open(f"{S}/meta.json"))["items"]
REF = {"own": "own-noHS", "full": "full-noHS", "tone": "tone-noHS"}
cache = {}


def get(L, n):
    k = (L, n)
    if k not in cache:
        cache[k] = None
        if os.path.exists(f"{S}/acr/{n}.tif") and os.path.exists(f"{S}/{L}/{n}.f32"):
            a = cache.get(("acr", n))
            if a is None:
                a = lab(acr(S, n)[::2, ::2].reshape(-1, 3)); cache[("acr", n)] = a
            b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); b = b[2:].reshape(h, w, 3).astype(np.float64)
            if b[::2, ::2].reshape(-1, 3).shape[0] == a.shape[0]:
                cache[k] = lab(b[::2, ::2].reshape(-1, 3))
    return cache[k]


rows = {}
for n, it in sorted(meta.items()):
    p = it.get("photo")
    if not p:
        continue
    kind = n.split("__")[1]
    grp = "R6" if "R6" in it.get("camera", "") else "other"
    ref = REF.get(kind, "base" if kind != "base" and not kind.endswith("noHS") else None)
    for L in dirs:
        b = get(L, n)
        if b is None:
            continue
        a = cache[("acr", n)]
        r = rows.setdefault((kind, grp), {}).setdefault(L, [[], []])
        r[0].append(de2000(a, b).mean())
        if ref:
            b0, a0 = get(L, f"{p}__{ref}"), cache.get(("acr", f"{p}__{ref}"))
            if b0 is not None and a0 is not None:
                r[1].append(np.abs((b[:, 0] - b0[:, 0]) - (a[:, 0] - a0[:, 0])).mean())
print(f"{'setting':16s} {'grp':5s} " + " ".join(f"{L[:14]:>15s}" for L in dirs) + "   (dE / effect err, n)")
tot = {L: [] for L in dirs}
for (kind, grp), d in sorted(rows.items()):
    cells = []
    for L in dirs:
        de, ef = d.get(L, [[], []])
        cells.append(f"{np.mean(de):5.2f}/{np.mean(ef):4.2f}" if de and ef else (f"{np.mean(de):5.2f}/  . " if de else "      .   "))
        tot[L] += de
    n = len(d.get(dirs[0], [[]])[0])
    print(f"{kind:16s} {grp:5s} " + " ".join(f"{c:>15s}" for c in cells) + f"   {n}")
print(f"{'all':22s}" + " ".join(f"{np.mean(v):15.3f}" for v in tot.values()))
