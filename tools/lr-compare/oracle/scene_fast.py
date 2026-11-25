"""scene_fast.py SET LCDIR [PHOTOS…] [--table]: scene_ana's numbers without re-reading Camera Raw's TIFFs: the Camera Raw side
(effects per band, highlight detail, neutral-pixel colour and the base's neutral mask) is cached once in SET/crcache.json and
SET/crbase/*.npy; only LightCraft's renders are read. Prints the done-when checks (docs/lr-match.md → Round 14):
(a) Shadows +50 / +100 act like Camera Raw within ±10, (b) Clarity ±100 mid-band ΔL* within 2 (sign at −100),
(c) Dehaze −50 / −100 mid lift within 3 L*, highlight detail ≥ 0.7× Camera Raw's, neutral-colour ΔE ≤ 3 (R6, GR III)."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000
from scene_ana import lc_load, hp, rms, band_eff, BANDS, parse, equiv

S, L = sys.argv[1], sys.argv[2]
args = [a for a in sys.argv[3:] if not a.startswith("--")]
meta = json.load(open(f"{S}/meta.json"))["items"]
CACHE = f"{S}/crcache.json"
os.makedirs(f"{S}/crbase", exist_ok=True)
cr = json.load(open(CACHE)) if os.path.exists(CACHE) else {}


def fit_rot(a, b, rot):
    if a.shape[:2] == b.shape[:2][::-1] and a.shape[0] != a.shape[1]:
        if rot[0] is None:
            e = {k: float(np.abs(np.rot90(b, k)[::8, ::8, 1] - a[::8, ::8, 1]).mean()) for k in (1, 3)}
            rot[0] = min(e, key=e.get)
        b = np.ascontiguousarray(np.rot90(b, rot[0]))
    return b


def neutral(a0lab):
    C0 = np.hypot(a0lab[..., 1], a0lab[..., 2])
    neu = (C0 < 8) & (a0lab[..., 0] > 25) & (a0lab[..., 0] < 85)
    if neu.mean() < 0.005:
        neu = (C0 < 14) & (a0lab[..., 0] > 20) & (a0lab[..., 0] < 90)
    return neu


def cr_stats(p):
    """Camera Raw's numbers for photo p, cached."""
    if p in cr and all(n in cr[p] for n in meta if n.startswith(p + "__")):
        return cr[p]
    out = cr.setdefault(p, {})
    a0 = np.clip(acr(S, f"{p}__base"), 0, 1)
    a0l = lab(a0)
    neu = neutral(a0l)
    np.save(f"{S}/crbase/{p}_L.npy", a0l[..., 0].astype(np.float32))
    np.save(f"{S}/crbase/{p}_neu.npy", neu)
    ta = a0l[..., 0] >= np.percentile(a0l[..., 0], 95)
    for n in meta:
        if not n.startswith(p + "__") or n in out or not os.path.exists(f"{S}/acr/{n}.tif"):
            continue
        a = lab(np.clip(acr(S, n), 0, 1)) if not n.endswith("__base") else a0l
        out[n] = {"eff": {k: band_eff(a0l[..., 0], a[..., 0], *v) for k, v in BANDS.items()},
                  "hi": rms(hp(a[..., 0])[ta]), "ab": a[neu].mean(0).tolist()}
    json.dump(cr, open(CACHE, "w"))
    return out


def lc_stats(p, n, st):
    """LightCraft's numbers for item n of photo p against Camera Raw's base (cached arrays)."""
    a0L = np.load(f"{S}/crbase/{p}_L.npy")
    neu = np.load(f"{S}/crbase/{p}_neu.npy")
    b = np.clip(lc_load(S, L, n), 0, 1)
    b0 = np.clip(lc_load(S, L, f"{p}__base"), 0, 1)
    if a0L.shape != b.shape[:2]:
        # LightCraft's oracle render ignores the EXIF turn: pick the quarter turn that matches Camera Raw's base
        if "rot" not in st:
            c = {k: np.abs(lab(np.rot90(b0, k)[::8, ::8])[..., 0] - a0L[::8, ::8]).mean() for k in (1, 3)}
            st["rot"] = min(c, key=c.get)
        b, b0 = np.ascontiguousarray(np.rot90(b, st["rot"])), np.ascontiguousarray(np.rot90(b0, st["rot"]))
    bl, b0l = lab(b), lab(b0)
    tb = b0l[..., 0] >= np.percentile(b0l[..., 0], 95)
    return {"eff": {k: band_eff(b0l[..., 0], bl[..., 0], *v) for k, v in BANDS.items()}, "hi": rms(hp(bl[..., 0])[tb]), "ab": bl[neu].mean(0).tolist(),
            "L0": b0l[..., 0]}


def run():
    photos = args or sorted({n.split("__")[0] for n in meta if os.path.exists(f"{S}/acr/{n.split('__')[0]}__base.tif")})
    rows = {}
    for p in photos:
        crp = cr_stats(p)
        st = {}
        for n in sorted(meta):
            if not n.startswith(p + "__") or n.endswith("__base") or n not in crp or not os.path.exists(f"{S}/{L}/{n}.f32"):
                continue
            if not os.path.exists(f"{S}/{L}/{p}__base.f32"):
                continue
            r = lc_stats(p, n, st)
            c = crp[n]
            ma, mb = np.array(c["ab"]), np.array(r["ab"])
            rows[n] = {"cr": c["eff"], "lc": r["eff"], "hi": r["hi"] / max(c["hi"], 1e-9), "neu": float(de2000(ma[None], mb[None])[0])}
    return rows


def summarize(rows, table=True):
    ps = sorted({parse(n)[0] for n in rows})
    if table:
        for n in sorted(rows):
            r = rows[n]
            f = lambda d: " ".join(f"{d[b]:+6.1f}" for b in BANDS)
            print(f"{n:28s} CR {f(r['cr'])} LC {f(r['lc'])} hiDet {r['hi']:5.2f} neutralDE {r['neu']:4.1f}")
    print("\nchecks:")
    worst = {"a": 0, "b": 0, "c_mid": 0, "c_hi": 9, "c_hazeDE": 0}
    for p in ps:
        # (a) Shadows acts like
        curve = [(parse(n)[2], rows[n]["cr"]["sh"]) for n in rows if parse(n)[0] == p and parse(n)[1] == "Shadows2012" and parse(n)[2] > 0] + [(0, 0.0)]
        line = f"{p:10s} "
        for v in (50, 100):
            n = f"{p}__Shadows2012_{v:+d}"
            if n in rows:
                q = equiv(curve, rows[n]["lc"]["sh"])
                line += f"S{v:+d} acts like {('%.0f' % q) if q is not None else '>' + str(v) + '?'} (LC {rows[n]['lc']['sh']:+.1f} CR {rows[n]['cr']['sh']:+.1f}) | "
                if q is not None:
                    worst["a"] = max(worst["a"], abs(q - v))
                else:
                    worst["a"] = max(worst["a"], 99 if rows[n]["lc"]["sh"] > rows[n]["cr"]["sh"] else 15)
        for v in (-100, 100):
            n = f"{p}__Clarity2012_{v:+d}"
            if n in rows:
                d = rows[n]["lc"]["mid"] - rows[n]["cr"]["mid"]
                line += f"C{v:+d} mid {rows[n]['lc']['mid']:+.1f}/{rows[n]['cr']['mid']:+.1f} | "
                worst["b"] = max(worst["b"], abs(d))
        for v in (-50, -100):
            n = f"{p}__Dehaze_{v:+d}"
            if n in rows:
                d = rows[n]["lc"]["mid"] - rows[n]["cr"]["mid"]
                line += f"D{v:+d} mid {rows[n]['lc']['mid']:+.1f}/{rows[n]['cr']['mid']:+.1f} hi {rows[n]['hi']:.2f} neu {rows[n]['neu']:.1f} | "
                worst["c_mid"] = max(worst["c_mid"], abs(d))
                worst["c_hi"] = min(worst["c_hi"], rows[n]["hi"])
                if p in ("BHM_9815", "R0003064"):
                    worst["c_hazeDE"] = max(worst["c_hazeDE"], rows[n]["neu"])
        print(line)
    print("worst: (a) |acts-like − value| %.0f (≤10)  (b) |Clarity mid Δ| %.1f (≤2)  (c) |Dehaze mid Δ| %.1f (≤3), min hiDetail ratio %.2f (≥0.7), R6/GRIII neutral dE %.1f (≤3)"
          % (worst["a"], worst["b"], worst["c_mid"], worst["c_hi"], worst["c_hazeDE"]))
    return worst


if __name__ == "__main__":
    rows = run()
    summarize(rows, "--table" in sys.argv)
    json.dump(rows, open(f"{S}/{L}.rows.json", "w"))
