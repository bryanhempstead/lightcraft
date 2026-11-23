"""make_hdr_real.py SET SRC [stems…]: HDR oracle set. APFS clones of SRC's raws with sidecars per variant,
a manifest rendered by Camera Raw (32-bit = the HDR master, 16-bit = what Photoshop gets for an
HDR edit opened at 16 bit, i.e. the SDR rendition) and meta.json for the comparison."""
import sys, os, json, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET, SRC = sys.argv[1], sys.argv[2]
only = set(sys.argv[3:])
VARIANTS = os.environ.get("VARIANTS", "")
HDR = dict(HDREditMode="1", HDRMaxValue="+4.0")
ALL = {
    "sdr": ({}, "16"),
    "hdr4": (HDR, "32"),
    "hdr4s": (HDR, "16"),
    "hdr2": (dict(HDR, HDRMaxValue="+2.0"), "32"),
    "hdr4_hl-60": (dict(HDR, Highlights2012="-60"), "32"),
    "hdr4_w+50": (dict(HDR, Whites2012="+50"), "32"),
    "hdr4_e+1": (dict(HDR, Exposure2012="+1.00"), "32"),
    "hdr4_e-1": (dict(HDR, Exposure2012="-1.00"), "32"),
    "hdr4_c+50": (dict(HDR, Contrast2012="+50"), "32"),
    "hdr4_sb+50": (dict(HDR, SDRBrightness="+50"), "16"),
    "hdr4_sbl+100": (dict(HDR, SDRBlend="+100"), "16"),
    "hdr4_sbl-100": (dict(HDR, SDRBlend="-100"), "16"),
}
names = VARIANTS.split(",") if VARIANTS else list(ALL)
for d in ("v", "acr"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)
man, meta = [], {}
for f in sorted(os.listdir(SRC)):
    stem, ext = os.path.splitext(f)
    if only and stem not in only:
        continue
    for n in names:
        st, bits = ALL[n]
        name = f"{stem}__{n}"
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", f"{SRC}/{f}", dst], check=True)
        s = dict(dng.BASE)
        s.update(st)
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin\t{bits}")
        meta[name] = {"photo": stem, "variant": n, "settings": st, "bits": bits}
open(f"{SET}/manifest.txt", "w").write("\n".join(man) + "\n")
json.dump({"items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(man))
