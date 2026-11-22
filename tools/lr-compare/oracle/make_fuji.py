"""make_fuji.py SET ACC: Bryan's Fujifilm raws (X-T2, X100F; acceptance split A only) through Camera Raw
at Camera Raw's minimum size, with sidecars: `base` (Adobe Standard, every slider at 0), `E+1`, and
`full` (the photo's own tone / colour / curve settings, no look, geometry, lens or detail) — R23:
is the brightness gap (X-T2 +2.9 L*, X100F PV5 −2.6) in the base render or in the edit?
Originals are APFS-cloned into SET/v (read-only use of the library). LightCraft side:
`ORACLE_SIZE=1536 oracle_render SET/lc SET/v/<name>.RAF` (sidecar wins), then lccmp.py."""
import sys, os, json, re, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET, ACC = sys.argv[1], sys.argv[2]
for d in ("acr", "v", "src"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
DROP = re.compile(r"^(Crop|Upright|Perspective|Lens|AutoLateralCA|Defringe|Sharp|Luminance(Smoothing|NoiseReduction)|ColorNoise|Grain|PostCrop|Vignette|HDR|SDR|CameraProfileDigest|Has|AlreadyApplied|Look)")
sample = [p for p in json.load(open(f"{ACC}/sample.json")) if p["model"] in ("X-T2", "X100F") and p["split"] == "A" and os.path.exists(p["path"])]
man, meta = [], {}
for p in sample:
    stem, ext = os.path.splitext(os.path.basename(p["path"]))
    src = f"{SET}/src/{stem}{ext}"
    if not os.path.exists(src):
        subprocess.run(["/bin/cp", p["path"], src], check=True)
    own = {k: v for k, v in (p.get("settings") or {}).items() if not DROP.match(k)}
    curves = {k: list(zip(v[::2], v[1::2])) for k, v in (p.get("curves") or {}).items()}
    for n, st, cv in (("base", {}, dng.LINEAR), ("E+1", {"Exposure2012": "+1.00"}, dng.LINEAR), ("full", own, curves or dng.LINEAR)):
        s = dict(dng.BASE)
        s.update(st)
        s["CameraProfile"] = "Adobe Standard"
        s["ProcessVersion"] = p.get("pv", "15.4") if n == "full" else "15.4"
        name = f"{stem}__{n}"
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=cv))
        man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin")
        meta[name] = {"photo": p["id"], "model": p["model"], "pv": p.get("pv"), "settings": st}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(sample), len(man))
