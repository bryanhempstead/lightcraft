"""Per-slider EV-shift tables D_k(ev, v) from the oracle ramps (t2): d(v) − d(0), on a fine EV grid."""
import numpy as np, dtab
G = np.arange(-12, 4.001, 0.25)
VALS = {"E": [-4, -3, -2, -1.5, -1, -0.5, 0.5, 1, 1.5, 2, 3, 4], "C": [-100, -75, -50, -25, 25, 50, 75, 100],
        "H": [-100, -75, -50, -25, 25, 50, 75, 100], "S": [-100, -75, -50, -25, 25, 50, 75, 100],
        "W": [-100, -75, -50, -25, 25, 50, 75, 100], "K": [-100, -75, -50, -25, 25, 50, 75, 100]}
def on_grid(e, d, ok):
    e, d = e[ok], d[ok]
    out = np.interp(G, e, d)
    # beyond the measured range: hold the end value
    return out
def name(k, v): return f"E{v:+}" if k == "E" else f"{k}{v:+d}"
def load(S="t2"):
    e0, d0, ok0 = dtab.d_of(S, "ramp__base")
    base = on_grid(e0, d0, ok0)
    T = {}
    for k, vs in VALS.items():
        rows = {}
        for v in vs:
            e, d, ok = dtab.d_of(S, "ramp__" + name(k, v))
            okb = ok & ok0
            rows[v] = on_grid(e, d - d0, okb) if okb.sum() > 10 else np.zeros_like(G)
            # where the slider clips the output (no measurement), extrapolate the shift's trend
            lo = e[okb].min() if okb.any() else 0
            m = G < lo
            if m.any() and okb.sum() > 10:
                rows[v][m] = rows[v][~m][0] + (G[m] - G[~m][0]) * 0.0
        T[k] = rows
    return base, T
def D(T, k, v, ev):
    vs = sorted(T[k]); xs = [0.0] + vs; 
    tab = np.array([np.zeros_like(G)] + [T[k][x] for x in vs]); order = np.argsort(xs)
    xs = np.array(xs)[order]; tab = tab[order]
    v = np.clip(v, xs[0], xs[-1]); j = np.clip(np.searchsorted(xs, v) - 1, 0, len(xs) - 2); t = (v - xs[j]) / (xs[j + 1] - xs[j])
    row = tab[j] * (1 - t) + tab[j + 1] * t
    return np.interp(ev, G, row)
