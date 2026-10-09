#!/usr/bin/env python3
"""lr-compare: measure LightCraft renders against Lightroom Classic's own previews.

Lightroom keeps a rendered preview of every edited photo (``<catalog> Previews.lrdata``:
``previews.db`` maps image id -> uuid + digest, the levels are ``<uuid>-<digest>_<px>`` JPEG files,
Adobe RGB). Those are Lightroom's renders of the photo's develop settings, so they are ground
truth for "does LightCraft render this edit like Lightroom does".

Steps (all writes go to WORK; the catalog and previews are only read, from copies):

  lr_compare.py prepare  --work W [--catalog X.lrcat] [--ids 1,2,3 | --spec groups.json] [--seed 7]
      copy the catalog (+ -wal) and previews.db, pick the photos, copy their largest preview
      to W/previews/<id>.jpg and write W/sample.json + W/records.json (a migrate-lightroom
      records file holding only these photos)
  lr_compare.py migrate  --work W --tag T     migrate the sample into W/lib-T (scratch library)
  lr_compare.py render   --work W --tag T     export each photo at its preview's size (W/renders/T)
  lr_compare.py calibrate --work W [--out DIR] [--looks "Adobe Color"] [--per-camera 40] [--exclude sample.json]
      fit Lightroom-matched camera profiles (lightcraft-cli calibrate --lightroom) from the
      catalog's raws that are at Lightroom's default settings, against their previews
  lr_compare.py compare  --work W --tag T [--size 512] [--base T0] [--strip ID,…]
      per photo: mean / p95 CIEDE2000, mean dL*, da*, db* (LightCraft - Lightroom), dL* per
      tone band, measured after aligning our render to the preview (scale / rotation / shift
      search; photos whose geometry differs are flagged `geom` — a crop or lens-correction
      mismatch, reported apart from colour); writes W/results-T.json and prints a table (with
      the change vs --base);
      --strip writes W/strips/<id>.jpg: Lightroom | base | T side by side.

Needs numpy + Pillow, and `lightcraft-cli` (LIGHTCRAFT_CLI, default target/release/lightcraft-cli).
Adobe RGB (1998) and sRGB are converted from their published primaries / transfer curves; no
ICC profile is read.
"""
import argparse, glob, json, os, random, shutil, sqlite3, subprocess, sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
CLI = os.environ.get("LIGHTCRAFT_CLI", os.path.join(HERE, "..", "..", "target", "release", "lightcraft-cli"))
DEFAULT_CATALOG_DIR = os.path.expanduser("~/Pictures/Lightroom")

# ---------------------------------------------------------------- colour maths (published specs)
SRGB_TO_XYZ = np.array([[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]])
# Adobe RGB (1998): primaries R(0.64,0.33) G(0.21,0.71) B(0.15,0.06), D65, gamma 563/256
ADOBE_TO_XYZ = np.array([[0.5767309, 0.1855540, 0.1881852], [0.2973769, 0.6273491, 0.0752741], [0.0270343, 0.0706872, 0.9911085]])
D65 = np.array([0.95047, 1.0, 1.08883])


def srgb_decode(v):
    return np.where(v <= 0.04045, v / 12.92, ((v + 0.055) / 1.055) ** 2.4)


def adobe_decode(v):
    return np.power(np.clip(v, 0, 1), 563.0 / 256.0)


def to_lab(img8, space):
    v = img8.astype(np.float64) / 255.0
    lin, m = (adobe_decode(v), ADOBE_TO_XYZ) if space == "adobe" else (srgb_decode(v), SRGB_TO_XYZ)
    xyz = lin @ m.T / D65
    f = np.where(xyz > (6 / 29) ** 3, np.cbrt(xyz), xyz / (3 * (6 / 29) ** 2) + 4 / 29)
    return np.stack([116 * f[..., 1] - 16, 500 * (f[..., 0] - f[..., 1]), 200 * (f[..., 1] - f[..., 2])], -1)


def de2000(lab1, lab2):
    L1, a1, b1 = lab1[..., 0], lab1[..., 1], lab1[..., 2]
    L2, a2, b2 = lab2[..., 0], lab2[..., 1], lab2[..., 2]
    C1, C2 = np.hypot(a1, b1), np.hypot(a2, b2)
    Cb = (C1 + C2) / 2
    G = 0.5 * (1 - np.sqrt(Cb**7 / (Cb**7 + 25.0**7)))
    a1p, a2p = (1 + G) * a1, (1 + G) * a2
    C1p, C2p = np.hypot(a1p, b1), np.hypot(a2p, b2)
    h1p = np.degrees(np.arctan2(b1, a1p)) % 360
    h2p = np.degrees(np.arctan2(b2, a2p)) % 360
    dLp, dCp = L2 - L1, C2p - C1p
    dh = h2p - h1p
    dh = np.where(dh > 180, dh - 360, np.where(dh < -180, dh + 360, dh))
    dh = np.where(C1p * C2p == 0, 0, dh)
    dHp = 2 * np.sqrt(C1p * C2p) * np.sin(np.radians(dh) / 2)
    Lbp, Cbp = (L1 + L2) / 2, (C1p + C2p) / 2
    hs = h1p + h2p
    hbp = np.where(C1p * C2p == 0, hs, np.where(np.abs(h1p - h2p) <= 180, hs / 2, np.where(hs < 360, (hs + 360) / 2, (hs - 360) / 2)))
    T = 1 - 0.17 * np.cos(np.radians(hbp - 30)) + 0.24 * np.cos(np.radians(2 * hbp)) + 0.32 * np.cos(np.radians(3 * hbp + 6)) - 0.20 * np.cos(np.radians(4 * hbp - 63))
    dtheta = 30 * np.exp(-(((hbp - 275) / 25) ** 2))
    Rc = 2 * np.sqrt(Cbp**7 / (Cbp**7 + 25.0**7))
    Sl = 1 + 0.015 * (Lbp - 50) ** 2 / np.sqrt(20 + (Lbp - 50) ** 2)
    Sc, Sh = 1 + 0.045 * Cbp, 1 + 0.015 * Cbp * T
    Rt = -np.sin(np.radians(2 * dtheta)) * Rc
    return np.sqrt((dLp / Sl) ** 2 + (dCp / Sc) ** 2 + (dHp / Sh) ** 2 + Rt * (dCp / Sc) * (dHp / Sh))


# ---------------------------------------------------------------- prepare
SQL = """SELECT i.id_local, rf.absolutePath||fo.pathFromRoot||f.baseName||'.'||f.extension, cm.value, lower(f.extension), d.text,
 (SELECT count(*) FROM Adobe_libraryImageDevelopHistoryStep h WHERE h.image=i.id_local AND h.name NOT LIKE 'Import (%')
 FROM Adobe_images i JOIN AgLibraryFile f ON f.id_local=i.rootFile JOIN AgLibraryFolder fo ON fo.id_local=f.folder
 JOIN AgLibraryRootFolder rf ON rf.id_local=fo.rootFolder LEFT JOIN Adobe_imageDevelopSettings d ON d.image=i.id_local
 LEFT JOIN AgHarvestedExifMetadata em ON em.image=i.id_local LEFT JOIN AgInternedExifCameraModel cm ON cm.id_local=em.cameraModelRef
 WHERE i.masterImage IS NULL"""


def look_of(text):
    import re
    m = re.search(r'Look = \{.*?Name = "([^"]*)"', text or "", re.S)
    return m.group(1) if m else ""


QUARTER_TURNS = {"AB": 0, "BC": 1, "CD": 2, "DA": 3}


def store_preview(src, orientation, dst):
    """Lightroom keeps previews in the file's own orientation (previews.db `orientation`: AB, BC,
    CD, DA = 0..3 quarter turns clockwise to display them); store it upright, losslessly."""
    im = Image.open(src).convert("RGB")
    q = QUARTER_TURNS.get((orientation or "AB").strip(), 0)
    if q:
        im = im.transpose({1: Image.ROTATE_270, 2: Image.ROTATE_180, 3: Image.ROTATE_90}[q])
    im.save(dst)


def newest_catalog():
    cats = glob.glob(os.path.join(DEFAULT_CATALOG_DIR, "*.lrcat"))
    return max(cats, key=os.path.getmtime) if cats else None


def prepare(a):
    os.makedirs(a.work, exist_ok=True)
    cat = a.catalog or newest_catalog()
    if not cat:
        sys.exit("no catalog")
    prev_dir = a.previews or cat[: -len(".lrcat")] + " Previews.lrdata"
    cdir = os.path.join(a.work, "catalog")
    os.makedirs(cdir, exist_ok=True)
    shutil.copyfile(cat, os.path.join(cdir, "c.lrcat"))
    if os.path.isfile(cat + "-wal"):
        shutil.copyfile(cat + "-wal", os.path.join(cdir, "c.lrcat-wal"))
    shutil.copyfile(os.path.join(prev_dir, "previews.db"), os.path.join(cdir, "previews.db"))
    db = sqlite3.connect(os.path.join(cdir, "c.lrcat"))
    pv = sqlite3.connect(os.path.join(cdir, "previews.db"))
    rows = db.execute(SQL).fetchall()

    def preview(iid):
        e = pv.execute("SELECT uuid, digest, orientation FROM ImageCacheEntry WHERE imageId=?", (iid,)).fetchone()
        if not e:
            return None
        fs = [f for f in glob.glob(os.path.join(prev_dir, e[0][0], e[0][:4], f"{e[0]}-{e[1]}_*")) if f.rsplit("_", 1)[1].isdigit()]
        return (max(fs, key=lambda f: int(f.rsplit("_", 1)[1])), e[2]) if fs else None

    picked = []
    if a.ids:
        want = {int(x) for x in a.ids.split(",")}
        cand = [r for r in rows if r[0] in want]
        quota = None
    else:
        # spec: [{"model": "...", "ext": "dng", "look": "Summer Fields" | "" , "n": 3}, ...]
        spec = json.load(open(a.spec))
        random.seed(a.seed)
        cand, quota = [], []
        for g in spec:
            grp = [r for r in rows if r[2] == g["model"] and r[3] == g["ext"].lower() and look_of(r[4]) == g.get("look", "")]
            random.shuffle(grp)
            must = set(g.get("include", []))
            grp.sort(key=lambda r: (r[0] not in must, r[5] == 0))
            quota.append((grp, g.get("n", 2)))
    groups = quota or [(cand, len(cand))]
    for grp, n in groups:
        got = 0
        for r in grp:
            if got >= n:
                break
            if not os.path.isfile(r[1]):
                continue
            pr = preview(r[0])
            if not pr or int(pr[0].rsplit("_", 1)[1]) < a.min_px:
                continue
            os.makedirs(os.path.join(a.work, "previews"), exist_ok=True)
            dst = os.path.join(a.work, "previews", f"{r[0]}.png")
            store_preview(pr[0], pr[1], dst)
            picked.append({"id": r[0], "path": r[1], "model": r[2], "look": look_of(r[4]), "edits": r[5], "preview": dst})
            got += 1
    json.dump(picked, open(os.path.join(a.work, "sample.json"), "w"), indent=1)
    # records for exactly these photos
    allrec = os.path.join(cdir, "records-all.json")
    subprocess.run([CLI, "migrate-lightroom", "--dry-run", "--no-presets", "--catalog", os.path.join(cdir, "c.lrcat"), "--records-out", allrec],
                   check=True, stdout=subprocess.DEVNULL)
    rec = json.load(open(allrec))
    ids = {p["id"] for p in picked}
    rec["images"] = [i for i in rec["images"] if i["id"] in ids]
    rec["collections"] = []
    json.dump(rec, open(os.path.join(a.work, "records.json"), "w"))
    os.remove(allrec)
    for f in ("c.lrcat", "c.lrcat-wal"):
        if os.path.exists(os.path.join(cdir, f)):
            os.remove(os.path.join(cdir, f))
    print(f"{len(picked)} photos -> {a.work}/sample.json")


# ---------------------------------------------------------------- calibrate
# Sliders that must be at Lightroom's defaults for a photo to calibrate on.
ZERO_KEYS = ["Exposure2012", "Contrast2012", "Highlights2012", "Shadows2012", "Whites2012", "Blacks2012", "Texture", "Clarity2012", "Dehaze",
             "Vibrance", "Saturation", "ParametricShadows", "ParametricDarks", "ParametricLights", "ParametricHighlights", "GrainAmount",
             "PostCropVignetteAmount", "SplitToningShadowSaturation", "SplitToningHighlightSaturation", "ColorGradeMidtoneSat",
             "ColorGradeGlobalSat", "ColorGradeShadowSat", "ColorGradeHighlightSat", "ColorGradeShadowLum", "ColorGradeHighlightLum",
             "ColorGradeMidtoneLum", "ColorGradeGlobalLum", "CropTop", "CropLeft", "CropAngle", "PerspectiveVertical", "PerspectiveHorizontal",
             "PerspectiveRotate", "UprightCenterMode"] + [f"{k}Adjustment{b}" for k in ("Hue", "Saturation", "Luminance")
             for b in ("Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta")]
ONE_KEYS = ["CropRight", "CropBottom"]


def top_level(text):
    """Top-level `key = value` pairs of a develop-settings text (nested tables skipped)."""
    import re
    d, depth = {}, 0
    for line in (text or "").splitlines():
        body = line[len("s = { "):] if line.startswith("s = { ") else line
        if depth <= 1:
            m = re.match(r"^(\w+) = (.*?),?\s*\}?$", body)
            if m and "{" not in m.group(2):
                d[m.group(1)] = m.group(2).strip().strip('"')
        bare = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
        depth += bare.count("{") - bare.count("}")
    return d


def strip_look(text):
    """The settings text without its `Look = { … }` table (a look's own parameters)."""
    i = text.find("\nLook = {")
    if i < 0:
        return text
    depth, j = 0, i + 1
    while j < len(text):
        depth += {"{": 1, "}": -1}.get(text[j], 0)
        j += 1
        if depth == 0 and text[j - 1] == "}":
            break
    return text[:i] + text[j:]


def is_default(text):
    import re
    d = top_level(text)
    for k in ZERO_KEYS:
        if k in d and d[k] not in ("0", "0.0", "-0", "") and not d[k].startswith("0 "):
            return False
    for k in ONE_KEYS:
        if k in d and d[k] not in ("1", "1.0"):
            return False
    if d.get("ConvertToGrayscale") == "true" or d.get("WhiteBalance") not in (None, "As Shot"):
        return False
    curves = re.findall(r"^ToneCurvePV2012\w* = \{([^}]*)\}", strip_look(text or ""), re.M)
    if any(re.sub(r"\s", "", c) != "0,0,255,255" for c in curves):
        return False
    return not re.search(r"MaskGroupBasedCorrections = \{\s*\{", text or "")


def calibrate(a):
    """Lightroom-matched camera profiles from the catalog's photos at default settings."""
    import re
    os.makedirs(a.work, exist_ok=True)
    cat = a.catalog or newest_catalog()
    prev_dir = a.previews or cat[: -len(".lrcat")] + " Previews.lrdata"
    cdir = os.path.join(a.work, "catalog")
    os.makedirs(cdir, exist_ok=True)
    db_path = os.path.join(cdir, "c.lrcat")
    shutil.copyfile(cat, db_path)
    if os.path.isfile(cat + "-wal"):
        shutil.copyfile(cat + "-wal", db_path + "-wal")
    shutil.copyfile(os.path.join(prev_dir, "previews.db"), os.path.join(cdir, "previews.db"))
    db, pv = sqlite3.connect(db_path), sqlite3.connect(os.path.join(cdir, "previews.db"))
    exclude = set()
    for f in a.exclude or []:
        exclude |= {p["id"] for p in json.load(open(f))}
    by_model, wb_only = {}, {}
    raw_ext = {"dng", "cr3", "cr2", "raf", "arw", "nef", "orf", "rw2", "pef"}

    def as_shot(text):
        d = top_level(text)
        try:
            return (float(d["Temperature"]), float(d.get("Tint", 0))) if d.get("WhiteBalance") == "As Shot" and "Temperature" in d else None
        except ValueError:
            return None
    for iid, path, model, ext, text, _ in db.execute(SQL):
        if ext not in raw_ext or not model or iid in exclude:
            continue
        if a.camera and model not in a.camera.split(","):
            continue
        wb = as_shot(text)
        if look_of(text) in a.looks.split(",") and is_default(text):
            by_model.setdefault(model, []).append((iid, path, wb))
        elif wb:
            wb_only.setdefault(model, []).append((iid, path, wb))
    # as-shot white balance from the develop history too (Lightroom writes Temp / Tint into the
    # steps taken while the photo was at As Shot): history texts are a u32 length + zlib
    import zlib
    paths = {iid: (path, model) for iid, path, model, ext, text, _ in db.execute(SQL) if ext in raw_ext and model and iid not in exclude}
    seen = {iid for rows in wb_only.values() for iid, _, _ in rows} | {iid for rows in by_model.values() for iid, _, wb in rows if wb}
    for img, txt in db.execute("SELECT image, text FROM Adobe_libraryImageDevelopHistoryStep"):
        if img not in paths or img in seen or txt is None:
            continue
        b = txt if isinstance(txt, bytes) else txt.encode("latin1")
        try:
            t = zlib.decompress(b[4:]).decode("utf8", "replace")
        except zlib.error:
            t = b.decode("utf8", "replace")
        if 'WhiteBalance = "As Shot"' not in t:
            continue
        k, ti = re.search(r"\nTemperature = ([\d.]+)", t), re.search(r"\nTint = (-?[\d.]+)", t)
        if k:
            path, model = paths[img]
            wb_only.setdefault(model, []).append((img, path, (float(k.group(1)), float(ti.group(1)) if ti else 0.0)))
            seen.add(img)
    random.seed(a.seed)
    items = []
    for model, rows in sorted(by_model.items()):
        random.shuffle(rows)
        got = 0
        for iid, path, wb in rows:
            if got >= a.per_camera:
                break
            if not os.path.isfile(path):
                continue
            e = pv.execute("SELECT uuid, digest FROM ImageCacheEntry WHERE imageId=?", (iid,)).fetchone()
            if not e:
                continue
            fs = [f for f in glob.glob(os.path.join(prev_dir, e[0][0], e[0][:4], f"{e[0]}-{e[1]}_*")) if f.rsplit("_", 1)[1].isdigit()]
            fs = [f for f in fs if int(f.rsplit("_", 1)[1]) >= 600]
            if not fs:
                continue
            item = {"raw": path, "preview": min(fs, key=lambda f: int(f.rsplit("_", 1)[1])), "model": model, "id": iid}
            if wb:
                item["temp"], item["tint"] = wb
            items.append(item)
            got += 1
        # more as-shot photos (any other settings) for the white-balance model: header reads only
        extra = [r for r in wb_only.get(model, []) if os.path.isfile(r[1])]
        random.shuffle(extra)
        for iid, path, wb in extra[: a.wb_per_camera]:
            items.append({"raw": path, "temp": wb[0], "tint": wb[1], "model": model, "id": iid})
        print(f"{model}: {got} of {len(rows)} default-setting photos, {min(len(extra), a.wb_per_camera)} more as-shot for white balance", file=sys.stderr)
    for f in ("c.lrcat", "c.lrcat-wal"):
        if os.path.exists(os.path.join(cdir, f)):
            os.remove(os.path.join(cdir, f))
    lst = os.path.join(a.work, "calibrate.json")
    json.dump(items, open(lst, "w"), indent=1)
    cmd = [CLI, "calibrate", "--lightroom", lst, "--min-files", str(a.min_files)]
    if a.out:
        cmd += ["--out", a.out]
    if a.dry_run:
        print(" ".join(cmd))
        return
    subprocess.run(cmd, check=True)


# ---------------------------------------------------------------- migrate / render
def lib_dir(a):
    return os.path.join(a.work, f"lib-{a.tag}")


def migrate(a):
    lib = lib_dir(a)
    if os.path.exists(lib):
        shutil.rmtree(lib)
    cmd = [CLI, "migrate-lightroom", "--no-presets", "--library", lib, "--records", os.path.join(a.work, "records.json")]
    for p in a.profiles or []:
        cmd += ["--profiles", p]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    open(os.path.join(a.work, f"migrate-{a.tag}.json"), "w").write(out)
    d = json.loads(out)
    print(json.dumps({k: d.get("develop", {}).get(k) for k in ("applied", "creativeLooks", "creativeLooksMatched", "unmapped")}))


def photo_ids(lib):
    out = subprocess.run([CLI, "run", "--library", lib, "catalog.query", "limit=100000"], check=True, capture_output=True, text=True).stdout
    res = json.loads(out.splitlines()[0])["result"]["photos"]
    return {p["fileName"]: p["id"] for p in res if not p.get("copyOf")}


def render(a):
    lib = a.lib or lib_dir(a)
    sample = json.load(open(os.path.join(a.work, "sample.json")))
    ids = photo_ids(lib)
    outdir = os.path.join(a.work, "renders", a.tag)
    os.makedirs(outdir, exist_ok=True)
    script = []
    for p in sample:
        pid = ids.get(os.path.basename(p["path"]))
        if pid is None:
            print("not in library:", p["path"], file=sys.stderr)
            continue
        w, h = Image.open(p["preview"]).size
        out = os.path.join(outdir, f"{p['id']}.jpg")
        if os.path.exists(out):
            os.remove(out)
        script.append(json.dumps({"command": "app.export", "params": {"ids": [pid], "path": out, "longEdge": min(max(w, h), a.max_size or 100000), "quality": 97, "sharpen": "none", "metadata": "none", "conflict": "overwrite"}}))
    sp = os.path.join(outdir, "script.jsonl")
    open(sp, "w").write("\n".join(script) + "\n")
    r = subprocess.run([CLI, "run", "--library", lib, "--keep-going", "--script", sp], capture_output=True, text=True)
    bad = [l for l in r.stdout.splitlines() if '"ok":false' in l]
    for l in bad:
        print(l[:300], file=sys.stderr)
    print(f"rendered {len(script) - len(bad)} / {len(script)} -> {outdir}")


# ---------------------------------------------------------------- compare
def load_pair(prev, ours, size):
    a = Image.open(prev).convert("RGB")
    b = Image.open(ours).convert("RGB")
    if b.size != a.size:
        b = b.resize(a.size, Image.LANCZOS)
    s = size / max(a.size)
    if s < 1:
        ns = (max(1, round(a.size[0] * s)), max(1, round(a.size[1] * s)))
        a, b = a.resize(ns, Image.BOX), b.resize(ns, Image.BOX)
    return np.asarray(a), np.asarray(b)


def _grad(l):
    gx = np.zeros_like(l)
    gy = np.zeros_like(l)
    gx[:, 1:-1] = l[:, 2:] - l[:, :-2]
    gy[1:-1, :] = l[2:, :] - l[:-2, :]
    g = np.hypot(gx, gy)
    return (g - g.mean()) / (g.std() + 1e-9)


def _warp(img, scale, rot, tx, ty, size, resample=Image.BILINEAR):
    """Our image mapped by a similarity (about the centre) + shift; outside = NaN mask via alpha."""
    w, h = size
    cx, cy = w / 2, h / 2
    c, s_ = np.cos(np.radians(rot)) / scale, np.sin(np.radians(rot)) / scale
    # inverse map: output (x, y) -> input
    a, b = c, s_
    d, e = -s_, c
    xo = cx - a * (cx + tx) - b * (cy + ty)
    yo = cy - d * (cx + tx) - e * (cy + ty)
    return img.transform(size, Image.AFFINE, (a, b, xo, d, e, yo), resample=resample)


def align(a_img, b_img):
    """Similarity transform (scale, rotation °, shift px at a_img's size) best mapping our render
    onto Lightroom's preview: gradient correlation, scales 0.85..1.15, rotations -2..2°."""
    n = 256
    k = n / max(a_img.size)
    sz = (max(8, round(a_img.size[0] * k)), max(8, round(a_img.size[1] * k)))
    la = np.asarray(a_img.convert("L").resize(sz, Image.BOX), np.float64)
    gb = b_img.convert("L").resize(sz, Image.BOX)
    ga = _grad(la)
    Fa = np.conj(np.fft.fft2(ga))
    best = (-1e9, 1.0, 0.0, 0.0, 0.0)

    def score(scale, rot):
        wb = np.asarray(_warp(gb, scale, rot, 0, 0, sz), np.float64)
        g = _grad(wb)
        r = np.real(np.fft.ifft2(Fa * np.fft.fft2(g)))
        i = np.unravel_index(np.argmax(r), r.shape)
        dy, dx = i[0] if i[0] < sz[1] / 2 else i[0] - sz[1], i[1] if i[1] < sz[0] / 2 else i[1] - sz[0]
        return r[i] / ga.size, -dx, -dy

    for scale in np.arange(0.85, 1.151, 0.025):
        for rot in (-2, -1, 0, 1, 2):
            sc, dx, dy = score(scale, rot)
            if sc > best[0]:
                best = (sc, scale, rot, dx, dy)
    _, s0, r0, _, _ = best
    for scale in np.arange(s0 - 0.02, s0 + 0.021, 0.005):
        for rot in np.arange(r0 - 0.75, r0 + 0.76, 0.25):
            sc, dx, dy = score(scale, rot)
            if sc > best[0]:
                best = (sc, scale, rot, dx, dy)
    sc, scale, rot, dx, dy = best
    return {"corr": round(float(sc), 3), "scale": round(float(scale), 3), "rot": round(float(rot), 2), "dx": float(dx) / k, "dy": float(dy) / k}


def metrics(prev, ours, size):
    a = Image.open(prev).convert("RGB")
    b = Image.open(ours).convert("RGB")
    if b.size != a.size:
        b = b.resize(a.size, Image.LANCZOS)
    al = align(a, b)
    moved = abs(al["scale"] - 1) > 0.004 or abs(al["rot"]) > 0.1 or max(abs(al["dx"]), abs(al["dy"])) > 0.004 * max(a.size)
    s = min(1.0, size / max(a.size))
    ns = (max(1, round(a.size[0] * s)), max(1, round(a.size[1] * s)))
    a8 = np.asarray(a.resize(ns, Image.BOX))
    if moved:
        bw = _warp(b.convert("RGBA"), al["scale"], al["rot"], al["dx"], al["dy"], a.size, Image.BICUBIC)
        bw = np.asarray(bw.resize(ns, Image.BOX))
        valid = bw[..., 3] >= 255
        b8 = bw[..., :3]
    else:
        b8 = np.asarray(b.resize(ns, Image.BOX))
        valid = np.ones(ns[::-1], bool)
    # ignore a 1 % border (resampling edges)
    m = max(1, round(0.01 * max(ns)))
    valid[:m, :] = valid[-m:, :] = False
    valid[:, :m] = valid[:, -m:] = False
    la, lb = to_lab(a8, "adobe"), to_lab(b8, "srgb")
    la, lb = la[valid], lb[valid]
    de = de2000(la, lb)
    d = lb - la
    bands = {}
    for lo, hi in ((0, 25), (25, 50), (50, 75), (75, 101)):
        mk = (la[..., 0] >= lo) & (la[..., 0] < hi)
        if mk.sum() > 50:
            bands[f"{lo}-{hi}"] = [round(float(d[..., 0][mk].mean()), 2), round(float(d[..., 1][mk].mean()), 2), round(float(d[..., 2][mk].mean()), 2), round(float(de[mk].mean()), 2), round(float(mk.mean()), 3)]
    # colour at a coarse scale (64 px blocks of the compare size averaged): what is left when
    # detail (sharpening, noise, demosaicing, sub-pixel registration) is taken out
    ka = max(1, round(max(ns) / 128))
    def coarse(img):
        h2, w2 = img.shape[0] // ka * ka, img.shape[1] // ka * ka
        return img[:h2, :w2].reshape(h2 // ka, ka, w2 // ka, ka, -1).mean(axis=(1, 3))
    va = coarse(valid[..., None].astype(float))[..., 0] > 0.999
    a_lo = to_lab(coarse(a8.astype(float)).clip(0, 255), "adobe")[va]
    b_lo = to_lab(coarse(b8.astype(float)).clip(0, 255), "srgb")[va]
    de_lo = de2000(a_lo, b_lo)
    return {
        "de_lo": round(float(de_lo.mean()), 3),
        "p95_lo": round(float(np.percentile(de_lo, 95)), 3) if de_lo.size else 0.0,
        "align": al,
        "geometry": "moved" if moved else "ok",
        "de": round(float(de.mean()), 3),
        "p95": round(float(np.percentile(de, 95)), 3),
        "dL": round(float(d[..., 0].mean()), 2),
        "da": round(float(d[..., 1].mean()), 2),
        "db": round(float(d[..., 2].mean()), 2),
        "L_lr": round(float(la[..., 0].mean()), 1),
        "chroma_lr": round(float(np.hypot(la[..., 1], la[..., 2]).mean()), 2),
        "chroma_us": round(float(np.hypot(lb[..., 1], lb[..., 2]).mean()), 2),
        "bands": bands,
    }


def compare(a):
    sample = json.load(open(os.path.join(a.work, "sample.json")))
    base = None
    if a.base:
        bp = os.path.join(a.work, f"results-{a.base}.json")
        base = {r["id"]: r for r in json.load(open(bp))} if os.path.exists(bp) else None
    res = []
    for p in sample:
        ours = os.path.join(a.work, "renders", a.tag, f"{p['id']}.jpg")
        if not os.path.exists(ours):
            continue
        m = metrics(p["preview"], ours, a.size)
        m.update({"id": p["id"], "file": os.path.basename(p["path"]), "model": p["model"], "look": p["look"]})
        res.append(m)
    json.dump(res, open(os.path.join(a.work, f"results-{a.tag}.json"), "w"), indent=1)
    print(f"{'file':<22}{'camera':<20}{'look':<15}{'dE':>6}{'p95':>7}{'dE128':>7}{'dL':>7}{'da':>6}{'db':>6}{'C lr/us':>12}" + ("   base dE" if base else ""))
    for m in sorted(res, key=lambda m: (m["model"] or "", m["look"], m["file"])):
        extra = f"   {base[m['id']]['de']:6.2f}" if base and m["id"] in base else ""
        flag = " " if m["de"] <= 2 and m["p95"] <= 5 else "*"
        print(f"{m['file'][:21]:<22}{(m['model'] or '?')[:19]:<20}{m['look'][:14]:<15}{m['de']:6.2f}{m['p95']:7.2f}{m['de_lo']:7.2f}{m['dL']:7.2f}{m['da']:6.2f}{m['db']:6.2f}{m['chroma_lr']:6.1f}/{m['chroma_us']:<5.1f}{extra} {flag}{' geom' if m['geometry'] != 'ok' else ''}")
    def summary(key, rows):
        groups = {}
        for m in rows:
            groups.setdefault(key(m), []).append(m["de"])
        return {k: round(float(np.mean(v)), 2) for k, v in sorted(groups.items())}
    def summary2(key, rows):
        groups = {}
        for m in rows:
            groups.setdefault(key(m), []).append(m["de_lo"])
        return {k: round(float(np.mean(v)), 2) for k, v in sorted(groups.items())}
    print("mean dE by camera:", summary(lambda m: m["model"] or "?", res))
    print("mean dE by look:  ", summary(lambda m: m["look"] or "(none)", res))
    print("mean dE128 by camera:", summary2(lambda m: m["model"] or "?", res))
    print(f"all: dE {np.mean([m['de'] for m in res]):.2f}, dE128 {np.mean([m['de_lo'] for m in res]):.2f}; pass (dE<=2 & p95<=5): {sum(m['de'] <= 2 and m['p95'] <= 5 for m in res)}/{len(res)}")
    if a.strip:
        os.makedirs(os.path.join(a.work, "strips"), exist_ok=True)
        for sid in a.strip.split(","):
            p = next((x for x in sample if str(x["id"]) == sid), None)
            if not p:
                continue
            ims = [Image.open(p["preview"]).convert("RGB")]
            for t in ([a.base] if a.base else []) + [a.tag]:
                f = os.path.join(a.work, "renders", t, f"{sid}.jpg")
                if os.path.exists(f):
                    ims.append(Image.open(f).convert("RGB").resize(ims[0].size, Image.LANCZOS))
            h = 900
            ims = [im.resize((round(im.size[0] * h / im.size[1]), h), Image.LANCZOS) for im in ims]
            # (the Lightroom preview is Adobe RGB: convert to sRGB numerically for viewing)
            lr = np.asarray(ims[0]).astype(np.float64) / 255
            xyz = adobe_decode(lr) @ ADOBE_TO_XYZ.T
            srgb_lin = np.clip(xyz @ np.linalg.inv(SRGB_TO_XYZ).T, 0, 1)
            enc = np.where(srgb_lin <= 0.0031308, 12.92 * srgb_lin, 1.055 * srgb_lin ** (1 / 2.4) - 0.055)
            ims[0] = Image.fromarray((enc * 255 + 0.5).astype(np.uint8))
            W = sum(i.size[0] for i in ims) + 10 * (len(ims) - 1)
            out = Image.new("RGB", (W, h), (40, 40, 40))
            x = 0
            for im in ims:
                out.paste(im, (x, 0))
                x += im.size[0] + 10
            out.save(os.path.join(a.work, "strips", f"{sid}-{a.tag}.jpg"), quality=90)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("--work", required=True)
    p.add_argument("--catalog")
    p.add_argument("--previews")
    p.add_argument("--ids")
    p.add_argument("--spec")
    p.add_argument("--seed", type=int, default=7)
    p.add_argument("--min-px", type=int, default=900)
    p.set_defaults(fn=prepare)
    for name, fn in (("migrate", migrate), ("render", render)):
        p = sub.add_parser(name)
        p.add_argument("--work", required=True)
        p.add_argument("--tag", required=True)
        p.add_argument("--lib")
        p.add_argument("--profiles", action="append", help="(migrate) profile folders/files to import first")
        p.add_argument("--max-size", type=int, default=1200, help="(render) longest edge rendered (compare works at 512 px)")
        p.set_defaults(fn=fn)
    p = sub.add_parser("calibrate", help="fit Lightroom-matched camera profiles from default-setting photos")
    p.add_argument("--work", required=True)
    p.add_argument("--catalog")
    p.add_argument("--previews")
    p.add_argument("--out", help="profiles folder (default: LightCraft's, <config>/camera-profiles)")
    p.add_argument("--looks", default="Adobe Color", help="comma-separated Lightroom profiles/looks the photos may use")
    p.add_argument("--camera", help="comma-separated camera models (default: all)")
    p.add_argument("--per-camera", type=int, default=40)
    p.add_argument("--min-files", type=int, default=5)
    p.add_argument("--wb-per-camera", type=int, default=150, help="extra as-shot photos per camera for the white-balance model")
    p.add_argument("--exclude", action="append", help="sample.json whose photos are left out (held-out test sets)")
    p.add_argument("--seed", type=int, default=11)
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=calibrate)
    p = sub.add_parser("compare")
    p.add_argument("--work", required=True)
    p.add_argument("--tag", required=True)
    p.add_argument("--base")
    p.add_argument("--size", type=int, default=512)
    p.add_argument("--strip")
    p.set_defaults(fn=compare)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
