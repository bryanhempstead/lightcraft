# Camera Raw oracle (Bryan's fork)

Measure Lightroom's PV2012 rendering exactly: synthetic LinearRaw DNGs (`dng.py`, `scenes.py`; real
camera colour from the installed Adobe Standard DCPs via `dcp_info`) carrying Camera Raw settings in
their XMP, rendered by Camera Raw in Photoshop (`ps.sh` + `batch.jsx`, 16-bit ProPhoto TIFFs) and by
LightCraft (`cargo run --release -p lightcraft-engine --example oracle_render`), compared pixel by
pixel. `make_set*.py` build the experiments (slider sweeps on ramps, probes on backgrounds,
flats / edges / patches, composition pairs, real raws with one-slider sidecars via APFS clones);
`gen_tables.py` writes `crates/pipeline/src/tone_adobe.rs`. Keep outputs small: ≤1024 px synthetics,
Camera Raw's minimum size for real raws, extract to arrays and delete the TIFFs. Never touch a
Photoshop with open documents.

Colour (round 4b): `make_grid.py` (dense HSV grid) / `make_color.py` (chart) sweep Calibration, HSL,
Saturation, Vibrance and Summer Fields; `patches.py` reduces each render to per-patch means (TIFF /
f32 deleted), `lcchart.sh` renders LightCraft's side, `gcmp.py` / `cmp.py` compare (ΔE2000).
`inv.py` inverts Camera Raw's base on the grid so `calfit2.py` can fit Calibration as a ProPhoto
matrix; `hslproto.py` fits the HSL / Saturation / Vibrance tables (`hslfit.py`, OkLCh as in `ok.py`),
`combo.py` checks combinations, `gen_colortab.py` packs `crates/pipeline/src/colortab.bin`.
White balance: `make_wb.py` (synthetic Temp / Tint sweep), `make_rwb.py` + `rwbcmp.py` (real raws).
Scripts that read Bryan's renders (`make_real.py`, `make_rwb.py`) hold the scratch path in `L`.
