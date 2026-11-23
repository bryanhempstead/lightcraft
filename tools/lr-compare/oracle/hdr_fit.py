"""hdr_fit.py SET…: Camera Raw's HDR tone as LightCraft's SDR tone (the HDR-off twin) times 2^g, with
g(Δ, Whites, Contrast) on Δ = EV past where the SDR tone reaches 0.5. Prints the tables (Rust) and
the fit's error per variant."""
import sys, os, json, re, numpy as np, hdr_common as hc
SETS = sys.argv[1:]
if not SETS:
    sys.exit(__doc__)
D = np.round(np.arange(-8.0, 5.01, 0.25), 2)
rows = []
for SET in SETS:
    meta = json.load(open(f"{SET}/meta.json"))["items"]
    for n, m in meta.items():
        st = m["settings"]
        if "_ramp__" not in n or st.get("HDREditMode") != "1" or m["camera"] != "Canon EOS R6":
            continue
        if "Shadows2012" in st or st.get("HDRMaxValue") != "+4.0":
            continue
        try:
            a, _ = hc.acr(f"{SET}/acr/{n}.tif"); s = hc.lc(f"{SET}/sdrtwin/{n}.f32"); d = np.load(f"{SET}/lc1-dump/{n}.npy")
        except (FileNotFoundError, AssertionError):
            continue
        h = min(a.shape[0], s.shape[0], d.shape[0]); w = min(a.shape[1], s.shape[1], d.shape[1]); r = slice(4, h - 4)
        A = np.median(hc.Y(a[r, :w]), 0); S = np.median(hc.Y(s[r, :w]), 0); ev = np.median(d[r, :w, 0], 0)
        k = np.argmax(S >= 0.5)
        x = ev - ev[k]
        ok = (A < 15.5) & (S > 0.002) & (np.r_[np.diff(ev) > 0, True])
        # the ramp's last 0.5 EV is raw-clipped: drop where the scene stops rising
        ok &= ev < ev.max() - 0.3
        rows.append(dict(n=n, wv=float(st.get("Whites2012", 0)), cv=float(st.get("Contrast2012", 0)), hv=float(st.get("Highlights2012", 0)), bv=float(st.get("Blacks2012", 0)), x=x[ok], g=np.log2(A[ok] / S[ok]), A=A[ok], S=S[ok]))
print(len(rows), "ramps")


def table(sel):
    if not sel:
        return np.full(len(D), np.nan)
    xs = np.concatenate([r["x"] for r in sel]); gs = np.concatenate([r["g"] for r in sel])
    t = np.full(len(D), np.nan)
    for i, d in enumerate(D):
        m = abs(xs - d) < 0.13
        if m.sum() > 3:
            t[i] = np.median(gs[m])
    return t


WS, CS, HS, BS = [-100, -50, 0, 25, 50, 75, 100], [-100, -50, 0, 50, 100], [-100, -60, -50, 50, 60, 100], [-100, -50, 50, 100]
only = lambda **kv: [r for r in rows if all(r[k] == kv.get(k, 0) for k in ("wv", "cv", "hv", "bv"))]
g0 = table(only())
gw = {w: table(only(wv=w)) - g0 for w in WS if w}
gc = {c: table(only(cv=c)) - g0 for c in CS if c}
gh = {h: table(only(hv=h)) - g0 for h in HS}
gb = {b: table(only(bv=b)) - g0 for b in BS}
np.set_printoptions(precision=3, suppress=True, linewidth=200)
print("D ", D)
print("g0", g0)
for w, t in gw.items(): print(f"w{w:+d}", t)
for c, t in gc.items(): print(f"c{c:+d}", t)
for h, t in gh.items(): print(f"h{h:+d}", t)
for b, t in gb.items(): print(f"b{b:+d}", t)
np.savez(os.environ.get("OUT", "gtables.npz"), D=D, g0=g0, **{f"w{w}": t for w, t in gw.items()}, **{f"c{c}": t for c, t in gc.items()}, **{f"h{h}": t for h, t in gh.items()}, **{f"b{b}": t for b, t in gb.items()})
