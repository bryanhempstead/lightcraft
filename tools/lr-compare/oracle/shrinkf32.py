import sys, numpy as np, os
b = np.fromfile(sys.argv[1], '<f4'); w, h = b[:2].view('<u4'); a = b[2:].reshape(h, w, 3)[::3, ::3]
np.save(sys.argv[2], a.astype(np.float32)); os.remove(sys.argv[1])
