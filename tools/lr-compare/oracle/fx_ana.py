"""fx_ana.py SET [LCDIR] [filter]: the frequency scene of make_fx.py, per band: the gain of the sine
grating (amplitude in log2 luminance, relative to the same renderer's `base`) and the mean shift
(EV), for Camera Raw and (optionally) LightCraft, side by side. `--json OUT` writes the numbers."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, Y

args = [a for a in sys.argv[1:] if not a.startswith("--")]
S = args[0]
L = args[1] if len(args) > 1 else None
f = args[2] if len(args) > 2 else ""
meta = json.load(open(f"{S}/meta.json"))
BAND = meta["band"]


def load(kind, name):
    if kind == "acr":
        return acr(S, name) if os.path.exists(f"{S}/acr/{name}.tif") else None
    p = f"{S}/{kind}/{name}.f32"
    if not os.path.exists(p):
        return None
    b = np.fromfile(p, "<f4"); w, h = b[:2].view("<u4")
    return b[2:].reshape(h, w, 3).astype(np.float64)


def bands(img, sc):
    """per band: (amplitude, phase-locked) of the sine at its period, and the mean log2 level"""
    out = []
    lg = np.log2(np.maximum(Y(img), 1e-9))
    w = lg.shape[1]
    x = np.arange(w)
    for i, (p, a, b) in enumerate(meta["freq_bands"][str(sc)]):
        y0, y1 = i * BAND * sc, (i + 1) * BAND * sc
        row = lg[y0 + BAND * sc // 4:y1 - BAND * sc // 4].mean(0)
        m = slice(w // 8, w - w // 8)  # away from the frame's left / right edges
        s, c = np.sin(2 * np.pi * x[m] / p), np.cos(2 * np.pi * x[m] / p)
        A = np.stack([s, c, np.ones_like(s)], 1)
        k, *_ = np.linalg.lstsq(A, row[m], rcond=None)
        out.append((k[0], k[2]))
    return np.array(out)


res = {}
for sc in (1, 3):
    base = f"freq{sc}__base"
    if base not in meta["items"]:
        continue
    kinds = ["acr"] + ([L] if L else [])
    b0 = {k: load(k, base) for k in kinds}
    labels = [f"{p}/{a}/{b}" for p, a, b in meta["freq_bands"][str(sc)]]
    print(f"== freq{sc}: band period/amp/base; gain (acr | lc), mean shift EV (acr | lc)")
    print(" " * 18 + "".join(f"{l:>13s}" for l in labels))
    for n in sorted(meta["items"]):
        if not n.startswith(f"freq{sc}__") or n == base or f not in n:
            continue
        row = {}
        for k in kinds:
            im = load(k, n)
            if im is None or b0[k] is None:
                continue
            r0, r1 = bands(b0[k], sc), bands(im, sc)
            row[k] = (r1[:, 0] / r0[:, 0], r1[:, 1] - r0[:, 1])
        if "acr" not in row:
            continue
        g = row["acr"][0]
        print(f"{n.split('__')[1]:>10s} gain ac" + "".join(f"{v:13.3f}" for v in g))
        if L in row:
            print(f"{'':>10s} gain lc" + "".join(f"{v:13.3f}" for v in row[L][0]))
        print(f"{'':>10s} mean ac" + "".join(f"{v:13.3f}" for v in row["acr"][1]))
        if L in row:
            print(f"{'':>10s} mean lc" + "".join(f"{v:13.3f}" for v in row[L][1]))
        res[n] = {k: [v[0].tolist(), v[1].tolist()] for k, v in row.items()}
for a in sys.argv[1:]:
    if a.startswith("--json="):
        json.dump(res, open(a[7:], "w"))
