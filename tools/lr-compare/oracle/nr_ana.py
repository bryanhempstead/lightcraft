"""nr_ana.py SET [LCDIR=lc]: noise reduction on make_detail.py's `noisy` flats, Camera Raw vs LightCraft.

Per setting and grey band: luminance noise left (L* sd), colour noise left (sd of a*, b* combined)
and the colour noise's grain (lag-1 autocorrelation of a*), each as a ratio to the same file
rendered with no noise reduction at all (so the two renderers' own base noise cancels)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

SET = sys.argv[1]
LCD = sys.argv[2] if len(sys.argv) > 2 else "lc"
meta = json.load(open(f"{SET}/meta.json"))


def load(kind, name):
    if kind == "acr":
        return common.lab(common.acr(SET, name))
    b = np.fromfile(f"{SET}/{LCD}/{name}.f32", "<f4")
    w, h = b[:2].view("<u4")
    return common.lab(b[2:].reshape(h, w, 3).astype(np.float64))


def rho1(a):
    a = a - a.mean()
    return 0.5 * ((a[:, 1:] * a[:, :-1]).mean() + (a[1:] * a[:-1]).mean()) / max(a.var(), 1e-12)


def stats(lab, bi):
    h = lab.shape[0]
    t = lab[bi * h // 5 + 16:(bi + 1) * h // 5 - 16, 16:-16]
    return t[..., 0].std(), np.hypot(t[..., 1].std(), t[..., 2].std()), rho1(t[..., 1])


names = sorted(n for n, v in meta["items"].items() if v["image"] == "noisy")
base = {k: load(k, "noisy__n0") for k in ("acr", "lc")}
bs = {k: [stats(base[k], b) for b in range(5)] for k in base}
print("base noise (L* sd / chroma sd) acr:", " ".join(f"{s[0]:.2f}/{s[1]:.2f}" for s in bs["acr"]), "| lc:", " ".join(f"{s[0]:.2f}/{s[1]:.2f}" for s in bs["lc"]))
print(f"{'setting':<24} band | L left acr  lc | C left acr  lc | rho1(a*) acr  lc")
err = []
for n in names:
    if n.endswith("__n0"):
        continue
    img = {k: load(k, n) for k in ("acr", "lc")}
    for b in (1, 2, 3):
        a, l = stats(img["acr"], b), stats(img["lc"], b)
        ra = (a[0] / bs["acr"][b][0], a[1] / bs["acr"][b][1])
        rl = (l[0] / bs["lc"][b][0], l[1] / bs["lc"][b][1])
        err += [np.log(rl[0] / ra[0]), np.log(rl[1] / ra[1])]
        print(f"{n.split('__')[1]:<24} {b}    | {ra[0]:.3f} {rl[0]:.3f} | {ra[1]:.3f} {rl[1]:.3f} | {a[2]:+.3f} {l[2]:+.3f}")
print(f"\nmean |log ratio| of noise left (L and C): {np.mean(np.abs(err)):.3f}")
