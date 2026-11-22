"""r12lift.py LCDIR SET [SET…]: positive Shadows alone on Bryan's raws — per photo and setting, Camera Raw's and LightCraft's
mean lift in L* (pixels below L* 50 in the base render) and their ratio, with the photo's statistics from LightCraft's
base render (L* percentiles, fine texture): what makes the rule over- or under-lift a photo?"""
import sys, os, json, re, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab

L, sets = sys.argv[1], sys.argv[2:]


def lc(S, n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); return b[2:].reshape(h, w, 3).astype(np.float64)


def shadows_only(st):
    s = float(st.get("Shadows2012", "0")); o = [k for k, v in st.items() if k != "Shadows2012" and float(v) != 0]
    return s if s > 0 and not o else None


print(f"{'photo':10s} {'cam':5s} {'S':>4s} {'CR':>6s} {'LC':>6s} {'ratio':>6s}   p1   p10  p50  p90  tex")
rows = []
for S in sets:
    meta = json.load(open(f"{S}/meta.json"))["items"]
    for n, it in sorted(meta.items()):
        p = it.get("photo")
        s = shadows_only(it.get("settings", {})) if p else None
        if s is None or not all(os.path.exists(x) for x in (f"{S}/acr/{n}.tif", f"{S}/acr/{p}__base.tif", f"{S}/{L}/{n}.f32", f"{S}/{L}/{p}__base.f32")):
            continue
        a, a0 = lab(acr(S, n)[::2, ::2])[..., 0], lab(acr(S, f"{p}__base")[::2, ::2])[..., 0]
        b, b0 = lab(lc(S, n)[::2, ::2])[..., 0], lab(lc(S, f"{p}__base")[::2, ::2])[..., 0]
        if a.shape != b.shape:
            continue
        m = a0 < 50
        cr, us = (a - a0)[m].mean(), (b - b0)[m].mean()
        q = np.percentile(b0, [1, 10, 50, 90])
        tex = np.abs(b0[1:, :] - b0[:-1, :]).mean()
        cam = "R6" if "R6" in it.get("camera", "") else it.get("camera", "")[:5]
        rows.append((p, cam, s, cr, us))
        print(f"{p:10s} {cam:5s} {s:4.0f} {cr:6.2f} {us:6.2f} {us / max(cr, 1e-3):6.2f}  " + " ".join(f"{v:4.0f}" for v in q) + f" {tex:4.2f}")
