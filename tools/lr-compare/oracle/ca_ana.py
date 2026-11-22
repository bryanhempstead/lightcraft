"""ca_ana.py SET [LCDIR]: lateral chromatic aberration left in make_ca.py's renders.

Per render: tiles (96 px) across the frame with enough edge contrast; in each, the sub-pixel
shift of the red and blue planes against green (gradient cross-correlation, parabolic peak), its
radial part fitted as `shift = a·r + b·r³` (r = distance from the centre / half diagonal, px).
Prints the radial misregistration at r = 0.5 / 0.9 (px, + = the plane is magnified) and the mean
|shift| of the edge tiles. Camera Raw: SET/acr/*.tif (16-bit TIFF); LightCraft: SET/LCDIR/*.f32."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tiff

SET = sys.argv[1]
LCD = sys.argv[2] if len(sys.argv) > 2 else None
meta = json.load(open(f"{SET}/meta.json"))
T = 96


def load(path):
    if path.endswith(".tif"):
        a, _ = tiff.read(path)
        return a
    b = np.fromfile(path, "<f4")
    w, h = b[:2].view("<u4")
    return np.clip(b[2:].reshape(h, w, 3), 0, None) ** (1 / 2.2)


def shift(a, b, m=3):
    """sub-pixel (dy, dx) of b against a (b(x) ≈ a(x - d))"""
    fa, fb = np.fft.rfft2(a - a.mean()), np.fft.rfft2(b - b.mean())
    c = np.fft.irfft2(fb * np.conj(fa), s=a.shape)
    c = np.fft.fftshift(c)
    cy, cx = a.shape[0] // 2, a.shape[1] // 2
    w = c[cy - m:cy + m + 1, cx - m:cx + m + 1]
    iy, ix = np.unravel_index(np.argmax(w), w.shape)
    if not (0 < iy < 2 * m and 0 < ix < 2 * m):
        return None
    def par(l, c0, r):
        d = l - 2 * c0 + r
        return 0.0 if abs(d) < 1e-12 else 0.5 * (l - r) / d
    dy = iy - m + par(w[iy - 1, ix], w[iy, ix], w[iy + 1, ix])
    dx = ix - m + par(w[iy, ix - 1], w[iy, ix], w[iy, ix + 1])
    return dy, dx


def measure(img):
    h, w, _ = img.shape
    g = img.mean(2)
    cy, cx, hd = h / 2, w / 2, np.hypot(h, w) / 2
    rows = []
    for y0 in range(0, h - T, T):
        for x0 in range(0, w - T, T):
            t = img[y0:y0 + T, x0:x0 + T]
            gr = np.hypot(*np.gradient(t[..., 1]))
            if gr.mean() < float(os.environ.get("CA_MIN_GRAD", "0.01")) or t.max() > 0.98:
                continue
            gs = [np.hypot(*np.gradient(t[..., c])) for c in range(3)]
            yy, xx = y0 + T / 2 - cy, x0 + T / 2 - cx
            r = np.hypot(yy, xx)
            if r < 1:
                continue
            out = []
            for c in (0, 2):
                s = shift(gs[1], gs[c])
                if s is None:
                    break
                out.append((s[0] * yy + s[1] * xx) / r)
            if len(out) == 2:
                rows.append((r / hd, out[0], out[1], gr.mean()))
    a = np.array(rows)
    if len(a) < 10:
        return None
    res = {}
    for k, c in (("R", 1), ("B", 2)):
        X = np.stack([a[:, 0], a[:, 0] ** 3], 1)
        wt = a[:, 3]
        coef = np.linalg.lstsq(X * wt[:, None], a[:, c] * wt, rcond=None)[0]
        res[k] = (coef[0] * 0.5 + coef[1] * 0.125, coef[0] * 0.9 + coef[1] * 0.729, float(np.average(np.abs(a[a[:, 0] > 0.6, c]), weights=a[a[:, 0] > 0.6, 3])))
    res["n"] = len(a)
    return res


print(f"{'render':<28} {'src':>4} {'tiles':>5} | R@.5   R@.9  |R|edge | B@.5   B@.9  |B|edge")
for n in sorted(meta["items"]):
    for kind, path in (("acr", f"{SET}/acr/{n}.tif"), ("lc", f"{SET}/{LCD}/{n}.f32" if LCD else "")):
        if not path or not os.path.exists(path):
            continue
        m = measure(load(path))
        if m is None:
            print(f"{n:<28} {kind:>4} too few tiles")
            continue
        print(f"{n:<28} {kind:>4} {m['n']:>5} | {m['R'][0]:+.2f} {m['R'][1]:+.2f} {m['R'][2]:.2f} | {m['B'][0]:+.2f} {m['B'][1]:+.2f} {m['B'][2]:.2f}")
