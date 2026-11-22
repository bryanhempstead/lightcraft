"""chartcmp.py SET LCDIR NAME: Camera Raw vs LightCraft on `make_color.py`'s chart, per CIELAB hue
bin of Camera Raw's patch colour: ΔL*, ΔC*, Δh°, ΔE2000 (LightCraft − Camera Raw)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000


def patches(S, img):
    m = json.load(open(f"{S}/meta.json")); cw, ch = m["cell"]
    out = []
    for i in range(len(m["cells"])):
        r, c = divmod(i, 24)
        y0, x0, y1, x1 = 4 + r * ch + 9, c * cw + 9, 4 + (r + 1) * ch - 9, (c + 1) * cw - 9
        out.append(img[y0:y1, x0:x1].reshape(-1, 3).mean(0))
    return np.array(out)


def lcimg(S, L, n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); return b[2:].reshape(h, w, 3).astype(np.float64)


if __name__ == "__main__":
    S, L, N = sys.argv[1], sys.argv[2], sys.argv[3]
    a = lab(patches(S, acr(S, f"chart__{N}"))); b = lab(patches(S, lcimg(S, L, f"chart__{N}")))
    ca, cb = np.hypot(a[:, 1], a[:, 2]), np.hypot(b[:, 1], b[:, 2])
    ha = np.degrees(np.arctan2(a[:, 2], a[:, 1])) % 360; hb = np.degrees(np.arctan2(b[:, 2], b[:, 1])) % 360
    dh = (hb - ha + 180) % 360 - 180; d = de2000(a, b)
    ok = ca > 8
    print(f"{N}: all dE {d.mean():.2f}")
    for k in range(12):
        m = ok & (ha >= 30 * k) & (ha < 30 * k + 30)
        if m.sum():
            print(f"  {30 * k:3d}-{30 * k + 30:<3d} n {m.sum():3d} dL {(b - a)[m, 0].mean():+5.2f} dC {(cb - ca)[m].mean():+5.2f} dh {dh[m].mean():+6.1f} dE {d[m].mean():5.2f}")
