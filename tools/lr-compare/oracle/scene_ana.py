"""scene_ana.py SET LCDIR [filter]: Shadows +, Clarity and negative Dehaze against Camera Raw at the SAME slider value
(make_scene.py's set; docs/lr-match.md → Round 14). Per item: mean ΔE2000, the slider's *effect* (item − the photo's
base render, mean L*) in the base's L* bands sh / mid / hi for both engines, highlight detail (local contrast in the
base's brightest 5 %, LightCraft / Camera Raw), the neutral surfaces' colour ΔE and, per photo and slider, the Camera Raw
slider value whose effect equals LightCraft's (`acts like`). Definitions are the 2026-11-25 study's."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000

BANDS = {"sh": (0, 30), "mid": (30, 70), "hi": (70, 101)}


def lc_load(S, L, n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4")
    w, h = b[:2].view("<u4")
    return b[2:].reshape(h, w, 3).astype(np.float64)


def box(x, r):
    """(2r+1)² box mean, edges clamped."""
    k = 2 * r + 1
    p = np.pad(x, ((r + 1, r), (0, 0)), mode="edge")
    c = np.cumsum(p, 0)
    x = (c[k:] - c[:-k]) / k
    p = np.pad(x, ((0, 0), (r + 1, r)), mode="edge")
    c = np.cumsum(p, 1)
    return (c[:, k:] - c[:, :-k]) / k


class Cache:
    """Camera Raw / LightCraft renders as clipped L*a*b* (LightCraft's oracle render ignores the EXIF turn: rotated to fit)."""

    def __init__(self, S, L):
        self.S, self.L, self.c, self.rot = S, L, {}, None

    def get(self, n):
        if n not in self.c:
            ok = os.path.exists(f"{self.S}/acr/{n}.tif") and os.path.exists(f"{self.S}/{self.L}/{n}.f32")
            if not ok:
                self.c[n] = None
            else:
                a = np.clip(acr(self.S, n), 0, 1)
                b = np.clip(lc_load(self.S, self.L, n), 0, 1)
                if a.shape[:2] == b.shape[:2][::-1] and a.shape[0] != a.shape[1]:
                    if self.rot is None:
                        e = {k: float(np.abs(np.rot90(b, k)[::8, ::8, 1] - a[::8, ::8, 1]).mean()) for k in (1, 3)}
                        self.rot = min(e, key=e.get)
                    b = np.ascontiguousarray(np.rot90(b, self.rot))
                self.c[n] = (lab(a), lab(b)) if a.shape == b.shape else None
        return self.c[n]


def hp(L):
    return L - box(L, 2)


def rms(x):
    return float(np.sqrt(np.mean(x * x))) if x.size else float("nan")


def band_eff(L0, L1, lo, hi):
    m = (L0 >= lo) & (L0 < hi)
    return float((L1 - L0)[m].mean()) if m.any() else float("nan")


def item_metrics(C, n):
    """The study's definitions: bands on each engine's own base render (sh L* < 30, mid 30-70, hi >= 70), detail = rms of
    L* less its 5x5 mean in the base's brightest 5 %, neutral-pixel colour (Camera Raw's base neutrals)."""
    r = C.get(n)
    p = n.split("__")[0]
    b0 = C.get(f"{p}__base")
    if r is None or b0 is None:
        return None
    (a, b), (a0, bb0) = r, b0
    d = de2000(a, b)
    out = {"dE": float(d.mean()), "p95": float(np.percentile(d, 95))}
    out["eff_cr"] = {k: band_eff(a0[..., 0], a[..., 0], *v) for k, v in BANDS.items()}
    out["eff_lc"] = {k: band_eff(bb0[..., 0], b[..., 0], *v) for k, v in BANDS.items()}
    ta, tb = a0[..., 0] >= np.percentile(a0[..., 0], 95), bb0[..., 0] >= np.percentile(bb0[..., 0], 95)
    out["hi_detail"] = {"cr": rms(hp(a[..., 0])[ta]), "lc": rms(hp(b[..., 0])[tb])}
    C0 = np.hypot(a0[..., 1], a0[..., 2])
    neu = (C0 < 8) & (a0[..., 0] > 25) & (a0[..., 0] < 85)
    if neu.mean() < 0.005:
        neu = (C0 < 14) & (a0[..., 0] > 20) & (a0[..., 0] < 90)
    ma, mb = a[neu].mean(0), b[neu].mean(0)
    out["colour_dE"] = float(de2000(ma[None], mb[None])[0])
    out["mean_ab"] = {"cr": [float(ma[1]), float(ma[2])], "lc": [float(mb[1]), float(mb[2])]}
    return out


def parse(n):
    p, s = n.split("__")
    if s == "base":
        return p, "base", 0
    k, v = s.rsplit("_", 1)
    return p, k, int(v)


def equiv(cr_curve, target):
    """CR slider value whose effect equals `target` (linear between the measured values; None outside)."""
    pts = sorted(cr_curve)
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        if (y0 - target) * (y1 - target) <= 0 and y0 != y1:
            return x0 + (x1 - x0) * (target - y0) / (y1 - y0)
    # beyond the measured range: continue the last segment (positive sliders only reach +100, so 102 means "just above")
    (x0, y0), (x1, y1) = pts[-2], pts[-1]
    if y1 != y0 and x1 > 0 and (target - y1) * (y1 - y0) > 0:
        return x1 + (x1 - x0) * (target - y1) / (y1 - y0)
    return None


def run(S, L, f="", quiet=False):
    meta = json.load(open(f"{S}/meta.json"))["items"]
    C = Cache(S, L)
    rows = {}
    for n in sorted(meta):
        if f not in n or n.endswith("__base"):
            continue
        m = item_metrics(C, n)
        if m is None:
            continue
        rows[n] = m
        e, l = m["eff_cr"], m["eff_lc"]
        fm = lambda x: " ".join(f"{x[b]:+6.1f}" for b in BANDS)
        hd = m["hi_detail"]
        if not quiet:
            print(f"{n:28s} dE {m['dE']:5.2f} p95 {m['p95']:5.1f} | eff CR {fm(e)} LC {fm(l)} | hiDet LC/CR {hd['lc'] / max(hd['cr'], 1e-9):4.2f} | neutral dE {m['colour_dE']:4.1f}")
    return rows


def acts_like(rows):
    """{(photo, slider, value): (band, LC effect, CR effect, CR value with LC's effect)}"""
    out = {}
    for p in sorted({parse(n)[0] for n in rows}):
        for k in sorted({parse(n)[1] for n in rows if parse(n)[0] == p}):
            band = "mid" if k in ("Clarity2012", "Dehaze") else ("hi" if k == "Highlights2012" else "sh")
            cr = [(parse(n)[2], rows[n]["eff_cr"][band]) for n in rows if parse(n)[0] == p and parse(n)[1] == k]
            cr.append((0, 0.0))
            for n in sorted(rows, key=lambda n: parse(n)[2]):
                if parse(n)[0] == p and parse(n)[1] == k:
                    out[(p, k, parse(n)[2])] = (band, rows[n]["eff_lc"][band], rows[n]["eff_cr"][band], equiv(cr, rows[n]["eff_lc"][band]))
    return out


if __name__ == "__main__":
    S, L = sys.argv[1], sys.argv[2]
    f = sys.argv[3] if len(sys.argv) > 3 else ""
    rows = run(S, L, f)
    print()
    for (p, k, v), (band, lc, cr, q) in acts_like(rows).items():
        print(f"  {p} {k} {v:+d} ({band}): LC {lc:+6.1f} CR {cr:+6.1f} acts like CR {'%+.0f' % q if q is not None else 'n/a'}")
