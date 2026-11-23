"""make_acc.py: the acceptance set with Camera Raw as its ground truth (round 13).

Round 12's acceptance set held Lightroom's own previews, kept in a scratch folder that was lost, and the catalog's
`Previews.lrdata` is no longer on this Mac. Camera Raw is Lightroom's renderer, so a photo opened in Camera Raw with
its catalog settings (`luaxmp.to_xmp`: every slider, curve, look, mask, crop, lens and detail setting) is Lightroom's
render of the edit. This builds an `lr_compare.py` work folder whose "previews" are those renders:

  make_acc.py pick SET [--spec acceptance.json] [--stems A,B,…] [--masks N] [--seed 13] [--catalog C]
      picks photos (the spec's look × camera × process-version cells, plus named photos), copies each original into
      SET/v/<id>.<ext> with its settings as SET/v/<id>.xmp, writes SET/sample.json, SET/records.json (the
      `migrate-lightroom` records of those photos, pointing at the copies) and the manifest for ps.sh
  make_acc.py finish SET
      every Camera Raw render SET/acr/<id>.tif (16-bit ProPhoto) → SET/previews/<id>.png (8-bit Adobe RGB, Bradford
      D50 → D65, the published primaries and curve), the TIFF deleted

Then `lr_compare.py migrate / render / compare / gate` as with Lightroom's previews (`--no-publish`!).
Readable originals only; Lightroom's data is only read (the catalog is copied first)."""
import argparse, json, os, random, re, shutil, sqlite3, subprocess, sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.dirname(HERE))
import luaxmp, tiff  # noqa: E402
import lr_compare as lrc  # noqa: E402

PP_TO_XYZ_D50 = np.array([[0.7976749, 0.1351917, 0.0313534], [0.2880402, 0.7118741, 0.0000857], [0.0, 0.0, 0.8252100]])
BRADFORD = np.array([[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]])
D50, D65 = np.array([0.9642, 1.0, 0.8249]), np.array([0.95047, 1.0, 1.08883])
ADAPT = np.linalg.inv(BRADFORD) @ np.diag((BRADFORD @ D65) / (BRADFORD @ D50)) @ BRADFORD
PP_TO_ADOBE = np.linalg.inv(lrc.ADOBE_TO_XYZ) @ ADAPT @ PP_TO_XYZ_D50
# photos earlier rounds' oracle sets name (make_r11 / make_r12 / make_rhs): kept in the set when readable
NAMED = ["BHM_8476", "BHM_8490", "BHM_4458", "BHM_4913", "BHM_5513", "BHM_7515", "BHM_8388", "BHM_4707", "BHM_4582", "BHM_8231",
         "BHM_9574", "BHM_5214", "BHM_4612", "BHM_8547", "BHM_7845", "BHM_4832", "BHM_8061", "BHM_9795", "BHM_8480", "BHM_0333",
         "L1009234", "L1006270", "L1005500", "L1004944", "L1009603", "R0002574", "R0003062"]


def pick(a):
    os.makedirs(f"{a.set}/v", exist_ok=True)
    os.makedirs(f"{a.set}/acr", exist_ok=True)
    cat = os.path.join(a.set, "c.lrcat")
    if not os.path.exists(cat):
        shutil.copyfile(a.catalog or lrc.newest_catalog(), cat)
    db = sqlite3.connect(cat)
    rows = db.execute(lrc.SQL).fetchall()
    lenses = dict(db.execute("SELECT e.image, l.value FROM AgHarvestedExifMetadata e LEFT JOIN AgInternedExifLens l ON l.id_local = e.lensRef"))
    random.seed(a.seed)
    picked, seen = [], set()

    def take(r):
        if r[0] in seen or not r[4] or not os.path.isfile(r[1]):
            return False
        seen.add(r[0])
        picked.append(r)
        return True
    stems = (a.stems.split(",") if a.stems else []) + NAMED
    by_stem = {}
    for r in rows:
        by_stem.setdefault(os.path.splitext(os.path.basename(r[1]))[0], []).append(r)
    for s in stems:
        # the edited copy whose original is readable (the stems name raws of the R6 / M262 / GR III)
        for r in sorted(by_stem.get(s, []), key=lambda r: -r[5]):
            if r[3] in ("cr3", "dng") and take(r):
                break
    for g in json.load(open(a.spec)):
        exts = [e.lower() for e in (g["ext"] if isinstance(g["ext"], list) else [g["ext"]])]
        grp = [r for r in rows if r[2] == g["model"] and r[3] in exts and lrc.look_of(r[4]) == g.get("look", "")
               and (not g.get("pv") or lrc.process_version(r[4]) == g["pv"])]
        random.shuffle(grp)
        grp.sort(key=lambda r: r[5] == 0)  # edited first, as the spec's cells are Bryan's edits
        have = sum(1 for r in picked if r[2] == g["model"] and r[3] in exts and lrc.look_of(r[4]) == g.get("look", "")
                   and (not g.get("pv") or lrc.process_version(r[4]) == g["pv"]))
        n = max(0, g.get("n", 2) - have)
        for r in grp:
            if n <= 0:
                break
            n -= take(r)
    # Bryan's most heavily edited photos: masks (no AI masks, which Camera Raw would recompute) on top of a look
    masked = [r for r in rows if r[4] and "MaskGroupBasedCorrections" in r[4] and 'What = "Mask/Image"' not in r[4]
              and r[3] in ("cr3", "dng", "raf") and r[5] > 0]
    random.shuffle(masked)
    n = a.masks
    for r in masked:
        if n <= 0:
            break
        n -= take(r)
    sample, man = [], []
    for r in picked:
        iid, path, model, ext, text, edits = r[0], r[1], r[2], r[3], r[4], r[5]
        dst = os.path.abspath(f"{a.set}/v/{iid}.{ext}")
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", path, dst], check=True)
        try:
            xmp = luaxmp.to_xmp(text)
        except (ValueError, IndexError, AttributeError) as e:
            print("skip", iid, path, e, file=sys.stderr)
            continue
        open(f"{a.set}/v/{iid}.xmp", "w").write(xmp)
        if ext in ("jpg", "jpeg"):
            luaxmp.embed_jpeg(dst, text)  # Camera Raw reads a JPEG's own XMP, never a sidecar
        if not os.path.exists(f"{a.set}/previews/{iid}.png"):
            man.append(f"{dst}\t{os.path.abspath(a.set)}/acr/{iid}.tif\tmin")
        sample.append({"id": iid, "path": dst, "original": path, "model": model, "look": lrc.look_of(text), "edits": edits,
                       "preview": os.path.abspath(f"{a.set}/previews/{iid}.png"), "previewQuality": "camera-raw", "previewSpace": "adobe",
                       "lrRender": True, "lens": lenses.get(iid), "ext": ext, "pv": lrc.process_version(text), "split": lrc.split_of(iid),
                       "settings": lrc.top_level(lrc.strip_look(text)), "curves": lrc.curves_of(lrc.strip_look(text)),
                       "masks": text.count('What = "Correction"'), "set": "acc13"})
    json.dump(sample, open(f"{a.set}/sample.json", "w"), indent=1)
    open(os.path.join(HERE, "manifest.txt"), "w").write("\n".join(man) + ("\n" if man else ""))
    # migrate-lightroom records of exactly these photos, pointing at the copies
    allrec = os.path.join(a.set, "records-all.json")
    subprocess.run([lrc.CLI, "migrate-lightroom", "--dry-run", "--no-presets", "--catalog", cat, "--records-out", allrec], check=True,
                   stdout=subprocess.DEVNULL)
    rec = json.load(open(allrec))
    where = {p["id"]: p["path"] for p in sample}
    images = []
    for i in rec["images"]:
        if i["id"] in where:
            i = dict(i, path=where[i["id"]], root=os.path.dirname(where[i["id"]]) + "/")
            images.append(i)
    rec["images"], rec["collections"] = images, []
    json.dump(rec, open(f"{a.set}/records.json", "w"))
    os.remove(allrec)
    print(f"{len(sample)} photos ({len(man)} to render) -> {a.set}/sample.json")


def finish(a):
    os.makedirs(f"{a.set}/previews", exist_ok=True)
    sample = json.load(open(f"{a.set}/sample.json"))
    done = 0
    for p in sample:
        t = f"{a.set}/acr/{p['id']}.tif"
        if not os.path.exists(t):
            continue
        img, desc = tiff.read(t)
        if "ProPhoto" not in desc:
            print("not ProPhoto:", t, desc, file=sys.stderr)
            continue
        lin = np.clip(img, 0, 1) ** 1.8 @ PP_TO_ADOBE.T
        enc = np.clip(lin, 0, 1) ** (256 / 563)
        Image.fromarray((enc * 255 + 0.5).astype(np.uint8)).save(p["preview"])
        os.remove(t)
        done += 1
    have = [p for p in sample if os.path.exists(p["preview"])]
    print(f"{done} converted; {len(have)} / {len(sample)} photos have a Camera Raw render")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("pick")
    p.add_argument("set")
    p.add_argument("--spec", default=os.path.join(os.path.dirname(HERE), "acceptance.json"))
    p.add_argument("--stems")
    p.add_argument("--seed", type=int, default=13)
    p.add_argument("--masks", type=int, default=0, help="also this many edited raws with masks")
    p.add_argument("--catalog")
    p.set_defaults(fn=pick)
    p = sub.add_parser("finish")
    p.add_argument("set")
    p.set_defaults(fn=finish)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
