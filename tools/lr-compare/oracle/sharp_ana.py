"""sharp_ana.py SET [LCDIR=lc]: sharpening on make_detail.py's `edges` scene, Camera Raw vs LightCraft.

Per setting (relative to the same scene rendered without sharpening, L*): overshoot at the three
step edges (peak change / step height, both sides) and the halo's width (px where the change is
above 10 % of its peak), the 1 px lines' peak change / line depth, the gain of the 3 / 6 / 12 px
sine gratings (sd ratio) and of the low-contrast noise textures (sd ratio; what edge masking keeps
out)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

SET = sys.argv[1]
LCD = sys.argv[2] if len(sys.argv) > 2 else "lc"
meta = json.load(open(f"{SET}/meta.json"))


def load(kind, name):
    if kind == "acr":
        return common.lab(common.acr(SET, name))[..., 0]
    b = np.fromfile(f"{SET}/{LCD}/{name}.f32", "<f4")
    w, h = b[:2].view("<u4")
    return common.lab(b[2:].reshape(h, w, 3).astype(np.float64))[..., 0]


def measure(img, base):
    d = img - base
    out = {}
    # step edges at x0 + 128 (rows 20..140)
    for k in range(3):
        x = 64 + k * 320 + 128
        p = d[20:140, x - 24:x + 24].mean(0)
        b = base[20:140, x - 24:x + 24].mean(0)
        step = abs(b[-4] - b[4])
        peak = np.abs(p).max()
        width = int((np.abs(p) > 0.1 * peak).sum()) if peak > 1e-3 else 0
        out[f"edge{k}"] = (float(peak / max(step, 1e-6)), width)
    for k in range(2):
        x = 160 + k * 320
        p = d[190:240, x - 6:x + 7].mean(0)
        b = base[190:240, x - 6:x + 7].mean(0)
        depth = abs(b[6] - b[0])
        out[f"line{k}"] = float(np.abs(p).max() / max(depth, 1e-6))
    for k, per in enumerate([3, 6, 12]):
        x0 = 32 + k * 330
        a = img[285:372, x0 + 10:x0 + 290]
        b = base[285:372, x0 + 10:x0 + 290]
        out[f"grat{per}"] = float(a.std(1).mean() / max(b.std(1).mean(), 1e-6))
    for k, sd in enumerate([0.05, 0.15, 0.4]):
        x0 = 32 + k * 330
        a = img[408:504, x0 + 8:x0 + 292]
        b = base[408:504, x0 + 8:x0 + 292]
        out[f"tex{sd}"] = float(a.std() / max(b.std(), 1e-6))
    out["flat"] = float(np.abs(d[260:270, 40:1000]).mean())
    return out


names = sorted(n for n, v in meta["items"].items() if v["image"] == "edges" and not n.endswith("__s0"))
base = {k: load(k, "edges__s0") for k in ("acr", "lc")}
keys = ["edge0", "edge1", "edge2", "line0", "line1", "grat3", "grat6", "grat12", "tex0.05", "tex0.15", "tex0.4"]
print(f"{'setting':<22} " + " ".join(f"{k:>15}" for k in keys))
res = {}
err = []
for n in names:
    m = {k: measure(load(k, n), base[k]) for k in ("acr", "lc")}
    res[n] = m
    cells = []
    for k in keys:
        a, l = m["acr"][k], m["lc"][k]
        if isinstance(a, tuple):
            cells.append(f"{a[0]:.2f}/{l[0]:.2f} w{a[1]}/{l[1]}")
            err.append(np.log(max(l[0], 1e-3) / max(a[0], 1e-3)))
        else:
            cells.append(f"{a:.2f}/{l:.2f}")
            if k.startswith(("grat", "tex")):
                err.append(np.log(max(l - 1 + 1e-3, 1e-3) / max(a - 1 + 1e-3, 1e-3)) if a > 1.02 else 0.0)
            else:
                err.append(np.log(max(l, 1e-3) / max(a, 1e-3)))
    print(f"{n.split('__')[1]:<22} " + " ".join(f"{c:>15}" for c in cells))
print(f"\nmean |log ratio| (edges, lines; gains over 1): {np.mean(np.abs(err)):.3f}")
json.dump(res, open(f"{SET}/sharp-{LCD}.json", "w"))
