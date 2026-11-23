"""make_r12.py SET ACC [WHAT]: Round 12 experiments for the Camera Raw oracle (docs/lr-match.md → Round 12).

Highlights / Shadows (R02) on Bryan's own slider combinations: his split-A raws that carry Highlights or Shadows
(ACC = the acceptance work folder, `sample.json`; readable volumes only), as APFS clones with sidecars holding

- `own`       the photo's own Basic tone (Exposure, Contrast, Highlights, Shadows, Whites, Blacks), Adobe
              Standard, linear curve, nothing else;
- `own-noHS`  the same without Highlights / Shadows (so `own − own-noHS` is the H/S effect);
- `HS0`       only the photo's Highlights and Shadows (Exposure 0);
- `S0` / `H0` only its Shadows / only its Highlights;

and, on a few photos (`PAIRS`), a sweep of Shadows × Highlights pairs at Exposure 0.

WHAT: `hs` (default) or `hs,pairs`. Writes SET/v/* clones + sidecars, SET/meta.json and the manifest for ps.sh."""
import sys, os, json, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng

SET, ACC = sys.argv[1], sys.argv[2]
WHAT = sys.argv[3].split(",") if len(sys.argv) > 3 else ["hs", "pairs"]
for d in ("v", "src", "acr"):
    os.makedirs(f"{SET}/{d}", exist_ok=True)

TONE = ("Exposure2012", "Contrast2012", "Highlights2012", "Shadows2012", "Whites2012", "Blacks2012")
# R6 split-A photos with H/S (the acceptance cells that failed in round 11), and M262 / GR III controls
PHOTOS = ["BHM_8476", "BHM_8490", "BHM_4458", "BHM_4913", "BHM_5513", "BHM_7515", "BHM_8388", "BHM_4707", "BHM_4582",
          "BHM_8231", "BHM_9574", "BHM_5214", "BHM_4612", "BHM_8547", "BHM_7845", "BHM_4832", "BHM_8061", "BHM_9795",
          "L1009234", "L1006270", "L1005500", "R0002574"]
PAIRS = ["BHM_8490", "BHM_8476", "L1009234"]
PAIR_SET = [(34, -90), (65, -100), (100, -100), (50, -50), (100, -30), (30, -100), (69, 30), (34, 0), (0, -90), (100, 0), (0, -100), (0, 30)]


def sgn(v):
    v = float(v)
    return f"{v:+.2f}" if abs(v - round(v)) > 1e-9 else f"{int(round(v)):+d}"


sample = {os.path.splitext(os.path.basename(p.get("original") or p["path"]))[0]: p for p in json.load(open(f"{ACC}/sample.json"))}
man, meta = [], {}
for stem in PHOTOS:
    p = sample.get(stem)
    if p is None or not os.path.exists(p["path"]):
        print("skip", stem)
        continue
    assert p["split"] == "A", stem
    st = p["settings"]
    own = {k: sgn(st.get(k, "0")) for k in TONE}
    jobs = []
    if "hs" in WHAT:
        jobs += [("own", own), ("own-noHS", {k: v for k, v in own.items() if k not in ("Highlights2012", "Shadows2012")}),
                 ("HS0", {"Highlights2012": own["Highlights2012"], "Shadows2012": own["Shadows2012"]}),
                 ("S0", {"Shadows2012": own["Shadows2012"]}), ("H0", {"Highlights2012": own["Highlights2012"]}), ("base", {})]
    if "pairs" in WHAT and stem in PAIRS:
        jobs += [(f"p_S{s:+d}H{h:+d}", {"Shadows2012": f"{s:+d}", "Highlights2012": f"{h:+d}"}) for s, h in PAIR_SET]
    ext = os.path.splitext(p["path"])[1]
    src = f"{SET}/src/{stem}{ext}"
    if not os.path.exists(src):
        subprocess.run(["/bin/cp", p["path"], src], check=True)
    for n, ss in jobs:
        name = f"{stem}__{n}"
        if name in meta:
            continue
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE)
        s.update(ss)
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        if not os.path.exists(f"{SET}/acr/{name}.tif"):
            man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin")
        meta[name] = {"photo": stem, "camera": p["model"], "settings": ss}

old = json.load(open(f"{SET}/meta.json")) if os.path.exists(f"{SET}/meta.json") else {"items": {}}
old["items"].update(meta)
json.dump({"camera": "", "items": old["items"]}, open(f"{SET}/meta.json", "w"))
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + ("\n" if man else ""))
print(len(man), "files to render,", len(meta), "items")
