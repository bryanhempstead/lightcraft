"""curvefit.py SET: how Camera Raw applies the point / parametric curve, read off the neutral ramp
(`make_curve.py`): the curve as applied (encoded in, encoded out) against the DNG SDK's spline
(`dng_spline_solver`: C2, natural ends) and a monotone (Fritsch–Carlson) spline, in the sRGB and
gamma-1.8 encodings."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr



def srgb(v):
    v = np.clip(v, 0, 1)
    return np.where(v <= 0.0031308, 12.92 * v, 1.055 * v ** (1 / 2.4) - 0.055)


def g18(v):
    return np.clip(v, 0, 1) ** (1 / 1.8)


def sdk_spline(xs, ys):
    """dng_spline_solver::Solve + Evaluate (DNG SDK 1.7.1), x/y in 0..1."""
    X, Y = list(xs), list(ys)
    n = len(X)
    A = X[1] - X[0]; B = (Y[1] - Y[0]) / A
    Sl = [0.0] * n; Sl[0] = B
    for j in range(2, n):
        C = X[j] - X[j - 1]; D = (Y[j] - Y[j - 1]) / C
        Sl[j - 1] = (B * C + D * A) / (A + C); A, B = C, D
    Sl[n - 1] = 2 * B - Sl[n - 2]; Sl[0] = 2 * Sl[0] - Sl[1]
    if n > 2:
        E, F, G = [0.0] * n, [0.0] * n, [0.0] * n
        F[0] = 0.5; E[n - 1] = 0.5
        G[0] = 0.75 * (Sl[0] + Sl[1]); G[n - 1] = 0.75 * (Sl[n - 2] + Sl[n - 1])
        for j in range(1, n - 1):
            A = (X[j + 1] - X[j - 1]) * 2
            E[j] = (X[j + 1] - X[j]) / A; F[j] = (X[j] - X[j - 1]) / A; G[j] = 1.5 * Sl[j]
        for j in range(1, n):
            A = 1 - F[j - 1] * E[j]
            if j != n - 1: F[j] /= A
            G[j] = (G[j] - G[j - 1] * E[j]) / A
        for j in range(n - 2, -1, -1):
            G[j] = G[j] - F[j] * G[j + 1]
        Sl = G

    def ev(x):
        if x <= X[0]: return Y[0]
        if x >= X[-1]: return Y[-1]
        j = int(np.searchsorted(X, x))
        x0, x1, y0, y1 = X[j - 1], X[j], Y[j - 1], Y[j]
        a = x1 - x0; t = (x - x0) / a; u = 1 - t
        return y0 * u ** 3 + (y0 + Sl[j - 1] * a / 3) * 3 * t * u * u + (y1 - Sl[j] * a / 3) * 3 * t * t * u + y1 * t ** 3
    return np.vectorize(ev)


def monotone(xs, ys):
    xs, ys = np.array(xs, float), np.array(ys, float); n = len(xs)
    d = np.diff(ys) / np.diff(xs)
    m = np.zeros(n); m[0], m[-1] = d[0], d[-1]
    for i in range(1, n - 1):
        m[i] = 0 if d[i - 1] * d[i] <= 0 else (d[i - 1] + d[i]) / 2
    for i in range(n - 1):
        if d[i] == 0: m[i] = m[i + 1] = 0; continue
        a, b = m[i] / d[i], m[i + 1] / d[i]; s = a * a + b * b
        if s > 9: t = 3 / np.sqrt(s); m[i], m[i + 1] = t * a * d[i], t * b * d[i]

    def ev(x):
        if x <= xs[0]: return ys[0]
        if x >= xs[-1]: return ys[-1]
        j = int(np.searchsorted(xs, x)); h = xs[j] - xs[j - 1]; t = (x - xs[j - 1]) / h
        return (2 * t ** 3 - 3 * t * t + 1) * ys[j - 1] + (t ** 3 - 2 * t * t + t) * h * m[j - 1] + (-2 * t ** 3 + 3 * t * t) * ys[j] + (t ** 3 - t * t) * h * m[j]
    return np.vectorize(ev)

if __name__ == "__main__":
    S = sys.argv[1]
    meta = json.load(open(f"{S}/meta.json"))
    base = acr(S, "ramp__linear")[8:24].mean(0)  # 1024 x 3 linear ProPhoto
    for name in sorted(meta["items"]):
        if not name.startswith("ramp__") or name == "ramp__linear" or "P_" in name:
            continue
        cur = acr(S, name)[8:24].mean(0)
        pts = meta["items"][name]["curves"]["ToneCurvePV2012"]
        xs, ys = [p[0] / 255 for p in pts], [p[1] / 255 for p in pts]
        for enc_name, enc in (("srgb", srgb), ("g1.8", g18)):
            x = enc(base[:, 1]); y = enc(cur[:, 1])
            ok = (x > 0.01) & (x < 0.99)
            for sp_name, sp in (("sdk", sdk_spline(xs, ys)), ("mono", monotone(xs, ys))):
                if len(xs) == 2:
                    continue
                e = (sp(x[ok]) - y[ok]) * 255
                print(f"{name:24s} {enc_name:5s} {sp_name:5s} mean|err| {np.abs(e).mean():6.2f}  max {np.abs(e).max():6.2f} (levels)")
        # colour of a neutral stays neutral? channel spread
        print(f"{name:24s} neutral spread {np.abs(cur.max(1) - cur.min(1)).max():.4f}")
