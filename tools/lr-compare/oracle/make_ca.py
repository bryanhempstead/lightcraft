"""make_ca.py SET SAMPLE.json [N=2]: lateral chromatic aberration experiment on real raws.

Picks N fit-split (A) photos of the sample whose Lightroom lens profile carries a chromatic model
(e.g. Canon EF 24mm f/1.4L II, EF 50mm f/1.2L), APFS-clones each with four sidecars — profile
off / on × Remove Chromatic Aberration off / on, nothing else — and writes the manifest for ps.sh
(full size). `ca_ana.py` measures how far red and blue sit from green across the frame."""
import sys, os, json, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET, SAMPLE = sys.argv[1], sys.argv[2]
N = int(sys.argv[3]) if len(sys.argv) > 3 else 2
LENSES = os.environ.get("LENSES", "EF 24mm f1.4L II,EF 50mm f1.2L").split(",")
for d in ("src", "v", "acr"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
pick = []
for p in json.load(open(SAMPLE)):
    fn = (p.get("settings") or {}).get("LensProfileFilename", "")
    if p.get("split") == "A" and any(l in fn for l in LENSES) and os.path.exists(p["path"]) and len(pick) < N:
        if all(fn != q[1] for q in pick):
            pick.append((p, fn))
VARIANTS = {
    "p0c0": {"LensProfileEnable": "0", "AutoLateralCA": "0"},
    "p1c0": {"LensProfileEnable": "1", "LensProfileSetup": "LensDefaults", "AutoLateralCA": "0"},
    "p1c1": {"LensProfileEnable": "1", "LensProfileSetup": "LensDefaults", "AutoLateralCA": "1"},
    "p0c1": {"LensProfileEnable": "0", "AutoLateralCA": "1"},
}
man, meta = [], {}
for p, fn in pick:
    base = os.path.basename(p["path"])
    stem, ext = os.path.splitext(base)
    src = f"{SET}/src/{base}"
    if not os.path.exists(src):
        subprocess.run(["/bin/cp", p["path"], src], check=True)
    for vname, st in VARIANTS.items():
        name = f"{stem}__{vname}"
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE)
        s.update(st)
        if st["LensProfileEnable"] == "1":
            s["LensProfileFilename"] = fn
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"photo": p["id"], "model": p["model"], "lens": p.get("lens"), "profile": fn, "settings": st, "file": base}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print([os.path.basename(p["path"]) + " " + fn for p, fn in pick], len(man))
