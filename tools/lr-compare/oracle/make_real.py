"""make_real.py SET: Bryan's real raws (training photos only, never split B) with single-slider sidecars."""
import sys, os, json, shutil, random, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng
L = "/private/tmp/claude-501/-Users-bryanhempstead-second-brain/b00a0f05-b80e-4fee-9535-4e0dcf9ffa2b/scratchpad/lc"
SET = sys.argv[1]; os.makedirs(f"{SET}/src", exist_ok=True); os.makedirs(f"{SET}/v", exist_ok=True); os.makedirs(f"{SET}/acr", exist_ok=True)
split = json.load(open(f"{L}/r2/split.json"))
want = {"Canon EOS R6": 4, "LEICA M (Typ 262)": 4, "RICOH GR III": 2, "X-T2": 1, "X100F": 1}
pick = []
random.seed(7)
pool = [p for w in ["t1", "w1", "w5", "w4"] for p in json.load(open(f"{L}/{w}/sample.json")) if split.get(str(p["id"]), "T") != "B" and os.path.exists(p["path"]) and "-Pano" not in p["path"]]
random.shuffle(pool)
for p in pool:
    if want.get(p["model"], 0) > 0 and os.path.basename(p["path"]) not in [os.path.basename(x["path"]) for x in pick]:
        pick.append(p); want[p["model"]] -= 1
N = {"E": "Exposure2012", "C": "Contrast2012", "H": "Highlights2012", "S": "Shadows2012", "W": "Whites2012", "K": "Blacks2012", "CL": "Clarity2012", "TX": "Texture", "DH": "Dehaze"}
SETTINGS = [("base", {})]
for k in ["H", "S"]:
    for v in [-100, -50, 50, 100]: SETTINGS.append((f"{k}{v:+d}", {N[k]: f"{v:+d}"}))
for v in [-50, 50]: SETTINGS.append((f"C{v:+d}", {N["C"]: f"{v:+d}"}))
for k in ["CL", "TX", "DH"]:
    for v in [-50, 50]: SETTINGS.append((f"{k}{v:+d}", {N[k]: f"{v:+d}"}))
SETTINGS += [("SFtone", {"Highlights2012": "-90", "Shadows2012": "+43", "Whites2012": "-36", "Blacks2012": "+30"}),
             ("combo1", {"Exposure2012": "-0.33", "Contrast2012": "-58", "Highlights2012": "-100", "Shadows2012": "+68", "Whites2012": "-33", "Blacks2012": "+31"}),
             ("combo2", {"Exposure2012": "+0.40", "Contrast2012": "-63", "Highlights2012": "-52", "Shadows2012": "+83", "Whites2012": "-36", "Blacks2012": "+30"}),
             ("combo5", {"Exposure2012": "+1.80", "Contrast2012": "+32", "Highlights2012": "-100", "Shadows2012": "+40", "Blacks2012": "-19"})]
man, meta = [], {}
for p in pick:
    base = os.path.basename(p["path"]); stem, ext = os.path.splitext(base)
    src = f"{SET}/src/{base}"
    if not os.path.exists(src): shutil.copyfile(p["path"], src)
    for sname, st in SETTINGS:
        name = f"{stem}__{sname}"
        dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst): subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE); s.update(st)
        open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin"); meta[name] = {"photo": p["id"], "model": p["model"], "settings": st, "file": base}
open("manifest.txt", "w").write("\n".join(man) + "\n"); json.dump({"items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print(len(pick), [os.path.basename(p["path"]) for p in pick], len(man))
