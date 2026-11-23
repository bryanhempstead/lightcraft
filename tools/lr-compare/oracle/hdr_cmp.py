"""hdr_cmp.py SET LCDIR [-v]: Camera Raw HDR masters against LightCraft's, per item and per variant.
ΔEITP (BT.2124, SDR white = 203 nits) overall, on the SDR part (CR luminance ≤ 1) and on the
headroom part (> 1); luminance ratio (log2) per band; headroom use (99.9th pct / max)."""
import sys, json, numpy as np, hdr_common as hc
SET, LC = sys.argv[1], sys.argv[2]
verbose = "-v" in sys.argv
meta = json.load(open(f"{SET}/meta.json"))["items"]
BANDS = [(0, 0.05), (0.05, 0.2), (0.2, 0.45), (0.45, 0.8), (0.8, 1.0), (1.0, 2.0), (2.0, 4.0), (4.0, 1e9)]
rows = {}
for name, m in sorted(meta.items()):
    try:
        a, kind = hc.acr(f"{SET}/acr/{name}.tif")
        b = hc.lc(f"{SET}/{LC}/{name}.f32")
    except (FileNotFoundError, AssertionError) as e:
        continue
    a, b = hc.fit(a, b)
    a, b = a[::2, ::2], b[::2, ::2]
    d = hc.de_itp(a, b)
    ya, yb = hc.Y(a), hc.Y(b)
    lo, hi = ya <= 1.0, ya > 1.0
    r = dict(de=d.mean(), de_sdr=d[lo].mean(), de_hdr=d[hi].mean() if hi.sum() > 20 else np.nan, hfrac=hi.mean(),
             p999=(np.percentile(ya, 99.9), np.percentile(yb, 99.9)), mx=(ya.max(), yb.max()))
    bands = []
    for l, h in BANDS:
        k = (ya > l) & (ya <= h) & (yb > 0)
        bands.append(np.median(np.log2(np.maximum(yb[k], 1e-6) / np.maximum(ya[k], 1e-6))) if k.sum() > 50 else np.nan)
    r["bands"] = bands
    rows.setdefault(m["variant"], []).append((name, r))
    if verbose:
        print(f"{name:28s} {kind} ΔEITP {r['de']:5.2f} sdr {r['de_sdr']:5.2f} hdr {r['de_hdr']:6.2f} ({100*r['hfrac']:4.1f}%)"
              f" p99.9 {r['p999'][0]:5.2f}/{r['p999'][1]:5.2f} max {r['mx'][0]:5.2f}/{r['mx'][1]:5.2f} | "
              + " ".join("  .  " if np.isnan(x) else f"{x:+.2f}" for x in bands))
print("variant        n  ΔEITP  sdr   hdr   | log2 LC/CR per CR band " + " ".join(f"{l}-{h if h < 1e8 else '∞'}" for l, h in BANDS))
for v, rs in rows.items():
    g = lambda k: np.nanmean([r[k] for _, r in rs])
    bands = np.nanmean(np.array([r["bands"] for _, r in rs], dtype=float), 0)
    print(f"{v:12s} {len(rs):3d} {g('de'):5.2f} {g('de_sdr'):5.2f} {g('de_hdr'):6.2f} | " + " ".join("  .  " if np.isnan(x) else f"{x:+.2f}" for x in bands))
# Visualize HDR's ranges (brightest channel: ≤ white, up to 1, 2, 3 stops, over 3): share of the
# pixels that land in a different range in LightCraft than in Camera Raw, per variant
def vis(x):
    m = x.max(-1)
    return np.digitize(m, [1.0, 2.0, 4.0, 8.0])
print("Visualize HDR: pixels in another range (%)")
for v in rows:
    out = []
    for name, _ in rows[v]:
        a, _k = hc.acr(f"{SET}/acr/{name}.tif"); b = hc.lc(f"{SET}/{LC}/{name}.f32"); a, b = hc.fit(a, b)
        va, vb = vis(a[::2, ::2]), vis(b[::2, ::2])
        hi = (va > 0) | (vb > 0)
        out.append(100 * (va != vb)[hi].mean() if hi.sum() > 50 else np.nan)
    print(f"{v:12s} {np.nanmean(out):5.1f} (of the pixels above white in either)")
