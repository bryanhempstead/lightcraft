"""gen_para.py SET OUT.rs: writes `crates/pipeline/src/parametric_adobe.rs` from the parametric-curve
ramps of `make_curve.py` (Camera Raw's offset y − x per region and slider stop, 65 samples)."""
import sys, os, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
S, OUT = sys.argv[1], sys.argv[2]
sys.argv = [sys.argv[0], S]
import parafit as P

N = 65
xs = np.linspace(0, 1, N)
stops = [-100, -50, 50, 100]
rows = []
for r in P.REG:
    per = []
    for v in stops:
        d = P.tab[r][P.STOPS.index(v)]
        per.append(np.interp(xs, P.X, d))
    rows.append(per)
t = np.array(rows)
t[:, :, 0] = 0.0
t[:, :, -1] = 0.0


def arr(a):
    return "[" + ", ".join(f"{v:.5f}" for v in a) + "]"


body = ",\n".join("    [\n" + ",\n".join(f"        {arr(s)}" for s in reg) + ",\n    ]" for reg in t)
open(OUT, "w").write(f'''//! Lightroom's parametric tone curve, measured with the Camera Raw oracle (`tools/lr-compare/oracle`
//! `make_curve.py` → `gen_para.py`; Bryan's fork, `docs/lr-match.md` → *Parametric curve (R08)*). Generated: don't edit.
//!
//! Per region (Shadows, Darks, Lights, Highlights) and slider stop (−100, −50, +50, +100): Camera
//! Raw's change of the encoded value (`y − x`, ProPhoto primaries with the sRGB curve) at {N} evenly
//! spaced inputs, with the default splits (25 / 50 / 75).

/// Slider stops of [`PARAMETRIC_TABLE`] (0 = no change).
pub const PARAMETRIC_STOPS: [f32; 4] = [-100.0, -50.0, 50.0, 100.0];
/// Samples per curve over `0..1`.
pub const PARAMETRIC_N: usize = {N};
#[rustfmt::skip]
pub const PARAMETRIC_TABLE: [[[f32; PARAMETRIC_N]; 4]; 4] = [
{body},
];
''')
print("wrote", OUT, t.shape)
