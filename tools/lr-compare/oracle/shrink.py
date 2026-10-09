import sys, numpy as np, os
b = np.fromfile(sys.argv[1], '<f4'); w, h, n = int(b[0]), int(b[1]), int(b[2])
d = b[9:9 + w * h * n].reshape(h, w, n)[::3, ::3]
np.save(sys.argv[2], {"hdr": b[:9].copy(), "d": d.astype(np.float16)}, allow_pickle=True)
os.remove(sys.argv[1])
