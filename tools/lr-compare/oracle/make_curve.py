"""make_curve.py SET: point curves (master / per channel) and the parametric curve on the ramp and the
colour grid (R09 / R08): how Camera Raw interpolates the curve points and applies them to colour."""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes
SET = sys.argv[1]; os.makedirs(SET + "/dng", exist_ok=True); os.makedirs(SET + "/acr", exist_ok=True)
CAM = os.environ.get("CAM", "Canon EOS R6"); info = dng.dcp(CAM, [0.5, 1.0, 0.65])
ID = [(0, 0), (255, 255)]


def pts(flat):
    return [(flat[i], flat[i + 1]) for i in range(0, len(flat), 2)]


POINT = {  # name -> (master, red, green, blue)
    "linear": (ID, ID, ID, ID),
    "bryan": (pts([0, 6, 62, 39, 98, 84, 181, 167, 255, 211]), ID, ID, ID),
    "bryan2": (pts([0, 0, 68, 41, 112, 86, 176, 186, 255, 225]), ID, ID, ID),
    "scurve": (pts([0, 0, 64, 40, 192, 215, 255, 255]), ID, ID, ID),
    "close": (pts([0, 0, 100, 60, 110, 130, 255, 255]), ID, ID, ID),
    "red": (ID, pts([0, 0, 128, 160, 255, 255]), ID, ID),
    "blue": (ID, ID, ID, pts([0, 20, 128, 118, 255, 240])),
}
PARA = {"P_bryan": dict(ParametricShadows="-2", ParametricDarks="+13", ParametricLights="-3", ParametricHighlights="-6")}
for reg in ["Shadows", "Darks", "Lights", "Highlights"]:
    for v in [-100, -50, 50, 100]:
        PARA[f"P_{reg}{v:+d}"] = {f"Parametric{reg}": f"{v:+d}"}
PARA["P_splits"] = dict(ParametricLights="+50", ParametricShadowSplit="10", ParametricMidtoneSplit="40", ParametricHighlightSplit="90")
PARA["P_splits2"] = dict(ParametricDarks="+50", ParametricShadowSplit="40", ParametricMidtoneSplit="60", ParametricHighlightSplit="80")

ramp = dng.scene_to_camera(scenes.ramp(h=32), info)
gimg, cells, rects = scenes.grid()
gcam = dng.scene_to_camera(gimg, info)
valid = [bool(((c := gcam[r[0]:r[2], r[1]:r[3]]) >= 0).all() and (c <= 1).all()) for r in rects]
man, meta = [], {}


def emit(name, img, st, curves):
    s = dict(dng.BASE); s.update(st)
    dng.write_dng(f"{SET}/dng/{name}.dng", img, CAM, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=curves))
    man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
    meta[name] = {"settings": st, "curves": {k: v for k, v in curves.items()}}


for n, (m, r, g, b) in POINT.items():
    c = {"ToneCurvePV2012": m, "ToneCurvePV2012Red": r, "ToneCurvePV2012Green": g, "ToneCurvePV2012Blue": b}
    emit(f"ramp__{n}", ramp, {}, c)
    emit(f"grid__{n}", gcam, {}, c)
for n, st in PARA.items():
    emit(f"ramp__{n}", ramp, st, dng.LINEAR)
c = {"ToneCurvePV2012": POINT["bryan"][0], "ToneCurvePV2012Red": ID, "ToneCurvePV2012Green": ID, "ToneCurvePV2012Blue": ID}
emit("ramp__P_bryan+bryan", ramp, PARA["P_bryan"], c)
emit("grid__P_bryan+bryan", gcam, PARA["P_bryan"], c)
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"camera": CAM, "items": meta, "cells": cells.tolist(), "rects": rects, "valid": valid}, open(f"{SET}/meta.json", "w"))
print(len(man))
