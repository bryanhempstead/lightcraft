"""hdr_align.py SET CAM anchor: Camera Raw's HDR ramp output against EV past the point where the variant's
SDR twin (LightCraft, HDR off) reaches `anchor`; universal if the rows agree."""
import sys, numpy as np, hdr_common as hc
SET, cam, anchor = sys.argv[1], sys.argv[2], float(sys.argv[3])
VS = ["h4", "h4_e+1", "h4_e+3", "h4_e-1", "h4_w+50", "h4_w+100", "h4_w-50", "h4_c+50", "h4_c-50", "h4_b-50", "h4_hl+60", "h4_hl-60"]
out = {}
for v in VS:
    n = f"{cam}_ramp__{v}"
    a, _ = hc.acr(f"{SET}/acr/{n}.tif"); s = hc.lc(f"{SET}/sdrtwin/{n}.f32")
    d = np.load(f"{SET}/lc1-dump/{n}.npy")
    h = min(a.shape[0], s.shape[0], d.shape[0]); w = min(a.shape[1], s.shape[1], d.shape[1]); r = slice(4, h - 4)
    A = np.median(hc.Y(a[r, :w]), 0); S = np.median(hc.Y(s[r, :w]), 0); ev = np.median(d[r, :w, 0], 0)
    k = np.argmax(S >= anchor)
    e0 = ev[k] if S[k] >= anchor else np.nan
    out[v] = (ev - e0, A, S)
grid = np.arange(-1.0, 3.01, 0.25)
print(f"anchor SDR {anchor}: CR HDR at EV past it")
print("      " + " ".join(f"{g:+6.2f}" for g in grid))
for v, (x, A, S) in out.items():
    print(f"{v:9s}" + " ".join(f"{A[np.argmin(abs(x - g))]:6.2f}" if np.nanmin(abs(x - g)) < 0.05 else "     ." for g in grid))
