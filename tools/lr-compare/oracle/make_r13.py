"""make_r13.py SET [WHAT]: Round 13 experiments for the Camera Raw oracle (docs/lr-match.md → Round 13).

WHAT `deh` (default): negative Dehaze's spatial part. Full-frame sine gratings (periods 3…144 px, amplitude and
level variants), 2-D checkerboards, a flat field, a step edge and thin bright / dark lines, each at Dehaze 0 / −25 /
−50 / −100 (R6 colour, 1,024 × 683): how the mean level and the contrast move with the detail's scale, and over
how wide a window the move is decided (the edge's and the lines' profiles).

Writes SET/dng/*.dng, SET/meta.json and the manifest for ps.sh."""
import sys, os, json
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET = sys.argv[1]
WHAT = sys.argv[2].split(",") if len(sys.argv) > 2 else ["deh"]
for d in ("dng", "acr"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
CAM = "Canon EOS R6"
info = dng.dcp(CAM, [0.5, 1.0, 0.65])
W, H = 1024, 683


def grey(ev):
    return np.repeat((2.0 ** ev)[..., None], 3, -1)


def grating(p, a=0.4, b=-3.5):
    x = np.arange(W)
    return grey(np.repeat((b + a * np.sin(2 * np.pi * x / p))[None, :], H, 0))


def checker(p, a=0.4, b=-3.5):
    y, x = np.mgrid[0:H, 0:W]
    return grey(b + a * np.where(((x // (p // 2)) + (y // (p // 2))) % 2 == 0, 1.0, -1.0))


def edge(lo=-5.0, hi=-2.0):
    x = np.arange(W)
    return grey(np.repeat(np.where(x < W // 2, lo, hi)[None, :], H, 0).astype(float))


def lines(bg, d, widths=(1, 3, 9, 27)):
    ev = np.full((H, W), bg)
    for i, w in enumerate(widths):
        x0 = 128 + 256 * i
        ev[:, x0:x0 + w] = bg + d
    return grey(ev)


jobs = []
if "deh" in WHAT:
    sw = [("base", {})] + [(f"Deh{v:+d}", {"Dehaze": f"{v:+d}"}) for v in (-25, -50, -100)]
    for p in (3, 5, 8, 13, 21, 34, 55, 89, 144):
        jobs.append((f"g{p}", grating(p), sw))
    for p, a, b in ((8, 0.1, -3.5), (8, 1.2, -3.5), (34, 0.1, -3.5), (34, 1.2, -3.5), (13, 0.4, -6.5), (13, 0.4, -1.2), (89, 0.4, -6.5),
                    (89, 0.4, -1.2)):
        jobs.append((f"g{p}a{a}b{b}", grating(p, a, b), sw))
    jobs += [("c8", checker(8), sw), ("c34", checker(34), sw), ("flat", grey(np.full((H, W), -3.5)), sw),
             ("flatb", grey(np.full((H, W), -1.2)), sw), ("edge", edge(), sw), ("lineb", lines(-5.0, 3.0), sw), ("lined", lines(-2.0, -3.0), sw)]

if "cla" in WHAT:
    # Clarity's regional move: flat two-level layouts (no texture), so any change is regional; the profile away from
    # the boundary shows the kernel, the level pairs how the move depends on the levels on both sides
    sw = [("base", {})] + [(f"Cla{v:+d}", {"Clarity2012": f"{v:+d}"}) for v in (-50, 50, 100)]
    y = np.arange(H)[:, None]
    x = np.arange(W)[None, :]
    for hi, lo in ((-1.0, -4.0), (-1.0, -2.0), (-1.0, -7.0), (-3.0, -4.0), (-3.0, -6.0)):
        jobs.append((f"top{hi:+.0f}{lo:+.0f}", grey(np.where(y < 0.4 * H, hi, lo) + 0 * x), sw))
    jobs.append(("left-1-4", grey(np.where(x < W // 2, -1.0, -4.0) + 0 * y), sw))
    for side in (60, 200):
        ev = np.full((H, W), -5.0)
        ev[H // 2 - side // 2:H // 2 + side // 2, W // 2 - side // 2:W // 2 + side // 2] = -0.5
        jobs.append((f"win{side}", grey(ev), sw))
    ev = np.full((H, W), -1.0)
    ev[H // 2 - 100:H // 2 + 100, W // 2 - 100:W // 2 + 100] = -5.0
    jobs.append(("hole200", grey(ev), sw))

man, meta = [], {}
for iname, img, sweep in jobs:
    cam = dng.scene_to_camera(img, info)
    for sname, st in sweep:
        s = dict(dng.BASE)
        s.update(st)
        name = f"{iname}__{sname}"
        xmp = dng.xmp_packet(s, curves=dng.LINEAR)
        dng.write_dng(f"{SET}/dng/{name}.dng", cam, CAM, info["spec"]["camera_white"], info, xmp)
        if not os.path.exists(f"{SET}/acr/{name}.tif"):
            man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"image": iname, "settings": st}
json.dump({"camera": CAM, "items": meta}, open(f"{SET}/meta.json", "w"))
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + ("\n" if man else ""))
print(len(man), "files")
