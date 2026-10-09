# Matching Lightroom Classic's renders

For someone moving a Lightroom Classic catalog over, "right" means "looks like it did in Lightroom".
This page is how LightCraft measures that, what was changed because of the measurements, and what
still differs. Rounds 1–2 were black-box (Lightroom's *output* — the previews it keeps of the
user's own photos — compared with ours). Since round 3 (Bryan's fork, AGENTS.md → *Fork rules*)
LightCraft renders raws on Adobe's own camera, look and lens profiles installed with Lightroom, read
at run time with Adobe's DNG SDK; nothing of Adobe's is copied into the repository, and everything
fitted from a user's catalog stays on that user's machine.

## Measuring: `tools/lr-compare`

Lightroom keeps a rendered preview of every photo it has shown (`<catalog> Previews.lrdata`:
`previews.db` maps image id → uuid + digest; each level is a `<uuid>-<digest>_<px>` JPEG in Adobe
RGB, stored in the file's own orientation, `ImageCacheEntry.orientation` AB/BC/CD/DA = quarter turns
to show it). Those are Lightroom's renders of the photo's develop settings, so they are ground truth.

`tools/lr-compare/lr_compare.py` (numpy + Pillow):

1. `prepare` copies the catalog (+ `-wal`) and `previews.db`, picks photos (`--ids` or a spec of
   camera / look / count), stores each photo's largest preview upright, and writes a
   `migrate-lightroom` records file holding only those photos;
2. `migrate` migrates them into a scratch library (never the user's);
3. `render` exports each at its preview's size (≤ `--max-size`);
4. `compare` aligns our render to the preview (scale / rotation / shift search on gradients),
   converts both to CIELAB (Adobe RGB and sRGB from their published primaries and curves) and
   reports per photo: mean and 95th-percentile CIEDE2000 at 512 px, mean CIEDE2000 of 64-pixel
   blocks at 128 px (`dE128`: colour without sharpening / noise / sub-pixel differences), mean ΔL*,
   Δa*, Δb* overall and per tone band, and photos whose geometry differs (`geom`). `--strip` writes
   Lightroom | before | after side by side.
5. `calibrate` fits Lightroom-matched camera profiles (below) from the catalog.

## What the measurements showed, and what changed

Sample: 167 photos from Bryan's catalog (Leica M Typ 262 DNG, Ricoh GR III DNG, Canon R6 CR3,
Fujifilm X-T2 / X100F RAF, iPhone JPEG / ProRAW; Summer Fields, Nautica, Adobe Color, none; most
with his heavy presets), in four sets: `w1` (38, mixed), `w3` (27, Lightroom default settings),
`w4` (18, Summer Fields with few edits), `w5` (84, edited; the set the knobs below were tuned on).
None of them were used to fit the camera profiles.

| Problem found | Measured | Change |
|---|---|---|
| Creative looks (Summer Fields 4,220 photos, Nautica 870) not applied at all | Summer Fields photos ΔE 14.3 | RGB colour tables decoded and applied (`crs_table.rs`, `lut.rs`), matched by name on migration / `library.rematchProfiles` |
| Base camera rendering ≠ Lightroom's | default-setting photos ΔE 7.4 (Leica 9.9) | Lightroom-matched camera profiles fitted from the catalog: `calibrate --lightroom` (matrix, hue/sat table, tone + chroma curve; log-binned tone fit down to deep shadows) → ΔE 2.6 |
| Fujifilm DR200/DR400 shots 1–2 EV off | DR100 photos +15 L* brighter than DR200 | RAF raw exposure bias (record `0x9650`) as baseline exposure |
| CR3 / RAF custom white balance read relative to 6500 K | Summer Fields R6 photos Δb* −6…−12 (too blue) | camera white-balance model per camera from as-shot photos and Lightroom's as-shot Temp/Tint (also mined from develop history) |
| Basic tone sliders far from Lightroom's | neutral-pixel residual 1.24 EV | Lightroom-matched Exposure / Contrast / Highlights / Shadows / Whites / Blacks (`tone::lr`): EV moves per slider by scene EV, fitted on neutral pixels of ~100 photos → 0.61 EV |
| Where the look table sits | after the tone curves: ΔE 7.0 vs right after the base tone map: 7.8 | tables apply to the finished colour, after the curves |
| Saturation / mixer saturation too weak, parametric curve too strong | small (−0.25 ΔE) | `SATURATION_GAIN` 1.25, parametric amount 0.22 → 0.11, wider regions |
| `crs:CropAngle` read with the wrong sign | straightened crops 2× their angle off | negated; `library.repairLightroomMigration` fixes migrated libraries |
| Stale XMP sidecars overriding photos Lightroom shows unedited | two default iPhone JPEGs ΔE 11.4 / 6.0 → 1.0 / 1.4 | the catalog wins on migration; repair command for migrated libraries |

Before → after (mean CIEDE2000 per photo at 512 px; `dE128` in brackets):

| | photos | before | after |
|---|---|---|---|
| Canon EOS R6 | 41 | 9.5 | 6.0 (5.3) |
| Leica M (Typ 262) | 40 | 12.2 | 7.7 (6.9) |
| Ricoh GR III | 42 | 11.6 | 6.7 (6.1) |
| Fujifilm X-T2 | 20 | 10.1 | 5.4 (4.7) |
| Fujifilm X100F (no default photos to calibrate on) | 14 | 19.7 | 9.5 |
| iPhone 13 Pro Max (JPEG) | 9 | 9.5 | 8.4 |
| Summer Fields | 95 | 14.3 | 8.0 (7.4) |
| Nautica | 12 | 9.7 | 9.0 (8.3) |
| Adobe Color | 52 | 8.1 | 5.0 (4.4) |
| default settings (`w3`) | 27 | 7.4 | 2.6 (2.1) |

## What still differs, and why

- **Heavily edited photos are still ~7 ΔE off on average.** A per-photo global colour transform
  (cubic in Lab) fitted to each photo would bring our render to ~2.8: so about 4 of the 7 are
  global but photo-dependent colour/tone differences, and ~3 are spatial. The global part is
  Lightroom's slider behaviour that a fixed per-slider curve doesn't capture: Process Version 2012+
  tone is image-adaptive (the residual tone bias correlates with the photo's own brightness,
  r ≈ 0.3–0.4) and its Highlights/Shadows are local; its HSL, Saturation and Calibration work in a
  different space from ours (OkLCh). Each needs its own reverse engineering from these previews.
- **Spatial**: Lightroom applies lens profiles (vignetting, distortion) LightCraft has no clean-room
  data for (Canon R6 corners −6 L*), masks are approximated, local tone differs.
- **Per camera**: edited GR III photos come out ~0.35 EV brighter and X-T2 ~0.2 EV darker than
  Lightroom (the slider fit is shared by all cameras).
- **Not handled**: iPhone ProRAW (`ProfileGainTableMap` not applied; renders far too dark),
  iPhone HDR JPEGs with presets (Lightroom's incremental white balance and tone on these differ),
  Adobe Monochrome photos, X100F (no default-setting photos to calibrate on), looks whose table is
  a hue/saturation `LookTable` (Glacial, North Star, Salt Flats also carry one).
- **The Adobe Color / Adobe Standard split**: profiles are fitted to Adobe Color photos (the default);
  creative looks sit on Adobe Standard in Lightroom. Removing the Color look's curve made things
  worse, so the same base is used for both.

## Round 2 (2026-10-09)

### The ground truth was partly wrong

`previews.db` keeps, for photos Lightroom never rendered, the **camera's embedded JPEG**
(`Pyramid.quality` `embedded` / `thumbnail` / `bigThumbnail`, sRGB). Round 1 read those as
Lightroom's Adobe RGB renders: 34 of its 167 test photos, all 27 "default settings" photos, and
the calibration sets of the GR III, X-T2 and most of the Leica (5,223 of its 5,383 default photos
only have the camera JPEG). `lr-compare` now takes only previews Lightroom rendered from the
current settings (quality `smallRender` / `standard` / `final` / `full` and the preview digest =
the develop-settings digest), records `previewQuality` / `previewSpace` / `lrRender` in
`sample.json`, and `compare` skips the rest (`--all-previews` decodes them in their own space).
A new held-out set `w6` has 40 default-setting photos Lightroom actually rendered.

### Method

Report set: the 166 round-1 photos (127 with a real Lightroom render) + `w6`, split by a fixed
hash stratified by camera and look into **A** (63, may be fitted on) and **B** (103, held out;
all of `w6`). Fitting also uses 335 extra edited photos (`t1`), 45 B&W photos (`t2`), 6 no-grain
Leica defaults with big previews (`w7`) and 60 R6 defaults with lens corrections (`w8`); none of
them is in B. Analysis: a debug build dumps the per-pixel pipeline state (scene colour, base,
tone output, mask alpha, image coordinates) next to Lightroom's preview; tone targets come from
a linearised per-pixel inversion (ΔL* over our own dL*/dEV).

### What changed (held-out split B, mean ΔE2000 / p95 / photos passing)

| change | effect |
|---|---|
| calibrate only on real Lightroom renders (Leica, R6) | default-setting photos (w6) 3.00 → 2.34, Leica 6.41 → 5.53 |
| `calibrate` carries the Leica look onto cameras with no default renders, on their own DNG matrices (`Pool::transfer`; GR III, iPhone ProRAW); `--same-sensor X100F=X-T2` | GR III 7.42 → 5.68, iPhone 13 Pro ProRAW 22.5 → 11.6, X100F 9.8 → 8.1 |
| Highlights / Shadows relative to the photo's key (`tone::lr::image_key`), all six tables refitted (twice) on real renders | B 5.28 → 4.93 (first fit), 4.63 → 4.51 (second) |
| per-camera white-balance map (`lr-compare wbmap`, `wb_map` in the profile) | X-T2 8.09 → 7.12, B 4.93 → 4.83 |
| detail: sharpening radius in source pixels (`finish::Sharpen`), fine film grain in source-pixel cells, halo-free Highlights/Shadows (`finish::halo_weight`), colour NR strength | 1:1 metrics below; B 4.83 → 4.71 |
| tight crops decode enough of the original (`media::source_edge_needed`) | no upscaled preview in the loupe |
| lens vignetting per camera + lens (`lr-compare lensfit`, `lenses` in the profile) | R6 4.26 → 4.09, passing 0 → 4 |
| B&W mix from the unadjusted colour, gain fitted on `t2` | t2 4.80 → 4.57, L1004995 17.2 → 11.2 |
| 8-bit rendered files through a reference curve, so Basic tone acts on them as on raws | iPhone JPEGs 8.49 → 7.22 |

| group | n | round 1 ΔE / p95 / pass | round 2 ΔE / p95 / pass |
|---|---|---|---|
| split A (fit) | 63 | 8.05 / 16.3 / 3 | 6.23 / 13.3 / 3 |
| **split B (held out)** | 103 | 5.88 / 13.5 / 2 | **4.46 / 10.7 / 21** |
| all | 166 | 6.70 / 14.6 / 5 | 5.13 / 11.7 / 24 |
| B · Canon EOS R6 | 33 | 4.27 / 11.3 / 0 | 3.79 / 9.7 / 4 |
| B · Leica M Typ 262 | 34 | 4.99 / 12.0 / 1 | 3.35 / 8.6 / 16 |
| B · Ricoh GR III | 18 | 7.41 / 18.2 / 0 | 5.30 / 14.7 / 0 |
| B · Fujifilm X-T2 | 6 | 7.87 / 13.6 / 0 | 7.04 / 12.3 / 0 |
| B · Fujifilm X100F | 7 | 8.44 / 15.0 / 0 | 6.39 / 12.8 / 0 |
| B · iPhone 13 Pro (ProRAW) | 1 | 22.55 / 31.7 / 0 | 9.92 / 13.0 / 0 |
| B · iPhone 13 Pro Max (JPEG) | 4 | 8.29 / 14.4 / 1 | 6.88 / 12.9 / 1 |
| B · Adobe Color | 50 | 3.85 / 8.7 / 1 | 2.84 / 6.8 / 20 |
| B · Summer Fields | 46 | 7.69 / 17.5 / 0 | 5.84 / 13.9 / 0 |
| B · Nautica | 3 | 11.95 / 25.9 / 0 | 10.15 / 24.9 / 0 |
| B · default settings | 40 | 2.98 / 7.0 / 1 | 2.16 / 5.4 / 19 |
| B · edited | 63 | 7.73 / 17.5 / 1 | 5.92 / 14.1 / 2 |

### Detail at 1:1 (`lr-compare detail`)

Band-pass L* energy at the preview's full size (28 photos with previews ≥ 2,400 px; ratio to
Lightroom, 1 = same): fine texture in flat areas (grain, noise) 0.79 → 0.98, mid-scale mottling
1.84 → 1.35, edge rims (halos) 1.13 → 0.81, chroma noise 1.6 → 0.87. Round 1's sharpening
used the 7 px texture band (Radius ignored) and drew dark rims and an "HDR" look at Bryan's
Sharpening 126; its grain was smooth value noise in 3.5 px cells (blotchy); colour NR 25 left
twice Lightroom's chroma noise. Regression test: `highlights_and_shadows_leave_no_halos_at_edges`.

### What still misses, and why

- **Edited photos are still ~6 ΔE off on average and almost none pass p95 ≤ 5.** Per pixel,
  a per-photo exposure offset (sd ~0.35 EV) is only about half explained by the slider model:
  Lightroom's Basic tone is image adaptive beyond the image key we model. Most of his presets
  share one slider set, so the six tables are poorly separable. p95 is dominated by local
  structure: masks (approximated), Lightroom's local tone operator (edge contrast at mid scale is
  ~20 % lower here), heavy grain (random, never pixel-identical) and geometry (lens distortion is
  not corrected for CR3/RAF: the alignment search absorbs scale, not barrel shape).
- **Nautica on iPhone JPEGs** (~16 ΔE): the base JPEG and Lightroom agree in brightness; the gap
  is the Nautica table / incremental white balance on rendered files.
- **X-T2** keeps round 1's profile (fitted to its camera JPEGs; RAF has no colour matrices to
  carry a look onto). X100F uses it too.
- **Extreme white balance** (46,000 K, tint −105) and Adobe Color's own look table on edited
  photos remain far off; **merged panoramas** (16-bit TIFF from macOS) keep the display path.
- R6 tone was calibrated on lens-corrected previews, so its curve partly includes the vignetting
  correction that `lensfit` now also applies (default R6 photos read ~1 L* bright).
- Leica Q2 (219 photos) and Canon 5D Mark II (172) are not in the sample: `calibrate` gives the
  Q2 the transferred look automatically; the 5D is unmeasured.

## Round 3 (2026-10-09): Adobe's own profiles through the DNG SDK

Bryan's fork dropped the clean-room rule, so the base rendering is now Adobe's, not a fit of it.

### What is Adobe's now

- **Adobe DNG SDK 1.7.1** (`crates/dng-sdk-sys`, C++ built by `cc` from `vendor/dng_sdk_1_7_1`,
  fetched from `download.adobe.com` by `tools/fetch-dng-sdk.sh`, sha256-pinned; the zip carries no
  licence of its own, so it is never committed). A C ABI shim catches every exception; without the
  SDK (other OS, wasm, not fetched) every call reports "unavailable" and the old path runs.
- **Camera colour** (`crates/engine/src/adobe.rs`): the camera's `Adobe Standard` DCP from
  `/Library/Application Support/Adobe/CameraRaw/CameraProfiles/Adobe Standard/` (1,470 installed;
  R6, M Typ 262, GR III, X-T2, X100F, Q2, 5D Mark II all present). `dng_color_spec` gives the
  camera → XYZ D50 matrix for the as-shot white (dual-illuminant interpolation, forward matrices,
  `AnalogBalance`), `HueSatMapForWhite` the interpolated 90×30 hue/sat map (applied at decode in
  linear ProPhoto before exposure, our `HsvTable` verified against `RefBaselineHueSatMap` to
  2e-4), `dng_temperature` the as-shot Temp / Tint. Lightroom's per-camera fitted profiles in
  `~/Library/Application Support/LightCraft/camera-profiles/` are no longer used for cameras with
  an Adobe DCP (they stay as the fallback, e.g. iPhone ProRAW).
- **Tone** (`crates/pipeline/src/adobe.rs`, CPU and GPU): the DCP's own `LookTable` (36×8×16,
  every Adobe Standard has one), the exposure ramp's `DefaultBlackRender` black (Shadows 5 =
  0.005), then the `ProfileToneCurve` or the ACR3 default curve applied as `RefBaselineRGBTone`
  (largest and smallest channel through the curve, the middle interpolated) in ProPhoto — the
  order of `dng_render`. Measured against the SDK's own reference render of Leica defaults first:
  `dng_render` with Adobe Standard alone already gave 13/19 passing Lightroom's previews.
- **Looks**: Adobe Color, Monochrome, Neutral, Portrait, Landscape, Vivid are read from
  `…/CameraRaw/Settings/Adobe/Profiles/Adobe Raw/*.xmp` (their `LookTable` decoded by the SDK's
  big-table code, `ToneCurvePV2012` composed under the user's curves, hidden settings added) and
  used as profile ids `adobe:<name>`; `lc.color` on a raw with an Adobe base renders as Adobe Color
  (Lightroom's default). Migration maps `CameraProfile` / `Look` to them; creative looks (Summer
  Fields, Nautica) keep their RGB tables on the Adobe Standard base, now with the table's own
  gamut handling (clip vs extend) as the SDK applies it (our decode matches
  `dng_rgb_to_rgb_table_data` to 1e-3 in gamut).
- **White balance**: Temp / Tint → xy (`dng_temperature`) → the profile's camera → PCS matrix for
  that white, as one working-space matrix (exact, apart from the hue/sat map staying at the
  as-shot white).
- **Lens profiles** (`crates/engine/src/lcp.rs`): files without lens opcodes take the lens's Adobe
  `.lcp` (system / user / Lightroom's bundled `LensProfiles/1.0`), interpolated in focal length,
  aperture (APEX) and focus distance, as `WarpRectilinear` + `FixVignetteRadial` (what the DNG
  Converter does); the migration now carries Lightroom's lens-correction default for unedited
  photos. R6 defaults: corner ΔL* −4…−10 → within ±1.5.

### Fitted on top of the exact base

Basic tone (`tone::lr::ADOBE_*`) refitted with a per-pixel dump of the pipeline
(`LIGHTCRAFT_LR_DUMP`, `crates/pipeline/src/dump.rs`) and Lightroom's previews: Lightroom's default
tone over the ACR3 curve (`ADOBE_BASE`) on default-setting photos only, then the six sliders on
edited ones (split A + training photos, B held out); two iterations. Per-camera exposure offsets
(Camera Raw's per-camera baseline and white levels live in its binaries): R6 +0.13 EV at ISO 100,
+0.22 above (its ISO 100 white level differs), GR III +0.03, X-T2 −0.53 (−1.06 from ISO 500,
DR400), X100F −0.37; `adobe-exposure.json` in the config folder can override them.

### Results (held-out split B; mean ΔE2000 / p95 / photos passing ΔE ≤ 2 & p95 ≤ 5)

| group | n | round 2 | round 3 |
|---|---|---|---|
| split A (fit) | 63 | 6.23 / 13.3 / 3 | 5.71 / 12.6 / 3 |
| **split B (held out)** | 103 | 4.46 / 10.7 / 21 | **4.46 / 10.7 / 23** |
| all | 166 | 5.13 / 11.7 / 24 | 4.94 / 11.4 / 26 |
| B · Canon EOS R6 | 33 | 3.79 / 9.7 / 4 | 3.82 / 10.4 / 5 |
| B · Leica M Typ 262 | 34 | 3.35 / 8.6 / 16 | 3.51 / 8.5 / 17 |
| B · Ricoh GR III | 18 | 5.30 / 14.7 / 0 | 5.42 / 14.3 / 0 |
| B · Fujifilm X-T2 | 6 | 7.04 / 12.3 / 0 | 7.45 / 12.3 / 0 |
| B · Fujifilm X100F | 7 | 6.39 / 12.8 / 0 | 4.94 / 10.3 / 0 |
| B · Adobe Color | 50 | 2.84 / 6.8 / 20 | 2.76 / 7.2 / 22 |
| B · Summer Fields | 46 | 5.84 / 13.9 / 0 | 5.91 / 13.4 / 0 |
| B · Nautica | 3 | 10.15 / 24.9 / 0 | 11.13 / 25.8 / 0 |
| B · default settings | 39 | 2.19 / 5.5 / 18 | 2.20 / 6.0 / 20 |
| B · edited | 64 | 5.84 / 14.0 / 3 | 5.84 / 13.5 / 3 |
| B&W set `t2` | 45 | 4.57 | 4.32 (6 pass) |

The base is now right by construction (and no longer needs a Lightroom catalog to calibrate a
camera: any camera with an Adobe Standard DCP works), but the totals barely move: round 2's
per-camera fits had absorbed much of the base error, and what remains is Lightroom's Basic-tone
operator on heavily edited photos.

### What still misses, and why

- **Edited photos (~5.8 ΔE)**. With the base exact, the per-pixel tone error on edited photos is
  ~3.7 L* (held out); even a perfect per-photo exposure offset would only take it to ~3.0, and that
  offset is not predictable from the sliders or image statistics (R² < 0 held out). Lightroom's
  PV2012 Highlights / Shadows / Whites / Blacks are local and image-adaptive in ways a per-slider
  table can't express, and Bryan's presets use extreme values (Contrast −100, Shadows +100,
  Exposure −2.7). Tried and rejected on held-out data: white/black-point-relative Whites / Blacks,
  percentile-relative Highlights / Shadows, the profile table right after the base curve (B 5.00
  vs 4.47), calibration on the camera's own primaries in xy (B 4.54–4.95 vs 4.47).
- **Calibration** (Summer Fields uses Green Hue +44, Blue Sat +38): ours moves Rec.2020 primaries
  in OkLCh; greens / teals still drift (the X100F hot-spring water renders brown, not teal).
- **Grain and local contrast at 1:1** (`lr-compare detail` on the 28-photo full-size set, ratio
  to Lightroom): grain made finer (cells `0.5 + 0.03·Size` source px, roughness's coarse octave at
  1.6× instead of 2.3×, weight 0.35 instead of 0.7): mottle 1.39 → 1.23 (training photos), 1.87
  → 1.75 (w1), fine 0.95–1.0; the L1007499 face crop is visibly closer. Still: edges ~0.8
  (flatter at 1:1), mottle > 1 even without grain (part of it may be Lightroom's preview JPEG
  quantisation), chroma noise 0.76–1.33.
- **X-T2**: its Fujifilm raw exposure bias isn't what Camera Raw compensates (per-photo residuals
  +0.3 EV at DR100, −0.45 EV at DR400 before the ISO split); colour still ~4 b* blue on the six
  held-out photos.
- iPhone ProRAW (no Adobe Standard DCP matched: `Apple Embedded Color Profile`) and JPEGs keep
  the round-2 path.
- The SDK's opcode lists (`WarpRectilinear`, `GainMap`) are not used for pixels yet: LightCraft's
  own opcode code applies DNG lens corrections; ProRAW `ProfileGainTableMap` is still missing.

Next: a controlled oracle for Lightroom's sliders (Camera Raw in Photoshop rendering synthetic
DNGs with known settings, if Bryan agrees to it being scripted) would replace the regression on
previews with exact measurements of PV2012.

## Round 4 (2026-10-09): Camera Raw as an oracle

Bryan approved scripting Camera Raw in Photoshop 2026 (AppleScript `do javascript`, no GUI input;
`tools/lr-compare/oracle/`). Synthetic LinearRaw DNGs with real camera colour (Adobe Standard DCP
matrices) and known settings in their XMP, rendered by Camera Raw and by LightCraft
(`oracle_render`), compared pixel by pixel; also Bryan's real raws (training photos only) with
one-slider sidecars.

What the oracle showed:
- **Exposure, Whites, Blacks are global per-pixel curves** (a thin probe ramp renders the same on
  any background), non-linear in the slider value and applied in sequence (Exposure first).
  Measured as EV-shift tables at 8–12 slider values (`crates/pipeline/src/tone_adobe.rs`): on the
  ramps LightCraft now matches Camera Raw to ΔE ~0.1 per slider, 0.1–0.9 for pairs/triples.
- **Contrast is global but image-adaptive**: its pivot moves ~0.21 EV per EV of the image's key.
- **Lightroom's default tone** differs from the DNG reference: no "Shadows 5" black ramp (deep
  shadows were up to 2 L* too dark) — now the measured `BASE` table.
- **Highlights / Shadows are local and image-adaptive**: no effect at all on a uniform image; a thin
  bright line on a dark background is pulled almost to the background at Highlights −100. Fits on
  real raws with one slider each did not transfer to his combined presets (B 4.22 → 4.33), so
  H/S stay fitted on previews on top of the exact global tone.

Tone alone (4a): held-out B 4.22 / 10.4 / 24, but the grid photos' colour still lost to round 2.
Then the colour (4b), on a dense ProPhoto HSV grid (36 hues × 10 saturations × 13 levels,
`make_grid.py`) and a chart, every Calibration / HSL / Saturation / Vibrance slider at ±50 / ±100:
- **Calibration is a primaries matrix in linear ProPhoto** (fitted through Camera Raw's own base
  inverted on the grid, `inv.py` / `calfit2.py`): each slider moves its primary's column to
  `e_i + a·e_next + b·e_prev`, columns rescaled to keep white; hue ≈ ±0.16 per 50, saturation
  ≈ ∓0.18 per 50, offsets add (`colorops::CALIB_TABLE`). Summer Fields' calibration on the grid:
  ΔE 2.64 → 1.55.
- **HSL, Saturation, Vibrance** act on the finished colour as smooth functions of its OkLCh hue,
  lightness and chroma; measured as tables (36 × 5 × 4, Δhue / ln chroma ratio / ΔL, per slider
  stop; `hslfit.py`, `gen_colortab.py` → `crates/pipeline/src/colortab.bin`, 126 KB) and summed per
  render (Camera Raw's own combinations match the sum: his HSL preset 1.89 → 0.38 in the model,
  0.77 in LightCraft; Saturation −30 1.39 → 0.82). Used for Adobe camera bases only.
- **Custom white balance moves twice as far** as the DNG colour spec says, for every camera tried
  except the Leica M (Typ 262) (real CR3 / RAF / GR III DNG and synthetic DNGs; `make_rwb.py`,
  `make_wb.py`): the neutral is `shot · (sdk / shot)^k`, k = 2 (Leica 1), chosen per camera on split A
  (`engine::adobe::WB_STRENGTH`). Mechanism unknown; extreme Temp (34 600 K) is still short on X-T2.
- X-T2 exposure bias: split A dL +0.2 — no change needed.

| | round 2 (jc6) | round 4a (o3) | **round 4 (o5)** |
|---|---|---|---|
| held-out B | 4.46 / 10.7 / 21 | 4.22 / 10.4 / 24 | **3.95 / 10.1 / 24** |
| all 166 | 5.13 / 11.7 / 24 | 4.69 / 11.2 / 28 | **4.31 / 10.8 / 28** |
| B · edited | 5.84 / 14.0 / 3 | 5.43 / 13.0 / 4 | 5.00 / 12.6 / 4 |
| B · Summer Fields | 5.84 / 13.9 / 0 | 5.36 / 12.7 / 1 | 4.88 / 12.3 / 1 |
| B · R6 / Leica / GR III | 3.79 / 3.35 / 5.30 | 3.82 / 3.10 / 4.88 | 3.46 / 2.94 / 4.69 |
| B · X-T2 / X100F | 7.04 / 6.39 | 7.30 / 4.85 | 7.27 / 4.06 |
| L1007499 | 2.72 / 5.3 | 4.00 / 7.2 | **2.68 / 6.2** |
| R0001321 | 6.07 / 34.5 | 6.14 / 33.5 | 5.66 / 33.1 |
| _DSF4237 | 3.13 / 5.6 | 4.46 / 8.8 | **2.45 / 4.6** |
| IMG_4270 | 15.86 / 22.0 | 15.86 / 22.0 | 16.07 / 21.5 |
| L1004995 | 11.18 / 33.6 | 10.16 / 30.6 | 10.97 / 32.2 |

Not done: Highlights / Shadows and Clarity / Texture / Dehaze against spatial targets (they stay
fitted on previews); the WB mechanism; Nautica (3 photos, ~10).
