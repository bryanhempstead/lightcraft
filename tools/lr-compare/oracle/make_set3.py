"""make_set3.py SET: Highlights / Shadows structure — flats, step edges, patch sizes."""
import sys, os, json, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng
SET = sys.argv[1]; os.makedirs(SET + "/dng", exist_ok=True); os.makedirs(SET + "/acr", exist_ok=True)
CAM = "Canon EOS R6"; info = dng.dcp(CAM, [0.5, 1.0, 0.65])
W = 1024
def flat(ev, h=128, w=256): return np.full((h, w, 3), 2.0 ** ev)
def edge(lo, hi, h=256):
    img = np.full((h, W, 3), 2.0 ** lo); img[:, W // 2:] = 2.0 ** hi; return img
def patch(bg, fg, size, h=512):
    img = np.full((h, W, 3), 2.0 ** bg); c = (h // 2, W // 2); s = size // 2
    img[c[0] - s:c[0] + s, c[1] - s:c[1] + s] = 2.0 ** fg; return img
S_ = [("base", {})] + [(f"{k}{v:+}", {n: f"{v:+d}"}) for k, n in [("H", "Highlights2012"), ("S", "Shadows2012")] for v in [-100, -50, 50, 100]]
images = {f"flat{e:+}": flat(e) for e in range(-11, 1)}
images.update({"edge-6-1.5": edge(-6, -1.5), "edge-8-4": edge(-8, -4), "edge-4-0.5": edge(-4, -0.5)})
for size in [8, 32, 128, 384]:
    images[f"patchD{size}"] = patch(-7, -1, size)   # bright patch on dark
    images[f"patchB{size}"] = patch(-1, -7, size)   # dark patch on bright
man, meta, cache = [], {}, {}
for iname, img in images.items():
    cam = dng.scene_to_camera(img, info)
    for sname, st in S_:
        s = dict(dng.BASE); s.update(st); name = f"{iname}__{sname}"
        dng.write_dng(f"{SET}/dng/{name}.dng", cam, CAM, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif"); meta[name] = {"image": iname, "settings": st}
open("manifest.txt", "w").write("\n".join(man) + "\n")
json.dump({"camera": CAM, "items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man))
