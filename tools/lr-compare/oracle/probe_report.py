import sys, json, numpy as np
from common import *
S = sys.argv[1]; sets = sys.argv[2].split(',')
ev_r = np.linspace(-14, 0.5, 1024); ev_p = np.linspace(-12, 0, 1024)
evs = [-10, -8, -6, -5, -4, -3, -2, -1, -0.5, 0]
for st in sets:
    print(f"== {st}: ACR L* of the probe ramp at EV (rows: full-frame ramp, then probe on backgrounds -8 -6 -4 -2 -0.5)")
    get = lc if "lc" in sys.argv else acr
    a = lab(get(S, f"ramp__{st}"))[64, :, 0]
    print("ramp      " + " ".join(f"{a[np.argmin(abs(ev_r - e))]:6.1f}" for e in evs) + "   bg L*")
    for bg in ["-8.0", "-6.0", "-4.0", "-2.0", "-0.5"]:
        p = lab(get(S, f"probe{bg}__{st}"))
        row = p[256, :, 0]
        print(f"bg {bg:5s}  " + " ".join(f"{row[np.argmin(abs(ev_p - e))]:6.1f}" for e in evs) + f"   {p[40, 512, 0]:5.1f}")
