"""make_hdr_syn.py SET NAME…: synthetic HDR oracle ramps (R6 colour) for variants named like h4_w+50_e+2
(h<limit>, e/w/c/hl/b/sh ± value; 'sdr' prefix = HDR off). Image: ramp (default) or grid (IMG=grid)."""
import sys, os, json, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes
SET = sys.argv[1]
KEYS = {"e": "Exposure2012", "w": "Whites2012", "c": "Contrast2012", "hl": "Highlights2012", "b": "Blacks2012", "sh": "Shadows2012"}


def parse(name):
    st = {}
    for tok in name.split("_"):
        m = re.fullmatch(r"h(\d)", tok)
        if m:
            st.update(HDREditMode="1", HDRMaxValue=f"+{m.group(1)}.0")
            continue
        m = re.fullmatch(r"(e|w|c|hl|b|sh)([+-]\d+(?:\.\d+)?)", tok)
        if m:
            k, v = m.groups()
            st[KEYS[k]] = f"{float(v):+.2f}" if k == "e" else f"{int(float(v)):+d}"
    return st


for d in ("v", "acr"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
cam = os.environ.get("CAM", "Canon EOS R6")
tag = "R6" if "R6" in cam else "M262"
info = dng.dcp(cam, [0.5, 1.0, 0.65])
iname = os.environ.get("IMG", "ramp")
if iname == "grid":
    gimg, cells, rects = scenes.grid()
    img = dng.scene_to_camera(gimg, info)
else:
    img = dng.scene_to_camera(scenes.ramp(h=32, lo=-14.0, hi=0.5), info)
mp = f"{SET}/meta.json"
meta = json.load(open(mp))["items"] if os.path.exists(mp) else {}
man = []
for n in sys.argv[2:]:
    st = parse(n)
    name = f"{tag}_{iname}__{n}"
    s = dict(dng.BASE); s.update(st)
    dng.write_dng(f"{SET}/v/{name}.dng", img, cam, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=dng.LINEAR))
    man.append(f"{os.path.abspath(SET)}/v/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif\t\t{'32' if st.get('HDREditMode') else '16'}")
    meta[name] = {"photo": f"{tag}_{iname}", "variant": n, "settings": st, "bits": "32", "camera": cam}
open(f"{SET}/manifest.txt", "w").write("\n".join(man) + "\n")
json.dump({"items": meta}, open(mp, "w"))
print(len(man))
