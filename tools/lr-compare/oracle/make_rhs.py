"""make_rhs.py SET ACC: Highlights / Shadows on real photos and on more synthetic layouts (R02).

Real raws: split-A photos of the acceptance set (ACC = its work folder, `sample.json`), APFS clones
with single-slider and combined sidecars. Synthetic (R6 colour): a smooth gradient with no edges, a
three-level field, a low-contrast two-level field, a small bright square, a Gaussian blob, plus one
neutral ramp per camera (the base tone curve, to read renders back as scene EV)."""
import sys, os, json, subprocess
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET, ACC = sys.argv[1], sys.argv[2]
for d in ("dng", "acr", "v", "src"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
W, H = 1024, 512
PHOTOS = ["L1009234", "L1005500", "L1004944", "L1009603", "BHM_5214", "BHM_7515", "BHM_9795", "BHM_0333", "R0003062", "R0002574"]
REAL = [("base", {}), ("H-100", {"Highlights2012": "-100"}), ("H-50", {"Highlights2012": "-50"}), ("H+50", {"Highlights2012": "+50"}),
        ("S+50", {"Shadows2012": "+50"}), ("S+100", {"Shadows2012": "+100"}), ("S-50", {"Shadows2012": "-50"}),
        ("HS", {"Highlights2012": "+30", "Shadows2012": "+69"}), ("H-100S+100", {"Highlights2012": "-100", "Shadows2012": "+100"}),
        ("tone", dict(Exposure2012="-1.10", Contrast2012="-72", Highlights2012="+30", Shadows2012="+69", Whites2012="-36", Blacks2012="+30")),
        ("tone-noHS", dict(Exposure2012="-1.10", Contrast2012="-72", Whites2012="-36", Blacks2012="+30"))]
SYN = [("base", {}), ("H-100", {"Highlights2012": "-100"}), ("H+50", {"Highlights2012": "+50"}), ("S+100", {"Shadows2012": "+100"}),
       ("S-50", {"Shadows2012": "-50"}), ("HS", {"Highlights2012": "+30", "Shadows2012": "+69"})]
x = np.linspace(0, 1, W)[None, :]
yy, xx = np.mgrid[0:H, 0:W]


def ev_img(e):
    return np.repeat((2.0 ** e)[..., None], 3, -1)


SCENES = {
    "grad": ev_img(np.broadcast_to(-8 + 7 * x, (H, W))),
    "lvl3": ev_img(np.where(xx < W / 3, -7.0, np.where(xx < 2 * W / 3, -4.0, -1.0))),
    "lowc": ev_img(np.where(xx < W / 2, -4.5, -2.5)),
    "sq5": ev_img(np.where((abs(xx - W / 2) < 80) & (abs(yy - H / 2) < 80), -1.0, -6.0)),
    "blob": ev_img(-6 + 5 * np.exp(-((xx - W / 2) ** 2 + (yy - H / 2) ** 2) / (2 * 120.0 ** 2))),
}
man, meta = [], {}
cam = "Canon EOS R6"
info = dng.dcp(cam, [0.5, 1.0, 0.65])
for sn, img in SCENES.items():
    c = dng.scene_to_camera(img, info)
    for n, st in SYN:
        s = dict(dng.BASE); s.update(st)
        name = f"{sn}__{n}"
        dng.write_dng(f"{SET}/dng/{name}.dng", c, cam, info["spec"]["camera_white"], info, dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"scene": sn, "settings": st, "camera": cam}
sample = {os.path.splitext(os.path.basename(p.get("original") or p["path"]))[0]: p for p in json.load(open(f"{ACC}/sample.json"))}
cams = set()
for stem in PHOTOS:
    p = sample[stem]
    assert p["split"] == "A", stem
    cams.add(p["model"])
    ext = os.path.splitext(p["path"])[1]
    src = f"{SET}/src/{stem}{ext}"
    if not os.path.exists(src):
        subprocess.run(["/bin/cp", p["path"], src], check=True)
    for n, st in REAL:
        name = f"{stem}__{n}"
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE); s.update(st)
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin")
        meta[name] = {"photo": stem, "camera": p["model"], "settings": st}
for cm in sorted(cams):
    inf = dng.dcp(cm, [0.5, 1.0, 0.65])
    img = ev_img(np.broadcast_to(np.linspace(-14, 1, W)[None, :], (128, W)))
    name = "ramp_" + cm.replace(" ", "_").replace("(", "").replace(")", "")
    s = dict(dng.BASE)
    dng.write_dng(f"{SET}/dng/{name}.dng", dng.scene_to_camera(img, inf), cm, inf["spec"]["camera_white"], inf, dng.xmp_packet(s, curves=dng.LINEAR))
    man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
    meta[name] = {"scene": "ramp", "settings": {}, "camera": cm}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man))
