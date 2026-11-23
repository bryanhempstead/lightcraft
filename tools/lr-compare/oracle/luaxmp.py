"""luaxmp.py: Lightroom's develop settings (the catalog's Lua table, `Adobe_imageDevelopSettings.text`) as a
Camera Raw XMP packet, so Camera Raw can render a photo with exactly its Lightroom edit (round 13's acceptance
ground truth, `make_acc.py`).

Lightroom's XMP writer is generic over the table: scalars become `crs:` attributes, arrays of scalars `rdf:Seq`s
(point curves as "x, y" pairs), arrays of tables `rdf:Seq`s of `rdf:Description`s, nested tables
`rdf:Description`s and `{["x-default"] = …}` language alternatives `rdf:Alt`s. A look whose RGB table is referenced
only by digest gets the table from Camera Raw's installed settings (`look_tables`)."""
import glob, os, re
from xml.sax.saxutils import escape, quoteattr


def parse(text):
    """Parse `s = { … }` into Python (dict for keyed tables, list for positional ones)."""
    s = text.strip()
    if s.startswith("s ="):
        s = s[3:]
    pos = [0]

    def ws():
        while pos[0] < len(s) and s[pos[0]] in " \t\r\n":
            pos[0] += 1

    def string():
        q = s[pos[0]]
        pos[0] += 1
        out = []
        while s[pos[0]] != q:
            c = s[pos[0]]
            if c == "\\":
                pos[0] += 1
                c = {"n": "\n", "t": "\t", "r": "\r"}.get(s[pos[0]], s[pos[0]])
            out.append(c)
            pos[0] += 1
        pos[0] += 1
        return "".join(out)

    def value():
        ws()
        c = s[pos[0]]
        if c == "{":
            return table()
        if c in "\"'":
            return string()
        m = re.compile(r"[-+0-9.eE]+|true|false|nil").match(s, pos[0])
        tok = m.group(0)
        pos[0] = m.end()
        if tok == "true":
            return True
        if tok == "false":
            return False
        if tok == "nil":
            return None
        return float(tok) if any(ch in tok for ch in ".eE") else int(tok)

    def table():
        pos[0] += 1  # {
        d, arr = {}, []
        while True:
            ws()
            if s[pos[0]] == "}":
                pos[0] += 1
                break
            if s[pos[0]] == "[":
                pos[0] += 1
                ws()
                k = string() if s[pos[0]] in "\"'" else value()
                ws()
                pos[0] += 1  # ]
                ws()
                pos[0] += 1  # =
                d[k] = value()
            else:
                m = re.compile(r"([A-Za-z_]\w*)\s*=(?!=)").match(s, pos[0])
                if m:
                    pos[0] = m.end()
                    d[m.group(1)] = value()
                else:
                    arr.append(value())
            ws()
            if pos[0] < len(s) and s[pos[0]] in ",;":
                pos[0] += 1
        return d if d else arr

    return value()


def scalar(v):
    if v is True:
        return "True"
    if v is False:
        return "False"
    if isinstance(v, float):
        return repr(v).rstrip("0").rstrip(".") if "e" not in repr(v) else f"{v:.6f}"
    return str(v)


CURVE = re.compile(r"^ToneCurve(PV2012)?(Red|Green|Blue)?$")
# Lightroom-only or derived keys Camera Raw does not take from XMP
SKIP = re.compile(r"^(Extended|ToneCurve$|ToneCurveRed$|ToneCurveGreen$|ToneCurveBlue$)")


def body(d, depth=0):
    """(attributes, child elements) of one table as an rdf:Description."""
    attrs, kids = [], []
    for k, v in d.items():
        if not isinstance(k, str) or not re.match(r"^[A-Za-z_]\w*$", k) or v is None or (depth == 0 and SKIP.match(k)):
            continue
        if isinstance(v, (dict, list)):
            x = element(k, v, depth + 1)
            if x:
                kids.append(x)
        else:
            attrs.append(f"crs:{k}={quoteattr(scalar(v))}")
    return attrs, kids


def description(d, depth):
    attrs, kids = body(d, depth)
    a = (" " + " ".join(attrs)) if attrs else ""
    return f"<rdf:Description{a}>{''.join(kids)}</rdf:Description>" if kids else f"<rdf:Description{a}/>"


def element(k, v, depth):
    if isinstance(v, dict):
        if v and all(isinstance(x, str) and (x == "x-default" or re.match(r"^[a-z]{2}(-[A-Z]{2})?$", x)) for x in v):
            lis = "".join(f"<rdf:li xml:lang={quoteattr(lang)}>{escape(scalar(t))}</rdf:li>" for lang, t in v.items())
            return f"<crs:{k}><rdf:Alt>{lis}</rdf:Alt></crs:{k}>"
        if not v:
            return ""
        return f"<crs:{k}>{description(v, depth)}</crs:{k}>"
    if not v:
        return ""
    if CURVE.match(k) and all(isinstance(x, (int, float)) for x in v):
        pts = "".join(f"<rdf:li>{scalar(v[i])}, {scalar(v[i + 1])}</rdf:li>" for i in range(0, len(v) - 1, 2))
        return f"<crs:{k}><rdf:Seq>{pts}</rdf:Seq></crs:{k}>"
    lis = []
    for x in v:
        if isinstance(x, dict):
            lis.append(f"<rdf:li>{description(x, depth)}</rdf:li>")
        elif isinstance(x, list):
            continue
        else:
            lis.append(f"<rdf:li>{escape(scalar(x))}</rdf:li>")
    return f"<crs:{k}><rdf:Seq>{''.join(lis)}</rdf:Seq></crs:{k}>"


_TABLES = None


def look_tables():
    """{digest: table text} from every Camera Raw settings / profile XMP installed on this Mac."""
    global _TABLES
    if _TABLES is None:
        _TABLES = {}
        roots = [os.path.expanduser("~/Library/Application Support/Adobe/CameraRaw"), "/Library/Application Support/Adobe/CameraRaw"]
        for r in roots:
            for f in glob.glob(os.path.join(r, "**", "*.xmp"), recursive=True):
                try:
                    t = open(f, encoding="utf-8", errors="replace").read()
                except OSError:
                    continue
                for m in re.finditer(r'crs:Table_([0-9A-F]{32})="([^"]*)"', t):
                    _TABLES.setdefault(m.group(1), m.group(2))
    return _TABLES


def packet(attrs, kids, extra_ns=""):
    return ('<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
            f'<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"{extra_ns}\n ' + "\n ".join(attrs) + ">\n"
            + "\n".join(kids) + "\n</rdf:Description></rdf:RDF></x:xmpmeta>\n")


def embed_jpeg(path, text):
    """Write the settings into a JPEG (Camera Raw reads a JPEG's own XMP, not a sidecar): the look (with its large
    table) goes into Extended XMP segments, everything else into the standard one."""
    import hashlib, struct
    full = to_xmp(text)
    i = full.find("<crs:Look>")
    j = full.find("</crs:Look>") + len("</crs:Look>")
    main, ext = (full, None) if i < 0 else (full[:i] + full[j:], None)
    if i >= 0:
        ext = packet([], [full[i:j]]).encode()
        guid = hashlib.md5(ext).hexdigest().upper()
        main = main.replace('xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"',
                            f'xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" xmlns:xmpNote="http://ns.adobe.com/xmp/note/" xmpNote:HasExtendedXMP="{guid}"', 1)
    b = open(path, "rb").read()
    if b[:2] != b"\xff\xd8":
        raise ValueError("not a JPEG")
    segs, k = [], 2
    while k + 4 <= len(b) and b[k] == 0xFF and b[k + 1] not in (0xDA, 0xD9):
        n = struct.unpack(">H", b[k + 2:k + 4])[0]
        segs.append(b[k:k + 2 + n])
        k += 2 + n
    rest = b[k:]
    xap, xext = b"http://ns.adobe.com/xap/1.0/\0", b"http://ns.adobe.com/xmp/extension/\0"
    keep = [s for s in segs if not (s[1] == 0xE1 and (s[4:].startswith(xap) or s[4:].startswith(xext)))]
    mb = xap + main.encode()
    if len(mb) + 2 > 65535:
        raise ValueError("standard XMP too large")
    new = [b"\xff\xe1" + struct.pack(">H", len(mb) + 2) + mb]
    if ext is not None:
        for off in range(0, len(ext), 65400):
            chunk = ext[off:off + 65400]
            body = xext + guid.encode() + struct.pack(">II", len(ext), off) + chunk
            new.append(b"\xff\xe1" + struct.pack(">H", len(body) + 2) + body)
    # after APP0 / Exif, before the rest
    head = [s for s in keep if s[1] in (0xE0,) or (s[1] == 0xE1 and s[4:10] == b"Exif\0\0")]
    tail = [s for s in keep if s not in head]
    open(path, "wb").write(b"\xff\xd8" + b"".join(head + new + tail) + rest)


def to_xmp(text, override=None, drop=None):
    """The XMP packet of a develop-settings text. `override`: {key: value} set at top level; `drop`: a regex of
    top-level keys to leave out."""
    d = parse(text)
    if not isinstance(d, dict):
        raise ValueError("not a settings table")
    if drop:
        d = {k: v for k, v in d.items() if not re.match(drop, k)}
    # what Lightroom's XMP writer adds: the settings are real, not yet baked in, and a crop is on when set
    d.setdefault("HasSettings", True)
    d.setdefault("AlreadyApplied", False)
    crop = {"CropTop": 0, "CropLeft": 0, "CropRight": 1, "CropBottom": 1, "CropAngle": 0}
    d.setdefault("HasCrop", any(abs(float(d.get(k, v)) - v) > 1e-9 for k, v in crop.items()))
    if d.get("HasCrop"):
        # the catalog leaves out keys at their default; Camera Raw ignores a crop that lacks any of them
        for k, v in crop.items():
            d.setdefault(k, v)
    d.update(override or {})
    look = d.get("Look")
    if isinstance(look, dict) and isinstance(look.get("Parameters"), dict):
        for key in ("RGBTable", "LookTable"):
            dig = look["Parameters"].get(key)
            if isinstance(dig, str) and dig in look_tables():
                look["Parameters"][f"Table_{dig}"] = look_tables()[dig]
    attrs, kids = body(d)
    return packet(attrs, kids)


if __name__ == "__main__":
    import sqlite3, sys
    db = sqlite3.connect(sys.argv[1])
    t = db.execute("SELECT text FROM Adobe_imageDevelopSettings WHERE image=?", (int(sys.argv[2]),)).fetchone()[0]
    print(to_xmp(t))
