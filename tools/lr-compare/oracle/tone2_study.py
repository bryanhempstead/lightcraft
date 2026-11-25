#!/usr/bin/env python3
"""tone2_study.py: the 2026-11-25 slider study's Exposure / Whites / Contrast steps on the four study raws, LightCraft
builds (before / after) against Camera Raw at identical slider values. Reuses camera_check.py's metrics.

  tone2_study.py prep                 clone the raws + sidecars into WORK, fill acr/ from the reference bank, list the rest
  tone2_study.py cr                   render the missing Camera Raw steps (hdr_ps.sh: lock, quits Photoshop at the end)
  tone2_study.py lc NAME BIN          render every step with the oracle_render BIN into WORK/lc_NAME/
  tone2_study.py cmp NAME [NAME...]   metrics per raw / step for each build (json to WORK/res_NAME.json) + table
  tone2_study.py strips NAME...       before / after / CR strips for the key steps into OUT (compare/.../fix-tone2/)
WORK = /private/tmp/lightcraft-tone2 (or $T2_WORK)."""
import json, os, shutil, subprocess, sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import camera_check as cc  # noqa: E402
import dng  # noqa: E402
from common import lab  # noqa: E402

WORK = os.environ.get("T2_WORK", "/private/tmp/lightcraft-tone2")
OUT = os.path.expanduser("~/Library/Application Support/LightCraft/compare/2026-11-25/fix-tone2")
BANKDIR = os.path.expanduser("~/Library/Application Support/LightCraft/reference/RICOH_IMAGING_COMPANY_LTD_RICOH_GR_III")
RAWS = {
    "BHM_9815": "/Users/bryanhempstead/Pictures/Lightroom/SYNC/BHM_9815.CR3",
    "L1006344": "/Volumes/BONUSBOY/Ciara+Logan.Wedding.Bismarck.October.3rd.2026/raw/M262/L1006344.DNG",
    "DSC01746": "/Volumes/BONUSBOY/Ciara+Logan.Wedding.Bismarck.October.3rd.2026/raw/ILCE-7SM3/DSC01746.ARW",
    "R0003064": "/Volumes/BONUSBOY/*Katherine & Blake Engagement - August 2026/griii/R0003064.DNG",
}
cc.SLIDER_BAND["Contrast2012"] = "mid"


def steps():
    out = [("base", "base", 0, {})]
    for s, vals in (("Exposure2012", (-3, -2, -1, 1, 2, 3)), ("Whites2012", (-100, -50, 50, 100)), ("Contrast2012", (-100, -50, 50, 100))):
        for v in vals:
            out.append((f"{s}_{v:+d}", s, v, {s: f"{v:+d}"}))
    return out


def names():
    return [(f"{k}__{n}", k, n, sl, v, st) for k in RAWS for n, sl, v, st in steps()]


def prep():
    for d in ("src", "v", "acr"):
        os.makedirs(f"{WORK}/{d}", exist_ok=True)
    man = []
    for k, path in RAWS.items():
        ext = os.path.splitext(path)[1]
        src = f"{WORK}/src/{k}{ext}"
        if not os.path.isfile(src):
            if subprocess.run(["/bin/cp", "-c", path, src], capture_output=True).returncode != 0:
                shutil.copyfile(path, src)
    for n, k, step, sl, v, st in names():
        ext = os.path.splitext(RAWS[k])[1]
        dst = f"{WORK}/v/{n}{ext}"
        if not os.path.isfile(dst):
            subprocess.run(["/bin/cp", "-c", f"{WORK}/src/{k}{ext}", dst], check=True)
        s = dict(dng.BASE)
        s.update(st)
        open(f"{WORK}/v/{n}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        acr = f"{WORK}/acr/{n}.tif"
        bank = f"{BANKDIR}/{step}.tif"
        if not os.path.isfile(acr) and k == "R0003064" and os.path.isfile(bank):
            shutil.copyfile(bank, acr)
        if not os.path.isfile(acr):
            man.append(f"{dst}\t{acr}\tmin")
    open(f"{WORK}/manifest.txt", "w").write("\n".join(man) + "\n")
    print(f"{len(man)} Camera Raw renders to make")


def cr():
    r = subprocess.run([f"{HERE}/hdr_ps.sh", WORK], env=dict(os.environ, PS_WHO="tone2_study"))
    sys.exit(r.returncode)


def lc_run(name, binp):
    subprocess.run([f"{HERE}/lcrun.sh", WORK, f"lc_{name}"], env=dict(os.environ, ORACLE_BIN=binp, J="3", LIGHTCRAFT_GPU=os.environ.get("LIGHTCRAFT_GPU", "0")), check=True)


def lc_load(name, n):
    b = np.fromfile(f"{WORK}/lc_{name}/{n}.f32", "<f4")
    w, h = b[:2].view("<u4")
    return b[2:].reshape(h, w, 3).astype(np.float64)


def cmp_(build):
    res = {}
    for k in RAWS:
        meta = {f"{k}__{n}": dict(photo=k, slider=sl, value=v, settings=st, step=n) for n, sl, v, st in steps()}
        cc.Pair.rot = None
        r, _ = cc.measure(meta, lambda n, b=build: cc.Pair(cc.read_cr(WORK, n), lc_load(b, n)))
        res[k] = r
    json.dump(res, open(f"{WORK}/res_{build}.json", "w"), indent=1)
    return res


def table(builds):
    R = {b: json.load(open(f"{WORK}/res_{b}.json")) for b in builds}
    rows = ["| raw | step | " + " | ".join(f"{b} dE / p95 / clip LC|CR / acts" for b in builds) + " |", "|---|---|" + "---|" * len(builds)]
    for k in RAWS:
        for n, sl, v, st in steps():
            cells = []
            for b in builds:
                r = R[b][k][n]
                e = r.get("effect")
                al = (f"{e['acts_like']:+.2f}" if abs(v) < 10 else f"{e['acts_like']:+.0f}") if e and "acts_like" in e else ""
                cells.append(f"{r['dE_mean']:.2f} / {r['dE_p95']:.1f} / {r['clip_pct']['lc']:.1f}|{r['clip_pct']['cr']:.1f} / {al}")
            rows.append(f"| {k} | {n} | " + " | ".join(cells) + " |")
    return "\n".join(rows)


def slider_means(builds):
    R = {b: json.load(open(f"{WORK}/res_{b}.json")) for b in builds}
    print("mean dE by raw x slider (+base):")
    for k in RAWS:
        for sl in ("base", "Exposure2012", "Whites2012", "Contrast2012"):
            vals = []
            for b in builds:
                v = [r["dE_mean"] for r in R[b][k].values() if r["slider"] == sl]
                vals.append(sum(v) / len(v))
            print(f"  {k:9s} {sl:13s} " + "  ".join(f"{b}={x:.3f}" for b, x in zip(builds, vals)))


def strips(builds):
    os.makedirs(OUT, exist_ok=True)
    pick = ["Exposure2012_+3", "Whites2012_+100", "Contrast2012_+100", "Contrast2012_-100"]
    tw = 360
    for k in RAWS:
        rows = []
        for b in builds + ["CR"]:
            cells = []
            for s in pick:
                n = f"{k}__{s}"
                cc.Pair.rot = None
                p = cc.Pair(cc.read_cr(WORK, n), lc_load(builds[0], n))
                img = p.a if b == "CR" else cc.Pair(cc.read_cr(WORK, n), lc_load(b, n)).b
                th = int(round(tw * img.shape[0] / img.shape[1]))
                im = np.asarray(Image.fromarray(cc.srgb8(img)).resize((tw, th), Image.BOX))
                cells.append(cc.label(im, f"{b} {s.replace('2012', '')}"))
            rows.append(np.concatenate(cells, 1))
        w = max(r.shape[1] for r in rows)
        Image.fromarray(np.concatenate(rows, 0)).save(f"{OUT}/{k}.jpg", quality=88)
    print("strips in", OUT)


if __name__ == "__main__":
    a = sys.argv[1:]
    if a[0] == "prep":
        prep()
    elif a[0] == "cr":
        cr()
    elif a[0] == "lc":
        lc_run(a[1], a[2])
    elif a[0] == "cmp":
        for b in a[1:]:
            cmp_(b)
        slider_means(a[1:])
        open(f"{WORK}/table.md", "w").write(table(a[1:]) + "\n")
    elif a[0] == "strips":
        strips(a[1:])
