#!/usr/bin/env python3
"""dehaze3_study.py: Dehaze at the same slider value in LightCraft and Camera Raw (2026-11-27, Round 15). The four study
raws' Camera Raw renders come from the reference bank (`~/Library/Application Support/LightCraft/reference/<model>/`: base and
Dehaze -100 / -50 / -25 / +25 / +50 / +100); the eight held-out raws' base / Dehaze -100 / -50 from the 2026-11-25 set
(`/private/tmp/lightcraft-scene2/acr`, `S2_ACR`). No Photoshop needed.

  dehaze3_study.py prep              SET (= $DZ3_SET, /private/tmp/lightcraft-dz3): raw copies, clones + sidecars, acr/ links, meta.json
  dehaze3_study.py lc NAME BIN       render every item with the oracle_render BIN into SET/lc_NAME/ (CPU unless LIGHTCRAFT_GPU=1)
  dehaze3_study.py cmp NAME...       per raw / step: dE mean / p95, highlight-detail ratio, acts like; per-slider-value means
  dehaze3_study.py tab NAME...         the same table from the saved res_NAME.json (no re-measuring)
  dehaze3_study.py strips NAME... OUT   LightCraft rows over Camera Raw strips per raw
Metrics are scene_ana.py's (the 2026-11-25 / 11-27 studies' definitions)."""
import json, os, subprocess, sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import scene_ana as sa  # noqa: E402
import camera_check as cc  # noqa: E402

SET = os.environ.get("DZ3_SET", "/private/tmp/lightcraft-dz3")
S2 = os.environ.get("S2_ACR", "/private/tmp/lightcraft-scene2/acr")
BANK = os.path.expanduser("~/Library/Application Support/LightCraft/reference")
STUDY = {"BHM_9815": "Canon_Canon_EOS_R6", "L1006344": "Leica_Camera_AG_LEICA_M_Typ_262", "DSC01746": "SONY_ILCE_7SM3", "R0003064": "RICOH_IMAGING_COMPANY_LTD_RICOH_GR_III"}
G = "/Volumes/BONUSBOY/*Katherine & Blake Engagement - August 2026/griii/"
M = "/Volumes/BONUSBOY/Ciara+Logan.Wedding.Bismarck.October.3rd.2026/raw/M262/"
SY = "/Users/bryanhempstead/Pictures/Lightroom/SYNC/"
HELD = {"R0003180": G + "R0003180.DNG", "L1004503": M + "L1004503.DNG", "BHM_8480": SY + "BHM_8480.CR3", "BHM_4857": SY + "BHM_4857.CR3",
        "BHM_4860": SY + "BHM_4860.CR3", "L1004631": M + "L1004631.DNG", "R0003037": G + "R0003037.DNG", "R0003369": G + "R0003369.DNG"}
STUDY_V = [-100, -50, -25, 25, 50, 100]
HELD_V = [-100, -50]
sys.path.insert(0, HERE)


def items():
    out = []
    for k in list(STUDY) + list(HELD):
        for v in (STUDY_V if k in STUDY else HELD_V):
            out.append((k, f"{k}__Dehaze_{v:+d}", v))
        out.append((k, f"{k}__base", 0))
    return out


def prep():
    import dng
    for d in ("src", "v", "acr"):
        os.makedirs(f"{SET}/{d}", exist_ok=True)
    paths = {k: json.load(open(f"{BANK}/{m}/index.json"))["file"] for k, m in STUDY.items()}
    paths.update(HELD)
    meta = {}
    for k, n, v in items():
        path = paths[k]
        ext = os.path.splitext(path)[1]
        src = f"{SET}/src/{k}{ext}"
        if not os.path.exists(src):
            if subprocess.run(["/bin/cp", "-c", path, src], capture_output=True).returncode != 0:
                subprocess.run(["/bin/cp", path, src], check=True)
        dst = f"{SET}/v/{n}{ext}"
        if not os.path.exists(dst):
            subprocess.run(["/bin/cp", "-c", src, dst], check=True)
        s = dict(dng.BASE)
        if v:
            s["Dehaze"] = f"{v:+d}"
        open(f"{SET}/v/{n}.xmp", "w").write(dng.xmp_packet(s, curves=dng.LINEAR))
        step = n.split("__", 1)[1]
        srcacr = f"{BANK}/{STUDY[k]}/{step}.tif" if k in STUDY else f"{S2}/{n}.tif"
        link = f"{SET}/acr/{n}.tif"
        if os.path.lexists(link):
            os.remove(link)
        os.symlink(srcacr, link)
        meta[n] = {"photo": k, "settings": {"Dehaze": f"{v:+d}"} if v else {}}
    json.dump({"camera": "", "items": meta}, open(f"{SET}/meta.json", "w"))
    print(len(meta), "items")


def lc_run(name, binp, flt="."):
    subprocess.run([f"{HERE}/lcrun.sh", SET, f"lc_{name}", flt], env=dict(os.environ, ORACLE_BIN=binp, J=os.environ.get("J", "3"), LIGHTCRAFT_GPU=os.environ.get("LIGHTCRAFT_GPU", "0")), check=True)


def cmp_(build):
    rows = sa.run(SET, f"lc_{build}", "", quiet=True)
    al = sa.acts_like(rows)
    res = {}
    for n, m in rows.items():
        p, k, v = sa.parse(n)
        a = al.get((p, k, v))
        res[n] = dict(photo=p, v=v, dE=m["dE"], p95=m["p95"], hd=m["hi_detail"]["lc"] / max(m["hi_detail"]["cr"], 1e-9),
                      eff_lc=m["eff_lc"], eff_cr=m["eff_cr"], acts=(a[3] if a else None), cdE=m["colour_dE"], ab=m["mean_ab"])
    json.dump(res, open(f"{SET}/res_{build}.json", "w"), indent=1)
    return res


def fmt_acts(a):
    return "n/a" if a is None else f"{a:+.0f}"


def table(builds):
    R = {b: json.load(open(f"{SET}/res_{b}.json")) for b in builds}
    ns = sorted([n for n in R[builds[0]] if all(n in R[b] for b in builds)], key=lambda n: (list(STUDY) + list(HELD)).index(sa.parse(n)[0]) * 1000 + sa.parse(n)[2])
    print("raw        value | per build: dE / p95 / hiDet(LC/CR) / acts-like")
    for n in ns:
        p, k, v = sa.parse(n)
        print(f"{p:9s} {v:+4d} | " + " || ".join(f"{R[b][n]['dE']:5.2f} {R[b][n]['p95']:5.1f} {R[b][n]['hd']:4.2f} {fmt_acts(R[b][n]['acts']):>5s}" for b in builds))
    print("\nmean over raws (study 4 | held-out 8): dE per value")
    for grp, ks in (("study", list(STUDY)), ("held", list(HELD))):
        vals = sorted({sa.parse(n)[2] for n in ns if sa.parse(n)[0] in ks})
        for v in vals:
            sel = [n for n in ns if sa.parse(n)[0] in ks and sa.parse(n)[2] == v]
            print(f"  {grp:6s} {v:+4d} " + "  ".join(f"{b}={np.mean([R[b][n]['dE'] for n in sel]):.2f}/hd{np.mean([R[b][n]['hd'] for n in sel]):.2f}" for b in builds))
        sel = [n for n in ns if sa.parse(n)[0] in ks]
        print(f"  {grp:6s} all  " + "  ".join(f"{b}={np.mean([R[b][n]['dE'] for n in sel]):.2f}" for b in builds))
        neg = [n for n in sel if sa.parse(n)[2] < 0]
        pos = [n for n in sel if sa.parse(n)[2] > 0]
        print(f"  {grp:6s} neg  " + "  ".join(f"{b}={np.mean([R[b][n]['dE'] for n in neg]):.2f}" for b in builds))
        if pos:
            print(f"  {grp:6s} pos  " + "  ".join(f"{b}={np.mean([R[b][n]['dE'] for n in pos]):.2f}" for b in builds))


def lc_load(b, n):
    return sa.lc_load(SET, f"lc_{b}", n)


def strips(builds, out):
    os.makedirs(out, exist_ok=True)
    tw = 330
    for k in list(STUDY) + list(HELD):
        vs = STUDY_V if k in STUDY else HELD_V
        pick = [f"Dehaze_{v:+d}" for v in ([-100, -50, 50, 100] if k in STUDY else [-100, -50])]
        pick = [s for s in pick if int(s.split("_")[1]) in vs]
        if not all(os.path.exists(f"{SET}/lc_{b}/{k}__{s}.f32") for b in builds for s in pick):
            continue
        rows = []
        for b in builds + ["CR"]:
            cells = []
            for s in pick:
                n = f"{k}__{s}"
                cc.Pair.rot = None
                cr = cc.read_cr(SET, n)
                p = cc.Pair(cr, lc_load(builds[0], n))
                img = p.a if b == "CR" else cc.Pair(cr, lc_load(b, n)).b
                th = int(round(tw * img.shape[0] / img.shape[1]))
                im = np.asarray(Image.fromarray(cc.srgb8(img)).resize((tw, th), Image.BOX))
                cells.append(cc.label(im, f"{b} {s}"))
            rows.append(np.concatenate(cells, 1))
        Image.fromarray(np.concatenate(rows, 0)).save(f"{out}/{k}__Dehaze.jpg", quality=88)
    print("strips in", out)


if __name__ == "__main__":
    a = sys.argv[1:]
    if a[0] == "prep":
        prep()
    elif a[0] == "lc":
        lc_run(a[1], a[2], a[3] if len(a) > 3 else ".")
    elif a[0] == "cmp":
        for b in a[1:]:
            cmp_(b)
        table(a[1:])
    elif a[0] == "tab":  # print the table from saved res_*.json without re-measuring
        table(a[1:])
    elif a[0] == "strips":
        strips(a[1:-1], a[-1])
