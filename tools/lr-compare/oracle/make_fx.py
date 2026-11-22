"""make_fx.py SET WHAT[,WHAT…]: Round 10 experiments for the Camera Raw oracle (docs/lr-match.md → Round 10).

WHAT:
- `fx`    (R21) Texture / Clarity / Dehaze sweeps on the frequency scene (1,024 and 3,072 px), the colour
          grid and the spatial scene;
- `grade` (R20) Color Grading wheels, blending and balance on the colour grid;
- `bw`    (R22) B&W: the grey mixer and the Adobe Monochrome profile on the grid;
- `pc`    (R19) Point Color: two XMP encodings and a sweep of sampled hue / shift / range on the grid;
- `look`  (R04) the Nautica look (Bryan's imported Archipelago profile) on the grid;
- `mask`  (R16) local sliders in a linear-gradient mask covering the frame (and a half-frame
          gradient / a radial) on the spatial and frequency scenes.

The frequency scene (`freq{S}`): horizontal bands, each a sine grating in log2 luminance along x
around a base level: periods 3…233 px × S at 0.4 EV around −3.5 EV, then amplitude (0.1 / 1.2 EV)
and level (−6.5 / −1.2 EV) variants at periods 8 and 34 px × S. `fx_ana.py` fits each band's gain.
Writes SET/dng/*.dng, SET/meta.json and the manifest for ps.sh."""
import sys, os, json, re, math
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng, scenes

SET = sys.argv[1]
WHAT = sys.argv[2].split(",") if len(sys.argv) > 2 else ["fx"]
os.makedirs(SET + "/dng", exist_ok=True)
os.makedirs(SET + "/acr", exist_ok=True)
CAM = os.environ.get("CAM", "Canon EOS R6")
info = dng.dcp(CAM, [0.5, 1.0, 0.65])
SETTINGS_DIR = os.path.expanduser("~/Library/Application Support/Adobe/CameraRaw/ImportedSettings")
ADOBE_RAW = "/Library/Application Support/Adobe/CameraRaw/Settings/Adobe/Profiles/Adobe Raw"

PERIODS = [3, 5, 8, 13, 21, 34, 55, 89, 144, 233]
# (period, amplitude EV, base EV)
VARIANTS = [(8, 0.1, -3.5), (8, 1.2, -3.5), (34, 0.1, -3.5), (34, 1.2, -3.5), (8, 0.4, -6.5), (8, 0.4, -1.2), (34, 0.4, -6.5), (34, 0.4, -1.2)]
BAND = 40


def freq_bands(S):
    return [(p * S, 0.4, -3.5) for p in PERIODS] + [(p * S, a, b) for p, a, b in VARIANTS]


def freq(S):
    w, bands = 1024 * S, freq_bands(S)
    img = np.zeros((BAND * S * len(bands), w, 3))
    x = np.arange(w)
    for i, (p, a, b) in enumerate(bands):
        img[i * BAND * S:(i + 1) * BAND * S] = (2.0 ** (b + a * np.sin(2 * np.pi * x / p)))[None, :, None]
    return img


def look_from_file(path, name, amount=1.0):
    """a Look block carrying every rendering parameter of an installed / imported look profile"""
    t = open(path).read()
    attrs = dict(re.findall(r'crs:([A-Za-z_0-9]+)="([^"]*)"', t))
    uuid = attrs.get("UUID", "")
    meta = {"PresetType", "Cluster", "UUID", "CameraModelRestriction", "Copyright", "ContactInfo"}
    params = " ".join(f'crs:{k}="{v}"' for k, v in attrs.items() if k not in meta and not k.startswith("Supports"))
    sup = " ".join(f'crs:{k}="{v}"' for k, v in attrs.items() if k.startswith("Supports"))
    return (f'<crs:Look><rdf:Description crs:Name="{name}" crs:Amount="{amount}" crs:UUID="{uuid}" {sup}>'
            f'<crs:Parameters><rdf:Description {params}/></crs:Parameters></rdf:Description></crs:Look>')


def point_struct(src_h, src_s, src_l, hue=0.0, sat=0.0, lum=0.0, rng=0.5, ranges=None):
    def r(k, v):
        lo_n, lo_f, up_f, up_n = v
        return (f'<crs:{k} rdf:parseType="Resource"><crs:LowerNone>{lo_n}</crs:LowerNone><crs:LowerFull>{lo_f}</crs:LowerFull>'
                f'<crs:UpperFull>{up_f}</crs:UpperFull><crs:UpperNone>{up_n}</crs:UpperNone></crs:{k}>')
    rr = "".join(r(k, v) for k, v in (ranges or {}).items())
    return (f'<rdf:li rdf:parseType="Resource"><crs:SrcHue>{src_h}</crs:SrcHue><crs:SrcSat>{src_s}</crs:SrcSat><crs:SrcLum>{src_l}</crs:SrcLum>'
            f'<crs:HueShift>{hue}</crs:HueShift><crs:SatScale>{sat}</crs:SatScale><crs:LumScale>{lum}</crs:LumScale>'
            f'<crs:RangeAmount>{rng}</crs:RangeAmount>{rr}</rdf:li>')


def point_string(src_h, src_s, src_l, hue=0.0, sat=0.0, lum=0.0, rng=0.5, ranges=None):
    """Camera Raw's XMP form: 19 numbers (sample as linear-ProPhoto HSV, hue in sixths of a turn;
    shifts −1..1; Range 0..1; hue / sat / lum ranges as outer, inner, inner, outer points). The
    defaults are Lightroom's for a fresh sample (RAWmakase `PointColor::sampled`); -1 ranges are
    Lightroom's empty selection, which Camera Raw drops."""
    around = lambda c: [min(max(v, 0.0), 1.0) for v in (c - 0.73, c - 0.18, c + 0.18, c + 0.73)]
    enc = lambda v: 12.92 * v if v <= 0.0031308 else 1.055 * v ** (1 / 2.4) - 0.055
    r = {"HueRange": [0.0, 1 / 3, 2 / 3, 1.0], "SatRange": around(src_s), "LumRange": around(enc(src_l))}
    r.update(ranges or {})
    vals = [src_h, src_s, src_l, hue, sat, lum, rng]
    for k in ("HueRange", "SatRange", "LumRange"):
        vals += list(r[k])
    return "<rdf:li>" + ", ".join(f"{v:.6f}" for v in vals) + "</rdf:li>"


def points_xml(lis, variances=None):
    v = variances if variances is not None else [0.0] * len(lis)
    return ("<crs:PointColors><rdf:Seq>" + "".join(lis) + "</rdf:Seq></crs:PointColors><crs:ColorVariance><rdf:Seq>"
            + "".join(f"<rdf:li>{x:.6f}</rdf:li>" for x in v) + "</rdf:Seq></crs:ColorVariance>")


def mask_xml(locals_, comp):
    a = " ".join(f'crs:{k}="{v}"' for k, v in locals_.items())
    return ('<crs:MaskGroupBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" '
            f'crs:CorrectionActive="true" crs:CorrectionName="m" crs:CorrectionSyncID="A1" {a}><crs:CorrectionMasks><rdf:Seq>{comp}'
            '</rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li></rdf:Seq></crs:MaskGroupBasedCorrections>')


GRAD_ALL = '<rdf:li crs:What="Mask/Gradient" crs:MaskActive="true" crs:MaskName="g" crs:MaskBlendMode="0" crs:MaskInverted="false" crs:MaskValue="1" crs:ZeroX="-2" crs:ZeroY="0.5" crs:FullX="-1" crs:FullY="0.5"/>'
GRAD_HALF = '<rdf:li crs:What="Mask/Gradient" crs:MaskActive="true" crs:MaskName="g" crs:MaskBlendMode="0" crs:MaskInverted="false" crs:MaskValue="1" crs:ZeroX="0.35" crs:ZeroY="0.5" crs:FullX="0.65" crs:FullY="0.5"/>'
RADIAL = ('<rdf:li crs:What="Mask/CircularGradient" crs:MaskActive="true" crs:MaskName="r" crs:MaskBlendMode="0" crs:MaskInverted="false" crs:MaskValue="1" '
          'crs:Top="0.2" crs:Left="0.25" crs:Bottom="0.8" crs:Right="0.75" crs:Angle="0" crs:Midpoint="50" crs:Roundness="0" crs:Feather="50" crs:Flipped="false" crs:Version="2"/>')

def grating(p, a=0.4, b=-3.5, w=1024, h=683):
    """one sine grating filling the frame (no neighbouring bands for a wide filter to average in)"""
    x = np.arange(w)
    return np.repeat((2.0 ** (b + a * np.sin(2 * np.pi * x / p)))[None, :, None], h, 0).repeat(3, 2)


GPERIODS = [3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377]
# extra gratings (period, amplitude, base) at 1,024 px: amplitude and level of the detail
GEXTRA = [(13, 0.1, -3.5), (13, 1.2, -3.5), (89, 0.1, -3.5), (89, 1.2, -3.5), (13, 0.4, -6.5), (13, 0.4, -1.2), (89, 0.4, -6.5), (89, 0.4, -1.2),
          (13, 2.5, -3.5), (89, 2.5, -3.5)]
G3 = [9, 24, 63, 165, 432, 1131]

jobs = []  # (scene name, image, [(setting name, settings, extra xml)])
if "fx2" in WHAT:
    sw = [("base", {}, "")] + [(f"{k[:3]}{v:+d}", {k: f"{v:+d}"}, "") for k in ("Texture", "Clarity2012") for v in (-50, 50, 100)]
    sw += [(f"Deh{v:+d}", {"Dehaze": f"{v:+d}"}, "") for v in (-50, 30, 100)]
    for p in GPERIODS:
        jobs.append((f"g{p}", grating(p), sw))
    for p, a, b in GEXTRA:
        jobs.append((f"g{p}a{a}b{b}", grating(p, a, b), sw))
    for p in G3:
        jobs.append((f"G{p}", grating(p, w=3072, h=2048), [x for x in sw if x[0] in ("base", "Tex+100", "Cla+50", "Cla-50")]))
if "fx" in WHAT:
    sw = [("base", {}, "")]
    for k in ("Texture", "Clarity2012"):
        for v in (-100, -50, -25, 25, 50, 100):
            sw.append((f"{k[:3]}{v:+d}", {k: f"{v:+d}"}, ""))
    for v in (-50, -25, 15, 30, 60, 100):
        sw.append((f"Deh{v:+d}", {"Dehaze": f"{v:+d}"}, ""))
    jobs.append(("freq1", freq(1), sw))
    jobs.append(("freq3", freq(3), [x for x in sw if x[0] in ("base", "Tex+50", "Tex+100", "Tex-50", "Cla+50", "Cla+100", "Cla-50")]))
    jobs.append(("grid", scenes.grid()[0], sw))
    jobs.append(("spatial", scenes.spatial(), sw))
if "grade" in WHAT:
    sw = [("base", {}, "")]
    wheels = {"sh": ("SplitToningShadowHue", "SplitToningShadowSaturation", "ColorGradeShadowLum"),
              "mi": ("ColorGradeMidtoneHue", "ColorGradeMidtoneSat", "ColorGradeMidtoneLum"),
              "hi": ("SplitToningHighlightHue", "SplitToningHighlightSaturation", "ColorGradeHighlightLum"),
              "gl": ("ColorGradeGlobalHue", "ColorGradeGlobalSat", "ColorGradeGlobalLum")}
    for w, (h, s, l) in wheels.items():
        for hue in (30, 150, 270):
            sw.append((f"{w}h{hue}s50", {h: str(hue), s: "50"}, ""))
        sw.append((f"{w}h210s100", {h: "210", s: "100"}, ""))
        sw.append((f"{w}h210s20", {h: "210", s: "20"}, ""))
        sw.append((f"{w}l+50", {l: "+50"}, ""))
        sw.append((f"{w}l-50", {l: "-50"}, ""))
    split = {"SplitToningShadowHue": "30", "SplitToningShadowSaturation": "50", "SplitToningHighlightHue": "210", "SplitToningHighlightSaturation": "50"}
    for b in (0, 50, 100):
        sw.append((f"split_b{b}", dict(split, ColorGradeBlending=str(b)), ""))
    for bal in (-50, 50):
        sw.append((f"split_bal{bal:+d}", dict(split, SplitToningBalance=f"{bal:+d}"), ""))
    jobs.append(("grid", scenes.grid()[0], sw))
if "bw" in WHAT:
    mono = look_from_file(f"{ADOBE_RAW}/Adobe Monochrome.xmp", "Adobe Monochrome")
    sw = [("bw", {"ConvertToGrayscale": "True"}, ""), ("mono", {"ConvertToGrayscale": "True"}, mono)]
    for b in ("Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"):
        for v in (-60, 60):
            sw.append((f"bw{b}{v:+d}", {"ConvertToGrayscale": "True", f"GrayMixer{b}": f"{v:+d}"}, ""))
    sw.append(("monoBlue-40", {"ConvertToGrayscale": "True", "GrayMixerBlue": "-40"}, mono))
    jobs.append(("grid", scenes.grid()[0], sw))
if "pc" in WHAT:
    sw = [("base", {}, "")]
    # encoding probe: desaturate a red sample fully, struct vs comma string
    sw.append(("pcS_h0", {}, points_xml([point_struct(0.0, 0.6, 0.5, sat=-1.0)])))
    sw.append(("pcT_h0", {}, points_xml([point_string(0.0, 0.6, 0.5, sat=-1.0)])))
    for h in (0.5, 1.5, 2.5, 3.5, 4.5, 5.5):
        sw.append((f"pcT_h{h}", {}, points_xml([point_string(h, 0.6, 0.5, sat=-1.0)])))
        sw.append((f"pcS_h{h}", {}, points_xml([point_struct(h, 0.6, 0.5, sat=-1.0)])))
    for k, kw in (("hue", dict(hue=0.5)), ("lum", dict(lum=0.5)), ("lumneg", dict(lum=-0.5)), ("satpos", dict(sat=0.5))):
        sw.append((f"pcT_{k}", {}, points_xml([point_string(1.0, 0.6, 0.5, **kw)])))
        sw.append((f"pcS_{k}", {}, points_xml([point_struct(1.0, 0.6, 0.5, **kw)])))
    for r in (0.1, 1.0):
        sw.append((f"pcT_rng{r}", {}, points_xml([point_string(1.0, 0.6, 0.5, sat=-1.0, rng=r)])))
        sw.append((f"pcS_rng{r}", {}, points_xml([point_struct(1.0, 0.6, 0.5, sat=-1.0, rng=r)])))
    for s, l in ((0.3, 0.3), (0.9, 0.6)):
        sw.append((f"pcT_s{s}l{l}", {}, points_xml([point_string(1.0, s, l, sat=-1.0)])))
        sw.append((f"pcS_s{s}l{l}", {}, points_xml([point_struct(1.0, s, l, sat=-1.0)])))
    jobs.append(("grid", scenes.grid()[0], sw))
if "pc2" in WHAT:
    # valid swatches: hue (sixths of a turn) × shift kinds, range, variance, sample saturation / value
    sw = [("base", {}, "")]
    for h in (0.0, 1.0, 2.0, 3.0, 4.0, 5.0):
        sw.append((f"pc_h{h:g}_s-1", {}, points_xml([point_string(h, 0.6, 0.25, sat=-1.0)])))
    for k, kw in (("hue+1", dict(hue=1.0)), ("hue-1", dict(hue=-1.0)), ("lum+1", dict(lum=1.0)), ("lum-1", dict(lum=-1.0)), ("sat+1", dict(sat=1.0)),
                  ("lum+0.5", dict(lum=0.5)), ("sat-0.5", dict(sat=-0.5))):
        sw.append((f"pc_h1_{k}", {}, points_xml([point_string(1.0, 0.6, 0.25, **kw)])))
    for r in (0.0, 0.25, 1.0):
        sw.append((f"pc_h1_s-1_rng{r:g}", {}, points_xml([point_string(1.0, 0.6, 0.25, sat=-1.0, rng=r)])))
    for v in (-1.0, 1.0):
        sw.append((f"pc_h4_var{v:+g}", {}, points_xml([point_string(4.0, 0.6, 0.25)], [v])))
    for s_, l in ((0.3, 0.05), (0.9, 0.6)):
        sw.append((f"pc_h3.5_src{s_:g}_{l:g}", {}, points_xml([point_string(3.5, s_, l, sat=-1.0)])))
    sw.append(("pc_two", {}, points_xml([point_string(0.3, 0.6, 0.25, hue=0.5), point_string(4.0, 0.6, 0.25, lum=-0.5)])))
    sw.append(("pc_narrow", {}, points_xml([point_string(2.0, 0.6, 0.25, sat=-1.0, ranges={"HueRange": [0.3, 0.45, 0.55, 0.7]})])))
    jobs.append(("grid", scenes.grid()[0], sw))
if "mask2" in WHAT:
    # the local tone sliders' curves over the gradient, both signs and full strength (R16)
    sw = [("base", {}, "")]
    for k, lk in (("Con", "LocalContrast2012"), ("Whi", "LocalWhites2012"), ("Bla", "LocalBlacks2012"), ("Hig", "LocalHighlights2012"),
                  ("Sha", "LocalShadows2012")):
        for v in (-100, -50, 50, 100):
            sw.append((f"l{k}{v:+d}", {}, mask_xml({lk: f"{v / 100:.2f}"}, GRAD_ALL)))
    for v in (-2, 2):
        sw.append((f"lExp{v:+d}", {}, mask_xml({"LocalExposure2012": f"{v / 4:.2f}"}, GRAD_ALL)))
    jobs.append(("spatial", scenes.spatial(), sw))
    # the same curves on a darker and a brighter scene (is the pivot the image's?)
    for name, off in (("spatialD", -2.0), ("spatialB", 1.0)):
        img = scenes.spatial() * 2.0 ** off
        jobs.append((name, img, [x for x in sw if x[0] in ("base", "lCon+50", "lCon-50", "lWhi+50", "lBla+50", "lBla-50", "lWhi-50")]))
if "look" in WHAT:
    sw = [("base", {}, "")]
    for a in (1.0, 0.5):
        sw.append((f"nautica{a}", {}, look_from_file(f"{SETTINGS_DIR}/Nautica.xmp", "Nautica", a)))
    jobs.append(("grid", scenes.grid()[0], sw))
if "mask" in WHAT:
    sw = [("base", {}, "")]
    loc = [("lExp+1", "LocalExposure2012", "0.25", "Exposure2012", "+1.00"), ("lExp-1", "LocalExposure2012", "-0.25", "Exposure2012", "-1.00"),
           ("lCon+50", "LocalContrast2012", "0.5", "Contrast2012", "+50"), ("lCon-50", "LocalContrast2012", "-0.5", "Contrast2012", "-50"),
           ("lHig-60", "LocalHighlights2012", "-0.6", "Highlights2012", "-60"), ("lSha+60", "LocalShadows2012", "0.6", "Shadows2012", "+60"),
           ("lTex+40", "LocalTexture", "0.4", "Texture", "+40"), ("lCla+40", "LocalClarity2012", "0.4", "Clarity2012", "+40"),
           ("lDeh+30", "LocalDehaze", "0.3", "Dehaze", "+30"), ("lWhi+50", "LocalWhites2012", "0.5", "Whites2012", "+50"),
           ("lBla+50", "LocalBlacks2012", "0.5", "Blacks2012", "+50")]
    for n, lk, lv, gk, gv in loc:
        sw.append((n, {}, mask_xml({lk: lv}, GRAD_ALL)))
        sw.append(("g" + n[1:], {gk: gv}, ""))
    sw.append(("half_lExp+1", {}, mask_xml({"LocalExposure2012": "0.25"}, GRAD_HALF)))
    sw.append(("rad_lExp+1", {}, mask_xml({"LocalExposure2012": "0.25"}, RADIAL)))
    sw.append(("half_lSha+60", {}, mask_xml({"LocalShadows2012": "0.6"}, GRAD_HALF)))
    jobs.append(("spatial", scenes.spatial(), sw))
    jobs.append(("freq1", freq(1), [x for x in sw if x[0] in ("base", "lTex+40", "gTex+40", "lCla+40", "gCla+40", "lDeh+30", "gDeh+30", "lCon+50", "gCon+50")]))

man, meta = [], {}
for iname, img, sweep in jobs:
    cam = dng.scene_to_camera(img, info)
    for sname, st, extra in sweep:
        s = dict(dng.BASE)
        s.update(st)
        name = f"{iname}__{sname}"
        if name in meta:
            continue
        xmp = dng.xmp_packet(s, look=extra or None, curves=dng.LINEAR)
        dng.write_dng(f"{SET}/dng/{name}.dng", cam, CAM, info["spec"]["camera_white"], info, xmp)
        man.append(f"{os.path.abspath(SET)}/dng/{name}.dng\t{os.path.abspath(SET)}/acr/{name}.tif")
        meta[name] = {"image": iname, "settings": st, "extra": bool(extra)}
grid_img, cells, rects = scenes.grid()
gcam = dng.scene_to_camera(grid_img, info)
valid = [bool(((c := gcam[r[0]:r[2], r[1]:r[3]]) >= 0).all() and (c <= 1).all()) for r in rects]
old = json.load(open(f"{SET}/meta.json")) if os.path.exists(f"{SET}/meta.json") else {"items": {}}
old["items"].update(meta)
json.dump({"camera": CAM, "items": old["items"], "cells": cells.tolist(), "rects": rects, "valid": valid,
           "freq_bands": {str(S): freq_bands(S) for S in (1, 3)}, "band": BAND}, open(f"{SET}/meta.json", "w"))
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest.txt"), "w").write("\n".join(man) + "\n")
print(len(man), "files")
