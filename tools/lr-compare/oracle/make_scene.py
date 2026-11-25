"""make_scene.py SET [PHOTOS.json]: same-value slider study for Shadows +, Clarity and negative Dehaze (2026-11-25 study,
`docs/lr-match.md` → Round 14). Clones (cp -c from a SET/src copy) of the four study raws, one-slider sidecars on
dng.BASE (Adobe Standard, linear curve), plus a Highlights control. Writes SET/v/*, SET/meta.json and manifest.txt
for ps.sh / psq.sh (Camera Raw's minimum size, 1536 px). `PHOTOS.json`: {stem: path} to use other raws."""
import sys, os, json, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET = sys.argv[1]
for d in ("v", "src", "acr"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
PHOTOS = {
    "BHM_9815": "/Users/bryanhempstead/Pictures/Lightroom/SYNC/BHM_9815.CR3",
    "L1006344": "/Volumes/BONUSBOY/Ciara+Logan.Wedding.Bismarck.October.3rd.2026/raw/M262/L1006344.DNG",
    "DSC01746": "/Volumes/BONUSBOY/Ciara+Logan.Wedding.Bismarck.October.3rd.2026/raw/ILCE-7SM3/DSC01746.ARW",
    "R0003064": "/Volumes/BONUSBOY/*Katherine & Blake Engagement - August 2026/griii/R0003064.DNG",
}
SHORT = len(sys.argv) > 3 and sys.argv[3] == "short"
if len(sys.argv) > 2 and sys.argv[2] != "-":
    PHOTOS = json.load(open(sys.argv[2]))
SWEEP = {
    "Shadows2012": [-50, 25, 50, 100],
    "Dehaze": [-100, -50, -25, 50],
    "Highlights2012": [-50],
}
if SHORT:  # held-out photos: the sliders' ends only
    SWEEP = {"Shadows2012": [50, 100], "Dehaze": [-100, -50]}
man, meta = [], {}
for stem, path in PHOTOS.items():
    ext = os.path.splitext(path)[1]
    src = f"{SET}/src/{stem}{ext}"
    if not os.path.exists(src):
        subprocess.run(["/bin/cp", path, src], check=True)
    jobs = [("base", {})] + [(f"{k}_{v:+d}", {k: f"{v:+d}"}) for k, vs in SWEEP.items() for v in vs]
    for n, ss in jobs:
        name = f"{stem}__{n}"
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE)
        s.update(ss)
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        if not os.path.exists(f"{SET}/acr/{name}.tif") and not os.path.exists(f"{SET}/acr/{name}.npy"):
            man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin")
        meta[name] = {"photo": stem, "settings": ss}
old = json.load(open(f"{SET}/meta.json")) if os.path.exists(f"{SET}/meta.json") else {"items": {}}
old["items"].update(meta)
json.dump({"camera": "", "items": old["items"]}, open(f"{SET}/meta.json", "w"))
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest-short.txt" if SHORT else "manifest.txt"), "w").write("\n".join(man) + ("\n" if man else ""))
print(len(man), "files to render,", len(meta), "items")
