"""Output-referred colour-op tables (OkLCh of display-linear Rec.2020): per op & slider value, Δhue,
log chroma ratio, ΔL on a periodic hue x lightness x chroma tent grid, fitted to Camera Raw's grid renders."""
import json, numpy as np
from ok import oklch_pp, M2020
NH, LC_, CC = 36, np.array([0.2, 0.4, 0.6, 0.8, 1.0]), np.array([0.0, 0.06, 0.14, 0.26])
def tent(x, c):
    x = np.clip(x, c[0], c[-1]); i = np.clip(np.searchsorted(c, x) - 1, 0, len(c) - 2); f = (x - c[i]) / (c[i + 1] - c[i])
    W = np.zeros((len(x), len(c))); W[np.arange(len(x)), i] = 1 - f; W[np.arange(len(x)), i + 1] = f; return W
def hw(h):
    u = (h / (2 * np.pi) % 1) * NH; i = np.floor(u).astype(int) % NH; f = u - np.floor(u)
    W = np.zeros((len(h), NH)); W[np.arange(len(h)), i] = 1 - f; W[np.arange(len(h)), (i + 1) % NH] += f; return W
def design(lch, axes=("h", "l", "c")):
    Wh = hw(lch[:, 2]); Wl = tent(lch[:, 0], LC_) if "l" in axes else np.ones((len(lch), 1)); Wc = tent(lch[:, 1], CC) if "c" in axes else np.ones((len(lch), 1))
    return (Wh[:, :, None, None] * Wl[:, None, :, None] * Wc[:, None, None, :]).reshape(len(lch), -1), (NH, Wl.shape[1], Wc.shape[1])
def smooth_pen(shape, lam):
    n = np.prod(shape); idx = np.arange(n).reshape(shape); rows = []
    for ax in range(3):
        if shape[ax] < 3 and ax > 0: 
            if shape[ax] < 2: continue
        for j in range(shape[ax]):
            if ax > 0 and (j == 0 or j == shape[ax] - 1): continue
            a = np.take(idx, (j - 1) % shape[ax], ax).ravel(); b = np.take(idx, j, ax).ravel(); c = np.take(idx, (j + 1) % shape[ax], ax).ravel()
            for p, q, r in zip(a, b, c):
                row = np.zeros(n); row[p] += 1; row[q] -= 2; row[r] += 1; rows.append(row * lam)
    return np.r_[np.array(rows).reshape(-1, n), np.eye(n) * 1e-3]
def fit(base, op, ok, axes=("h", "l", "c"), lam=0.5):
    B, A = oklch_pp(base[ok]), oklch_pp(op[ok]); X, shape = design(B, axes)
    dh = (A[:, 2] - B[:, 2] + np.pi) % (2 * np.pi) - np.pi; lc = np.log(np.maximum(A[:, 1], 1e-4) / np.maximum(B[:, 1], 1e-4)); dl = A[:, 0] - B[:, 0]
    wc = np.clip(B[:, 1] / 0.04, 0, 1); P = smooth_pen(shape, lam); out = []
    for y, w in [(dh, wc), (lc, wc), (dl, np.ones(len(B)))]:
        Xw = np.r_[X * w[:, None], P]; yw = np.r_[y * w, np.zeros(len(P))]
        out.append(np.linalg.lstsq(Xw, yw, rcond=None)[0].reshape(shape))
    return np.stack(out, -1)
def apply(T, x, axes=("h", "l", "c")):
    L = oklch_pp(x); X, shape = design(L, axes); d = X @ T.reshape(-1, 3)
    return L, d
from ok import lch_to_pp
def adjust(x, deltas):
    L = oklch_pp(x); tot = 0
    for T, axes in deltas:
        X, _ = design(L, axes); tot = tot + X @ T.reshape(-1, 3)
    L2 = L.copy(); L2[:, 2] += tot[:, 0]; L2[:, 1] *= np.exp(tot[:, 1]); L2[:, 0] += tot[:, 2]; return lch_to_pp(L2)
