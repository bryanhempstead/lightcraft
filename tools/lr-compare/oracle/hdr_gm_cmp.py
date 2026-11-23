"""hdr_gm_cmp.py DECODED.f32 LC_HDR.f32 [CR.tif]: an exported file decoded to HDR by macOS ImageIO against
LightCraft's own HDR render (and Camera Raw's): ΔEITP overall / SDR part / headroom part, peak."""
import sys, numpy as np, hdr_common as hc
def rd(p):
    b = np.fromfile(p, '<f4'); w, h = b[:2].view('<u4'); return b[2:].reshape(h, w, 3).astype(np.float64)
dec = rd(sys.argv[1])
lc = hc.lc(sys.argv[2])
refs = [("LC", lc)]
if len(sys.argv) > 3:
    refs.append(("CR", hc.acr(sys.argv[3])[0]))
for n, r in refs:
    a, b = hc.fit(r, dec)
    d = hc.de_itp(a, b)
    ya = hc.Y(a); hi = ya > 1
    yb = hc.Y(b)
    ratio = np.median(np.log2(np.maximum(yb[hi], 1e-6) / ya[hi])) if hi.sum() > 20 else np.nan
    print(f"decoded vs {n}: ΔEITP {d.mean():5.2f} sdr {d[~hi].mean():5.2f} hdr {d[hi].mean() if hi.sum() > 20 else np.nan:6.2f}"
          f"  log2 Y ratio above 1: {ratio:+.3f}  peak {np.percentile(ya, 99.9):.2f}/{np.percentile(yb, 99.9):.2f}")
