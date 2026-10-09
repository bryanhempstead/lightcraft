"""make_set2.py SET: fine slider sweeps on the full-frame ramp; contrast on probes over many backgrounds."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes
SET = sys.argv[1]; os.makedirs(SET + "/dng", exist_ok=True); os.makedirs(SET + "/acr", exist_ok=True)
CAM = os.environ.get("CAM", "Canon EOS R6")
info = dng.dcp(CAM, [0.5, 1.0, 0.65])
items = []
ramp = scenes.ramp(h=32)
items.append(("ramp", ramp, "base", {}))
for v in [-4, -3, -2, -1.5, -1, -0.5, 0.5, 1, 1.5, 2, 3, 4]:
    items.append(("ramp", ramp, f"E{v:+}", {"Exposure2012": f"{v:+.2f}"}))
for k, n in [("C", "Contrast2012"), ("H", "Highlights2012"), ("S", "Shadows2012"), ("W", "Whites2012"), ("K", "Blacks2012")]:
    for v in [-100, -75, -50, -25, 25, 50, 75, 100]:
        items.append(("ramp", ramp, f"{k}{v:+}", {n: f"{v:+d}"}))
# contrast adaptivity: probes over many backgrounds, and background coverage
for bg in [-10, -9, -8, -7, -6, -5, -4, -3, -2, -1, -0.5]:
    img = scenes.probe(bg, h=256)
    for st, s in [("base", {}), ("C-100", {"Contrast2012": "-100"}), ("C+100", {"Contrast2012": "+100"})]:
        items.append((f"probe{bg:+}", img, st, s))
man, meta = [], {}
cache = {}
for iname, img, sname, st in items:
    if iname not in cache: cache[iname] = dng.scene_to_camera(img, info)
    s = dict(dng.BASE); s.update(st); name = f"{iname}__{sname}"
    dng.write_dng(f"{SET}/dng/{name}.dng", cache[iname], CAM, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=dng.LINEAR))
    man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif"); meta[name] = {"image": iname, "settings": st}
open("manifest.txt", "w").write("\n".join(man) + "\n")
json.dump({"camera": CAM, "items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man))
