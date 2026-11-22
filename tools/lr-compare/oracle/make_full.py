"""make_full.py SET ACC STEM,…: acceptance photos (split A only) through Camera Raw with their own
Lightroom settings — tone, curves, colour, calibration, white balance and the Summer Fields look —
without geometry, lens, detail and grain ("full"), and the same without Highlights / Shadows
("full-noHS"): does LightCraft's whole recipe match Camera Raw's on Bryan's own edits (R02 / R10)?"""
import sys, os, json, re, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET, ACC, STEMS = sys.argv[1], sys.argv[2], sys.argv[3].split(',')
for d in ("acr", "v", "src"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
sf = open(os.path.expanduser("~/Library/Application Support/Adobe/CameraRaw/ImportedSettings/Summer Fields.xmp")).read()
dig = re.search(r'crs:RGBTable="([0-9A-F]+)"', sf).group(1)
tab = re.search(rf'crs:Table_{dig}="([^"]*)"', sf).group(1)
uuid = re.search(r'crs:UUID="([0-9A-F]+)"', sf).group(1)


def look(amount):
    return (f'<crs:Look><rdf:Description crs:Name="Summer Fields" crs:Amount="{amount}" crs:UUID="{uuid}" crs:SupportsAmount="true" '
            'crs:SupportsMonochrome="true" crs:SupportsOutputReferred="true">'
            f'<crs:Parameters><rdf:Description crs:Version="18.4" crs:ProcessVersion="15.4" crs:ConvertToGrayscale="False" crs:RGBTable="{dig}" '
            f'crs:Table_{dig}="{tab}" crs:RGBTableAmount="0.5"/></crs:Parameters></rdf:Description></crs:Look>')


DROP = re.compile(r"^(Crop|Upright|Perspective|Lens|AutoLateralCA|Defringe|Sharp|Luminance(Smoothing|NoiseReduction)|ColorNoise|Grain|PostCrop|Vignette|HDR|SDR|CameraProfileDigest|Has|AlreadyApplied)")
sample = {os.path.splitext(os.path.basename(p["path"]))[0]: p for p in json.load(open(f"{ACC}/sample.json"))}
man, meta = [], {}
for stem in STEMS:
    p = sample[stem]
    assert p["split"] == "A", stem
    ext = os.path.splitext(p["path"])[1]
    src = f"{SET}/src/{stem}{ext}"
    if not os.path.exists(src):
        subprocess.run(["/bin/cp", p["path"], src], check=True)
    own = {k: v for k, v in p["settings"].items() if not DROP.match(k)}
    curves = {k: list(zip(v[::2], v[1::2])) for k, v in (p.get("curves") or {}).items()}
    lk = look(1) if p["look"] == "Summer Fields" else None
    for n, extra in (("full", {}), ("full-noHS", {"Highlights2012": "0", "Shadows2012": "0"})):
        s = dict(dng.BASE); s.update(own); s.update(extra)
        name = f"{stem}__{n}"
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, look=lk, curves=curves or dng.LINEAR))
        man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin")
        meta[name] = {"photo": stem, "camera": p["model"], "settings": extra}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
json.dump({"items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man))
