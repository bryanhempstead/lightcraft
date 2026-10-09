import json, numpy as np
from common import *
C = np.array(json.load(open("acr3.json"))["ys"]); XS = np.linspace(0, 1, len(C))
def inv_curve(y):  # curve^-1 (monotone)
    return np.interp(y, C, XS)
EVR = np.linspace(-14, 0.5, 1024)   # scene EV rel. raw white of ramp columns
GREY = 0.18
def d_of(set_, name, row=None):
    a = acr(set_, name); r = a.shape[0] // 2 if row is None else row
    y = Y(a[r])                       # linear output luminance
    s = inv_curve(np.clip(y, 0, 1))   # scene value the base curve needs
    ev_in = EVR - np.log2(GREY)       # LUT index (EV rel. grey)
    with np.errstate(divide='ignore'):
        d = np.log2(np.maximum(s, 1e-9) / GREY) - ev_in
    ok = (y > 1e-4) & (y < 0.995)
    return ev_in, d, ok
if __name__ == "__main__":
    import sys
    S = sys.argv[1]
    names = sys.argv[2].split(',')
    grid = np.arange(-11, 3.01, 1.0)
    print("name     " + " ".join(f"{g:6.0f}" for g in grid))
    for n in names:
        e, d, ok = d_of(S, "ramp__" + n)
        vals = [np.interp(g, e[ok], d[ok]) if ok.any() and e[ok].min() <= g <= e[ok].max() else np.nan for g in grid]
        print(f"{n:8s} " + " ".join(f"{v:+6.2f}" for v in vals))
