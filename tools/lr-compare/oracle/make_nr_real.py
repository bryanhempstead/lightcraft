"""make_nr_real.py SET RAW…: noise reduction on real high-ISO raws (Camera Raw knows each camera's
noise; synthetic flats carry no noise profile, so its luminance NR barely acts on them).

APFS-clones each raw with sidecars — no NR, Color 25 (the raw default), Luminance 15 / 30 / 60
with Color 25, Color 50 — nothing else, and writes the manifest for ps.sh (full size).
`nr_real_ana.py SET [LCDIR]` compares the noise left in flat areas."""
import sys, os, json, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET, RAWS = sys.argv[1], sys.argv[2:]
for d in ("src", "v", "acr"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
VARIANTS = {
    "n0": {"LuminanceSmoothing": "0", "ColorNoiseReduction": "0"},
    "c25": {"LuminanceSmoothing": "0", "ColorNoiseReduction": "25"},
    "c50": {"LuminanceSmoothing": "0", "ColorNoiseReduction": "50"},
    "l15": {"LuminanceSmoothing": "15", "ColorNoiseReduction": "25"},
    "l30": {"LuminanceSmoothing": "30", "ColorNoiseReduction": "25"},
    "l60": {"LuminanceSmoothing": "60", "ColorNoiseReduction": "25"},
}
man, meta = [], {}
for path in RAWS:
    base = os.path.basename(path)
    stem, ext = os.path.splitext(base)
    src = f"{SET}/src/{base}"
    if not os.path.exists(src):
        subprocess.run(["/bin/cp", path, src], check=True)
    for vname, st in VARIANTS.items():
        name = f"{stem}__{vname}"
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE)
        s.update(st)
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"file": base, "settings": st}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man))
