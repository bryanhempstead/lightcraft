"""hsorder.py SET: for each photo of `make_hsorder.py`, Camera Raw's H/S render passed through the global sliders' own
map (default render -> slider render, binned) against the render with both: small = H/S come first."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr
S = sys.argv[1]


def L(n):
    a = acr(S, n).max(-1)
    return np.log2(np.maximum(a, 1e-12)), a < 0.985


def lst(m):
    y = 2.0 ** m
    return np.where(y > 0.008856, 116 * np.cbrt(y) - 16, 903.3 * y)


for p in sorted({v['photo'] for v in json.load(open(f'{S}/meta.json'))['items'].values()}):
    b, vb = L(f'{p}__base'); hs, vh = L(f'{p}__HS')
    # base output (log max channel) -> HS output: per pixel, as a shift in "base output" units: use base output as the scene proxy
    for g, gh in [('g', 'gHS'), ('C', 'CHS'), ('Ep', 'EpHS')]:
        og, vg = L(f'{p}__{g}'); ogh, vgh = L(f'{p}__{gh}')
        ok = vb & vh & vg & vgh
        # empirical map base-output -> g-output (binned), monotone
        bins = np.linspace(np.percentile(b[ok], 0.2), np.percentile(b[ok], 99.8), 200)
        k = np.clip(np.searchsorted(bins, b) - 1, 0, 198)
        cnt = np.bincount(k[ok], minlength=199); tab = np.bincount(k[ok], weights=og[ok], minlength=199) / np.maximum(cnt, 1)
        gd = cnt > 10; xs = (bins[:-1] + bins[1:]) / 2
        f = lambda x: np.interp(x, xs[gd], np.maximum.accumulate(tab[gd]))
        pred = f(hs)  # HS first (as base output), then the global sliders
        e_before = np.abs(lst(pred) - lst(ogh))[ok].mean()
        noise = np.abs(lst(f(b)) - lst(og))[ok].mean()
        eff = (lst(ogh) - lst(og))[ok].mean(); eff0 = (lst(hs) - lst(b))[ok].mean()
        print(f'{p:9s} {g:3s} effect at base {eff0:+.2f} under {g} {eff:+.2f}; HS-before-{g} error {e_before:.2f} (map noise {noise:.2f})')
