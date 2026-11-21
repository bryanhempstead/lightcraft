#!/usr/bin/env python3
"""export_check: do LightCraft's exports carry the right colour, in every colour space and format?

Exports photos of a (scratch) library in sRGB, Display P3, Adobe RGB and ProPhoto RGB as 8-bit
JPEG, 8/16-bit PNG and 8/16-bit TIFF (plus a 32-bit float linear Rec.2020 TIFF as the reference:
the working-space render, no output gamut), then for every file:

  * reads the embedded ICC profile (own parser: version, class, PCS, colorants, white point,
    TRC) and checks it is the space asked for, D50 PCS, with the space's published primaries
    and curve;
  * converts the pixels to CIE XYZ (D50) through that profile, and the 8-bit files a second time
    through LittleCMS (Pillow ImageCms) and ColorSync (`sips --matchTo`) as independent CMMs;
  * compares them with the reference render (CIEDE2000) on the pixels that lie inside the
    file's gamut: a correct export differs only by quantisation there.

  export_check.py --library LIB --ids 1,2,3 --out DIR [--cli PATH] [--long-edge 900]

Reads only the library (exports with `app.export`); writes only DIR. Needs numpy + Pillow.
Exit status 1 when a check fails.
"""
import argparse, io, json, os, struct, subprocess, sys, zlib

import numpy as np
from PIL import Image, ImageCms

HERE = os.path.dirname(os.path.abspath(__file__))
CLI = os.environ.get("LIGHTCRAFT_CLI", os.path.join(HERE, "..", "..", "target", "release", "lightcraft-cli"))

D50 = np.array([0.9642, 1.0, 0.8249])


def xy_to_xyz(x, y):
    return np.array([x / y, 1.0, (1 - x - y) / y])


def rgb_to_xyz(prim, white):
    P = np.array([xy_to_xyz(*p) for p in prim]).T
    s = np.linalg.solve(P, xy_to_xyz(*white))
    return P * s


def bradford(src, dst):
    B = np.array([[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]])
    a, b = B @ src, B @ dst
    return np.linalg.inv(B) @ np.diag(b / a) @ B


D65xy, D50xy = (0.3127, 0.3290), (0.3457, 0.3585)
SPACES = {
    # published primaries, white, and curve: (kind, gamma)
    "srgb": ([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)], D65xy, ("srgb", None)),
    "displayP3": ([(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)], D65xy, ("srgb", None)),
    "adobeRgb": ([(0.64, 0.33), (0.21, 0.71), (0.15, 0.06)], D65xy, ("gamma", 563 / 256)),
    "proPhoto": ([(0.7347, 0.2653), (0.1596, 0.8404), (0.0366, 0.0001)], D50xy, ("gamma", 1.8)),
    "rec2020": ([(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)], D65xy, ("rec709", None)),
}


def to_d50_matrix(space):
    prim, white, _ = SPACES[space]
    m = rgb_to_xyz(prim, white)
    return bradford(xy_to_xyz(*white), xy_to_xyz(*D50xy)) @ m


# ------------------------------------------------------------------ ICC parsing
def s15(b):
    return struct.unpack(">i", b)[0] / 65536.0


def parse_icc(data):
    hdr = {
        "size": struct.unpack(">I", data[0:4])[0],
        "version": f"{data[8]}.{data[9] >> 4}.{data[9] & 15}",
        "class": data[12:16].decode("latin1"),
        "colour": data[16:20].decode("latin1"),
        "pcs": data[20:24].decode("latin1"),
        "intent": struct.unpack(">I", data[64:68])[0],
        "illuminant": [s15(data[68 + 4 * i:72 + 4 * i]) for i in range(3)],
    }
    n = struct.unpack(">I", data[128:132])[0]
    tags = {}
    for t in range(n):
        sig, off, ln = struct.unpack(">4sII", data[132 + 12 * t:144 + 12 * t])
        tags[sig.decode("latin1")] = data[off:off + ln]
    out = {"header": hdr, "tags": sorted(tags)}

    def xyz(tag):
        b = tags.get(tag)
        return None if b is None or b[:4] != b"XYZ " else np.array([s15(b[8 + 4 * i:12 + 4 * i]) for i in range(3)])

    out["wtpt"] = xyz("wtpt")
    cols = [xyz(t) for t in ("rXYZ", "gXYZ", "bXYZ")]
    out["matrix"] = None if any(c is None for c in cols) else np.array(cols).T

    def curve(tag):
        b = tags.get(tag)
        if b is None:
            return None
        kind = b[:4]
        if kind == b"curv":
            cnt = struct.unpack(">I", b[8:12])[0]
            if cnt == 0:
                return ("gamma", 1.0)
            if cnt == 1:
                return ("gamma", struct.unpack(">H", b[12:14])[0] / 256.0)
            return ("table", np.array(struct.unpack(f">{cnt}H", b[12:12 + 2 * cnt]), float) / 65535.0)
        if kind == b"para":
            ft = struct.unpack(">H", b[8:10])[0]
            k = {0: 1, 1: 3, 2: 4, 3: 5, 4: 7}[ft]
            return ("para", [s15(b[12 + 4 * i:16 + 4 * i]) for i in range(k)])
        return ("?", kind)

    out["trc"] = [curve(t) for t in ("rTRC", "gTRC", "bTRC")]
    for tag in ("desc",):
        b = tags.get(tag)
        if b is not None:
            if b[:4] == b"desc":
                ln = struct.unpack(">I", b[8:12])[0]
                out["desc"] = b[12:12 + ln].rstrip(b"\0").decode("latin1")
            elif b[:4] == b"mluc":
                cnt, _ = struct.unpack(">II", b[8:16])
                if cnt:
                    _, _, ln, off = struct.unpack(">2s2sII", b[16:28])
                    out["desc"] = b[off:off + ln].decode("utf-16-be")
    return out


def eval_curve(c, v):
    kind, p = c
    if kind == "gamma":
        return np.power(v, p)
    if kind == "table":
        return np.interp(v, np.linspace(0, 1, len(p)), p)
    if kind == "para":
        if len(p) == 1:
            return np.power(v, p[0])
        if len(p) == 5:
            g, a, b, cc, d = p
            return np.where(v >= d, np.power(np.maximum(a * v + b, 0), g), cc * v)
        if len(p) == 7:
            g, a, b, cc, d, e, f = p
            return np.where(v >= d, np.power(np.maximum(a * v + b, 0), g) + e, cc * v + f)
        if len(p) == 3:
            g, a, b = p
            return np.where(v >= -b / a, np.power(np.maximum(a * v + b, 0), g), 0)
        if len(p) == 4:
            g, a, b, cc = p
            return np.where(v >= -b / a, np.power(np.maximum(a * v + b, 0), g) + cc, cc)
    raise ValueError(f"curve {kind}")


def published_curve(space, v):
    kind, g = SPACES[space][2]
    if kind == "srgb":
        return np.where(v <= 0.04045, v / 12.92, ((v + 0.055) / 1.055) ** 2.4)
    if kind == "gamma":
        return v ** g
    return np.where(v < 0.081, v / 4.5, ((v + 0.099) / 1.099) ** (1 / 0.45))


# ------------------------------------------------------------------ pixel decoding
def read_png16(path):
    """(H, W, 3) float 0..1 of an 8/16-bit RGB PNG, plus its iCCP profile."""
    data = open(path, "rb").read()
    pos, idat, icc, ihdr = 8, b"", None, None
    while pos < len(data):
        ln, typ = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + ln]
        if typ == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif typ == b"IDAT":
            idat += body
        elif typ == b"iCCP":
            nul = body.index(b"\0")
            icc = zlib.decompress(body[nul + 2:])
        pos += 12 + ln
    w, h, depth, ctype = ihdr[0], ihdr[1], ihdr[2], ihdr[3]
    ch = {2: 3, 6: 4}[ctype]
    bpp = ch * depth // 8
    raw = zlib.decompress(idat)
    stride = w * bpp
    out = np.zeros((h, stride), np.uint8)
    prev = np.zeros(stride, np.int32)
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = np.frombuffer(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)], np.uint8).astype(np.int32)
        if f == 0:
            cur = line
        elif f == 2:
            cur = (line + prev) & 255
        else:
            cur = np.zeros(stride, np.int32)
            for x in range(stride):
                a = cur[x - bpp] if x >= bpp else 0
                b = prev[x]
                c = prev[x - bpp] if x >= bpp else 0
                if f == 1:
                    pr = a
                elif f == 3:
                    pr = (a + b) >> 1
                else:
                    p = a + b - c
                    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                    pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                cur[x] = (line[x] + pr) & 255
        out[y] = cur
        prev = cur
    if depth == 16:
        px = out.reshape(h, w * ch, 2)
        v = (px[..., 0].astype(np.uint32) << 8 | px[..., 1]).reshape(h, w, ch) / 65535.0
    else:
        v = out.reshape(h, w, ch) / 255.0
    return v[..., :3], icc


def read_tiff(path):
    """Uncompressed / deflate strip TIFF, RGB 8/16-bit or 32-bit float, plus its ICC profile."""
    data = open(path, "rb").read()
    le = data[:2] == b"II"
    e = "<" if le else ">"
    off = struct.unpack(e + "I", data[4:8])[0]
    n = struct.unpack(e + "H", data[off:off + 2])[0]
    tags = {}
    sizes = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 7: 1, 11: 4, 12: 8, 16: 8}
    fmt = {1: "B", 2: "c", 3: "H", 4: "I", 7: "B", 11: "f", 16: "Q"}
    for i in range(n):
        tag, typ, cnt = struct.unpack(e + "HHI", data[off + 2 + 12 * i:off + 10 + 12 * i])
        sz = sizes.get(typ, 1) * cnt
        vo = off + 10 + 12 * i
        if sz > 4:
            vo = struct.unpack(e + "I", data[vo:vo + 4])[0]
        raw = data[vo:vo + sz]
        if typ in fmt and typ not in (2, 7):
            tags[tag] = struct.unpack(e + fmt[typ] * cnt, raw)
        else:
            tags[tag] = raw
    w, h = tags[256][0], tags[257][0]
    bits = tags[258][0]
    comp = tags.get(259, (1,))[0]
    spp = tags.get(277, (3,))[0]
    sfmt = tags.get(339, (1,))[0]
    pred = tags.get(317, (1,))[0]
    if 324 in tags:
        offs, cnts = tags[324], tags[325]
    else:
        offs, cnts = tags[273], tags[279]
    tw = tags.get(322, (w,))[0]
    th = tags.get(323, tags.get(278, (h,)))[0]
    dt = {(8, 1): np.uint8, (16, 1): np.uint16, (32, 3): np.float32}[(bits, sfmt)]
    dt = np.dtype(dt).newbyteorder(e)
    tiled = 324 in tags
    img = np.zeros((h, w, spp), np.float64)
    across = (w + tw - 1) // tw if tiled else 1
    for k, (o, c) in enumerate(zip(offs, cnts)):
        b = data[o:o + c]
        if comp in (8, 32946):
            b = zlib.decompress(b)
        elif comp != 1:
            raise ValueError(f"TIFF compression {comp}")
        bw = tw if tiled else w
        a = np.frombuffer(b, dt).astype(np.float64 if sfmt == 3 else np.int64)
        rows = len(a) // (bw * spp)
        a = a[:rows * bw * spp].reshape(rows, bw, spp)
        if pred == 2:
            a = np.cumsum(a, axis=1) % (1 << bits)
        elif pred == 3:
            raise ValueError("floating-point predictor")
        y0 = (k // across) * th if tiled else k * th
        x0 = (k % across) * tw if tiled else 0
        hh, ww = min(rows, h - y0), min(bw, w - x0)
        img[y0:y0 + hh, x0:x0 + ww] = a[:hh, :ww]
    scale = 1.0 if sfmt == 3 else float((1 << bits) - 1)
    icc = tags.get(34675)
    return img[..., :3] / scale, (bytes(icc) if icc is not None else None)


def read_file(path):
    ext = path.rsplit(".", 1)[1].lower()
    if ext == "png":
        return read_png16(path)
    if ext in ("tif", "tiff"):
        return read_tiff(path)
    im = Image.open(path)
    return np.asarray(im.convert("RGB")) / 255.0, im.info.get("icc_profile")


# ------------------------------------------------------------------ colour difference
def xyz_to_lab(xyz, white=D50):
    r = xyz / white
    f = np.where(r > (6 / 29) ** 3, np.cbrt(r), r / (3 * (6 / 29) ** 2) + 4 / 29)
    return np.stack([116 * f[..., 1] - 16, 500 * (f[..., 0] - f[..., 1]), 200 * (f[..., 1] - f[..., 2])], -1)


def de2000(l1, l2):
    L1, a1, b1 = l1[..., 0], l1[..., 1], l1[..., 2]
    L2, a2, b2 = l2[..., 0], l2[..., 1], l2[..., 2]
    C1, C2 = np.hypot(a1, b1), np.hypot(a2, b2)
    Cb = (C1 + C2) / 2
    G = 0.5 * (1 - np.sqrt(Cb ** 7 / (Cb ** 7 + 25 ** 7)))
    a1p, a2p = (1 + G) * a1, (1 + G) * a2
    C1p, C2p = np.hypot(a1p, b1), np.hypot(a2p, b2)
    h1p, h2p = np.degrees(np.arctan2(b1, a1p)) % 360, np.degrees(np.arctan2(b2, a2p)) % 360
    dLp, dCp = L2 - L1, C2p - C1p
    dhp = h2p - h1p
    dhp = np.where(dhp > 180, dhp - 360, np.where(dhp < -180, dhp + 360, dhp))
    dhp = np.where(C1p * C2p == 0, 0, dhp)
    dHp = 2 * np.sqrt(C1p * C2p) * np.sin(np.radians(dhp / 2))
    Lbp, Cbp = (L1 + L2) / 2, (C1p + C2p) / 2
    hs = h1p + h2p
    hbp = np.where(np.abs(h1p - h2p) > 180, np.where(hs < 360, hs + 360, hs - 360) / 2, hs / 2)
    hbp = np.where(C1p * C2p == 0, hs, hbp)
    T = 1 - 0.17 * np.cos(np.radians(hbp - 30)) + 0.24 * np.cos(np.radians(2 * hbp)) + 0.32 * np.cos(np.radians(3 * hbp + 6)) - 0.20 * np.cos(np.radians(4 * hbp - 63))
    dth = 30 * np.exp(-(((hbp - 275) / 25) ** 2))
    Rc = 2 * np.sqrt(Cbp ** 7 / (Cbp ** 7 + 25 ** 7))
    Sl = 1 + 0.015 * (Lbp - 50) ** 2 / np.sqrt(20 + (Lbp - 50) ** 2)
    Sc, Sh = 1 + 0.045 * Cbp, 1 + 0.015 * Cbp * T
    Rt = -np.sin(np.radians(2 * dth)) * Rc
    return np.sqrt((dLp / Sl) ** 2 + (dCp / Sc) ** 2 + (dHp / Sh) ** 2 + Rt * (dCp / Sc) * (dHp / Sh))


# ------------------------------------------------------------------ the check
CASES = [  # (format, bitDepth, extension)
    ("jpeg", 8, "jpg"),
    ("png", 8, "png"),
    ("png", 16, "png"),
    ("tiff", 8, "tif"),
    ("tiff", 16, "tif"),
]


def lcms_xyz(path, icc):
    """Lab (D50) of an 8-bit file through LittleCMS (relative colorimetric, to Lab D50)."""
    im = Image.open(path).convert("RGB")
    src = ImageCms.ImageCmsProfile(io.BytesIO(icc))
    lab = ImageCms.createProfile("LAB", 5000)
    t = ImageCms.buildTransform(src, lab, "RGB", "LAB", renderingIntent=ImageCms.Intent.RELATIVE_COLORIMETRIC)
    L = np.asarray(ImageCms.applyTransform(im, t)).astype(float)
    signed = lambda v: np.where(v >= 128, v - 256, v)  # noqa: E731 (Pillow's LAB: a*, b* as signed bytes)
    return np.stack([L[..., 0] * 100 / 255, signed(L[..., 1]), signed(L[..., 2])], -1)


def colorsync_lab(path, out):
    """Lab (D50) of a file through ColorSync: `sips --matchTo` ROMM RGB, 16-bit, then the published ROMM maths."""
    romm = "/System/Library/ColorSync/Profiles/ROMM RGB.icc"
    if not os.path.exists(romm):
        return None
    dst = out + ".romm.tif"
    r = subprocess.run(["sips", "-s", "format", "tiff", "--matchTo", romm, path, "--out", dst], capture_output=True, text=True)
    if r.returncode != 0 or not os.path.exists(dst):
        return None
    v, icc = read_tiff(dst)
    p = parse_icc(icc)
    lin = np.stack([eval_curve(p["trc"][c], np.clip(v[..., c], 0, 1)) for c in range(3)], -1)
    return xyz_to_lab(lin @ p["matrix"].T)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--library", required=True)
    ap.add_argument("--ids", required=True, help="comma-separated LightCraft photo ids")
    ap.add_argument("--out", required=True)
    ap.add_argument("--cli", default=CLI)
    ap.add_argument("--long-edge", type=int, default=900)
    ap.add_argument("--spaces", default="srgb,displayP3,adobeRgb,proPhoto")
    ap.add_argument("--no-export", action="store_true", help="reuse the files in --out")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    ids = [int(x) for x in a.ids.split(",")]
    spaces = a.spaces.split(",")
    jobs = []
    for pid in ids:
        jobs.append((pid, "ref", "tiff", 32, "tif", "rec2020"))
        for sp in spaces:
            for fmt, bits, ext in CASES:
                jobs.append((pid, f"{sp}-{fmt}{bits}", fmt, bits, ext, sp))
    if not a.no_export:
        script = []
        for pid, name, fmt, bits, ext, sp in jobs:
            path = os.path.join(a.out, f"{pid}-{name}.{ext}")
            if os.path.exists(path):
                os.remove(path)
            params = {"ids": [pid], "path": path, "format": fmt, "bitDepth": bits, "colorSpace": sp, "longEdge": a.long_edge, "quality": 100,
                      "sharpen": "none", "metadata": "all", "conflict": "overwrite", "tiffCompression": "none"}
            script.append(json.dumps({"command": "app.export", "params": params}))
        sp_path = os.path.join(a.out, "script.jsonl")
        open(sp_path, "w").write("\n".join(script) + "\n")
        r = subprocess.run([a.cli, "run", "--library", a.library, "--keep-going", "--script", sp_path], capture_output=True, text=True)
        for l in r.stdout.splitlines():
            if '"ok":false' in l:
                print("export failed:", l[:300])
    failures = []
    rows = []
    for pid in ids:
        ref_v, ref_icc = read_file(os.path.join(a.out, f"{pid}-ref.tif"))
        rp = parse_icc(ref_icc)
        ref_xyz = ref_v @ rp["matrix"].T  # linear float: no curve
        ref_lab = xyz_to_lab(ref_xyz)
        for _, name, fmt, bits, ext, sp in [j for j in jobs if j[0] == pid and j[1] != "ref"]:
            path = os.path.join(a.out, f"{pid}-{name}.{ext}")
            if not os.path.exists(path):
                failures.append(f"{pid} {name}: not exported")
                continue
            v, icc = read_file(path)
            if icc is None:
                failures.append(f"{pid} {name}: no ICC profile embedded")
                continue
            p = parse_icc(icc)
            h = p["header"]
            want = to_d50_matrix(sp)
            mat_err = float(np.abs(p["matrix"] - want).max())
            x = np.linspace(0, 1, 1025)
            trc_err = max(float(np.abs(eval_curve(c, x) - published_curve(sp, x)).max()) for c in p["trc"])
            prof_ok = h["colour"] == "RGB " and h["pcs"] == "XYZ " and mat_err < 0.002 and trc_err < 0.002
            if not prof_ok:
                failures.append(f"{pid} {name}: profile {p.get('desc')} v{h['version']} matrix err {mat_err:.4f} trc err {trc_err:.4f}")
            lin = np.stack([eval_curve(p["trc"][c], v[..., c]) for c in range(3)], -1)
            xyz = lin @ p["matrix"].T
            lab = xyz_to_lab(xyz)
            # pixels the target can hold: the reference converted to the target is inside 0..1 with a margin
            tgt = ref_xyz @ np.linalg.inv(want).T
            inside = np.all((tgt > 0.002) & (tgt < 0.995), axis=-1)
            de = de2000(ref_lab, lab)
            din = de[inside]
            row = {"id": pid, "file": name, "icc": p.get("desc"), "ver": h["version"], "matErr": round(mat_err, 5), "trcErr": round(trc_err, 5),
                   "inGamut": round(float(inside.mean()), 3), "dE": round(float(din.mean()), 3), "dE99": round(float(np.percentile(din, 99)), 3),
                   "dEmax": round(float(din.max()), 2), "dEout": round(float(de[~inside].mean()), 2) if (~inside).any() else 0.0}
            if bits == 8 and ext in ("jpg", "png", "tif"):
                try:
                    lc = lcms_xyz(path, icc)
                    row["lcms"] = round(float(de2000(lc, lab).mean()), 3)
                except Exception as ex:  # noqa: BLE001
                    row["lcms"] = f"error {ex}"
                cs = colorsync_lab(path, os.path.join(a.out, f"{pid}-{name}"))
                if cs is not None:
                    row["colorsync"] = round(float(de2000(cs, lab).mean()), 3)
            # 8-bit ProPhoto (gamma 1.8, huge gamut) quantises coarsely, worse through JPEG's YCbCr:
            # ~1.2 ΔE mean is the format's own limit (Lightroom's 8-bit ProPhoto JPEGs have it too)
            tol = (1.6 if sp == "proPhoto" else 1.2) if bits == 8 else 0.4
            if row["dE"] > tol or row["dE99"] > 3 * tol + (1.0 if fmt == "jpeg" else 0):
                failures.append(f"{pid} {name}: in-gamut dE {row['dE']} p99 {row['dE99']} vs the working-space render")
            rows.append(row)
    keys = ["id", "file", "icc", "ver", "matErr", "trcErr", "inGamut", "dE", "dE99", "dEmax", "dEout", "lcms", "colorsync"]
    print("  ".join(f"{k:>10}" for k in keys))
    for r in rows:
        print("  ".join(f"{str(r.get(k, '')):>10}"[:24] for k in keys))
    json.dump(rows, open(os.path.join(a.out, "export-check.json"), "w"), indent=1)
    for f in failures:
        print("FAIL", f)
    print(f"{len(rows)} files, {len(failures)} failures")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
