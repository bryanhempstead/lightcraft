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

Round 13 (the acceptance set with Camera Raw as its ground truth; negative Dehaze and Clarity probes; H/S pairs):
`make_acc.py pick SET [--masks N]` / `make_acc.py finish SET` builds an `lr_compare.py` work folder whose "previews"
are Camera Raw's renders of each photo with its **whole catalog edit** (`luaxmp.py`: the catalog's Lua settings as
XMP — sliders, curves, the look with its table from the installed settings, masks, crop, lens, detail; a sidecar for
raws, standard + Extended XMP embedded in JPEGs, whose sidecars Camera Raw ignores). Then `lr_compare.py migrate /
render / compare --no-publish / gate` as with Lightroom's previews. `make_r13.py SET deh|cla` (flat fields, gratings,
checkers, an edge and lines at Dehaze −25 / −50 / −100; flat two-level layouts, windows and a hole at Clarity −50 /
+50 / +100), `r13deh.py` / `r13cla.py` (mean / contrast change and profiles, Camera Raw vs LightCraft), `clafit13.py`
(Clarity refit with extra large-scale features, leave one scene out). The scripts that read `ACC` key photos by
their original file name (`original`). Measurement switches: `LIGHTCRAFT_DZ_SP=sigma,key,offset,beta,alpha`
(negative Dehaze's spatial variant, CPU only), `LIGHTCRAFT_HS_PAIR=c,shadows,highlights` (the H/S pair blend's
fractions for one sign combination).

New-camera check (`camera_check.py`): for each camera model in the LightCraft library (`catalog.snap`, read-only) with no
check on file, one representative raw (the middle readable ISO <= 1600 frame; `--file` / `--photo-id` override) is cloned
to scratch with sidecars and rendered by LightCraft (`lcrun.sh`; `ORACLE_BIN` another `oracle_render`, else the main
checkout's release build) and compared with Camera Raw at identical settings, 1536 px: base (Adobe Standard, linear curve,
sharpening / NR / lens off), Exposure -2 / +2, Whites +100, Shadows +100, Highlights -100, Temp 3000 / 7500 K (as-shot
tint). Metrics are the 2026-11-25 slider study's: dE2000 mean / p95, L* per tone band, % clipped, "acts like CR N"
(LightCraft's effect over Camera Raw's in the slider's band; Temp: neutral b*). A step is flagged at mean dE > 3, p95 > 8,
clipped % differing by > 3 points or acts-like off by > 20 %. `--list` (models, raws, state, bank), `--model "RICOH GR III"`
(substring), `--all-new`, `--force`. Records: `~/Library/Application Support/LightCraft/camera-checks.json`; `report.md` +
`strip.jpg` per model in `compare/camera-checks/<model>/`; a flagged model adds a `needs-you.sh` test. item
(`--no-needs-you`).
Reference bank (for when Photoshop is gone): every Camera Raw render the check makes is kept in
`~/Library/Application Support/LightCraft/reference/<model>/<step>.tif` + `index.json` (file, tint, per-step stats). A
check compares against the bank and calls Photoshop (`hdr_ps.sh`, the shared lock) only for steps the bank lacks, then
banks them; `--no-photoshop` uses the bank only and reports what is missing. A model with no bank and no Photoshop gets a
base-only check against the embedded preview of Adobe's free DNG Converter (`/Applications/Adobe DNG Converter.app`;
"base via DNG Converter preview, defaults incl. sharpening", less exact, redone in full later; if the app is missing it says
to install it and skips). Photoshop is quit at the end of a run that used it when nobody holds the lock and no document is
open (`--keep-photoshop`).

Comparing builds: `vendor/` (the DNG SDK, LibRaw) is gitignored, so a fresh worktree builds **without** the Adobe base
(cargo only warns "Adobe DNG SDK not found ... building without it"; `ln -s <main checkout>/vendor vendor` first, and a
changed `vendor/` needs `touch crates/dng-sdk-sys/build.rs crates/libraw-sys/build.rs`). Such a binary renders every
mid-tone different (R6 Whites +100: 23 % clipped with the SDK, 0 % without; base ΔE vs Camera Raw 1.1 → 1.8, Leica
1.2 → 3.0): check `LIGHTCRAFT_PROFILE=1` prints `raw source (Adobe <model>)` before trusting a before / after.

Exposure / Whites / Contrast vs Camera Raw (2026-11-25 fix, `tone2_study.py`): `prep` clones the study's four raws with
one-slider sidecars (Exposure ±1…3, Whites ±50 / ±100, Contrast ±50 / ±100; GR III's banked steps reused), `cr` renders
the rest via `hdr_ps.sh`, `lc NAME BIN` renders a build, `cmp NAME…` gives per-step dE / p95 / clip / acts-like and a
before / after table, `strips` the before / after / Camera Raw strips. Findings: channels above raw white must reach the
tone table (Exposure +3 clips as Camera Raw does); Whites + is `exp(c · w^0.55)` stronger than the ramps' table on a photo
whose 99th percentile is low (`tone::lr::whites_gain`, Sony's is high: unchanged); Contrast's pivot follows the key
at 0.35 EV per EV from -1.82 (`lr::CONTRAST_PIVOT`). Contrast ± still acts like CR 70-85 %: the ramp tables' shape, not
only the pivot, differs (a plain gain on the shift made every raw worse).

Round 14 (negative Dehaze, Shadows +): `make_scene.py SET [PHOTOS.json [short]]` (clones of the study raws, or `PHOTOS.json`'s, with
one-slider sidecars for Shadows / Dehaze / Highlights; `short` = the sliders' ends only), rendered by `hdr_ps.sh SET`, then
`scene2_study.py lc NAME BIN` / `cmp NAME…` (per raw: dE, p95, "acts like", per-slider means) / `strips NAME…`
(`compare/2026-11-25/fix-scene2/`); `scene_curve.py`, `scene_ana.py`, `scene_fast.py` read one build's gain by level.
Switches: `LIGHTCRAFT_DZ_KEYPCT=<percentile>` (negative Dehaze's key), `LIGHTCRAFT_HS_SUP=<scale>` (Shadows +).
Findings: negative Dehaze is `(1 + s) · in + (-s) · A` near the brightest level and moves the shadows' chroma to the airlight's
(λ ≈ 0.95 |s|^1.2); the key is a high but not the highest percentile (95); Shadows + has no photo statistic that predicts its gain.

Round 15 (Dehaze, 2026-11-27): `dehaze3_study.py prep|lc NAME BIN|cmp NAME…|tab BUILD…|strips BUILD… OUT` (the study's four raws at
Dehaze -100…+100 and eight held-out raws at -100/-50; per raw dE / p95 / highlight-detail ratio / acts-like, before/after table, strips in
`compare/2026-11-27/fix-dehaze3/`). Findings: detail share 1+0.7 s, lambda 0.78 |s|^1.4, key shift 0.4 (key+2.6).
Positive airlight at the 99.5th percentile was tried and held back (breaks `tests_crop_tone`; `docs/lr-match.md` → Round 15).

Round 16 (Shadows + by level, 38 raws): measurement switches `LIGHTCRAFT_HS_SPREAD=<k>` / `LIGHTCRAFT_HS_GATE=<spread>` (positive Shadows adds
k × the rule's four networks' spread above the gate), `LIGHTCRAFT_HS_CEIL=<EV>` (highest level the rule reads, default +2), with
`LIGHTCRAFT_HS_SUP` (scale). Extra raws are made like `make_scene.py`'s (APFS clones with one-slider sidecars, Shadows +50 / +100, rendered
by `hdr_ps.sh`); every candidate is ~4 GB of float renders for 38 raws, so delete a build's renders after reading its table.
