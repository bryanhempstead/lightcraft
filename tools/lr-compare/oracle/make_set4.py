"""make_set4.py SET: pairs/triples of the global sliders on the ramp (composition order)."""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes
SET = sys.argv[1]; os.makedirs(SET + "/dng", exist_ok=True); os.makedirs(SET + "/acr", exist_ok=True)
CAM = "Canon EOS R6"; info = dng.dcp(CAM, [0.5, 1.0, 0.65])
N = {"E": "Exposure2012", "C": "Contrast2012", "W": "Whites2012", "K": "Blacks2012", "H": "Highlights2012", "S": "Shadows2012"}
combos = ["E+1,W-50", "E-1,W+50", "E+1,K+50", "E-1,K-50", "W-50,K+50", "W+100,K-100", "C+50,W-50", "C-50,K+50", "E+1,C-50", "E-1,C+50",
          "E+0.5,C-50,W-50,K+50", "E-1,C+50,W+50,K-50", "W-36,K+30", "C-58,W-33,K+31", "E+0.4,C-63,W-36,K+30", "E-0.75,C+34,K-13"]
ramp = scenes.ramp(h=32); cam = dng.scene_to_camera(ramp, info)
man, meta = [], {}
for c in combos:
    st = {}
    for t in c.split(','):
        k, v = t[0], float(t[1:]); st[N[k]] = f"{v:+.2f}" if k == "E" else f"{int(v):+d}"
    s = dict(dng.BASE); s.update(st); name = f"ramp__{c.replace(',', '_')}"
    dng.write_dng(f"{SET}/dng/{name}.dng", cam, CAM, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=dng.LINEAR))
    man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif"); meta[name] = {"image": "ramp", "settings": st}
open("manifest.txt", "w").write("\n".join(man) + "\n"); json.dump({"camera": CAM, "items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man))
