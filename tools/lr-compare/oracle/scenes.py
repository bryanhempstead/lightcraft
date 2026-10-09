"""Synthetic scenes (linear ProPhoto, scene EV relative to raw white = 0) for the oracle."""
import numpy as np, colorsys
W = 1024
def ramp(h=128, lo=-14.0, hi=0.5):
    ev = np.linspace(lo, hi, W)
    return np.repeat((2.0 ** ev)[None, :, None], h, 0).repeat(3, 2)
def probe(bg, h=512, lo=-12.0, hi=0.0, band=48):
    """uniform background at EV `bg` with a thin neutral probe ramp across the middle"""
    img = np.full((h, W, 3), 2.0 ** bg)
    ev = np.linspace(lo, hi, W)
    y0 = h // 2 - band // 2
    img[y0:y0 + band] = (2.0 ** ev)[None, :, None]
    return img
def spatial(h=512):
    """step edge (−6 | −1.5 EV) on top, a soft gradient, a textured mid-grey square, a bright window"""
    img = np.full((h, W, 3), 2.0 ** -3.5)
    img[:h // 2, :W // 2] = 2.0 ** -6
    img[:h // 2, W // 2:] = 2.0 ** -1.5
    x = np.linspace(-8, -0.5, W)
    img[h // 2:h // 2 + 64] = (2.0 ** x)[None, :, None]
    rng = np.random.default_rng(1)
    tex = 2.0 ** (-3.5 + 0.35 * rng.standard_normal((160, 160)))
    img[h // 2 + 96:h // 2 + 256, 64:224] = tex[..., None]
    img[h // 2 + 120:h // 2 + 200, 600:760] = 2.0 ** -6.5
    img[h // 2 + 140:h // 2 + 180, 650:710] = 2.0 ** -0.8
    return img
def chart(h=640):
    """24 hues x 4 saturations x 3 levels (ProPhoto HSV, linear), plus skin / foliage / teal / sky, on mid grey"""
    img = np.full((h, W, 3), 0.18 * 2.0 ** -1.0)
    cells = []
    for li, v in enumerate([2.0 ** -4.5, 2.0 ** -2.5, 2.0 ** -1.2]):
        for si, s in enumerate([0.2, 0.4, 0.6, 0.85]):
            for hi in range(24):
                r, g, b = colorsys.hsv_to_rgb(hi / 24, s, 1.0)
                cells.append((v * np.array([r, g, b]) ** 2.2 * 1.0))
    # skin, foliage, teal water, sky (linear ProPhoto, rough)
    extra = [[0.30, 0.20, 0.15], [0.20, 0.13, 0.10], [0.08, 0.10, 0.04], [0.04, 0.07, 0.03], [0.03, 0.08, 0.07], [0.05, 0.12, 0.12], [0.20, 0.28, 0.45], [0.10, 0.14, 0.22]]
    cells += [np.array(e) for e in extra]
    cw, chh = W // 24, (h - 40) // 13
    for i, c in enumerate(cells):
        row, col = divmod(i, 24)
        img[4 + row * chh + 3: 4 + (row + 1) * chh - 3, col * cw + 3:(col + 1) * cw - 3] = c
    return img, cells, (cw, chh)
GRID_H = np.arange(36) / 36.0
GRID_S = np.array([0.0, 0.08, 0.16, 0.25, 0.35, 0.5, 0.65, 0.8, 0.9, 0.97])
GRID_EV = np.linspace(-9.0, 0.0, 13)
def grid(px=8, cols=72):
    """dense HSV grid (linear ProPhoto): 36 hues x 10 saturations x 13 levels, px-square patches; returns (img, colours, rects)"""
    cols_ = []
    for ev in GRID_EV:
        for s in GRID_S:
            for hh in GRID_H:
                cols_.append(2.0 ** ev * np.array(colorsys.hsv_to_rgb(hh, s, 1.0)))
    n = len(cols_); rows = -(-n // cols)
    img = np.full((rows * px, cols * px, 3), 0.18 * 0.5)
    rects = []
    for i, c in enumerate(cols_):
        r, k = divmod(i, cols)
        img[r * px:(r + 1) * px, k * px:(k + 1) * px] = c
        rects.append((r * px + 2, k * px + 2, r * px + px - 2, k * px + px - 2))
    return img, np.array(cols_), rects
