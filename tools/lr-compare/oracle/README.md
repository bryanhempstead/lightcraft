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

Highlights / Shadows (R02, round 8): `make_rhs.py` (Bryan's split-A raws with one-slider sidecars + one
neutral ramp per camera, plus synthetic layouts), `hsfit_rhs.py` (fits `tone::lr::adobe_hs_lut`'s
constants in scene EV), `make_full.py` (acceptance photos with their own full settings and the Summer
Fields look), `make_hsorder.py` + `hsorder.py` (H/S come before Exposure / Contrast / Whites / Blacks).

Detail and Effects (round 9): `make_detail.py SET grain|grain6000|sharp|nr` (grey flats at three
sizes, an edge / grating / texture chart, noisy flats), then `grain_ana.py` / `sharp_ana.py` /
`nr_ana.py SET [LCDIR]`. Lateral CA on real raws: `make_ca.py SET sample.json` (profile × Remove
CA sidecars, full size) and `ca_ana.py SET [LCDIR]`.

Scene tone map (round 11: Clarity, positive Dehaze, the Highlights / Shadows rule; `scene_cr`):
`make_r11.py SET syn,mask,real ACC` (synthetic scenes incl. a hazy landscape and a dark room, masks next
to global sliders, Bryan's readable split-A raws with one-slider sidecars), `lcrun.sh SET OUT [filter]`
(now also the raw clones in SET/v, at 1536 px), `r11cmp.py A.txt B.txt` (two `lccmp.py` runs side by side,
per-setting means), `clafit.py SET DUMPDIR` (refits Clarity's weights from `LIGHTCRAFT_CLARITY_DUMP` term
dumps). `LIGHTCRAFT_HS_CR=1` renders with the H/S rule while it is off.

Round 12 (default black, H/S on Bryan's own combinations, Texture's colour, Calibration's order):
`make_r12.py SET ACC` (his split-A raws with H/S: own Basic tone, without H/S, H/S only, S / H only, plus a
Shadows × Highlights sweep on three photos), `r12ana.py SET LCDIR` (per photo: ΔE, ΔL* by band, each
setting's effect against its reference), `r12sum.py SET LCDIR…` (runs side by side, R6 apart), `r12lift.py`
(Shadows' lift per photo), `r12reg.py` (a change split into regional and detail parts), `r12grat.py`
(full-frame gratings: mean and contrast change). Measurement switches: `LIGHTCRAFT_BLACK_CR=0|shift|photo`,
`LIGHTCRAFT_HS_CR=0|1`, `LIGHTCRAFT_HS_SUP`, `LIGHTCRAFT_HS_PRE`, `LIGHTCRAFT_HUE_SAT_FINISH=0|1`,
`LIGHTCRAFT_TEX_CHROMA=gain,scale|0`, `LIGHTCRAFT_CLA_LEVEL`; `lcrun.sh` takes another build in `ORACLE_BIN`.

HDR tone (`docs/lr-match.md` → HDR tone): `make_hdr_real.py SET SRC [stems]` (APFS clones of raws in SRC
with HDR sidecars per variant) and `make_hdr_syn.py SET NAME…` (synthetic ramps / grid, names like
`h4_w+50_e+2`; `sdr_…` = HDR off), rendered by `hdr_ps.sh SET` (`batch_hdr.jsx`: HDR edits come back as
Rec. 2100 PQ; the shared lock `/private/tmp/lightcraft-photoshop.lock`, a 300 s no-progress watchdog,
never quits Photoshop) and `hdr_lcrun.sh SET OUT` (LightCraft's HDR masters, scene-EV dumps and SDR
twins). `hdr_cmp.py SET OUT` (ΔEITP overall / SDR / headroom part, log2 ratio per band, Visualize HDR
ranges), `hdr_align.py` (curves aligned on the SDR tone's half-white point), `hdr_fit.py SET…` +
`gen_hdr.py gtables.npz crates/pipeline/src/tone_hdr_cr.rs` (the tables). Gain maps: `hdrdecode.swift`
(`swiftc -O hdrdecode.swift -o hdrdecode`; decodes a file to HDR with macOS ImageIO as Preview does) and
`hdr_gm_cmp.py DECODED.f32 LC.f32 [CR.tif]`.
