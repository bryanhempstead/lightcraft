import json, sys, numpy as np
from hslfit import adjust
from common import lab
from cmp import de2000
S = "g1"; m = json.load(open(f"{S}/meta.json")); v = np.array(m["valid"]); s = np.array(m["cells"])
base = np.load(f"{S}/p_acr/grid__base.npy"); okb = v & (base.max(1) < .99) & (np.log2(s.max(1)) > -7.5)
V = [-100, -50, 0, 50, 100]
def table(T, o, val):
    if val == 0: return None
    i = np.searchsorted(V, val); lo, hi = V[i - 1], V[i] if V[i - 1] != val else V[i - 1]
    if val in V: return T[f"{o}{val:+d}"]
    a = (val - lo) / (hi - lo); tl = 0 if lo == 0 else T[f"{o}{lo:+d}"]; th = 0 if hi == 0 else T[f"{o}{hi:+d}"]
    return (1 - a) * tl + a * th
def settings_deltas(T, st, axes):
    out = []
    for k, val in st.items():
        o = k.replace("Adjustment", ""); t = table(T, o, int(val))
        if t is not None: out.append((t, axes))
    return out
if __name__ == "__main__":
    ax = sys.argv[1]; T = np.load(f"hsltab_{ax}.npy", allow_pickle=True).item(); axes = tuple(ax)
    meta = json.load(open(f"{S}/meta.json"))["items"]
    for n in ["grid__SFhsl", "grid__Sat-30"]:
        st = meta[n]["settings"]; a = np.load(f"{S}/p_acr/{n}.npy"); ok = okb & (a.max(1) < .99)
        pred = adjust(base[ok], settings_deltas(T, st, axes)); d = de2000(lab(pred), lab(a[ok]))
        lc = np.load(f"{S}/p_lc/{n}.npy"); d2 = de2000(lab(lc[ok]), lab(a[ok]))
        print(n, "model", d.mean().round(2), "p95", np.percentile(d, 95).round(2), "| lightcraft now", d2.mean().round(2))
