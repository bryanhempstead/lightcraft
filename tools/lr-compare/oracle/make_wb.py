"""make_color.py SET: the colour chart through Calibration / HSL / Saturation / Vibrance sweeps and Summer Fields."""
import sys, os, json, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes
SET = sys.argv[1]; os.makedirs(SET + "/dng", exist_ok=True); os.makedirs(SET + "/acr", exist_ok=True)
CAM = os.environ.get("CAM", "Canon EOS R6"); info = dng.dcp(CAM, [0.5, 1.0, 0.65])
img, cells, rects = scenes.grid()
cam = dng.scene_to_camera(img, info)
import numpy as np
valid = [bool(((c := cam[r[0]:r[2], r[1]:r[3]]) >= 0).all() and (c <= 1).all()) for r in rects]
sf = open(os.path.expanduser("~/Library/Application Support/Adobe/CameraRaw/ImportedSettings/Summer Fields.xmp")).read()
dig = re.search(r'crs:RGBTable="([0-9A-F]+)"', sf).group(1); tab = re.search(rf'crs:Table_{dig}="([^"]*)"', sf).group(1)
uuid = re.search(r'crs:UUID="([0-9A-F]+)"', sf).group(1)
LOOK = (f'<crs:Look><rdf:Description crs:Name="Summer Fields" crs:Amount="1" crs:UUID="{uuid}" crs:SupportsAmount="true" crs:SupportsMonochrome="true" crs:SupportsOutputReferred="true">'
        f'<crs:Parameters><rdf:Description crs:Version="18.4" crs:ProcessVersion="15.4" crs:ConvertToGrayscale="False" crs:RGBTable="{dig}" crs:Table_{dig}="{tab}" crs:RGBTableAmount="0.5"/></crs:Parameters></rdf:Description></crs:Look>')
BANDS = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"]
SFCOL = dict(RedHue="+3", GreenHue="+44", GreenSaturation="+44", BlueHue="-19", BlueSaturation="+38", HueAdjustmentOrange="+7", HueAdjustmentYellow="+63",
             HueAdjustmentGreen="+17", HueAdjustmentAqua="-6", HueAdjustmentBlue="+13", HueAdjustmentMagenta="-1", SaturationAdjustmentRed="+2",
             SaturationAdjustmentOrange="-26", SaturationAdjustmentYellow="-26", SaturationAdjustmentGreen="-45", SaturationAdjustmentAqua="-16",
             SaturationAdjustmentBlue="-12", LuminanceAdjustmentRed="+3", LuminanceAdjustmentOrange="-2", LuminanceAdjustmentGreen="+5",
             LuminanceAdjustmentAqua="+1", LuminanceAdjustmentBlue="+7", Saturation="-30")
calib = {k: v for k, v in SFCOL.items() if k.endswith("Hue") and "Adjustment" not in k or k.endswith("Saturation") and "Adjustment" not in k and k != "Saturation"}
hsl = {k: v for k, v in SFCOL.items() if "Adjustment" in k}
S = [("base", {}, None)]
for t in [2500, 3500, 5000, 7500, 10000, 15000, 25000, 34600, 50000]:
    for ti in [0, 30]:
        S.append((f"T{t}_{ti}", dict(WhiteBalance="Custom", Temperature=str(t), Tint=f"{ti:+d}"), None))
man, meta = [], {}
for name, st, look in S:
    s = dict(dng.BASE); s.update(st); n = f"grid__{name}"
    dng.write_dng(f"{SET}/dng/{n}.dng", cam, CAM, info["spec"]["camera_white"], info, dng.xmp_packet(s, look=look, curves=dng.LINEAR))
    man.append(f"{os.path.abspath(SET)}/dng/{n}.dng\t{os.path.abspath(SET)}/acr/{n}.tif"); meta[n] = {"settings": st, "look": bool(look)}
open(f"manifest-{SET}.txt", "w").write("\n".join(man) + "\n")
json.dump({"camera": CAM, "items": meta, "cells": cells.tolist(), "rects": rects, "valid": valid}, open(f"{SET}/meta.json", "w"))
print(len(man), sum(valid), len(valid), img.shape)
