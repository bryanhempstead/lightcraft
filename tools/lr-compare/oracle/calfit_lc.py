"""calfit_lc.py SET NAME [ORACLE_RENDER]: with LightCraft in the loop, the linear-ProPhoto calibration
matrix (white kept) that best reproduces Camera Raw's chart for one calibration setting (`make_color.py`
item NAME), starting from LightCraft's own matrix; prints the matrix and the chart ΔE before / after.
Needs an oracle_render whose `colorops::calibration_matrix` honours a `LIGHTCRAFT_CALIB_M` override
(9 numbers, row-major linear ProPhoto, white kept): an experiment patch, not in the tree."""
import sys, os, json, subprocess, tempfile, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import acr, lab
from cmp import de2000
from chartcmp import patches

S, N = sys.argv[1], sys.argv[2]
B = sys.argv[3] if len(sys.argv) > 3 else os.path.expanduser("~/crafts/lightcraft/target/release/examples/oracle_render")
ref = lab(patches(S, acr(S, f"chart__{N}")))
ok = np.hypot(ref[:, 1], ref[:, 2]) > 0
out = tempfile.mkdtemp()


def render(m=None):
    env = dict(os.environ, LIGHTCRAFT_GPU_BACKEND="off")
    if m is not None:
        env["LIGHTCRAFT_CALIB_M"] = ",".join(f"{v:.6f}" for v in m.ravel())
    subprocess.run([B, out, f"{S}/dng/chart__{N}.dng"], env=env, check=True, capture_output=True)
    b = np.fromfile(f"{out}/chart__{N}.f32", "<f4"); w, h = b[:2].view("<u4")
    return lab(patches(S, b[2:].reshape(h, w, 3).astype(np.float64)))


def cost(x):
    m = np.eye(3) + x.reshape(3, 3) * (1 - np.eye(3))
    return de2000(ref, render(m))[ok].mean()


x0 = np.zeros(9)
print("LightCraft's own:", round(float(de2000(ref, render())[ok].mean()), 3))
print("identity:", round(cost(x0), 3))
# Nelder–Mead on the 6 off-diagonal entries
idx = [1, 2, 3, 5, 6, 7]
def f(v):
    x = np.zeros(9); x[idx] = v; return cost(x)
from itertools import count
n = len(idx); simplex = [np.zeros(n)] + [np.eye(n)[i] * 0.15 for i in range(n)]
vals = [f(p) for p in simplex]
for it in range(int(os.environ.get("ITERS", "120"))):
    o = np.argsort(vals); simplex = [simplex[i] for i in o]; vals = [vals[i] for i in o]
    c = np.mean(simplex[:-1], 0); r = c + (c - simplex[-1]); fr = f(r)
    if fr < vals[0]:
        e = c + 2 * (c - simplex[-1]); fe = f(e)
        simplex[-1], vals[-1] = (e, fe) if fe < fr else (r, fr)
    elif fr < vals[-2]:
        simplex[-1], vals[-1] = r, fr
    else:
        k = c + 0.5 * (simplex[-1] - c); fk = f(k)
        if fk < vals[-1]:
            simplex[-1], vals[-1] = k, fk
        else:
            simplex = [simplex[0] + 0.5 * (p - simplex[0]) for p in simplex]; vals = [f(p) for p in simplex]
best = simplex[int(np.argmin(vals))]
x = np.zeros(9); x[idx] = best
print("fitted:", round(min(vals), 3), "off-diagonals (col i = primary i):", np.round(x.reshape(3, 3), 4).tolist())
