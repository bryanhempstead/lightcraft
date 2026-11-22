"""make_hs.py SET: what Highlights / Shadows respond to (R02). Scenes: uniform fields; a two-level field
(dark share f at −6 EV, the rest at −1.5 EV) with a thin probe ramp (−10..0 EV) across the middle, for
f = 0.1 / 0.5 / 0.9 (the image's key moves, the levels don't); the f = 0.5 field scaled by 2^k (the
whole scene brighter / darker); with Exposure, Contrast and Bryan's tone sliders before them."""
import sys, os, json
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes
SET = sys.argv[1]; os.makedirs(SET + "/dng", exist_ok=True); os.makedirs(SET + "/acr", exist_ok=True)
CAM = os.environ.get("CAM", "Canon EOS R6"); info = dng.dcp(CAM, [0.5, 1.0, 0.65])
W, H = scenes.W, 512


def field(f, k=0.0, lo=-6.0, hi=-1.5):
    img = np.full((H, W, 3), 2.0 ** (hi + k))
    img[:, : int(round(f * W))] = 2.0 ** (lo + k)
    ev = np.linspace(-10, 0, W) + k
    img[H // 2 - 6: H // 2 + 6] = (2.0 ** ev)[None, :, None]
    return img


SCENES = {f"u{e}": np.full((128, 256, 3), 2.0 ** e) for e in (-8, -6, -4, -2, 0)}
for f in (0.1, 0.5, 0.9):
    SCENES[f"f{int(f * 100)}"] = field(f)
for k in (-2, -1, 1):
    SCENES[f"f50k{k:+d}"] = field(0.5, k)
SL = {"base": {}, "S+100": {"Shadows2012": "+100"}, "S+50": {"Shadows2012": "+50"}, "H-100": {"Highlights2012": "-100"}, "H-50": {"Highlights2012": "-50"},
      "H+50": {"Highlights2012": "+50"}, "S-50": {"Shadows2012": "-50"}}
EXTRA = {"E-1": {"Exposure2012": "-1.00"}, "E-1,S+69": {"Exposure2012": "-1.00", "Shadows2012": "+69"}, "C-72": {"Contrast2012": "-72"},
         "C-72,S+69": {"Contrast2012": "-72", "Shadows2012": "+69"}, "S+69": {"Shadows2012": "+69"}, "H+30": {"Highlights2012": "+30"},
         "S+69,H+30": {"Shadows2012": "+69", "Highlights2012": "+30"},
         "tone": dict(Exposure2012="-1.10", Contrast2012="-72", Highlights2012="+30", Shadows2012="+69", Whites2012="-36", Blacks2012="+30"),
         "tone-noHS": dict(Exposure2012="-1.10", Contrast2012="-72", Whites2012="-36", Blacks2012="+30")}
man, meta = [], {}
for sn, img in SCENES.items():
    cam = dng.scene_to_camera(img, info)
    sets = dict(SL)
    if sn in ("f50",):
        sets.update(EXTRA)
    if sn.startswith("u"):
        sets = {k: v for k, v in SL.items() if k in ("base", "S+100", "H-100")}
    for n, st in sets.items():
        s = dict(dng.BASE); s.update(st)
        name = f"{sn}__{n.replace(',', '_')}"
        dng.write_dng(f"{SET}/dng/{name}.dng", cam, CAM, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"scene": sn, "settings": st}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"camera": CAM, "items": meta}, open(f"{SET}/meta.json", "w"))
print(len(man))
