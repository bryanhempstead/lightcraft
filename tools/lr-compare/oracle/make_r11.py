"""make_r11.py SET WHAT[,WHAT…] [ACC]: Round 11 experiments for the Camera Raw oracle (docs/lr-match.md → Round 11).

WHAT:
- `syn`   Clarity / Dehaze / Highlights / Shadows sweeps on synthetic scenes: the colour grid, the spatial
          scene, the frequency scene, and two photo-like scenes (`land`: a hazy landscape — sky, three ridges
          fading into haze, a textured foreground; `room`: a dark textured interior with a bright window and a
          lamp);
- `mask`  the masks' Highlights / Shadows / Clarity / Dehaze in a full-frame gradient (and a half-frame one)
          on the spatial and land scenes, each next to the same global slider (R16);
- `real`  Bryan's split-A raws (ACC = the acceptance work folder, `sample.json`; readable volumes only) as
          APFS clones with one-slider sidecars (Clarity, Dehaze, Highlights, Shadows, Texture, his tone recipe).

Writes SET/dng/*.dng (or SET/v/* clones + sidecars), SET/meta.json and the manifest for ps.sh."""
import sys, os, json, subprocess
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes

SET = sys.argv[1]
WHAT = sys.argv[2].split(",") if len(sys.argv) > 2 else ["syn"]
ACC = sys.argv[3] if len(sys.argv) > 3 else None
for d in ("dng", "acr", "v", "src"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
CAM = "Canon EOS R6"
info = dng.dcp(CAM, [0.5, 1.0, 0.65])

PERIODS = [3, 5, 8, 13, 21, 34, 55, 89, 144, 233]
VARIANTS = [(8, 0.1, -3.5), (8, 1.2, -3.5), (34, 0.1, -3.5), (34, 1.2, -3.5), (8, 0.4, -6.5), (8, 0.4, -1.2), (34, 0.4, -6.5), (34, 0.4, -1.2)]
BAND = 40


def freq():
    bands = [(p, 0.4, -3.5) for p in PERIODS] + VARIANTS
    img = np.zeros((BAND * len(bands), 1024, 3))
    x = np.arange(1024)
    for i, (p, a, b) in enumerate(bands):
        img[i * BAND:(i + 1) * BAND] = (2.0 ** (b + a * np.sin(2 * np.pi * x / p)))[None, :, None]
    return img


def noise(rng, h, w, scale):
    """smooth noise of unit-ish amplitude at `scale` px (sum of bilinear-upsampled random grids)"""
    out = np.zeros((h, w))
    for s in (scale, scale / 2, scale / 4):
        gh, gw = int(h / s) + 2, int(w / s) + 2
        g = rng.standard_normal((gh, gw))
        yy, xx = np.mgrid[0:h, 0:w] / s
        y0, x0 = yy.astype(int), xx.astype(int)
        ty, tx = yy - y0, xx - x0
        out += (g[y0, x0] * (1 - tx) * (1 - ty) + g[y0, x0 + 1] * tx * (1 - ty) + g[y0 + 1, x0] * (1 - tx) * ty + g[y0 + 1, x0 + 1] * tx * ty) * (s / scale) ** 0.5
    return out


def land(h=683, w=1024):
    """a hazy landscape: bright bluish sky, three ridges veiled more with distance, textured foreground"""
    rng = np.random.default_rng(7)
    air = np.array([0.55, 0.62, 0.75])
    img = np.zeros((h, w, 3))
    y = np.arange(h)[:, None] / h
    img[:] = air * (0.9 + 0.5 * (1 - y))[..., None]  # sky, brighter at the top
    x = np.arange(w)[None, :]
    surf = [np.array([0.10, 0.12, 0.07]), np.array([0.07, 0.09, 0.05]), np.array([0.05, 0.05, 0.03])]
    for k, (base, amp, t) in enumerate([(0.42, 0.06, 0.35), (0.55, 0.05, 0.6), (0.7, 0.04, 0.9)]):
        ridge = base + amp * np.sin(x / (90 + 40 * k) + k) + 0.02 * noise(rng, 1, w, 40)[0]
        mask = (np.arange(h)[:, None] / h) > ridge
        tex = 2.0 ** (0.5 * noise(rng, h, w, 6 + 6 * k))
        col = surf[k][None, None, :] * tex[..., None]
        img = np.where(mask[..., None], col * t + air * (1 - t) * 0.8, img)
    # a white house and a dark tree in the foreground
    img[int(0.78 * h):int(0.86 * h), 700:760] = [0.7, 0.68, 0.62]
    img[int(0.72 * h):int(0.95 * h), 200:240] = [0.012, 0.016, 0.01]
    return img


def room(h=683, w=1024):
    """a dark interior: textured walls and floor around −7…−4 EV, a bright window (0 EV) with mullions, a lamp"""
    rng = np.random.default_rng(3)
    ev = -5.5 + 0.6 * noise(rng, h, w, 20) + 0.25 * noise(rng, h, w, 3)
    ev[int(0.65 * h):] -= 1.2  # floor
    win = (slice(int(0.15 * h), int(0.55 * h)), slice(560, 860))
    ev[win] = -0.3 + 0.2 * noise(rng, int(0.55 * h) - int(0.15 * h), 300, 30)
    ev[int(0.15 * h):int(0.55 * h), 705:715] = -6.0  # mullion
    ev[int(0.34 * h):int(0.36 * h), 560:860] = -6.0
    yy, xx = np.mgrid[0:h, 0:w]
    ev = np.maximum(ev, -1.0 - ((xx - 250) ** 2 + (yy - 300) ** 2) / (2 * 30.0 ** 2) * 0.2)  # lamp glow
    img = np.repeat((2.0 ** ev)[..., None], 3, -1) * np.array([1.0, 0.9, 0.75])
    return img


def mask_xml(locals_, comp):
    a = " ".join(f'crs:{k}="{v}"' for k, v in locals_.items())
    return ('<crs:MaskGroupBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" '
            f'crs:CorrectionActive="true" crs:CorrectionName="m" crs:CorrectionSyncID="A1" {a}><crs:CorrectionMasks><rdf:Seq>{comp}'
            '</rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li></rdf:Seq></crs:MaskGroupBasedCorrections>')


GRAD_ALL = '<rdf:li crs:What="Mask/Gradient" crs:MaskActive="true" crs:MaskName="g" crs:MaskBlendMode="0" crs:MaskInverted="false" crs:MaskValue="1" crs:ZeroX="-2" crs:ZeroY="0.5" crs:FullX="-1" crs:FullY="0.5"/>'
GRAD_HALF = '<rdf:li crs:What="Mask/Gradient" crs:MaskActive="true" crs:MaskName="g" crs:MaskBlendMode="0" crs:MaskInverted="false" crs:MaskValue="1" crs:ZeroX="0.35" crs:ZeroY="0.5" crs:FullX="0.65" crs:FullY="0.5"/>'

CLA = [(f"Cla{v:+d}", {"Clarity2012": f"{v:+d}"}) for v in (-100, -50, -25, 25, 50, 100)]
DEH = [(f"Deh{v:+d}", {"Dehaze": f"{v:+d}"}) for v in (-50, -25, 15, 30, 60, 100)]
HS = [("H-100", {"Highlights2012": "-100"}), ("H-50", {"Highlights2012": "-50"}), ("H+50", {"Highlights2012": "+50"}),
      ("S+50", {"Shadows2012": "+50"}), ("S+100", {"Shadows2012": "+100"}), ("S-50", {"Shadows2012": "-50"}),
      ("HS", {"Highlights2012": "+30", "Shadows2012": "+69"}), ("H-100S+100", {"Highlights2012": "-100", "Shadows2012": "+100"})]
TEX = [("Tex+50", {"Texture": "+50"}), ("Tex-50", {"Texture": "-50"})]
TONE = [("tone", dict(Exposure2012="-1.10", Contrast2012="-72", Highlights2012="+30", Shadows2012="+69", Whites2012="-36", Blacks2012="+30")),
        ("tone-noHS", dict(Exposure2012="-1.10", Contrast2012="-72", Whites2012="-36", Blacks2012="+30"))]
BASE = [("base", {})]

jobs = []  # (scene, image, [(name, settings, extra xml)])
x0 = lambda L: [(n, s, "") for n, s in L]
if "syn" in WHAT:
    full = x0(BASE + CLA + DEH + HS + TEX)
    jobs += [("grid", scenes.grid()[0], full), ("spatial", scenes.spatial(), full), ("land", land(), full + x0(TONE)),
             ("room", room(), full + x0(TONE)),
             ("freq1", freq(), x0(BASE + [c for c in CLA if c[0] in ("Cla-50", "Cla+50", "Cla+100")] + [d for d in DEH if d[0] in ("Deh+30", "Deh+100", "Deh-50")]))]
if "mask" in WHAT:
    loc = [("Hig-60", "LocalHighlights2012", "-0.6", "Highlights2012", "-60"), ("Sha+60", "LocalShadows2012", "0.6", "Shadows2012", "+60"),
           ("Hig+40", "LocalHighlights2012", "0.4", "Highlights2012", "+40"), ("Sha-40", "LocalShadows2012", "-0.4", "Shadows2012", "-40"),
           ("Cla+40", "LocalClarity2012", "0.4", "Clarity2012", "+40"), ("Cla-40", "LocalClarity2012", "-0.4", "Clarity2012", "-40"),
           ("Deh+30", "LocalDehaze", "0.3", "Dehaze", "+30"), ("Deh+60", "LocalDehaze", "0.6", "Dehaze", "+60")]
    sw = [("base", {}, "")]
    for n, lk, lv, gk, gv in loc:
        sw.append(("l" + n, {}, mask_xml({lk: lv}, GRAD_ALL)))
        sw.append(("g" + n, {gk: gv}, ""))
    sw.append(("half_lCla+40", {}, mask_xml({"LocalClarity2012": "0.4"}, GRAD_HALF)))
    sw.append(("half_lDeh+60", {}, mask_xml({"LocalDehaze": "0.6"}, GRAD_HALF)))
    sw.append(("half_lHig-60", {}, mask_xml({"LocalHighlights2012": "-0.6"}, GRAD_HALF)))
    jobs += [("spatial", scenes.spatial(), sw), ("land", land(), sw)]

man, meta = [], {}
for iname, img, sweep in jobs:
    cam = dng.scene_to_camera(img, info)
    for sname, st, extra in sweep:
        s = dict(dng.BASE)
        s.update(st)
        name = f"{iname}__{sname}"
        if name in meta:
            continue
        xmp = dng.xmp_packet(s, look=extra or None, curves=dng.LINEAR)
        dng.write_dng(f"{SET}/dng/{name}.dng", cam, CAM, info["spec"]["camera_white"], info, xmp)
        man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"image": iname, "settings": st, "extra": bool(extra)}

PHOTOS = ["L1009234", "L1005500", "L1004944", "L1006270", "BHM_9795", "BHM_8476", "BHM_8480", "R0003062", "R0002574"]
REAL = BASE + [c for c in CLA if c[0] in ("Cla-50", "Cla+50", "Cla+100")] + [d for d in DEH if d[0] in ("Deh-25", "Deh-50", "Deh+30", "Deh+100")] + \
    [h for h in HS if h[0] in ("H-100", "H+50", "S+100", "S-50", "H-100S+100")] + [("Tex+50", {"Texture": "+50"})] + TONE
if "real" in WHAT:
    sample = {os.path.splitext(os.path.basename(p.get("original") or p["path"]))[0]: p for p in json.load(open(f"{ACC}/sample.json"))}
    for stem in PHOTOS:
        p = sample[stem]
        assert p["split"] == "A" and "/LOT48/" not in p["path"], stem
        ext = os.path.splitext(p["path"])[1]
        src = f"{SET}/src/{stem}{ext}"
        if not os.path.exists(src):
            subprocess.run(["/bin/cp", p["path"], src], check=True)
        for n, st in REAL:
            name = f"{stem}__{n}"
            dst = f"{SET}/v/{name}{ext}"
            if not os.path.exists(dst):
                subprocess.run(["/bin/cp", "-c", src, dst], check=True)
            s = dict(dng.BASE)
            s.update(st)
            open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
            man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin")
            meta[name] = {"photo": stem, "camera": p["model"], "settings": st}

grid_img, cells, rects = scenes.grid()
gcam = dng.scene_to_camera(grid_img, info)
valid = [bool(((c := gcam[r[0]:r[2], r[1]:r[3]]) >= 0).all() and (c <= 1).all()) for r in rects]
old = json.load(open(f"{SET}/meta.json")) if os.path.exists(f"{SET}/meta.json") else {"items": {}}
old["items"].update(meta)
json.dump({"camera": CAM, "items": old["items"], "cells": cells.tolist(), "rects": rects, "valid": valid}, open(f"{SET}/meta.json", "w"))
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
print(len(man), "files")
