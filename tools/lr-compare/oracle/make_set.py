"""make_set.py SET: DNGs (scene x settings) + manifest for Camera Raw, into SET/."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes
SET = sys.argv[1]; os.makedirs(SET + "/dng", exist_ok=True); os.makedirs(SET + "/acr", exist_ok=True)
CAM = os.environ.get("CAM", "Canon EOS R6")
info = dng.dcp(CAM, [0.5, 1.0, 0.65])
def sweep():
    out = [("base", {})]
    for v in [-2, -1, 1, 2]: out.append((f"E{v:+}", {"Exposure2012": f"{v:+.2f}"}))
    for k, n in [("C", "Contrast2012"), ("H", "Highlights2012"), ("S", "Shadows2012"), ("W", "Whites2012"), ("K", "Blacks2012")]:
        for v in [-100, -50, 50, 100]: out.append((f"{k}{v:+}", {n: f"{v:+d}"}))
    # Bryan's preset tone and some catalog combos
    out.append(("SFtone", {"Highlights2012": "-90", "Shadows2012": "+43", "Whites2012": "-36", "Blacks2012": "+30"}))
    out.append(("combo1", {"Exposure2012": "-0.33", "Contrast2012": "-58", "Highlights2012": "-100", "Shadows2012": "+68", "Whites2012": "-33", "Blacks2012": "+31"}))
    out.append(("combo2", {"Exposure2012": "+0.40", "Contrast2012": "-63", "Highlights2012": "-52", "Shadows2012": "+83", "Whites2012": "-36", "Blacks2012": "+30"}))
    out.append(("combo3", {"Exposure2012": "-0.75", "Contrast2012": "+34", "Highlights2012": "+23", "Shadows2012": "+40", "Blacks2012": "-13"}))
    out.append(("combo4", {"Exposure2012": "-2.70", "Contrast2012": "-100", "Highlights2012": "+77", "Shadows2012": "+67", "Whites2012": "+15", "Blacks2012": "+68"}))
    out.append(("combo5", {"Exposure2012": "+1.80", "Contrast2012": "+32", "Highlights2012": "-100", "Shadows2012": "+40", "Blacks2012": "-19"}))
    for k, n in [("CL", "Clarity2012"), ("TX", "Texture"), ("DH", "Dehaze")]:
        for v in [-50, 50]: out.append((f"{k}{v:+}", {n: f"{v:+d}"}))
    return out
imgs = {"ramp": scenes.ramp(), "spatial": scenes.spatial()}
for bg in [-8, -6, -4, -2, -0.5]: imgs[f"probe{bg:+.1f}"] = scenes.probe(bg)
man = []
meta = {}
for iname, img in imgs.items():
    cam = dng.scene_to_camera(img, info)
    for sname, st in sweep():
        s = dict(dng.BASE); s.update(st)
        name = f"{iname}__{sname}"
        dng.write_dng(f"{SET}/dng/{name}.dng", cam, CAM, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"image": iname, "settings": st}
open("manifest.txt", "w").write("\n".join(man) + "\n")
json.dump({"camera": CAM, "items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man), "files")
