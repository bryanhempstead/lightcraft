"""OkLab / OkLCh of linear ProPhoto values, as LightCraft computes them (Rec.2020 D65 via Bradford)."""
import numpy as np
P = np.array([[0.7976749, 0.1351917, 0.0313534], [0.2880402, 0.7118741, 0.0000857], [0.0, 0.0, 0.8252100]])
B = np.array([[0.9555766, -0.0230393, 0.0631636], [-0.0282895, 1.0099416, 0.0210077], [0.0122982, -0.0204830, 1.3299098]])
X2R = np.linalg.inv(np.array([[0.6369580, 0.1446169, 0.1688810], [0.2627002, 0.6779981, 0.0593017], [0.0, 0.0280727, 1.0609851]]))
M2020 = X2R @ B @ P
XYZ_TO_LMS = np.array([[0.8189330101, 0.3618667424, -0.1288597137], [0.0329845436, 0.9293118715, 0.0361456387], [0.0482003018, 0.2643662691, 0.6338517070]])
M1 = XYZ_TO_LMS @ B @ P  # ProPhoto -> LMS
M2 = np.array([[0.2104542553, 0.7936177850, -0.0040720468], [1.9779984951, -2.4285922050, 0.4505937099], [0.0259040371, 0.7827717662, -0.8086757660]])
def oklab_pp(x): return np.cbrt(np.maximum(x @ M1.T, 0)) @ M2.T
def oklch_pp(pp):
    l = oklab_pp(pp); return np.stack([l[..., 0], np.hypot(l[..., 1], l[..., 2]), np.arctan2(l[..., 2], l[..., 1])], -1)
def lch_to_pp(L):
    lab = np.stack([L[:, 0], L[:, 1] * np.cos(L[:, 2]), L[:, 1] * np.sin(L[:, 2])], 1)
    return ((lab @ np.linalg.inv(M2).T) ** 3) @ np.linalg.inv(M1).T
