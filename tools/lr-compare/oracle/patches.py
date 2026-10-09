"""patches.py SET KIND NAME...: mean linear ProPhoto per chart cell (KIND acr|lc) -> SET/p_KIND/NAME.npy, then delete the image"""
import sys, os, json, numpy as np
from common import acr, lc
S, K = sys.argv[1], sys.argv[2]; m = json.load(open(f"{S}/meta.json")); n = len(m["cells"]); R = m.get("rects")
os.makedirs(f"{S}/p_{K}", exist_ok=True)
for name in sys.argv[3:]:
    src = f"{S}/{'acr' if K == 'acr' else 'lc'}/{name}.{'tif' if K == 'acr' else 'f32'}"
    if not os.path.exists(src): continue
    img = acr(S, name) if K == "acr" else lc(S, name)
    out = np.zeros((n, 3))
    for i in range(n):
        if R: y0, x0, y1, x1 = R[i]
        else: r, c = divmod(i, 24); cw, ch = m["cell"]; y0, x0, y1, x1 = 4 + r * ch + 9, c * cw + 9, 4 + (r + 1) * ch - 9, (c + 1) * cw - 9
        out[i] = img[y0:y1, x0:x1].reshape(-1, 3).mean(0)
    np.save(f"{S}/p_{K}/{name}.npy", out); os.remove(src)
