"""nr_real_ana.py SET [LCDIR]: noise left by noise reduction on make_nr_real.py's raws.

Flat tiles (32 px) are picked once per photo from Camera Raw's no-NR render: the third of tiles
whose smoothed (8 px box) image varies least. In each, the high-pass (pixel − 3×3 box mean)
standard deviation of L* and of (a*, b*); per setting, the mean over the tiles as a ratio to the
same renderer's no-NR render. Camera Raw: SET/acr/*.tif; LightCraft: SET/LCDIR/*.f32."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

SET = sys.argv[1]
LCD = sys.argv[2] if len(sys.argv) > 2 else "lc"
meta = json.load(open(f"{SET}/meta.json"))
T = 32


def load(kind, name):
    if kind == "acr":
        return common.lab(common.acr(SET, name))
    b = np.fromfile(f"{SET}/{LCD}/{name}.f32", "<f4")
    w, h = b[:2].view("<u4")
    return common.lab(np.clip(b[2:].reshape(h, w, 3).astype(np.float64), 0, None))


def box(a, k):
    p = np.pad(a, ((k, k), (k, k)) + ((0, 0),) * (a.ndim - 2), mode="edge")
    c = np.cumsum(np.cumsum(p, 0), 1)
    c = np.pad(c, ((1, 0), (1, 0)) + ((0, 0),) * (a.ndim - 2))
    n = 2 * k + 1
    return (c[n:, n:] - c[:-n, n:] - c[n:, :-n] + c[:-n, :-n]) / (n * n)


def tiles_of(lab):
    h, w = lab.shape[:2]
    sm = box(lab[..., 0], 4)
    out = []
    for y in range(T, h - 2 * T, T):
        for x in range(T, w - 2 * T, T):
            t = sm[y:y + T, x:x + T]
            if 15 < t.mean() < 90:
                out.append((t.std(), y, x))
    out.sort()
    return [(y, x) for _, y, x in out[: max(len(out) // 3, 1)]]


def noise(lab, tiles):
    hp = lab - box(lab, 1)
    l = np.mean([hp[y:y + T, x:x + T, 0].std() for y, x in tiles])
    c = np.mean([np.hypot(hp[y:y + T, x:x + T, 1].std(), hp[y:y + T, x:x + T, 2].std()) for y, x in tiles])
    return l, c


files = sorted({v["file"] for v in meta["items"].values()})
err = []
for f in files:
    stem = os.path.splitext(f)[0]
    names = sorted(n for n, v in meta["items"].items() if v["file"] == f)
    have = {k: os.path.exists(f"{SET}/acr/{stem}__n0.tif") if k == "acr" else os.path.exists(f"{SET}/{LCD}/{stem}__n0.f32") for k in ("acr", "lc")}
    if not all(have.values()):
        print(f, "missing renders", have)
        continue
    base = {k: load(k, f"{stem}__n0") for k in ("acr", "lc")}
    tiles = tiles_of(base["acr"])
    b = {k: noise(base[k], tiles) for k in base}
    print(f"{f}: {len(tiles)} flat tiles; no-NR noise L* / chroma: acr {b['acr'][0]:.2f} / {b['acr'][1]:.2f}, lc {b['lc'][0]:.2f} / {b['lc'][1]:.2f}")
    for n in names:
        if n.endswith("__n0"):
            continue
        r = {k: noise(load(k, n), tiles) for k in ("acr", "lc")}
        ra = (r["acr"][0] / b["acr"][0], r["acr"][1] / b["acr"][1])
        rl = (r["lc"][0] / b["lc"][0], r["lc"][1] / b["lc"][1])
        # what is seen: the noise left in absolute terms (the two demosaics start apart)
        err += [np.log(r["lc"][0] / r["acr"][0]), np.log(r["lc"][1] / r["acr"][1])]
        print(f"  {n.split('__')[1]:<5} L left acr {ra[0]:.3f} lc {rl[0]:.3f} | C left acr {ra[1]:.3f} lc {rl[1]:.3f} | absolute L* {r['acr'][0]:.2f} / {r['lc'][0]:.2f}, chroma {r['acr'][1]:.2f} / {r['lc'][1]:.2f}")
if err:
    print(f"mean |log ratio| of the absolute noise left (with NR): {np.mean(np.abs(err)):.3f}")
