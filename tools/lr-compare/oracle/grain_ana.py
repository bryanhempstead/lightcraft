"""grain_ana.py SET [LCDIR=lc]: grain statistics of the make_detail.py flats, Camera Raw vs LightCraft.

Per flat image and band: the grain field = render − the same scene rendered without grain (L*),
its strength (sd, L*), mean shift, lag-1 autocorrelation and the strength left after a 2×2 box
downscale (1/2 = uncorrelated pixels, 1 = smooth). Prints one row per setting and a summary of
LightCraft / Camera Raw ratios."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

SET = sys.argv[1]
LCD = sys.argv[2] if len(sys.argv) > 2 else "lc"
meta = json.load(open(f"{SET}/meta.json"))


def lstar(pp):
    return common.lab(pp)[..., 0]


def load(kind, name):
    if kind == "acr":
        return lstar(common.acr(SET, name))
    b = np.fromfile(f"{SET}/{LCD}/{name}.f32", "<f4")
    w, h = b[:2].view("<u4")
    return lstar(b[2:].reshape(h, w, 3).astype(np.float64))


def stats(g):
    sd = g.std()
    a = g - g.mean()
    r1 = 0.5 * ((a[:, 1:] * a[:, :-1]).mean() + (a[1:] * a[:-1]).mean()) / max(a.var(), 1e-12)
    h, w = (a.shape[0] // 2) * 2, (a.shape[1] // 2) * 2
    b = a[:h, :w].reshape(h // 2, 2, w // 2, 2).mean((1, 3))
    return sd, g.mean(), r1, b.std() / max(sd, 1e-12)


rows = []
for img in sorted({v["image"] for v in meta["items"].values() if v["image"].startswith("flat")}):
    names = [n for n, v in meta["items"].items() if v["image"] == img]
    base = {k: load(k, f"{img}__g0") for k in ("acr", "lc")}
    h = base["acr"].shape[0]
    for n in names:
        if n.endswith("__g0"):
            continue
        out = {"name": n, "bands": []}
        for k in ("acr", "lc"):
            img_k = load(k, n)
            g = img_k - base[k]
            for bi in range(5):
                y0, y1 = bi * h // 5 + 16, (bi + 1) * h // 5 - 16
                sl = g[y0:y1, 16:-16]
                out.setdefault(k, []).append((float(base[k][y0:y1, 16:-16].mean()),) + tuple(float(x) for x in stats(sl)))
        rows.append(out)

print(f"{'setting':<26} band   L*acr  L*lc |  sd acr  sd lc  ratio | shift acr  lc | rho1 acr  lc | box2 acr  lc")
ratios = []
for r in rows:
    for bi in range(5):
        a, l = r["acr"][bi], r["lc"][bi]
        ratios.append((r["name"], bi, l[1] / max(a[1], 1e-9), l[3] - a[3], l[4] - a[4]))
        print(f"{r['name']:<26} {bi}   {a[0]:6.1f} {l[0]:6.1f} | {a[1]:6.3f} {l[1]:6.3f} {l[1] / max(a[1], 1e-9):5.2f} | {a[2]:+6.3f} {l[2]:+6.3f} | {a[3]:5.3f} {l[3]:5.3f} | {a[4]:5.3f} {l[4]:5.3f}")
rs = np.array([x[2] for x in ratios])
print(f"\nstrength ratio LC/ACR: median {np.median(rs):.3f}, mean |log| {np.mean(np.abs(np.log(rs))):.3f}; rho1 diff mean {np.mean([x[3] for x in ratios]):+.3f}; box2 diff mean {np.mean([x[4] for x in ratios]):+.3f}")
json.dump(rows, open(f"{SET}/grain-{LCD}.json", "w"))
