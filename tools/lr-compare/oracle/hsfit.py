"""Fit Highlights / Shadows tables (per slider value) on the real-raw oracle renders (r1)."""
import json, os, sys, numpy as np
from common import lab
S = "r1"; meta = json.load(open(f"{S}/meta.json"))["items"]
KN = np.arange(-10, 5, 1.0)
def hat(x):
    x = np.clip(x, KN[0], KN[-1]); return np.maximum(0, 1 - np.abs(x[..., None] - KN) / (KN[1] - KN[0]))
def load(n):
    dd = np.load(f"{S}/dump/{n}.npy", allow_pickle=True).item(); hdr, d = dd["hdr"], dd["d"].astype(np.float64)
    a = np.load(f"{S}/acrs/{n}.npy").astype(np.float64); b = np.load(f"{S}/lcs/{n}.npy").astype(np.float64)
    h = min(a.shape[0], b.shape[0], d.shape[0]); w = min(a.shape[1], b.shape[1], d.shape[1])
    return hdr, d[:h, :w], lab(a[:h, :w])[..., 0], lab(b[:h, :w])[..., 0]
def samples(n, rng):
    hdr, d, La, Lb = load(n)
    ev_in = d[..., 6]; ok = (d[..., 13] < 0.5) & (La > 2) & (La < 97) & (Lb > 2) & (Lb < 97) & np.isfinite(ev_in)
    # our own ev_in -> L* (monotone, binned) for this photo/settings
    e, L = ev_in[ok], Lb[ok]; edges = np.linspace(-12, 6, 145); idx = np.digitize(e, edges); bx, by = [], []
    for k in range(1, len(edges)):
        s = idx == k
        if s.sum() >= 5: bx.append(np.median(e[s])); by.append(np.median(L[s]))
    bx, by = np.array(bx), np.maximum.accumulate(np.array(by))
    keep = np.concatenate([[True], np.diff(by) > 1e-6]); bx, by = bx[keep], by[keep]
    tgt = np.interp(La, by, bx)  # ev_in that gives Lightroom's L*
    slope = np.interp(ev_in, bx, np.gradient(by, bx))
    ok &= (slope > 4) & (La > by[0] + 0.5) & (La < by[-1] - 0.5)
    ix = np.flatnonzero(ok.ravel()); ix = rng.choice(ix, min(5000, len(ix)), replace=False)
    f = lambda x: x.reshape(-1)[ix]
    key = float(hdr[6]) if np.isfinite(hdr[6]) else 0.0
    return dict(base=f(d[..., 3]) - key, l1=f(d[..., 4]) - key, wb=f(d[..., 5]), hs=f(d[..., 1]), ev=f(ev_in), tgt=f(tgt), slope=f(slope), Lb=f(Lb), La=f(La))
rng = np.random.default_rng(0)
photos = sorted({m["file"] for n, m in meta.items() if os.path.exists(f"{S}/acrs/{n}.npy")})
res = {}
for k in ["H", "S"]:
    for v in [-100, -50, 50, 100]:
        tab = []
        rows = []
        for p in photos:
            n = f"{os.path.splitext(p)[0]}__{k}{v:+d}"
            if not os.path.exists(f"{S}/acrs/{n}.npy"): continue
            s = samples(n, rng)
            F = s["wb"][:, None] * hat(s["base"]) + (1 - s["wb"][:, None]) * hat(s["l1"])
            y = s["tgt"] - (s["ev"] - s["hs"]); w = np.minimum(s["slope"], 40) ** 2
            rows.append((p, F, y, w, s))
        if not rows: continue
        def solve(rs):
            F = np.concatenate([r[1] for r in rs]); y = np.concatenate([r[2] for r in rs]); w = np.concatenate([r[3] for r in rs])
            D = np.diff(np.eye(len(KN)), 2, 0)
            A = (F * w[:, None]).T @ F / w.sum() + 1e-4 * np.eye(len(KN)) + 3e-2 * D.T @ D
            return np.linalg.solve(A, (F * w[:, None]).T @ y / w.sum())
        th = solve(rows)
        # leave-one-photo-out error (L*) vs ours now
        e0, e1 = [], []
        for i, r in enumerate(rows):
            t = solve(rows[:i] + rows[i + 1:]) if len(rows) > 1 else th
            pred = r[1] @ t; s = r[4]
            e0.append(np.mean(np.abs(s["Lb"] - s["La"]))); e1.append(np.mean(np.abs((pred - r[2]) * s["slope"])))
        res[f"{k}{v:+d}"] = [round(float(x), 3) for x in th]
        print(f"{k}{v:+d}: photos {len(rows)}  |dL| now {np.mean(e0):5.2f} -> held-out fit {np.mean(e1):5.2f}   table {np.round(th, 2)}")
json.dump(res, open("hsfit.json", "w"), indent=1)
