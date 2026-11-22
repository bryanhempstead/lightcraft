"""hsfit_rhs.py ITERS 'TH' SET [SET…]: fit LightCraft's Highlights / Shadows model (`tone::lr::adobe_hs_lut`,
R02) to Camera Raw renders of real raws (`make_rhs.py` sets: base + one-slider sidecars + one neutral ramp
per camera). Every render is read back as scene EV (largest channel through the inverted base curve of
its camera's ramp); the loss is the mean |ΔEV| of the predicted move over valid pixels (not clipped,
2 EV above the curve's black). The model, as LightCraft runs it: pivot P = log2 mean linear luminance
+ dP; Shadows lifts by ks·(P − clamp(B, lo, P)), Highlights moves by kh·(clamp(B, P, hi) − P) (kh by
sign), lo / hi = percentiles, rounded corners, ×smoothstep(0, 1, hi − lo); applied on the edge-aware base
B (guided filter, σ 1.5 % of the long edge, ε 0.35) and on the pixel next to edges (halo weight τ 0.3).
TH = [dP, ks, kh−, kh+, q_lo, q_hi, round_s, round_h] (JSON)."""
import sys, os, json
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr

SL = {'H-100': (-1, 0), 'H-50': (-0.5, 0), 'H+50': (0.5, 0), 'S+50': (0, 0.5), 'S+100': (0, 1), 'S-50': (0, -0.5), 'HS': (0.3, 0.69)}
DS, TAU = 4, 0.3


def box1(a, r, axis):
    r = int(max(1, round(r)))
    pad = [(0, 0)] * a.ndim; pad[axis] = (r + 1, r)
    c = np.cumsum(np.pad(a, pad, mode='edge'), axis=axis); n = a.shape[axis]
    return (np.take(c, np.arange(2 * r + 1, 2 * r + 1 + n), axis=axis) - np.take(c, np.arange(n), axis=axis)) / (2 * r + 1)


def gauss(a, s):
    r = max(1, int(round((np.sqrt(4 * s * s + 1) - 1) / 2)))
    for _ in range(3):
        a = box1(box1(a, r, 0), r, 1)
    return a


def guided(p, r, eps):
    mp = gauss(p, r); vp = gauss(p * p, r) - mp * mp
    a = vp / (vp + eps); b = mp - a * mp
    return gauss(a, r) * p + gauss(b, r)


def load(S):
    meta = json.load(open(f'{S}/meta.json'))['items']
    curves = {}

    def curve(cam):
        if cam not in curves:
            a = acr(S, 'ramp_' + cam.replace(' ', '_').replace('(', '').replace(')', '')).max(-1)
            row = np.log2(np.maximum(a[40:88].mean(0), 1e-12)); ev = np.linspace(-14, 1, row.size)
            o = np.maximum.accumulate(row); ok = np.concatenate([[True], np.diff(o) > 1e-5])
            curves[cam] = (o[ok], ev[ok])
        return curves[cam]

    def scene(n):
        o, ev = curve(meta[n]['camera']); a = acr(S, n)
        m = np.log2(np.maximum(a.max(-1), 1e-12))
        return np.interp(m, o, ev), (a.max(-1) < 0.985) & (m > o[0] + 2.0)
    out = []
    for p in sorted({v['photo'] for v in meta.values() if 'photo' in v}):
        l0, v0 = scene(f'{p}__base')
        items = []
        for n, (h, s) in SL.items():
            if f'{p}__{n}' in meta:
                l1, v1 = scene(f'{p}__{n}')
                items.append((n, h, s, (v0 & v1)[::DS, ::DS], np.clip(l1, -14, 1)[::DS, ::DS]))
        out.append(dict(p=p, l=np.clip(l0, -14, 1)[::DS, ::DS], items=items))
    return out


def ramp(d, w):
    return np.maximum(d, 0) if w <= 1e-4 else np.where(d <= -w, 0.0, np.where(d >= w, d, (d + w) ** 2 / (4 * w)))


def sstep(a, b, x):
    t = np.clip((x - a) / (b - a), 0, 1)
    return t * t * (3 - 2 * t)


def stats(l, th):
    v = np.sort(np.clip(l.ravel()[::7], -14, 10)); n = v.size
    p = lambda f: v[min(n - 1, max(0, int(f / 100 * n)))]
    return np.log2(np.mean(2.0 ** v)), p(th[4]), p(th[5])


def F(x, th, st, h, s):
    dP, ks, khn, khp, _, _, w, wh = th[:8]
    c, lo, hi = st; P = c + dP; rho = sstep(0.0, 1.0, hi - lo); d = 0.0
    if s:
        d = d + s * ks * rho * (ramp(P - x, w) - ramp(min(lo, P) - x, w))
    if h:
        d = d + h * (khn if h < 0 else khp) * rho * (ramp(x - P, wh) - ramp(x - max(hi, P), wh))
    return d


def loss(th, data, B, verbose=False):
    if not (0 <= th[4] < 50 and 50 < th[5] <= 100 and th[6] >= 0 and th[7] >= 0):
        return 1e9
    tot, n = 0.0, 0
    for d, b in zip(data, B):
        l = d['l']; st = stats(l, th); wb = np.exp(-((l - b) / TAU) ** 2)
        for nm_, h, s, ok, l1 in d['items']:
            e = np.abs(l + wb * F(b, th, st, h, s) + (1 - wb) * F(l, th, st, h, s) - l1)[ok].mean()
            tot += e; n += 1
            if verbose:
                print(f'  {d["p"]:9s} {nm_:6s} |dEV| {e:.3f}')
    return tot / max(n, 1)


def nelder_mead(f, x0, step, iters):
    pts = [np.array(x0, float)] + [np.array(x0, float) + np.eye(len(x0))[i] * step[i] for i in range(len(x0))]
    vals = [f(p) for p in pts]
    for _ in range(iters):
        o = np.argsort(vals); pts = [pts[i] for i in o]; vals = [vals[i] for i in o]
        c = np.mean(pts[:-1], 0); xr = c + (c - pts[-1]); fr = f(xr)
        if fr < vals[0]:
            xe = c + 2 * (c - pts[-1]); fe = f(xe)
            pts[-1], vals[-1] = (xe, fe) if fe < fr else (xr, fr)
        elif fr < vals[-2]:
            pts[-1], vals[-1] = xr, fr
        else:
            xc = c + 0.5 * (pts[-1] - c); fc = f(xc)
            if fc < vals[-1]:
                pts[-1], vals[-1] = xc, fc
            else:
                pts = [pts[0] + 0.5 * (p - pts[0]) for p in pts]; vals = [f(p) for p in pts]
    i = int(np.argmin(vals)); return pts[i], vals[i]


if __name__ == '__main__':
    data = [d for S in sys.argv[3:] for d in load(S)]
    B = [guided(d['l'], max(1.0, 0.015 * max(d['l'].shape)), 0.35) for d in data]
    th0 = json.loads(sys.argv[2])
    print('photos', len(data), 'start', round(loss(th0, data, B), 4), flush=True)
    th, v = nelder_mead(lambda t: loss(t, data, B), th0, [0.2, 0.1, 0.1, 0.1, 3, 2, 0.2, 0.2], int(sys.argv[1]))
    print('BEST', round(v, 4), json.dumps([round(float(x), 4) for x in th]))
    loss(th, data, B, verbose=True)
