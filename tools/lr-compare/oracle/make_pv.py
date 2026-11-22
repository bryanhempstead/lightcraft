"""make_pv.py SET: the same settings under ProcessVersion 11.0 (Lightroom's Process Version 5) and 15.4
(Version 6) on the colour grid and the spatial scene (R12): does the process version change the
render of the sliders Bryan uses?"""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes
SET = sys.argv[1]; os.makedirs(SET + "/dng", exist_ok=True); os.makedirs(SET + "/acr", exist_ok=True)
CAM = os.environ.get("CAM", "Canon EOS R6"); info = dng.dcp(CAM, [0.5, 1.0, 0.65])
RECIPE = dict(Exposure2012="-1.10", Contrast2012="-72", Highlights2012="+30", Shadows2012="+69", Whites2012="-36", Blacks2012="+30",
              Saturation="-35", RedHue="+3", GreenHue="+44", GreenSaturation="+44", BlueHue="-19", BlueSaturation="+38",
              HueAdjustmentOrange="+7", HueAdjustmentYellow="+63", HueAdjustmentGreen="+17", HueAdjustmentAqua="-6", HueAdjustmentBlue="+13",
              SaturationAdjustmentOrange="-26", SaturationAdjustmentYellow="-26", SaturationAdjustmentGreen="-45", SaturationAdjustmentAqua="-16",
              LuminanceAdjustmentRed="+22", LuminanceAdjustmentMagenta="+14", ParametricShadows="-18", ParametricDarks="+20",
              ParametricLights="+48", ParametricHighlights="-6")
CURVE = {"ToneCurvePV2012": [(0, 6), (62, 39), (98, 84), (181, 167), (255, 211)], "ToneCurvePV2012Red": [(0, 0), (255, 255)],
         "ToneCurvePV2012Green": [(0, 0), (255, 255)], "ToneCurvePV2012Blue": [(0, 0), (255, 255)]}
V = {"base": {}, "E+1": {"Exposure2012": "+1.00"}, "C+50": {"Contrast2012": "+50"}, "C-72": {"Contrast2012": "-72"},
     "H-60": {"Highlights2012": "-60"}, "S+69": {"Shadows2012": "+69"}, "W-36": {"Whites2012": "-36"}, "K+30": {"Blacks2012": "+30"},
     "Clar+40": {"Clarity2012": "+40"}, "Tex+40": {"Texture": "+40"}, "Dehaze+30": {"Dehaze": "+30"}, "Vib+40": {"Vibrance": "+40"},
     "Sat-35": {"Saturation": "-35"}, "recipe": RECIPE}
grid, cells, rects = scenes.grid()
gcam = dng.scene_to_camera(grid, info)
valid = [bool(((c := gcam[r[0]:r[2], r[1]:r[3]]) >= 0).all() and (c <= 1).all()) for r in rects]
spat = dng.scene_to_camera(scenes.spatial(), info)
man, meta = [], {}
for pv in ("11.0", "15.4"):
    for n, st in V.items():
        for scene, img in (("grid", gcam), ("spatial", spat)):
            s = dict(dng.BASE); s.update(st); s["ProcessVersion"] = pv
            name = f"{scene}__{n}__pv{pv[:2]}"
            dng.write_dng(f"{SET}/dng/{name}.dng", img, CAM, info["spec"]["camera_white"], info,
                          dng.xmp_packet(s, curves=CURVE if n == "recipe" else dng.LINEAR))
            man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
            meta[name] = {"settings": st, "pv": pv}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"camera": CAM, "items": meta, "cells": cells.tolist(), "rects": rects, "valid": valid}, open(f"{SET}/meta.json", "w"))
print(len(man))
