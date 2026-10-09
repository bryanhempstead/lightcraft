import numpy as np, json, os, tiff
PP_TO_XYZ = np.array([[0.7976749, 0.1351917, 0.0313534], [0.2880402, 0.7118741, 0.0000857], [0.0, 0.0, 0.8252100]])
D50 = np.array([0.9642, 1.0, 0.8249])
def acr(set_, name):
    a, d = tiff.read(f"{set_}/acr/{name}.tif"); assert 'ProPhoto' in d, d
    return a ** 1.8
def lc(set_, name):
    b = np.fromfile(f"{set_}/lc/{name}.f32", '<f4'); w, h = b[:2].view('<u4'); return b[2:].reshape(h, w, 3).astype(np.float64)
def lab(pp):
    xyz = pp @ PP_TO_XYZ.T / D50
    f = np.where(xyz > (6 / 29) ** 3, np.cbrt(np.maximum(xyz, 0)), xyz / (3 * (6 / 29) ** 2) + 4 / 29)
    return np.stack([116 * f[..., 1] - 16, 500 * (f[..., 0] - f[..., 1]), 200 * (f[..., 1] - f[..., 2])], -1)
def Y(pp): return pp @ PP_TO_XYZ[1]
