"""curvecolor.py SET: how Camera Raw applies a point curve to colour, on the grid patches of
`make_curve.py`: per channel (ProPhoto primaries, sRGB encoding) vs the DNG reference RGB tone
(hue-preserving: largest / smallest channel through the curve, the middle interpolated)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000
from curvefit import sdk_spline, srgb



def patches(img, R):
    return np.array([img[y0:y1, x0:x1].reshape(-1, 3).mean(0) for y0, x0, y1, x1 in R])


def dec(e):
    e = np.clip(e, 0, 1)
    return np.where(e <= 0.04045, e / 12.92, ((e + 0.055) / 1.055) ** 2.4)


def per_channel(c, f):
    return dec(f(srgb(c)))


def rgb_tone(c, f):
    out = np.zeros_like(c)
    for i, p in enumerate(c):
        e = srgb(p)
        hi, lo = e.argmax(), e.argmin()
        if hi == lo:
            out[i] = dec(f(e)); continue
        mid = 3 - hi - lo
        r = e.copy(); r[hi] = f(e[hi]); r[lo] = f(e[lo])
        r[mid] = r[lo] + (r[hi] - r[lo]) * (e[mid] - e[lo]) / (e[hi] - e[lo])
        out[i] = dec(r)
    return out

if __name__ == "__main__":
    S = sys.argv[1]
    meta = json.load(open(f"{S}/meta.json"))
    R = meta["rects"]; valid = np.array(meta["valid"])
    base = patches(acr(S, "grid__linear"), R)
    for name in sorted(n for n in meta["items"] if n.startswith("grid__") and n != "grid__linear" and "P_" not in n):
        cur = patches(acr(S, name), R)
        cv = meta["items"][name]["curves"]
        fs = []
        for k in ("ToneCurvePV2012", "ToneCurvePV2012Red", "ToneCurvePV2012Green", "ToneCurvePV2012Blue"):
            p = cv[k]
            sp = sdk_spline([q[0] / 255 for q in p], [q[1] / 255 for q in p])
            fs.append(lambda v, sp=sp: np.clip(sp(np.clip(v, 0, 1)), 0, 1))
        def chans(e):
            m = fs[0](e)
            return np.stack([fs[1](m[..., 0]), fs[2](m[..., 1]), fs[3](m[..., 2])], -1)
        ok = valid & (base.max(1) < 0.99) & (base.max(1) > 2 ** -7.5)
        pc = dec(chans(srgb(base)))
        d1 = de2000(lab(cur), lab(pc))[ok]
        line = f"{name:24s} per-channel dE {d1.mean():5.2f} p95 {np.percentile(d1, 95):5.2f}"
        if all(cv[k] == [[0, 0], [255, 255]] for k in ("ToneCurvePV2012Red", "ToneCurvePV2012Green", "ToneCurvePV2012Blue")):
            rt = rgb_tone(base, lambda v: fs[0](v))
            d2 = de2000(lab(cur), lab(rt))[ok]
            line += f"   rgb-tone dE {d2.mean():5.2f} p95 {np.percentile(d2, 95):5.2f}"
        print(line)
