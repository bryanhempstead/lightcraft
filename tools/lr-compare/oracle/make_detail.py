"""make_detail.py SET [grain|sharp|nr|all]: Detail / Effects experiments for the Camera Raw oracle.

Scenes (linear ProPhoto, EV relative to raw white, as in scenes.py):
- `flat{W}`: five horizontal grey bands (EV -7, -5, -3.5, -2, -0.8), at widths 1024 and 3072 px
  (grain strength / width per tone and whether grain scales with the image size);
- `edges`: step edges of three contrasts, 1 px and 3 px lines, a sine grating texture and a
  low-contrast noise texture (sharpening amount / radius / detail / edge masking);
- `noisy`: flat bands with Gaussian luminance + chroma noise added in camera RGB (noise reduction).
Writes SET/dng/*.dng (settings in their XMP), SET/meta.json and the manifest for ps.sh."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET = sys.argv[1]
WHAT = sys.argv[2] if len(sys.argv) > 2 else "all"
os.makedirs(SET + "/dng", exist_ok=True)
os.makedirs(SET + "/acr", exist_ok=True)
CAM = os.environ.get("CAM", "Canon EOS R6")
info = dng.dcp(CAM, [0.5, 1.0, 0.65])
BANDS = [-7.0, -5.0, -3.5, -2.0, -0.8]


def flat(w, h=None):
    h = h or w * 2 // 3
    img = np.zeros((h, w, 3))
    for i, ev in enumerate(BANDS):
        img[i * h // 5:(i + 1) * h // 5] = 2.0 ** ev
    return img


def edges(w=1024):
    h = 512
    img = np.full((h, w, 3), 2.0 ** -3.5)
    # step edges: low / mid / high contrast (rows 0..160)
    for k, (a, b) in enumerate([(-3.8, -3.2), (-4.5, -2.5), (-7.0, -1.0)]):
        x0 = 64 + k * 320
        img[0:160, x0:x0 + 128] = 2.0 ** a
        img[0:160, x0 + 128:x0 + 256] = 2.0 ** b
    # thin lines on grey (rows 176..256): 1 px dark, 1 px bright, 3 px dark
    for k, (ev, wd) in enumerate([(-6.0, 1), (-1.5, 1), (-6.0, 3)]):
        x0 = 160 + k * 320
        img[176:256, x0:x0 + wd] = 2.0 ** ev
    # sine gratings, periods 3, 6, 12 px (rows 272..384)
    x = np.arange(w)
    for k, per in enumerate([3.0, 6.0, 12.0]):
        x0, x1 = 32 + k * 330, 32 + k * 330 + 300
        g = 2.0 ** (-3.5 + 0.6 * np.sin(2 * np.pi * x[x0:x1] / per))
        img[272:384, x0:x1] = g[None, :, None]
    # low-contrast noise textures, sd 0.05 / 0.15 / 0.4 EV (rows 400..512)
    rng = np.random.default_rng(7)
    for k, sd in enumerate([0.05, 0.15, 0.4]):
        x0 = 32 + k * 330
        t = 2.0 ** (-3.5 + sd * rng.standard_normal((112, 300)))
        img[400:512, x0:x0 + 300] = t[..., None]
    return img


def noisy(w=1024):
    return flat(w)


def grain_sweep():
    out = [("g0", {})]
    for a, s, r in [(31, 11, 43), (15, 11, 43), (60, 11, 43), (50, 25, 50), (50, 0, 50), (50, 50, 50), (50, 100, 50), (50, 25, 0), (50, 25, 100), (100, 25, 50)]:
        out.append((f"g{a}_{s}_{r}", {"GrainAmount": str(a), "GrainSize": str(s), "GrainFrequency": str(r)}))
    return out


def sharp_sweep():
    out = [("s0", {})]
    for a, rad, det, m in [(126, 1.0, 25, 94), (126, 1.0, 25, 0), (40, 1.0, 25, 0), (80, 1.0, 25, 0), (150, 1.0, 25, 0), (40, 0.5, 25, 0), (40, 2.0, 25, 0), (40, 3.0, 25, 0),
                           (40, 1.0, 0, 0), (40, 1.0, 50, 0), (40, 1.0, 100, 0), (126, 1.0, 25, 30), (126, 1.0, 25, 60), (126, 1.0, 25, 100), (60, 1.2, 35, 50)]:
        out.append((f"s{a}_{rad}_{det}_{m}", {"Sharpness": str(a), "SharpenRadius": f"+{rad:.1f}", "SharpenDetail": str(det), "SharpenEdgeMasking": str(m)}))
    return out


def nr_sweep():
    out = [("n0", {})]
    for l, ld, lc, c, cd, cs in [(0, 50, 0, 25, 50, 50), (15, 50, 0, 25, 50, 50), (30, 50, 0, 25, 50, 50), (60, 50, 0, 25, 50, 50), (30, 0, 0, 25, 50, 50), (30, 100, 0, 25, 50, 50),
                                 (30, 50, 100, 25, 50, 50), (0, 50, 0, 50, 50, 50), (0, 50, 0, 100, 50, 50), (0, 50, 0, 50, 0, 50), (0, 50, 0, 50, 100, 50), (0, 50, 0, 50, 50, 100)]:
        out.append((f"n{l}_{ld}_{lc}_{c}_{cd}_{cs}", {"LuminanceSmoothing": str(l), "LuminanceNoiseReductionDetail": str(ld), "LuminanceNoiseReductionContrast": str(lc),
                                                       "ColorNoiseReduction": str(c), "ColorNoiseReductionDetail": str(cd), "ColorNoiseReductionSmoothness": str(cs)}))
    return out


jobs = []
if WHAT in ("grain", "all"):
    for w in (1024, 3072):
        jobs.append((f"flat{w}", flat(w), grain_sweep(), None))
if WHAT == "grain6000":
    # full-size photos (Bryan's cameras are 5,472-6,000 px wide) and a thin 3072 strip (does the
    # grain follow the long edge?)
    jobs.append(("flat6000x1500", flat(6000, 1500), grain_sweep(), None))
    jobs.append(("flat3072x600", flat(3072, 600), grain_sweep()[:3], None))
if WHAT in ("sharp", "all"):
    jobs.append(("edges", edges(), sharp_sweep(), None))
if WHAT in ("nr", "all"):
    jobs.append(("noisy", noisy(), nr_sweep(), "noise"))

man, meta = [], {}
for iname, img, sweep, noise in jobs:
    cam = dng.scene_to_camera(img, info)
    if noise:
        # shot-noise-like luminance noise plus independent per-channel (chroma) noise
        rng = np.random.default_rng(3)
        sd = 0.004 + 0.03 * np.sqrt(np.clip(cam, 0, None))
        cam = cam + sd * (0.6 * rng.standard_normal(cam.shape[:2])[..., None] + 0.8 * rng.standard_normal(cam.shape))
    for sname, st in sweep:
        s = dict(dng.BASE)
        s.update(st)
        name = f"{iname}__{sname}"
        dng.write_dng(f"{SET}/dng/{name}.dng", cam, CAM, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"image": iname, "settings": st}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"camera": CAM, "bands": BANDS, "items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man), "files")
