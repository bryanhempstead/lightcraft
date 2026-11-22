"""fx2_ana.py SET [LCDIR …] [--json=OUT]: the single-grating images of `make_fx.py … fx2` (`g{P}`,
`g{P}a{A}b{B}`, `G{P}` at 3,072 px): per image and setting, the gain of the grating (log2 luminance
amplitude relative to the renderer's own `base`) and the mean shift in EV — Camera Raw first, then
each LightCraft folder."""
import sys, os, json, re, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, Y

args = [a for a in sys.argv[1:] if not a.startswith("--")]
S, LS = args[0], args[1:]
meta = json.load(open(f"{S}/meta.json"))


def load(kind, name):
    if kind == "acr":
        return acr(S, name) if os.path.exists(f"{S}/acr/{name}.tif") else None
    p = f"{S}/{kind}/{name}.f32"
    if not os.path.exists(p):
        return None
    b = np.fromfile(p, "<f4"); w, h = b[:2].view("<u4")
    return b[2:].reshape(h, w, 3).astype(np.float64)


def fit(img, p):
    lg = np.log2(np.maximum(Y(img), 1e-9))
    h, w = lg.shape
    row = lg[h // 4:3 * h // 4].mean(0)
    x = np.arange(w)
    m = slice(w // 8, w - w // 8)
    A = np.stack([np.sin(2 * np.pi * x[m] / p), np.cos(2 * np.pi * x[m] / p), np.ones(m.stop - m.start)], 1)
    k, *_ = np.linalg.lstsq(A, row[m], rcond=None)
    return k[0], k[2]


def period(scene):
    return float(re.match(r"[gG](\d+)", scene).group(1))


scenes = sorted({n.split("__")[0] for n in meta["items"]}, key=lambda s: (s[0], period(s), s))
sets = sorted({n.split("__")[1] for n in meta["items"]} - {"base"})
res = {}
kinds = ["acr"] + LS
for st in sets:
    print(f"== {st}: gain  " + "  ".join(f"{k:>6s}" for k in kinds) + "  |  mean EV  " + "  ".join(f"{k:>6s}" for k in kinds))
    for sc in scenes:
        n = f"{sc}__{st}"
        if n not in meta["items"]:
            continue
        p = period(sc)
        g, mu = [], []
        for k in kinds:
            a, b = load(k, f"{sc}__base"), load(k, n)
            if a is None or b is None:
                g.append(np.nan); mu.append(np.nan); continue
            (a0, m0), (a1, m1) = fit(a, p), fit(b, p)
            g.append(a1 / a0); mu.append(m1 - m0)
        res[n] = {"gain": g, "mean": mu}
        print(f"{sc:>16s}       " + "  ".join(f"{v:6.3f}" for v in g) + "  |           " + "  ".join(f"{v:6.3f}" for v in mu))
for a in sys.argv[1:]:
    if a.startswith("--json="):
        json.dump({"kinds": kinds, "res": res}, open(a[7:], "w"))
