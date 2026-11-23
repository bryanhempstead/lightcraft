"""clafit13.py SET[,SET…] [AMOUNT…]: Clarity's regional move (round 13). Like `clafit.py` (Camera Raw's change of scene
log2 luminance against LightCraft's dumped terms, `LIGHTCRAFT_CLARITY_DUMP=SET/dump/<scene>.bin`), with extra
candidate features computed here from the dumped level grid: large-scale detail without a range limit
(`l − Gauss_σ(l)`, σ in EXTRA as fractions of the long edge), each by the base level's knots. Leave-one-scene-out:
for every scene, the RMS of the current weights, a refit of the five rows, and a refit with the extra rows."""
import sys, os, glob, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, Y


def load(path):
    b = np.fromfile(path, "<f4")
    return int(b[0]), int(b[1]), b[2:].reshape(-1, 48)


def tone_inverse(l, yb):
    """the global map l -> log2 Y of Camera Raw's base render (binned medians, monotone), as `clafit.py`"""
    lo, hi = np.percentile(l, 0.5), np.percentile(l, 99.5)
    edges = np.arange(lo, hi + 0.1, 0.1)
    c, v = [], []
    for a, b in zip(edges[:-1], edges[1:]):
        m = (l >= a) & (l < b)
        if m.sum() >= 20:
            c.append((a + b) / 2)
            v.append(np.median(yb[m]))
    return np.array(c), np.maximum.accumulate(np.array(v))

SETS = sys.argv[1].split(",")
AMOUNTS = sys.argv[2:] or ["Cla+50", "Cla+100"]
EXTRA = [float(v) for v in os.environ.get("EXTRA", "0.15,0.35").split(",") if v]
KN = 9
CUR = {}
src = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "crates", "pipeline", "src", "scene_cr.rs")).read()
for name, a in (("FINE_M100", "Cla-100"), ("FINE_M50", "Cla-50"), ("FINE_50", "Cla+50"), ("FINE_100", "Cla+100")):
    blk = src.split(f"const {name}: [[f32; KNOTS]; 5] = [")[1].split("];")[0]
    CUR[a] = np.array([[float(x) for x in r.strip(" []\n,").split(",")] for r in blk.split("],") if r.strip(" []\n,")])


def gauss(img, s):
    from numpy.fft import rfft2, irfft2
    h, w = img.shape
    p = int(3 * s) + 1
    pad = np.pad(img, p, mode="reflect")
    H, Wd = pad.shape
    fy = np.fft.fftfreq(H)[:, None]
    fx = np.fft.rfftfreq(Wd)[None, :]
    k = np.exp(-2 * (np.pi * s) ** 2 * (fx ** 2 + fy ** 2))
    return irfft2(rfft2(pad) * k, s=pad.shape)[p:p + h, p:p + w]


def features(path):
    w, h, v = load(path)
    xs, ys, l, T = v[:, 0].astype(int), v[:, 1].astype(int), v[:, 2], v[:, 3:]
    gx, gy = xs // 4, ys // 4
    grid = np.full((gy.max() + 1, gx.max() + 1), np.nan)
    grid[gy, gx] = l
    grid = np.where(np.isnan(grid), np.nanmean(grid), grid)
    hats = T[:, 36:45]
    hs = hats / np.maximum(hats.sum(1, keepdims=True), 1e-9)
    ex = []
    for s in EXTRA:
        g = gauss(grid, s * max(w, h) / 4)
        d = l - g[gy, gx]
        ex.append(hs * d[:, None])
    return xs, ys, l, T, (np.concatenate(ex, 1) if ex else np.zeros((len(l), 0)))


data = {a: [] for a in AMOUNTS}
for S in SETS:
    for f in sorted(glob.glob(f"{S}/dump/*.bin")):
        sc = os.path.basename(f)[:-4]
        xs, ys, l, T, E = features(f)
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
            lp = np.interp(yc, g, c)
            ok &= np.interp(lp, c, slope) > 0.25
            lb = np.interp(yb, g, c)
            data[a].append((os.path.basename(S) + ":" + sc, T[ok], E[ok], (lp - lb)[ok]))


def fit(rows, W0, lam_r=float(os.environ.get("LAMR", "2e-3")), lam_s=float(os.environ.get("LAMS", "1e-3"))):
    X = np.concatenate([r[0] for r in rows])
    y = np.concatenate([r[1] for r in rows])
    wt = np.concatenate([np.full(len(r[1]), 1.0 / len(r[1])) for r in rows])
    sw = np.sqrt(wt)[:, None]
    NF = X.shape[1]
    L = []
    for s_ in range(NF // KN):
        for k in range(KN - 1):
            r = np.zeros(NF); r[s_ * KN + k] = 1; r[s_ * KN + k + 1] = -1; L.append(r)
    L = np.array(L)
    A = np.vstack([X * sw, np.sqrt(lam_s) * L, np.sqrt(lam_r) * np.eye(NF)])
    b = np.concatenate([(y - X @ W0) * sw[:, 0], -np.sqrt(lam_s) * (L @ W0), np.zeros(NF)])
    return W0 + np.linalg.lstsq(A, b, rcond=None)[0]


rms = lambda p, y: float(np.sqrt(np.mean((p - y) ** 2)))
for a in AMOUNTS:
    rows = data[a]
    W5 = CUR[a].ravel()
    W7 = np.concatenate([W5, np.zeros(rows[0][2].shape[1])])
    print(f"== {a}  ({len(rows)} scenes; extra σ {EXTRA})")
    print(f"{'scene':28s} {'current':>8s} {'refit5':>8s} {'+extra':>8s}")
    tot = np.zeros(3)
    for i, (sc, T, E, y) in enumerate(rows):
        tr = [r for j, r in enumerate(rows) if j != i]
        w5 = fit([(r[1], r[3]) for r in tr], W5)
        w7 = fit([(np.concatenate([r[1], r[2]], 1), r[3]) for r in tr], W7)
        v = (rms(T @ W5, y), rms(T @ w5, y), rms(np.concatenate([T, E], 1) @ w7, y))
        tot += v
        print(f"{sc:28s} {v[0]:8.3f} {v[1]:8.3f} {v[2]:8.3f}")
    print(f"{'mean':28s} " + " ".join(f"{x / len(rows):8.3f}" for x in tot))
    wall = fit([(np.concatenate([r[1], r[2]], 1), r[3]) for r in rows], W7)
    json.dump({"weights": wall.reshape(-1, KN).tolist(), "extra": EXTRA}, open(f"{SETS[0]}/clafit13-{a}.json", "w"))
    print("all-scene fit:\n" + "\n".join("    [" + ", ".join(f"{x:.4f}" for x in r) + "]," for r in wall.reshape(-1, KN)))
