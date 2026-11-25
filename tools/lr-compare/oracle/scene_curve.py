"""scene_curve.py SET LCDIR ITEM [ITEM…]: the slider's gain by level, Camera Raw against LightCraft. For each ITEM
(`PHOTO__Slider_±N`) the mean change of log2 luminance (item over the same engine's base render, linear ProPhoto Y) in 1 EV
bins of the *regional* base level (the base's log2 Y blurred over ±2 % of the long edge), the share of the photo in each bin
and the photo's own statistics (percentiles of the regional level, in EV)."""
import sys, os, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, Y
from scene_ana import lc_load, box

S, L = sys.argv[1], sys.argv[2]


def load(n):
    a = acr(S, n)
    b = lc_load(S, L, n)
    if a.shape[:2] == b.shape[:2][::-1] and a.shape[0] != a.shape[1]:
        b = np.ascontiguousarray(np.rot90(b, 1))
        if np.abs(b[::8, ::8, 1] - a[::8, ::8, 1]).mean() > 0.05:
            b = np.ascontiguousarray(np.rot90(b, 2))
    return Y(a), Y(b)


def curve(n):
    p = n.split("__")[0]
    a0, b0 = load(f"{p}__base")
    a, b = load(n)
    r = max(2, int(0.02 * max(a0.shape)))
    reg = box(np.log2(np.maximum(a0, 1e-5)), r)
    ga = np.log2(np.maximum(a, 1e-5) / np.maximum(a0, 1e-5))
    gb = np.log2(np.maximum(b, 1e-5) / np.maximum(b0, 1e-5))
    pc = np.percentile(reg, [1, 10, 50, 90, 99, 99.9])
    print(f"{n}: regional level percentiles 1/10/50/90/99/99.9 = " + " ".join(f"{v:+.1f}" for v in pc))
    for lo in range(-12, 1):
        m = (reg >= lo - 0.5) & (reg < lo + 0.5)
        if m.mean() < 0.002:
            continue
        print(f"  level {lo:+3d} EV  share {100 * m.mean():5.1f}%  CR {ga[m].mean():+5.2f}  LC {gb[m].mean():+5.2f}  diff {gb[m].mean() - ga[m].mean():+5.2f}")


for n in sys.argv[3:]:
    curve(n)
