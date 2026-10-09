"""inv.py: invert a renderer's base mapping on the grid (output Lab -> scene ProPhoto) by local affine fits"""
import json, numpy as np
from common import lab
from cmp import load
class Inv:
    def __init__(self, S, kind="acr", k=14):
        m = json.load(open(f"{S}/meta.json")); self.v = np.array(m["valid"]); self.s = np.array(m["cells"])
        self.out = load(S, kind, "grid__base"); self.L = lab(self.out); self.k = k
        ok = self.v & (self.out.max(1) < 0.995) & (self.out.min(1) > 1e-5)
        self.q = np.cbrt(self.s[ok]); self.Lk = self.L[ok]
    def __call__(self, t, exclude_self=False):
        d = ((t[:, None, :] - self.Lk[None]) ** 2).sum(-1)
        if exclude_self: d[d < 1e-9] = np.inf
        idx = np.argsort(d, 1)[:, :self.k]; res = np.zeros((len(t), 3))
        for i in range(len(t)):
            X = np.c_[self.Lk[idx[i]], np.ones(self.k)]; w = 1 / (np.sqrt(d[i, idx[i]]) + 0.5)
            A = np.linalg.lstsq(X * w[:, None], self.q[idx[i]] * w[:, None], rcond=None)[0]
            res[i] = np.r_[t[i], 1] @ A
        return np.maximum(res, 0) ** 3
if __name__ == "__main__":
    I = Inv("g1"); s_hat = I(I.Lk, exclude_self=True); s = I.q ** 3
    e = lab(s_hat) - lab(s); print("LOO scene-Lab error mean", np.sqrt((e ** 2).sum(1)).mean().round(3), "p95", np.percentile(np.sqrt((e ** 2).sum(1)), 95).round(3))
