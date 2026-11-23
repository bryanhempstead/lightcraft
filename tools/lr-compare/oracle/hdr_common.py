"""Shared helpers for the HDR oracle: Camera Raw's PQ (BT.2100) / ProPhoto TIFFs and LightCraft's
float renders, all brought to linear Rec.2020 relative to SDR white; ICtCp ΔEITP (BT.2124)."""
import sys, os, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tiff

M1, M2, C1, C2, C3 = 2610 / 16384, 2523 / 4096 * 128, 3424 / 4096, 2413 / 4096 * 32, 2392 / 4096 * 32
SDR_NITS = 203.0  # BT.2408 reference white (checked against the 16-bit SDR renders, see calib)


def pq_eotf(e):
    e = np.clip(e, 0, 1) ** (1 / M2)
    return 10000 * (np.maximum(e - C1, 0) / (C2 - C3 * e)) ** (1 / M1)


def pq_oetf(nits):
    y = np.clip(nits / 10000, 0, 1) ** M1
    return ((C1 + C2 * y) / (1 + C3 * y)) ** M2


def xyz_m(prim, white=(0.3127, 0.3290)):
    def v(x, y): return np.array([x / y, 1, (1 - x - y) / y])
    P = np.stack([v(*p) for p in prim], 1)
    S = np.linalg.solve(P, v(*white))
    return P * S


R2020 = xyz_m([(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)])
PP = xyz_m([(0.7347, 0.2653), (0.1596, 0.8404), (0.0366, 0.0001)], (0.3457, 0.3585))
BRAD = np.array([[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]])


def adapt(src_w, dst_w):
    def w(x, y): return np.array([x / y, 1, (1 - x - y) / y])
    a, b = BRAD @ w(*src_w), BRAD @ w(*dst_w)
    return np.linalg.inv(BRAD) @ np.diag(b / a) @ BRAD


PP_TO_2020 = np.linalg.inv(R2020) @ adapt((0.3457, 0.3585), (0.3127, 0.3290)) @ PP


def acr(path):
    """Camera Raw TIFF -> (linear Rec.2020 relative to SDR white, kind)."""
    a, d = tiff.read(path)
    if 'PQ' in d or '2100' in d:
        return pq_eotf(a) / SDR_NITS, 'pq'
    assert 'ProPhoto' in d, d
    return (a ** 1.8) @ PP_TO_2020.T, 'sdr'


def lc(path):
    b = np.fromfile(path, '<f4')
    w, h = b[:2].view('<u4')
    return b[2:].reshape(h, w, 3).astype(np.float64) @ PP_TO_2020.T


def Y(rgb2020):
    return rgb2020 @ R2020[1]


def ictcp(rgb2020, white_nits=SDR_NITS):
    """Linear Rec.2020 relative to SDR white -> ICtCp (BT.2100 PQ)."""
    lms = rgb2020 @ (np.array([[1688, 2146, 262], [683, 2951, 462], [99, 309, 3688]]) / 4096).T
    l = pq_oetf(np.maximum(lms, 0) * white_nits)
    return l @ (np.array([[2048, 2048, 0], [6610, -13613, 7003], [17933, -17390, -543]]) / 4096).T


def de_itp(a, b):
    """ΔEITP (BT.2124) between linear Rec.2020 images (SDR white = 1, 203 nits)."""
    x, y = ictcp(a), ictcp(b)
    d = x - y
    return 720 * np.sqrt(d[..., 0] ** 2 + (0.5 * d[..., 1]) ** 2 + d[..., 2] ** 2)


def fit(a, b):
    """Crop both to the common size (Camera Raw's minimum size can be a pixel off)."""
    h, w = min(a.shape[0], b.shape[0]), min(a.shape[1], b.shape[1])
    return a[:h, :w], b[:h, :w]
