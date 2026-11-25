#!/usr/bin/env python3
"""camera_check.py: the new-camera check. One representative raw per camera model in the LightCraft library,
LightCraft vs Camera Raw at the same slider values, measured with the 2026-11-25 slider study's metrics, flagged,
recorded. Camera Raw's renders are kept in a reference bank, so the check keeps working after Photoshop is gone.

  camera_check.py --list                     every model in the library: raws, checked / unchecked
  camera_check.py --model "RICOH GR III"     check one model (substring of the library's camera name)
  camera_check.py --all-new                  check every model with a raw and no (complete) check on file
  options: --force (re-check, same file), --file PATH / --photo-id N (pick the raw), --no-photoshop (reference bank
           only), --keep (keep scratch renders), --no-needs-you, --keep-photoshop (don't quit it at the end), --library DIR

Reference, in this order: (1) the bank, ~/Library/Application Support/LightCraft/reference/<model>/<step>.tif + index.json
(every Camera Raw render the check ever made, its file, tint and stats); (2) Photoshop 2026 fills the steps the bank
lacks (hdr_ps.sh, the shared lock) and they go into the bank; (3) no bank and no Photoshop (or --no-photoshop): a
base-only check against the embedded preview of Adobe's free DNG Converter ("base via DNG Converter preview, defaults
incl. sharpening", less exact; a model checked that way is re-checked in full when Photoshop is available), or a note
to install it. Missing references are reported and the model skipped.

Base: Adobe Standard, linear curve, sharpening / NR / lens off. Steps: Exposure -2 / +2, Whites +100, Shadows +100,
Highlights -100, Temp 3000 K / 7500 K (as-shot tint). 1536 px (Camera Raw's minimum). A step is flagged when mean
dE2000 > 3, p95 > 8, clipped % differs by > 3 points, or "acts like CR" is off by > 20 (percent of the slider value:
LightCraft's effect / Camera Raw's in the slider's tone band, Temp: neutral b*). Never writes to the photo files or the
library: the raw is copied (APFS clone when possible) to scratch. Records: ~/Library/Application Support/LightCraft/
camera-checks.json; report.md + strip.jpg per model in .../compare/camera-checks/<model>/. See README.md."""
import argparse, datetime, json, os, re, shutil, subprocess, sys, time

import numpy as np
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import dng  # noqa: E402
import tiff  # noqa: E402
from common import PP_TO_XYZ, Y, lab, lc  # noqa: E402
from cmp import de2000  # noqa: E402

SUPPORT = os.path.expanduser("~/Library/Application Support/LightCraft")
CHECKS = f"{SUPPORT}/camera-checks.json"
OUTDIR = f"{SUPPORT}/compare/camera-checks"
BANK = f"{SUPPORT}/reference"
LOCK = os.environ.get("PS_LOCK", "/private/tmp/lightcraft-photoshop.lock")
PS_APP = "/Applications/Adobe Photoshop 2026"
PS_PROC = "Adobe Photoshop 2026.app/Contents/MacOS/Adobe Photoshop 2026"
DNGCONV = "/Applications/Adobe DNG Converter.app/Contents/MacOS/Adobe DNG Converter"
PREVIEW_NOTE = "base via DNG Converter preview, defaults incl. sharpening"
T_DE, T_P95, T_CLIP, T_ACTS = 3.0, 8.0, 3.0, 20.0   # flag thresholds
BANDS = {"sh": lambda L: L < 30, "mid": lambda L: (L >= 30) & (L < 70), "hi": lambda L: L >= 70}  # CR base L*
SLIDER_BAND = {"Exposure2012": "mid", "Whites2012": "hi", "Shadows2012": "sh", "Highlights2012": "hi"}
XYZ2SRGB = np.array([[3.1338561, -1.6168667, -0.4906146], [-0.9787684, 1.9161415, 0.0334540], [0.0719453, -0.2289914, 1.4052427]])
SRGB2PP = np.linalg.inv(PP_TO_XYZ) @ np.linalg.inv(XYZ2SRGB)   # linear sRGB (D50-adapted matrix) -> linear ProPhoto
PS_USED = False


def steps(tint):
    cust = lambda t: dict(WhiteBalance="Custom", Temperature=str(t), Tint=f"{tint:+d}")
    return [("base", "base", 0, {}),
            ("Exposure2012_-2", "Exposure2012", -2, {"Exposure2012": "-2"}), ("Exposure2012_+2", "Exposure2012", 2, {"Exposure2012": "+2"}),
            ("Whites2012_+100", "Whites2012", 100, {"Whites2012": "+100"}), ("Shadows2012_+100", "Shadows2012", 100, {"Shadows2012": "+100"}),
            ("Highlights2012_-100", "Highlights2012", -100, {"Highlights2012": "-100"}),
            ("Temperature_3000", "Temperature", 3000, cust(3000)), ("Temperature_7500", "Temperature", 7500, cust(7500))]


def slug(s):
    return re.sub(r"[^A-Za-z0-9]+", "_", s).strip("_")


def jload(path, default):
    try:
        return json.load(open(path))
    except (OSError, ValueError):
        return default


def jsave(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    json.dump(obj, open(path + ".tmp", "w"), indent=1)
    os.replace(path + ".tmp", path)


# ---------------------------------------------------------------- library (read-only)
def load_models(a):
    """camera name -> {"raws": [photo records], "other": n}. catalog.snap is opened read-only."""
    lib = a.library or os.environ.get("LIGHTCRAFT_LIBRARY") or os.path.expanduser("~/Pictures/LightCraft Library")
    with open(f"{lib}/catalog.snap", "rb") as f:
        photos = json.load(f)["catalog"]["photos"]
    models = {}
    for p in photos.values():
        if p.get("deleted"):
            continue
        m = models.setdefault((p.get("meta") or {}).get("camera") or "?", {"raws": [], "other": 0})
        if p.get("kind") == "raw":
            m["raws"].append(p)
        else:
            m["other"] += 1
    return models


def pick_file(raws, a, prior):
    """The raw to check: --file / --photo-id, else the file used last time (bank or check), else the middle (by id)
    of the readable raws at ISO <= 1600 (a representative frame, not a pano or a high-ISO one)."""
    if a.file:
        return next((p for p in raws if p["source"].get("path") == a.file), None), a.file
    if a.photo_id:
        p = next((p for p in raws if p["id"] == a.photo_id), None)
        return (p, p["source"]["path"]) if p else (None, None)
    if prior and os.path.isfile(prior.get("file", "")):
        return next((p for p in raws if p["id"] == prior.get("photo_id")), None), prior["file"]
    pool = sorted((p for p in raws if (p["meta"].get("iso") or 0) <= 1600 and "-Pano" not in p["source"].get("path", "")), key=lambda p: p["id"]) \
        or sorted(raws, key=lambda p: p["id"])
    mid = len(pool) // 2
    for k in range(len(pool)):
        p = pool[(mid + (k + 1) // 2 * (1 if k % 2 else -1)) % len(pool)]
        path = p["source"].get("path", "")
        if p["source"].get("type") == "file" and os.path.isfile(path):
            return p, path
    return None, None


# ---------------------------------------------------------------- renders
def prepare(work, path, key, tint, have, bank):
    """Scratch set: the raw (copied once), a clone + sidecar per step, the bank's Camera Raw renders in acr/, a manifest of
    the steps still to render in Camera Raw. Returns meta {name: {...step...}}."""
    for d in ("src", "v", "acr", "lc"):
        os.makedirs(f"{work}/{d}", exist_ok=True)
    ext = os.path.splitext(path)[1]
    src = f"{work}/src/{key}{ext}"
    if subprocess.run(["/bin/cp", "-c", path, src], capture_output=True).returncode != 0:  # other volume: plain copy
        shutil.copyfile(path, src)
    man, meta = [], {}
    for name, slider, value, st in steps(tint):
        n = f"{key}__{name}"
        dst = f"{work}/v/{n}{ext}"
        subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE)
        s.update(st)
        open(f"{work}/v/{n}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        if name in have:
            shutil.copyfile(f"{bank}/{name}.tif", f"{work}/acr/{n}.tif")
        else:
            man.append(f"{dst}\t{work}/acr/{n}.tif\tmin")
        meta[n] = dict(photo=key, slider=slider, value=value, settings=st, step=name)
    open(f"{work}/manifest.txt", "w").write("\n".join(man) + "\n")
    return meta


def oracle_bin():
    for c in (os.environ.get("ORACLE_BIN"), f"{os.path.dirname(os.path.dirname(os.path.dirname(HERE)))}/target/release/examples/oracle_render",
              os.path.expanduser("~/crafts/lightcraft/target/release/examples/oracle_render")):
        if c and os.path.isfile(c):
            return c
    sys.exit("no oracle_render: cargo build --release -p lightcraft-engine --example oracle_render (or set ORACLE_BIN)")


def render_lc(work):
    subprocess.run([f"{HERE}/lcrun.sh", work, "lc"], env=dict(os.environ, ORACLE_BIN=oracle_bin(), J="4"), check=True)


def render_cr(work):
    global PS_USED
    PS_USED = True
    r = subprocess.run([f"{HERE}/hdr_ps.sh", work], env=dict(os.environ, PS_WHO="camera_check"))
    if r.returncode:
        sys.exit(f"Camera Raw render failed (hdr_ps.sh exit {r.returncode}; 3 = Photoshop has open documents, 4 = stuck, 5 = lock busy)")


def quit_photoshop():
    """Quit Photoshop when nobody holds the shared lock and it has no open documents (it slows his system)."""
    running = lambda: subprocess.run(["pgrep", "-f", PS_PROC], capture_output=True).returncode == 0
    if not running():
        return print("Photoshop is not running")
    try:
        os.mkdir(LOCK)
    except FileExistsError:
        owner = open(f"{LOCK}/pid").read().strip() if os.path.exists(f"{LOCK}/pid") else ""
        try:
            os.kill(int(owner), 0)
            return print(f"Photoshop lock held by {open(f'{LOCK}/who').read().strip()}: leaving Photoshop running")
        except (ValueError, OSError):
            shutil.rmtree(LOCK, ignore_errors=True)
            os.mkdir(LOCK)
    try:
        open(f"{LOCK}/pid", "w").write(str(os.getpid()))
        open(f"{LOCK}/who", "w").write(f"camera_check quit {time.ctime()}")
        n = subprocess.run(["osascript", "-e", "with timeout of 60 seconds", "-e", 'tell application "Adobe Photoshop 2026" to do javascript "app.documents.length"',
                            "-e", "end timeout"], capture_output=True, text=True).stdout.strip()
        if n != "0":
            return print(f"Photoshop has documents open ({n or '?'}): leaving it running")
        subprocess.run(["osascript", "-e", 'tell application "Adobe Photoshop 2026" to quit'], capture_output=True)
        for _ in range(30):
            time.sleep(1)
            if not running():
                return print("Photoshop quit")
        print("Photoshop did not quit within 30 s")
    finally:
        shutil.rmtree(LOCK, ignore_errors=True)


# ---------------------------------------------------------------- measurement (the study's definitions)
def srgb8(pp):
    lin = np.clip(np.clip(pp, 0, 1) @ PP_TO_XYZ.T @ XYZ2SRGB.T, 0, 1)
    e = np.where(lin <= 0.0031308, 12.92 * lin, 1.055 * np.power(lin, 1 / 2.4) - 0.055)
    return (e * 255 + 0.5).astype(np.uint8)


class Pair:
    """Camera Raw (a) and LightCraft (b) renders of one step: same orientation and size, linear ProPhoto 0..1."""
    rot = None

    def __init__(self, a, b):
        a, b = np.clip(a, 0, 1), np.clip(b, 0, 1)
        if a.shape[:2] == b.shape[:2][::-1] and a.shape[0] != a.shape[1]:   # oracle_render ignores EXIF orientation
            if Pair.rot is None:
                errs = {k: float(np.abs(np.rot90(b, k)[::8, ::8, 1] - a[::8, ::8, 1]).mean()) for k in (1, 3)}
                Pair.rot = min(errs, key=errs.get)
            b = np.ascontiguousarray(np.rot90(b, Pair.rot))
        h, w = min(a.shape[0], b.shape[0]), min(a.shape[1], b.shape[1])
        if abs(a.shape[0] - b.shape[0]) > 2 or abs(a.shape[1] - b.shape[1]) > 2:
            raise SystemExit(f"the two renders differ in size ({a.shape[:2]} vs {b.shape[:2]}): not comparable")
        self.a, self.b = a[:h, :w], b[:h, :w]
        self.la, self.lb = lab(self.a), lab(self.b)


def read_cr(work, n):
    a, d = tiff.read(f"{work}/acr/{n}.tif")
    assert "ProPhoto" in d, d
    return a ** 1.8


def work_pair(work, n):
    return Pair(read_cr(work, n), lc(work, n))


def clip_pct(img):
    return float((img.max(-1) >= 0.995).mean() * 100)


def measure(meta, load):
    """Per-step metrics + flags. meta: {name: step dict} in step order, base first; load(name) -> Pair."""
    names = {m["step"]: n for n, m in meta.items()}
    base = load(names["base"])
    LA, LB = base.la[..., 0], base.lb[..., 0]
    bmA, bmB = {k: f(LA) for k, f in BANDS.items()}, {k: f(LB) for k, f in BANDS.items()}
    chroma = np.hypot(base.la[..., 1], base.la[..., 2])
    neu = (chroma < 8) & (LA > 25) & (LA < 85)
    if neu.mean() < 0.005:
        neu = (chroma < 14) & (LA > 20) & (LA < 90)
    wbb = lambda L: float(L[..., 2][neu].mean())
    res, pairs = {}, {}
    for step, n in names.items():
        m = meta[n]
        pr = base if step == "base" else load(n)
        pairs[step] = pr
        d = de2000(pr.la, pr.lb)
        r = dict(step=step, slider=m["slider"], value=m["value"], dE_mean=float(d.mean()), dE_p95=float(np.percentile(d, 95)),
                 clip_pct={"cr": clip_pct(pr.a), "lc": clip_pct(pr.b)},
                 crush_pct={"cr": float((Y(pr.a) <= 0.002).mean() * 100), "lc": float((Y(pr.b) <= 0.002).mean() * 100)},
                 dL_band={k: float((pr.lb[..., 0] - pr.la[..., 0])[bmA[k]].mean()) if bmA[k].any() else None for k in BANDS})
        flags = []
        if m["slider"] in SLIDER_BAND:
            k = SLIDER_BAND[m["slider"]]
            r["effect"] = dict(band=k, unit="L*", cr=float((pr.la[..., 0] - LA)[bmA[k]].mean()), lc=float((pr.lb[..., 0] - LB)[bmB[k]].mean()))
        elif m["slider"] == "Temperature":
            r["effect"] = dict(band="neutral", unit="b*", cr=wbb(pr.la) - wbb(base.la), lc=wbb(pr.lb) - wbb(base.lb))
        e = r.get("effect")
        if e and abs(e["cr"]) >= 2:   # a tiny Camera Raw effect gives no ratio worth reading
            e["acts_like_pct"] = 100 * e["lc"] / e["cr"]
            if m["slider"] != "Temperature":   # slider units (linear through 0 and this step); Temp: percent only
                e["acts_like"] = m["value"] * e["lc"] / e["cr"]
            if abs(e["acts_like_pct"] - 100) > T_ACTS:
                flags.append(f"acts like {e['acts_like_pct']:.0f}% of CR" if "acts_like" not in e else
                             f"acts like CR {e['acts_like']:+.0f}" if abs(m["value"]) >= 10 else f"acts like CR {e['acts_like']:+.2f}")
        if r["dE_mean"] > T_DE:
            flags.append(f"dE {r['dE_mean']:.1f}")
        if r["dE_p95"] > T_P95:
            flags.append(f"p95 {r['dE_p95']:.1f}")
        if abs(r["clip_pct"]["lc"] - r["clip_pct"]["cr"]) > T_CLIP:
            flags.append(f"clipped {r['clip_pct']['lc']:.1f}% vs {r['clip_pct']['cr']:.1f}%")
        r["flags"], r["status"] = flags, "flag" if flags else "pass"
        res[step] = r
    return res, pairs


def cr_stats(img):
    L = lab(np.clip(img, 0, 1))
    return dict(clip_pct=clip_pct(np.clip(img, 0, 1)), mean_L=float(L[..., 0].mean()), mean_a=float(L[..., 1].mean()), mean_b=float(L[..., 2].mean()))


def label(img, text):
    im = Image.fromarray(img)
    ImageDraw.Draw(im).rectangle([0, 0, 7 * len(text) + 6, 13], fill=(0, 0, 0))
    ImageDraw.Draw(im).text((3, 1), text, fill=(255, 255, 255))
    return np.asarray(im)


def strip(res, pairs, path, cr_name="CR"):
    """LightCraft row over Camera Raw row, one column per step."""
    tw = 288
    th = int(round(tw * pairs["base"].a.shape[0] / pairs["base"].a.shape[1]))
    th_ = lambda img: np.asarray(Image.fromarray(srgb8(img)).resize((tw, th), Image.BOX))
    short = lambda s: s.replace("2012", "").replace("Exposure", "Exp").replace("Highlights", "Hi").replace("Shadows", "Sh").replace("Whites", "Wh").replace("Temperature", "T").replace("_", " ")
    top = np.concatenate([label(th_(pairs[s].b), f"LC {short(s)}") for s in res], 1)
    bot = np.concatenate([label(th_(pairs[s].a), f"{cr_name} {short(s)}") for s in res], 1)
    Image.fromarray(np.concatenate([top, np.full((6, top.shape[1], 3), 255, np.uint8), bot], 0)).save(path, quality=88)


def report(model, entry, path):
    flagged = "; ".join(f"{s} ({', '.join(r['flags'])})" for s, r in entry["steps"].items() if r["flags"])
    L = [f"# {model}: new-camera check, {entry['date']}", "", f"**{entry['status'].upper()}**: {flagged or 'every step within thresholds'}", ""]
    if entry.get("note"):
        L += [f"**{entry['note']}**: less exact than the full check (Camera Raw's own render at its defaults, sharpening on; re-checked in full when Photoshop or a reference bank is available).", ""]
    L += [f"File `{entry['file']}` (library photo {entry['photo_id']}, ISO {entry.get('iso')}), reference: {entry['reference']}, as-shot tint {entry['tint']:+d}. "
          f"{'Base: Adobe Standard, linear curve, sharpening / NR / lens off. Same slider value in both engines, no tuning. ' if not entry.get('note') else ''}"
          f"Flag: mean dE2000 > {T_DE:g}, p95 > {T_P95:g}, clipped % differs by > {T_CLIP:g} points, or acts-like off by > {T_ACTS:g}%.", "",
          "| step | dE mean | p95 | clipped LC / CR % | effect LC / CR | acts like CR | flag |", "|---|---|---|---|---|---|---|"]
    for s, r in entry["steps"].items():
        e = r.get("effect")
        eff = f"{e['lc']:+.1f} / {e['cr']:+.1f} {e['unit']} ({e['band']})" if e else ""
        al = (f"{e['acts_like']:+.0f}" if abs(r["value"]) >= 10 else f"{e['acts_like']:+.2f}") if e and "acts_like" in e else (f"{e['acts_like_pct']:.0f}% of CR" if e and "acts_like_pct" in e else "")
        L.append(f"| {s} | {r['dE_mean']:.2f} | {r['dE_p95']:.2f} | {r['clip_pct']['lc']:.1f} / {r['clip_pct']['cr']:.1f} | {eff} | {al} | {'; '.join(r['flags']) or 'ok'} |")
    L += ["", "![strip](strip.jpg)", "", "Strip: LightCraft row over the reference row. Metrics as in the 2026-11-25 slider study "
          "(dE2000 per pixel; L* bands sh < 30 <= mid < 70 <= hi on the reference's base; clipped = max channel >= 0.995 linear)."]
    open(path, "w").write("\n".join(L) + "\n")


# ---------------------------------------------------------------- base-only fallback: DNG Converter's embedded preview
def preview_check(work, path, key, a):
    """(res, pairs) for the base step only: LightCraft at Camera Raw's defaults vs the preview Adobe DNG Converter embeds."""
    out = f"{work}/dngc"
    os.makedirs(out, exist_ok=True)
    src = f"{work}/src/{key}{os.path.splitext(path)[1]}"
    os.makedirs(os.path.dirname(src), exist_ok=True)
    if subprocess.run(["/bin/cp", "-c", path, src], capture_output=True).returncode != 0:
        shutil.copyfile(path, src)
    subprocess.run([DNGCONV, "-p2", "-d", out, "-o", f"{key}.dng", src], check=True)
    conv = f"{out}/{key}.dng"
    jpg = subprocess.run(["exiftool", "-b", "-PreviewImage", conv], capture_output=True).stdout
    cs = subprocess.run(["exiftool", "-s3", "-PreviewColorSpace", conv], capture_output=True, text=True).stdout.strip()
    if not jpg or cs not in ("sRGB", "1"):
        sys.exit(f"DNG Converter's preview is unusable (colour space {cs!r}, {len(jpg)} bytes)")
    open(f"{work}/preview.jpg", "wb").write(jpg)
    im = Image.open(f"{work}/preview.jpg").convert("RGB")
    e = np.asarray(im).astype(np.float64) / 255
    lin = np.where(e <= 0.04045, e / 12.92, ((e + 0.055) / 1.055) ** 2.4)
    cr = lin @ SRGB2PP.T
    # Camera Raw's defaults: Adobe Color, sharpening 40, colour NR 25, lens off; LightCraft at the preview's size
    os.makedirs(f"{work}/v", exist_ok=True)
    n = f"{key}__base"
    shutil.copyfile(src, f"{work}/v/{n}{os.path.splitext(path)[1]}")
    s = dict(dng.BASE)
    s.update(CameraProfile="Adobe Color", Sharpness="40", ColorNoiseReduction="25")
    open(f"{work}/v/{n}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
    os.makedirs(f"{work}/lc", exist_ok=True)
    subprocess.run([oracle_bin(), f"{work}/lc", f"{work}/v/{n}{os.path.splitext(path)[1]}"], env=dict(os.environ, LIGHTCRAFT_GPU="0", ORACLE_SIZE=str(max(im.size))),
                   check=True, stdout=subprocess.DEVNULL)
    meta = {n: dict(photo=key, slider="base", value=0, settings={}, step="base")}
    return measure(meta, lambda _: Pair(cr, lc(work, n)))


# ---------------------------------------------------------------- one model
def check(model, info, a, checks):
    t0 = time.time()
    bdir = f"{BANK}/{slug(model)}"
    idx = jload(f"{bdir}/index.json", None)
    p, path = pick_file(info["raws"], a, idx or checks["checks"].get(model))
    if not path:
        print(f"{model}: no readable raw (offline volume?)")
        return False
    if idx and idx.get("file") != path:
        idx = None   # the bank holds renders of another frame
    as_shot = (p or {}).get("as_shot_wb") or [5500, 0]
    tint = idx["tint"] if idx else int(round(as_shot[1]))
    key = re.sub(r"[^A-Za-z0-9_-]", "_", os.path.splitext(os.path.basename(path))[0])
    want = [s[0] for s in steps(tint)]
    have = [s for s in want if idx and os.path.isfile(f"{bdir}/{idx['steps'].get(s, {}).get('tif', '-')}")]
    missing = [s for s in want if s not in have]
    ps_ok = os.path.isdir(PS_APP) and not a.no_photoshop
    work = f"/private/tmp/lightcraft-camera-check/{slug(model)}"
    shutil.rmtree(work, ignore_errors=True)
    print(f"{model}: {path}\n  reference bank: {len(have)}/{len(want)} steps" + (f"; Camera Raw to render: {', '.join(missing)}" if missing and ps_ok else ""), flush=True)
    note, reference = None, "reference bank"
    try:
        if missing and not ps_ok:
            if have:
                print(f"  {model}: skipped, bank lacks {', '.join(missing)} and Photoshop is {'switched off (--no-photoshop)' if os.path.isdir(PS_APP) else 'not installed'}")
                return False
            if not os.path.isfile(DNGCONV):
                print(f"  {model}: no reference bank and no Photoshop: install Adobe DNG Converter for the base check; skipped")
                return False
            res, pairs = preview_check(work, path, key, a)
            note, reference, tint = PREVIEW_NOTE, "DNG Converter embedded preview", int(round(as_shot[1]))
        else:
            meta = prepare(work, path, key, tint, have, bdir)
            render_lc(work)
            if missing:
                render_cr(work)
                os.makedirs(bdir, exist_ok=True)
                new = idx or dict(model=model, file=path, photo_id=(p or {}).get("id"), tint=tint, size=1536, steps={})
                for n, m in meta.items():
                    if m["step"] in missing:
                        shutil.copyfile(f"{work}/acr/{n}.tif", f"{bdir}/{m['step']}.tif")
                        new["steps"][m["step"]] = dict(tif=f"{m['step']}.tif", settings=m["settings"], date=datetime.date.today().isoformat(), **cr_stats(read_cr(work, n)))
                new["updated"] = datetime.date.today().isoformat()
                jsave(f"{bdir}/index.json", new)
                reference = "Photoshop (bank filled)" if not have else f"bank + Photoshop ({len(missing)} steps added)"
            res, pairs = measure(meta, lambda n: work_pair(work, n))
        out = f"{OUTDIR}/{slug(model)}"
        os.makedirs(out, exist_ok=True)
        strip(res, pairs, f"{out}/strip.jpg", "CR" if not note else "prev")
    finally:
        if not a.keep:
            shutil.rmtree(work, ignore_errors=True)
    flagged = {s: r["flags"] for s, r in res.items() if r["flags"]}
    entry = dict(model=model, date=datetime.date.today().isoformat(), time=datetime.datetime.now().isoformat(timespec="seconds"), file=path,
                 photo_id=(p or {}).get("id"), iso=((p or {}).get("meta") or {}).get("iso"), tint=tint, size=1536 if not note else None,
                 reference=reference, oracle=oracle_bin(), steps=res, status="flag" if flagged else "pass", flagged=flagged,
                 runtime_s=round(time.time() - t0, 1))
    if note:
        entry.update(note=note, partial=True)
    checks["checks"][model] = entry
    jsave(CHECKS, checks)
    report(model, entry, f"{out}/report.md")
    print(f"{model}: {entry['status']} in {entry['runtime_s']} s ({reference}) -> {out}/report.md")
    for s, f in flagged.items():
        print(f"  {s}: {'; '.join(f)}")
    if flagged and not a.no_needs_you:
        what = "; ".join(f"{s.replace('2012', '').replace('_', ' ')} ({', '.join(f)})" for s, f in flagged.items()) + (f" [{note}]" if note else "")
        ny = os.path.expanduser("~/.agents/needs-you.sh")
        if f"{model}: " in subprocess.run([ny, "list", "lightcraft"], capture_output=True, text=True).stdout:
            print("  needs.you item for this model already open")
        else:
            subprocess.run([ny, "add", "lightcraft", "test.", f"{model}: {what}", "--why", "new-camera check vs Camera Raw", "--by", "claude"], check=False)
    return True


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--model")
    ap.add_argument("--all-new", action="store_true")
    ap.add_argument("--force", action="store_true", help="re-check a model that has a check on file")
    ap.add_argument("--file")
    ap.add_argument("--photo-id", type=int)
    ap.add_argument("--no-photoshop", action="store_true", help="reference bank only (else the DNG Converter base check)")
    ap.add_argument("--keep", action="store_true", help="keep the scratch renders")
    ap.add_argument("--no-needs-you", action="store_true")
    ap.add_argument("--keep-photoshop", action="store_true")
    ap.add_argument("--library")
    a = ap.parse_args()
    if not (a.list or a.model or a.all_new):
        ap.error("--list, --model or --all-new")
    models, checks = load_models(a), jload(CHECKS, {"version": 1, "checks": {}})
    ps_ok = os.path.isdir(PS_APP) and not a.no_photoshop
    # a base-only (DNG Converter) check is redone in full when the bank or Photoshop can do it
    done_ = lambda m: m in checks["checks"] and not (checks["checks"][m].get("partial") and (ps_ok or os.path.isfile(f"{BANK}/{slug(m)}/index.json")))
    if a.list:
        print(f"{'model':46s} {'raws':>5s} {'other':>5s}  state")
        for m, i in sorted(models.items(), key=lambda kv: -len(kv[1]["raws"])):
            c = checks["checks"].get(m)
            state = (f"checked {c['date']} {c['status']}" + (" (base via preview)" if c.get("partial") else "")) if c else ("unchecked" if i["raws"] else "no raw (nothing to check)")
            print(f"{m:46s} {len(i['raws']):5d} {i['other']:5d}  {state}" + ("  [bank]" if os.path.isfile(f"{BANK}/{slug(m)}/index.json") else ""))
        return
    if a.model:
        hit = [m for m in models if a.model.lower() in m.lower()]
        hit = [m for m in hit if m.lower() == a.model.lower()] or hit
        if len(hit) != 1:
            sys.exit(f"--model {a.model!r}: {'no match' if not hit else 'ambiguous: ' + ', '.join(hit)}")
        todo = hit
    else:
        todo = [m for m, i in models.items() if i["raws"] and not done_(m)]
        for m in models:
            if models[m]["raws"] and done_(m):
                print(f"{m}: already checked {checks['checks'][m]['date']} ({checks['checks'][m]['status']}), skipped")
    n = 0
    for m in todo:
        if m in checks["checks"] and not a.force and done_(m):
            print(f"{m}: already checked {checks['checks'][m]['date']} ({checks['checks'][m]['status']}); --force to repeat")
        elif not models[m]["raws"]:
            print(f"{m}: no raw files, nothing to check")
        else:
            n += check(m, models[m], a, checks)
    if not todo:
        print("every model with a raw has a check on file")
    if PS_USED and not a.keep_photoshop:
        quit_photoshop()


if __name__ == "__main__":
    main()
