import json, numpy as np
from ana import scene_op, s
from common import lab
def mat(i, a, b):
    C = np.eye(3); C[(i + 1) % 3, i] = a; C[(i + 2) % 3, i] = b
    k = np.linalg.solve(C, np.ones(3)); return C * k[None, :]
out = {}
for p in ["Red", "Green", "Blue"]:
    i = "RGB".index(p[0])
    for k in ["Hue", "Saturation"]:
        for v in [-100, -50, 50, 100]:
            n = f"grid__{p}{k}{v:+d}"; ok, sp = scene_op(n); s0 = s[ok]; L = lab(sp)
            def err(x):
                e = lab(s0 @ mat(i, *x).T) - L; return np.sqrt((e ** 2).sum(1) + 1e-9)
            x = np.zeros(2)
            for it in range(30):  # Gauss-Newton on the robust (L1-ish) residuals
                r = err(x); J = np.stack([(err(x + d) - r) / 1e-4 for d in np.eye(2) * 1e-4], 1)
                w = 1 / np.maximum(r, 0.5); dx = np.linalg.lstsq(J * w[:, None] ** .5, -r * w ** .5, rcond=None)[0]; x += dx
                if np.abs(dx).max() < 1e-5: break
            out[n] = x.tolist(); print(f"{p}{k}{v:+4d} a {x[0]:+.4f} b {x[1]:+.4f} err {err(x).mean():.2f}")
json.dump(out, open("calib_ab.json", "w"))
