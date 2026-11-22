"""Fit LightCraft's sharpening operator (log-luminance unsharp mask with Detail damping and Masking)
to Camera Raw's renders of make_detail.py's `edges` scene. Usage: [GSIG=1.6] [ITS=6] sharpfit.py SET"""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common
SET = sys.argv[1]
meta = json.load(open(f"{SET}/meta.json"))

# the scene (same generator as make_detail.edges)
def edges(w=1024):
    h = 512
    img = np.full((h, w), 2.0 ** -3.5)
    for k, (a, b) in enumerate([(-3.8, -3.2), (-4.5, -2.5), (-7.0, -1.0)]):
        x0 = 64 + k * 320
        img[0:160, x0:x0 + 128] = 2.0 ** a
        img[0:160, x0 + 128:x0 + 256] = 2.0 ** b
    for k, (ev, wd) in enumerate([(-6.0, 1), (-1.5, 1), (-6.0, 3)]):
        x0 = 160 + k * 320
        img[176:256, x0:x0 + wd] = 2.0 ** ev
    x = np.arange(w)
    for k, per in enumerate([3.0, 6.0, 12.0]):
        x0, x1 = 32 + k * 330, 32 + k * 330 + 300
        img[272:384, x0:x1] = (2.0 ** (-3.5 + 0.6 * np.sin(2 * np.pi * x[x0:x1] / per)))[None, :]
    rng = np.random.default_rng(7)
    for k, sd in enumerate([0.05, 0.15, 0.4]):
        x0 = 32 + k * 330
        img[400:512, x0:x0 + 300] = 2.0 ** (-3.5 + sd * rng.standard_normal((112, 300)))
    return img

EV = np.log2(edges())
acr = {n.split("__")[1]: common.lab(common.acr(SET, n))[..., 0] for n, v in meta["items"].items() if v["image"] == "edges"}
base = acr["s0"]
# ACR's tone: scene EV -> L* (from the unsharpened render)
order = np.argsort(EV.ravel())
ev_s, l_s = EV.ravel()[order], base.ravel()[order]
bins = np.linspace(ev_s[0], ev_s[-1], 400)
idx = np.clip(np.searchsorted(ev_s, bins), 0, len(ev_s) - 1)
lut_l = np.array([l_s[max(i - 50, 0):i + 50].mean() for i in idx])
def tone(ev):
    return np.interp(ev, bins, lut_l)
print("tone fit residual (L*):", np.abs(tone(EV) - base).mean())

def gauss_blur(a, sigma, taps=4):
    r = int(min(max(np.ceil(2.5 * sigma), 1), taps))
    k = np.exp(-np.arange(-r, r + 1) ** 2 / (2 * sigma * sigma))
    k /= k.sum()
    p = np.pad(a, r, mode="edge")
    t = sum(k[i] * p[:, i:i + a.shape[1]] for i in range(2 * r + 1))[r:-r]
    return sum(k[i] * np.pad(t, ((r, r), (0, 0)), mode="edge")[i:i + a.shape[0]] for i in range(2 * r + 1))

def range3(a):
    p = np.pad(a, 1, mode="edge")
    st = np.stack([p[i:i + a.shape[0], j:j + a.shape[1]] for i in range(3) for j in range(3)])
    return st.max(0) - st.min(0)

def smooth(e0, e1, x):
    t = np.clip((x - e0) / max(e1 - e0, 1e-9), 0, 1)
    return t * t * (3 - 2 * t)

MODEL = os.environ.get("MODEL", "grad")
GSIG = float(os.environ.get("GSIG", "0"))
def ls_grad(a, sigma, taps=4):
    r = int(min(max(np.ceil(2.5 * sigma), 1), taps))
    k = np.exp(-np.arange(-r, r + 1) ** 2 / (2 * sigma * sigma))
    d = np.arange(-r, r + 1)
    p = np.pad(a, r, mode="edge")
    H, W = a.shape
    gx = sum(k[j] * k[i] * d[i] * p[j:j + H, i:i + W] for j in range(2 * r + 1) for i in range(2 * r + 1))
    gy = sum(k[j] * k[i] * d[j] * p[j:j + H, i:i + W] for j in range(2 * r + 1) for i in range(2 * r + 1))
    n = sum(k[j] * k[i] * d[i] ** 2 for j in range(2 * r + 1) for i in range(2 * r + 1))
    return np.hypot(gx, gy) / n
GCACHE = {}
def sharpen(ev, amount, radius, detail, masking, P):
    gain, gexp, sig, t0, t1, ma, mb, taps = P
    sp = gain * (amount / 150.0) ** gexp
    sigma = sig * np.clip(radius, 0.5, 3.0)
    blur = gauss_blur(ev, sigma, int(taps))
    det = ev - blur
    t = t0 + t1 * detail / 100.0
    det = det / (1 + np.abs(det) / t)
    if masking <= 0:
        m = 1.0
    elif MODEL == "range":
        m = smooth(masking / 100 * ma, masking / 100 * ma + mb, range3(ev))
    else:
        sg = GSIG if GSIG > 0 else sigma
        key = round(sg, 4)
        if key not in GCACHE:
            GCACHE[key] = ls_grad(ev, sg, 4)
        c = 0.0075 * np.exp(ma * masking / 100.0)
        m = smooth(c * (1 - mb), c * (1 + mb), GCACHE[key])
    return ev + sp * det * m

def setting(name):
    a, r, d, m = name[1:].split("_")
    return float(a), float(r), float(d), float(m)

ROWS = np.r_[0:160, 176:256, 272:384, 400:512]
def loss(P, names):
    e = 0.0
    for n in names:
        sim = tone(sharpen(EV, *setting(n), P))
        d = (sim - acr[n])[ROWS, 16:-16]
        e += (d * d).mean()
    return e / len(names)

names = sorted(k for k in acr if k != "s0")
held = ["s126_1.0_25_94", "s60_1.2_35_50", "s126_1.0_25_60"]
fit = [n for n in names if n not in held]
LC = [1.0, 1.0, 0.8, 0.04, 0.5, 0.12, 0.08, 4]
if __name__ == "__main__":
    print("LightCraft today: fit %.3f held %.3f" % (loss(LC, fit), loss(LC, held)))
    P = [7.6375, 0.7725, 0.8428, 0.0013, 0.4172, 5.0187, 1.25, 4] if MODEL == "grad" else [3.0, 1.0, 0.8, 0.04, 0.5, 0.12, 0.08, 4]
    steps = [0.5, 0.1, 0.05, 0.01, 0.05, 0.3, 0.1, 0] if MODEL == "grad" else [0.5, 0.2, 0.1, 0.02, 0.1, 0.03, 0.03, 0]
    best = loss(P, fit)
    for it in range(int(os.environ.get("ITS", "6"))):
        for i in range(7):
            for sgn in (1, -1):
                while True:
                    Q = list(P)
                    Q[i] = max(Q[i] + sgn * steps[i], 1e-3)
                    l = loss(Q, fit)
                    if l < best - 1e-5:
                        P, best = Q, l
                    else:
                        break
        steps = [s * 0.5 for s in steps]
        print(it, "fit %.4f" % best, ["%.4f" % p for p in P])
    print("held-out: %.4f (LightCraft today %.4f)" % (loss(P, held), loss(LC, held)))
    for n in names:
        print(n, "today %.3f fitted %.3f" % (loss(LC, [n]), loss(P, [n])))
