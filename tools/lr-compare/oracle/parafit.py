"""parafit.py SET: Camera Raw's parametric curve, read off the ramp (`make_curve.py`): per region the
encoded offset y − x at slider stops ±50 / ±100 (default splits), then checks of a model on the
combinations: offsets interpolated in the slider value and summed, the x axis warped piecewise
linearly from the default splits (25 / 50 / 75) to the photo's."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr

S = sys.argv[1]
meta = json.load(open(f"{S}/meta.json"))
X = np.arange(256) / 255


def srgb(v):
    v = np.clip(v, 0, 1)
    return np.where(v <= 0.0031308, 12.92 * v, 1.055 * v ** (1 / 2.4) - 0.055)


xb = srgb(acr(S, "ramp__linear")[8:24].mean(0)[:, 1])


def curve(name):
    y = srgb(acr(S, name)[8:24].mean(0)[:, 1])
    return np.interp(X, xb, y)


REG = ["Shadows", "Darks", "Lights", "Highlights"]
STOPS = [-100, -50, 0, 50, 100]
tab = {r: np.stack([curve(f"ramp__P_{r}{v:+d}") - X if v else np.zeros(256) for v in STOPS]) for r in REG}


def model(st):
    d = np.zeros(256)
    for r in REG:
        v = float(st.get(f"Parametric{r}", 0))
        d += np.array([np.interp(v, STOPS, tab[r][:, i]) for i in range(256)])
    y = X + d
    sp = [float(st.get(k, dv)) / 100 for k, dv in (("ParametricShadowSplit", 25), ("ParametricMidtoneSplit", 50), ("ParametricHighlightSplit", 75))]
    if sp != [0.25, 0.5, 0.75]:
        # warp: the curve's x from the photo's splits to the default ones
        xw = np.interp(X, [0] + sp + [1], [0, 0.25, 0.5, 0.75, 1])
        y = np.interp(xw, X, y) - xw + X
    return y


if __name__ == "__main__":
    for n in ["P_bryan", "P_splits", "P_splits2"]:
        st = meta["items"][f"ramp__{n}"]["settings"]
        e = (model(st) - curve(f"ramp__{n}")) * 255
        print(f"{n:12s} model err mean {np.abs(e).mean():5.2f} max {np.abs(e).max():5.2f} levels")
    np.save(f"{S}/para_tab.npy", np.stack([tab[r] for r in REG]))
