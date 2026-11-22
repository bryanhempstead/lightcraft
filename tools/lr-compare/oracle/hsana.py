"""hsana.py SET LCDIR NAME [SUFFIX]: how a tone slider moves each region of the spatial scene and each
grid level, in EV of luminance against the base render, Camera Raw vs LightCraft."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, Y
import scenes

S, L, N = sys.argv[1], sys.argv[2], sys.argv[3]
suf = sys.argv[4] if len(sys.argv) > 4 else "__pv15"


def lc(n):
    b = np.fromfile(f"{S}/{L}/{n}.f32", "<f4"); w, h = b[:2].view("<u4"); return b[2:].reshape(h, w, 3).astype(np.float64)


def dev(get, scene):
    base, cur = Y(get(f"{scene}__base{suf}")), Y(get(f"{scene}__{N}{suf}"))
    return np.log2(np.maximum(cur, 1e-6) / np.maximum(base, 1e-6)), np.log2(np.maximum(base, 1e-6) / 0.18)


h = 512
regions = {"dark half (-6)": (slice(20, 236), slice(20, 492)), "bright half (-1.5)": (slice(20, 236), slice(532, 1004)),
           "bg (-3.5) mid": (slice(440, 500), slice(300, 560)), "texture (-3.5±)": (slice(h // 2 + 100, h // 2 + 252), slice(70, 218)),
           "dark window (-6.5)": (slice(h // 2 + 125, h // 2 + 135), slice(605, 755)), "bright patch (-0.8)": (slice(h // 2 + 145, h // 2 + 175), slice(655, 705))}
a, ba = dev(lambda n: acr(S, n), "spatial"); b, bb = dev(lc, "spatial")
print(f"{'region':22s}{'base EV':>9}{'ACR dEV':>9}{'LC dEV':>9}")
for k, (ys, xs) in regions.items():
    print(f"{k:22s}{ba[ys, xs].mean():9.2f}{a[ys, xs].mean():9.3f}{b[ys, xs].mean():9.3f}")
# the gradient row: input EV -8..-0.5 across
g = slice(h // 2 + 8, h // 2 + 56)
print("gradient (input EV: ACR dEV / LC dEV):")
xs = np.linspace(-8, -0.5, scenes.W)
print("  " + "  ".join(f"{xs[i]:.1f}:{a[g, i].mean():+.2f}/{b[g, i].mean():+.2f}" for i in range(16, scenes.W, 96)))
# edge profile across the step (row block in the top half, around x = 512)
e = slice(60, 200)
print("step edge profile (x: ACR/LC):")
print("  " + "  ".join(f"{x}:{a[e, x].mean():+.2f}/{b[e, x].mean():+.2f}" for x in (100, 300, 420, 480, 500, 510, 514, 524, 544, 600, 720, 900)))
