"""clafit.py SET DUMPDIR [holdout…]: refit Clarity's weights (`scene_cr`, fine evaluation) per amount from Camera
Raw's renders (round 11). Target: Camera Raw's change of scene log2 luminance (its render read back through its
own base render's global tone map, against LightCraft's scene level); features: LightCraft's terms, dumped with
`LIGHTCRAFT_CLARITY_DUMP=DUMPDIR/<scene>.bin oracle_render …` on one Clarity render per scene of a `make_r11.py`
set. OFFS=1 adds the level row; LAMR (towards RAWmakase's weights) and LAMS (smoothness along the knots) set the
regularization (round 11: OFFS=1 LAMR=5e-2 LAMS=1e-2). Scenes named after DUMPDIR are held out. Writes
DUMPDIR/fit.json."""
import sys, os, glob, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, Y
S, D = sys.argv[1], sys.argv[2]
OFFS = os.environ.get('OFFS') == '1'
hold = set(sys.argv[3:])
ORIG = {
 "Cla-100": [[-0.0272, -0.1927, -0.0764, -0.0689, 0.0068, 0.0567, -0.0718, 0.0036, -0.0100],[-0.0118, -0.1986, -0.1022, -0.3007, -0.5374, -0.5815, -0.5830, -0.4256, -0.0465],[0.0353, 0.0537, 0.0165, 0.0106, 0.0613, -0.0626, -0.0075, 0.0230, 0.0015],[-0.0129, -0.0107, -0.0058, -0.0347, -0.0396, 0.0305, 0.0223, 0.0098, -0.0591]],
 "Cla-50": [[-0.0168, -0.1137, -0.0451, -0.0490, 0.0023, 0.0330, -0.0431, 0.0020, -0.0053],[-0.0064, -0.1204, -0.0596, -0.1772, -0.3246, -0.3499, -0.3455, -0.2508, -0.0234],[0.0213, 0.0354, 0.0092, 0.0078, 0.0372, -0.0391, -0.0068, 0.0141, 0.0001],[-0.0078, -0.0079, -0.0029, -0.0225, -0.0251, 0.0186, 0.0153, 0.0058, -0.0346]],
 "Cla+50": [[0.2292, 0.0713, 0.2070, 0.1321, -0.0268, -0.0161, -0.0212, 0.0981, 0.0086],[0.0056, 0.4016, 0.2260, 0.3029, 0.2107, 0.1972, 0.1403, 0.0193, 0.0556],[0.0158, 0.1688, -0.0357, -0.0904, 0.0172, -0.2251, 0.0159, 0.2423, 0.1087],[0.3042, 0.2691, 0.4679, 0.3970, 0.3296, 0.4703, 0.2079, -0.1511, -0.0771]],
 "Cla+100": [[0.5227, 0.1151, 0.3593, 0.2642, -0.0568, -0.0340, -0.0428, 0.1978, 0.0093],[0.1186, 0.7805, 0.4183, 0.6086, 0.4204, 0.3962, 0.2819, 0.0322, 0.0939],[0.0989, 0.2990, 0.0878, -0.2082, 0.0490, -0.4586, 0.0347, 0.4822, 0.2054],[0.5386, 0.5871, 0.7840, 0.8144, 0.6365, 0.9440, 0.4094, -0.3026, -0.1482]],
}
AMOUNTS = ["Cla-100", "Cla-50", "Cla+50", "Cla+100"]


def load(path):
    b = np.fromfile(path, "<f4")
    w, h = int(b[0]), int(b[1])
    v = b[2:].reshape(-1, 48)
    return w, h, v


def tone_inverse(l, yb):
    """the global map l -> log2 Y of CR's base render (binned medians), returned as (centres, values), monotone"""
    lo, hi = np.percentile(l, 0.5), np.percentile(l, 99.5)
    edges = np.arange(lo, hi + 0.1, 0.1)
    c, v = [], []
    for a, b in zip(edges[:-1], edges[1:]):
        m = (l >= a) & (l < b)
        if m.sum() >= 20:
            c.append((a + b) / 2)
            v.append(np.median(yb[m]))
    c, v = np.array(c), np.maximum.accumulate(np.array(v))
    return c, v


rows = {a: ([], [], []) for a in AMOUNTS}  # X, y, scene
for f in sorted(glob.glob(f"{D}/*.bin")):
    sc = os.path.basename(f)[:-4]
    w, h, v = load(f)
    xs, ys, l, T = v[:, 0].astype(int), v[:, 1].astype(int), v[:, 2], (v[:, 3:] if OFFS else v[:, 3:39])
    yb = np.log2(np.maximum(Y(acr(S, f"{sc}__base"))[ys, xs], 1e-6))
    c, g = tone_inverse(l, yb)
    if len(c) < 10:
        continue
    slope = np.gradient(g, c)
    for a in AMOUNTS:
        if not os.path.exists(f"{S}/acr/{sc}__{a}.tif"):
            continue
        yc = np.log2(np.maximum(Y(acr(S, f"{sc}__{a}"))[ys, xs], 1e-6))
        ok = (yc > g[0] + 0.05) & (yc < g[-1] - 0.05) & (l > c[0]) & (l < c[-1])
        lp = np.interp(yc, g, c)  # CR's scene level after Clarity
        sl = np.interp(lp, c, slope)
        ok &= sl > 0.25
        lb = np.interp(yb, g, c)  # CR's own scene level of the base (absorbs the map's noise)
        d = (lp - lb)[ok]
        rows[a][0].append(T[ok]); rows[a][1].append(d); rows[a][2].extend([sc] * int(ok.sum()))

fits = {}
for a in AMOUNTS:
    X, yv, scn = np.concatenate(rows[a][0]), np.concatenate(rows[a][1]), np.array(rows[a][2])
    tr = ~np.isin(scn, list(hold))
    # balance scenes: weight each scene's pixels to the same total
    wts = np.zeros(len(yv))
    for s_ in set(scn):
        m = scn == s_
        wts[m] = 1.0 / m.sum()
    sw = np.sqrt(wts)[:, None]
    # ridge + smoothness along the knots
    NF = X.shape[1]
    L = []
    for s_ in range(NF // 9):
        for k in range(8):
            r = np.zeros(NF); r[s_ * 9 + k] = 1; r[s_ * 9 + k + 1] = -1; L.append(r)
    L = np.array(L)
    lam_s, lam_r = float(os.environ.get("LAMS", "2e-4")), float(os.environ.get("LAMR", "1e-5"))
    W0f = np.concatenate([np.array(ORIG[a]).ravel(), np.zeros(NF - 36)])
    A = np.vstack([X[tr] * sw[tr], np.sqrt(lam_s) * L, np.sqrt(lam_r) * np.eye(NF)])
    b = np.concatenate([(yv[tr] - X[tr] @ W0f) * sw[tr, 0], -np.sqrt(lam_s) * (L @ W0f), np.zeros(NF)])
    W = W0f + np.linalg.lstsq(A, b, rcond=None)[0]
    fits[a] = W.reshape(NF // 9, 9)
    pred = X @ W
    W0 = np.array(ORIG[a]).ravel()
    p0 = X[:, :36] @ W0
    per0 = {s_: np.sqrt(np.mean((p0[(scn == s_)] - yv[(scn == s_)]) ** 2)) for s_ in sorted(set(scn))}
    print(f"{a} RAWmakase weights: rms per scene " + " ".join(f"{k}:{v:.3f}" for k, v in per0.items()))
    for part, m in (("train", tr), ("hold", ~tr)):
        if m.sum() == 0:
            continue
        per = {s_: np.sqrt(np.mean((pred[(scn == s_)] - yv[(scn == s_)]) ** 2)) for s_ in sorted(set(scn[m]))}
        print(f"{a} {part}: rms per scene " + " ".join(f"{k}:{v:.3f}" for k, v in per.items()))
    print(f"{a} weights:\n" + "\n".join("    [" + ", ".join(f"{x:.4f}" for x in r) + "]," for r in fits[a]))
json.dump({k: v.tolist() for k, v in fits.items()}, open(f"{D}/fit.json", "w"))
