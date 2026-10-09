"""Minimal LinearRaw DNG writer + crs XMP packets for the Camera Raw oracle (scratch only)."""
import struct, json, subprocess, os, numpy as np
from fractions import Fraction

DCP_DIR = "/Library/Application Support/Adobe/CameraRaw/CameraProfiles/Adobe Standard"
DCP_INFO = os.path.expanduser("~/crafts/lightcraft/target/release/examples/dcp_info")
CAMERAS = {  # model -> (Make, Model as in Exif, DCP file stem)
    "Canon EOS R6": ("Canon", "Canon EOS R6", "Canon EOS R6"),
    "LEICA M (Typ 262)": ("LEICA CAMERA AG", "LEICA M (Typ 262)", "LEICA M (Typ 262)"),
    "RICOH GR III": ("RICOH IMAGING COMPANY, LTD.", "RICOH GR III", "RICOH GR III"),
    "Fujifilm X-T2": ("FUJIFILM", "X-T2", "Fujifilm X-T2"),
}

def dcp(model, neutral):
    out = subprocess.run([DCP_INFO, f"{DCP_DIR}/{CAMERAS[model][2]} Adobe Standard.dcp", *map(str, neutral)], capture_output=True, text=True, check=True).stdout
    return json.loads(out)

# ProPhoto (D50) linear -> XYZ D50
PP_TO_XYZ = np.array([[0.7976749, 0.1351917, 0.0313534], [0.2880402, 0.7118741, 0.0000857], [0.0, 0.0, 0.8252100]])

def scene_to_camera(pp, info):
    """ProPhoto linear scene colours (… x 3) -> camera RGB for the profile's colour spec."""
    c2p = np.array(info["spec"]["camera_to_pcs"])
    return pp @ (np.linalg.inv(c2p) @ PP_TO_XYZ).T

def srational(v, den=10000):
    return (int(round(v * den)), den)

def write_dng(path, cam_rgb, model, neutral, matrices, xmp, baseline=0.0):
    """cam_rgb: H x W x 3 floats in 0..1 (camera RGB, not white balanced)."""
    h, w, _ = cam_rgb.shape
    data = (np.clip(cam_rgb, 0, 1) * 65535 + 0.5).astype('<u2').tobytes()
    make, exif_model, _ = CAMERAS[model]
    entries = []  # (tag, type, count, value bytes)
    def add(tag, typ, vals):
        fmt = {1: 'B', 3: 'H', 4: 'I', 5: 'II', 10: 'ii', 2: None, 7: None}[typ]
        if typ in (2, 7):
            b = vals if isinstance(vals, bytes) else vals.encode() + b'\0'
            entries.append((tag, typ, len(b), b)); return
        if typ in (5, 10):
            b = b''.join(struct.pack('<' + fmt, *v) for v in vals); entries.append((tag, typ, len(vals), b)); return
        b = struct.pack('<' + fmt * len(vals), *vals); entries.append((tag, typ, len(vals), b))
    add(254, 4, [0]); add(256, 4, [w]); add(257, 4, [h]); add(258, 3, [16, 16, 16]); add(259, 3, [1])
    add(262, 3, [34892]); add(271, 2, make); add(272, 2, exif_model); add(273, 4, [0]); add(274, 3, [1])
    add(277, 3, [3]); add(278, 4, [h]); add(279, 4, [len(data)]); add(284, 3, [1]); add(305, 2, "lightcraft oracle")
    add(700, 1, xmp.encode('utf-8'))
    add(50706, 1, [1, 4, 0, 0]); add(50707, 1, [1, 1, 0, 0]); add(50708, 2, model)
    add(50714, 3, [0, 0, 0]); add(50717, 4, [65535, 65535, 65535])
    add(50721, 10, [srational(v) for v in np.ravel(matrices["cm1"])]); add(50722, 10, [srational(v) for v in np.ravel(matrices["cm2"])])
    add(50728, 5, [srational(v) for v in neutral]); add(50730, 10, [srational(baseline)])
    add(50731, 5, [(1, 1)]); add(50732, 5, [(1, 1)]); add(50734, 5, [(1, 1)])
    add(50778, 3, [matrices["illuminants"][0]]); add(50779, 3, [matrices["illuminants"][1]])
    add(50964, 10, [srational(v) for v in np.ravel(matrices["fm1"])]); add(50965, 10, [srational(v) for v in np.ravel(matrices["fm2"])])
    entries.sort(key=lambda e: e[0])
    n = len(entries); ifd_off = 8; ifd_size = 2 + 12 * n + 4
    extra_off = ifd_off + ifd_size
    blobs = b''; offsets = {}
    for tag, typ, cnt, b in entries:
        if len(b) > 4:
            if (extra_off + len(blobs)) % 2: blobs += b'\0'
            offsets[tag] = extra_off + len(blobs); blobs += b
    data_off = extra_off + len(blobs)
    data_off += data_off % 2
    out = bytearray(b'II*\0' + struct.pack('<I', ifd_off) + struct.pack('<H', n))
    for tag, typ, cnt, b in entries:
        if tag == 273: b = struct.pack('<I', data_off)
        if len(b) <= 4: val = b.ljust(4, b'\0')
        else: val = struct.pack('<I', offsets[tag])
        out += struct.pack('<HHI', tag, typ, cnt) + val
    out += struct.pack('<I', 0) + blobs
    out += b'\0' * (data_off - len(out)) + data
    open(path, 'wb').write(bytes(out))

def xmp_packet(settings, look=None, curves=None):
    """crs settings dict -> XMP packet. look: dict with name/amount/uuid/params xml (optional)."""
    attrs = "\n   ".join(f'crs:{k}="{v}"' for k, v in settings.items())
    curve_xml = ""
    for name, pts in (curves or {}).items():
        curve_xml += f"<crs:{name}><rdf:Seq>" + "".join(f"<rdf:li>{x}, {y}</rdf:li>" for x, y in pts) + f"</rdf:Seq></crs:{name}>"
    look_xml = look or ""
    return f'''<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   {attrs}>
   {curve_xml}{look_xml}
  </rdf:Description></rdf:RDF></x:xmpmeta>'''

# everything explicit: Lightroom's PV2012 defaults with sharpening, NR, lens corrections off
BASE = dict(Version="17.0", ProcessVersion="15.4", WhiteBalance="As Shot", Exposure2012="0", Contrast2012="0",
            Highlights2012="0", Shadows2012="0", Whites2012="0", Blacks2012="0", Texture="0", Clarity2012="0", Dehaze="0",
            Vibrance="0", Saturation="0", ParametricShadows="0", ParametricDarks="0", ParametricLights="0", ParametricHighlights="0",
            ParametricShadowSplit="25", ParametricMidtoneSplit="50", ParametricHighlightSplit="75", Sharpness="0", SharpenRadius="+1.0",
            SharpenDetail="25", SharpenEdgeMasking="0", LuminanceSmoothing="0", ColorNoiseReduction="0", ColorNoiseReductionDetail="50",
            ColorNoiseReductionSmoothness="50", LensProfileEnable="0", AutoLateralCA="0", LensManualDistortionAmount="0",
            VignetteAmount="0", PostCropVignetteAmount="0", GrainAmount="0", CameraProfile="Adobe Standard",
            ConvertToGrayscale="False", ToneCurveName2012="Linear", HasSettings="True", HasCrop="False", AlreadyApplied="False",
            OverrideLookVignette="False", EnableCalibration="True", EnableColorAdjustments="True", EnableDetail="True")
LINEAR = {"ToneCurvePV2012": [(0, 0), (255, 255)], "ToneCurvePV2012Red": [(0, 0), (255, 255)], "ToneCurvePV2012Green": [(0, 0), (255, 255)], "ToneCurvePV2012Blue": [(0, 0), (255, 255)]}
