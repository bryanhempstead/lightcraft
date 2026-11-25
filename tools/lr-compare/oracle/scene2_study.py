#!/usr/bin/env python3
"""scene2_study.py: negative Dehaze and Shadows + against Camera Raw at identical slider values (2026-11-25 study,
`make_scene.py` builds the set). Reuses camera_check.py's metrics (dE2000 mean / p95, clip, "acts like").

  scene2_study.py lc NAME BIN [filter]   render every item with the oracle_render BIN into SET/lc_NAME/ (CPU unless LIGHTCRAFT_GPU=1)
  scene2_study.py cmp NAME [NAME...]     per raw / step metrics for each build (json SET/res_NAME.json) + per-slider tables
  scene2_study.py strips NAME...         before / after / Camera Raw strips for the key steps into OUT (compare/.../fix-scene2/)
SET = /private/tmp/lightcraft-scene2 (or $S2_SET). Controls (Highlights -50, Dehaze +50, Shadows -50) must not move."""
import json, os, subprocess, sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import camera_check as cc  # noqa: E402

SET = os.environ.get("S2_SET", "/private/tmp/lightcraft-scene2")
OUT = os.path.expanduser("~/Library/Application Support/LightCraft/compare/2026-11-25/fix-scene2")
STUDY = ["BHM_9815", "L1006344", "DSC01746", "R0003064"]
HELD = ["R0003180", "L1004503", "BHM_8480", "BHM_4857", "BHM_4860", "L1004631", "R0003037", "R0003369"]
cc.SLIDER_BAND["Dehaze"] = "mid"


def steps(k):
    out = [("base", "base", 0, {})]
    sw = {"Shadows2012": [50, 100], "Dehaze": [-100, -50]} if k in HELD else {"Shadows2012": [-50, 25, 50, 100], "Dehaze": [-100, -50, -25, 50], "Highlights2012": [-50]}
    for s, vals in sw.items():
        for v in vals:
            out.append((f"{s}_{v:+d}", s, v, {s: f"{v:+d}"}))
    return out


def lc_run(name, binp, flt="."):
    subprocess.run([f"{HERE}/lcrun.sh", SET, f"lc_{name}", flt], env=dict(os.environ, ORACLE_BIN=binp, J="3", LIGHTCRAFT_GPU=os.environ.get("LIGHTCRAFT_GPU", "0")), check=True)


def lc_load(name, n):
    b = np.fromfile(f"{SET}/lc_{name}/{n}.f32", "<f4")
    w, h = b[:2].view("<u4")
    return b[2:].reshape(h, w, 3).astype(np.float64)


def cmp_(build):
    res = {}
    for k in STUDY + HELD:
        if not all(os.path.exists(f"{SET}/acr/{k}__{n}.tif") and os.path.exists(f"{SET}/lc_{build}/{k}__{n}.f32") for n, *_ in steps(k)):
            continue
        meta = {f"{k}__{n}": dict(photo=k, slider=sl, value=v, settings=st, step=n) for n, sl, v, st in steps(k)}
        cc.Pair.rot = None
        r, _ = cc.measure(meta, lambda n, b=build: cc.Pair(cc.read_cr(SET, n), lc_load(b, n)))
        res[k] = r
    json.dump(res, open(f"{SET}/res_{build}.json", "w"), indent=1)
    return res


def table(builds):
    R = {b: json.load(open(f"{SET}/res_{b}.json")) for b in builds}
    print("raw       step                dE mean / p95 / acts-like (LC effect | CR effect)  per build: " + ", ".join(builds))
    tot = {}
    for k in STUDY + HELD:
        if any(k not in R[b] for b in builds):
            continue
        for n, sl, v, st in steps(k):
            if n == "base":
                continue
            cells = []
            for b in builds:
                r = R[b][k][n]
                e = r.get("effect")
                al = f"{e['acts_like']:+.0f} ({e['lc']:+.1f}|{e['cr']:+.1f})" if e and "acts_like" in e else (f"({e['lc']:+.1f}|{e['cr']:+.1f})" if e else "")
                cells.append(f"{r['dE_mean']:5.2f} {r['dE_p95']:5.1f} {al}")
                tot.setdefault((b, sl, v), []).append(r["dE_mean"])
            print(f"{k:9s} {n:18s} " + "  ||  ".join(cells))
    print("\nmean dE per slider value over raws:")
    keys = sorted({(sl, v) for (_, sl, v) in tot})
    for sl, v in keys:
        print(f"  {sl:15s}{v:+4d}  " + "  ".join(f"{b}={np.mean(tot[(b, sl, v)]):.2f}" for b in builds))
    print("mean dE per slider:")
    for sl in sorted({s for s, _ in keys}):
        print(f"  {sl:15s}  " + "  ".join(f"{b}={np.mean([x for (bb, s, v), xs in tot.items() if bb == b and s == sl for x in xs]):.2f}" for b in builds))


def strips(builds):
    os.makedirs(OUT, exist_ok=True)
    pick = ["Shadows2012_+100", "Dehaze_-100", "Dehaze_-50", "Shadows2012_+50"]
    tw = 360
    for k in STUDY + HELD:
        if not all(os.path.exists(f"{SET}/lc_{b}/{k}__{s}.f32") and os.path.exists(f"{SET}/acr/{k}__{s}.tif") for b in builds for s in pick):
            continue
        rows = []
        for b in builds + ["CR"]:
            cells = []
            for s in pick:
                n = f"{k}__{s}"
                cc.Pair.rot = None
                p = cc.Pair(cc.read_cr(SET, n), lc_load(builds[0], n))
                img = p.a if b == "CR" else cc.Pair(cc.read_cr(SET, n), lc_load(b, n)).b
                th = int(round(tw * img.shape[0] / img.shape[1]))
                im = np.asarray(Image.fromarray(cc.srgb8(img)).resize((tw, th), Image.BOX))
                cells.append(cc.label(im, f"{b} {s.replace('2012', '')}"))
            rows.append(np.concatenate(cells, 1))
        Image.fromarray(np.concatenate(rows, 0)).save(f"{OUT}/{k}.jpg", quality=88)
    print("strips in", OUT)


if __name__ == "__main__":
    a = sys.argv[1:]
    if a[0] == "lc":
        lc_run(a[1], a[2], a[3] if len(a) > 3 else ".")
    elif a[0] == "cmp":
        for b in a[1:]:
            cmp_(b)
        table(a[1:])
    elif a[0] == "strips":
        strips(a[1:])
