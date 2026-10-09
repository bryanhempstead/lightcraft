"""make_rwb.py SET: real raws (training split) with custom white-balance sidecars."""
import sys, os, json, shutil, random, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng
L = "/private/tmp/claude-501/-Users-bryanhempstead-second-brain/b00a0f05-b80e-4fee-9535-4e0dcf9ffa2b/scratchpad/lc"
SET = sys.argv[1]; [os.makedirs(f"{SET}/{d}", exist_ok=True) for d in ["src", "v", "acr"]]
split = json.load(open(f"{L}/r2/split.json")); want = {"Canon EOS R6": 1, "LEICA M (Typ 262)": 1, "X-T2": 1, "X100F": 1}; pick = []
random.seed(11)
pool = [p for w in ["t1", "w1", "w5", "w4"] for p in json.load(open(f"{L}/{w}/sample.json")) if split.get(str(p["id"]), "T") != "B" and os.path.exists(p["path"]) and "-Pano" not in p["path"]]
random.shuffle(pool)
for p in pool:
    if want.get(p["model"], 0) > 0: pick.append(p); want[p["model"]] -= 1
SETTINGS = [("base", {})] + [(f"T{t}_{ti}", dict(WhiteBalance="Custom", Temperature=str(t), Tint=f"{ti:+d}")) for t, ti in [(3000, 0), (5000, 0), (8000, 0), (34600, 17)]]
man, meta = [], {}
for p in pick:
    base = os.path.basename(p["path"]); stem, ext = os.path.splitext(base); src = f"{SET}/src/{base}"
    if not os.path.exists(src): shutil.copyfile(p["path"], src)
    for sname, st in SETTINGS:
        name = f"{stem}__{sname}"; dst = f"{SET}/v/{name}{ext}"
        if not os.path.exists(dst): subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE); s.update(st); open(f"{SET}/v/{name}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f"{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin"); meta[name] = {"photo": p["id"], "model": p["model"], "settings": st, "file": base}
open("manifest.txt", "w").write("\n".join(man) + "\n"); json.dump({"items": meta}, open(f"{SET}/meta.json", "w"), indent=0)
print([os.path.basename(p["path"]) for p in pick], len(man))
