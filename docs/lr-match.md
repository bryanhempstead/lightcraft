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
- **Not handled**: iPhone ProRAW's `ProfileGainTableMap` applies only with the file's own rendering, not per photo's profile (round 7),
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
  own opcode code applies DNG lens corrections; ProRAW `ProfileGainTableMap`: round 7.

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

## Round 5 (2026-11-21): export and display colour, fresh held-out numbers

Fresh held-out set `w9` (103 photos, seed 1121, real Lightroom renders only; 96 edited, 7 default —
round 4's B had 40 defaults, so the totals are not comparable one to one). Current build:
**4.99 / 12.8 / 8 passing** (edited 5.26 / 13.5 / 1; defaults 1.29 / 3.2 / 7 of 7). Per camera:
R6 5.79, Leica M 4.39, GR III 4.33, X-T2 5.66, X100F 4.21, iPhone JPEG 6.96.

**Export** (`tools/lr-compare/export_check.py`: every space × JPEG / PNG 8/16 / TIFF 8/16 against a
32-bit linear Rec.2020 render, decoded through each file's own ICC profile, cross-checked with
LittleCMS and ColorSync): profiles carry the published primaries (≤ 1e-5) and curves exactly, and
both CMMs read them like we do (8-bit quantisation only). But the tone curves, a creative
profile's table and grain acted on the *output* primaries, so an Adobe RGB / P3 / ProPhoto export
of a curved photo differed from the sRGB one (16-bit: mean 0.1–0.3, p99 up to 2.4 ΔE in gamut).
They now act in one space for every output (ProPhoto primaries, sRGB curve — Lightroom's
Melissa RGB; `finish::STYLE_SPACE`, CPU and GPU), and Refine Saturation keeps luminance in linear
light. 16-bit exports now agree with the reference to ΔE 0.001–0.003 in every space. Fidelity
(sRGB exports): 4.99 → 4.97, R6 5.79 → 5.74, other cameras within ±0.03. Test:
`exports_in_every_space_agree_in_colour_with_curves_and_grain` (fails on the old code: 0.093).
Still differs from Lightroom's files: our profiles are ICC v4 (Lightroom embeds the v2 "sRGB
IEC61966-2.1" and "Adobe RGB (1998)", v4 Display P3); no Exif `ColorSpace` tag.

**Display**: the window's Metal layer is untagged (egui-wgpu asks wgpu for `Srgb`, which wgpu-hal
maps to a nil `colorspace`), so macOS does not colour-match it: a window capture on the Studio
Display shows the theme's sRGB accent (1, 101, 221) unchanged in display space, where a managed
sRGB value would be (41, 100, 213). The Develop view's sRGB render is therefore shown as display
(≈ P3) values: more saturated than the export. Fix needs a P3-tagged surface (egui-wgpu has no
option) + rendering the view in Display P3.

**Biggest gap found**: lens profiles chosen by name in Lightroom are not used.
`lcp::for_raw` matches the Exif lens only, and fisheye profiles are skipped: 213 photos use the
Sigma 15 mm diagonal fisheye profile (adapted lens, Exif "15.0 mm") — R6 default renders with it
are 11.0 ΔE (geometry: Lightroom de-fishes and crops), the other R6 defaults 2.7; 469 Leica M
photos use "Voigtlander VM 50mm f1.5 Nokton" by name (Exif lens "not selected").

## Round 6 (2026-11-21): lens profiles Lightroom chose, export profiles like Lightroom's

**Lens profiles by name.** Lightroom's `LensProfileName` / `LensProfileFilename` /
`LensProfileDigest` (+ `Setup`) now come over with the migration, XMP sidecars and presets as
`optics.lens_profile_ref`, and `library.rematchLensProfiles {dryRun?, force?}` adds them to a
library migrated earlier (one undo step). Renders resolve the installed `.lcp` by file name
(the copy whose MD5 is the digest when Camera Raw and Lightroom both hold one), else by profile
name; a profile that isn't installed falls back to the file's own lens match. New in `lcp.rs`:
manual-lens profiles (no `FocalLength`: the Voigtländer on the M262) and **fisheye models**:
`r_d = θ(1 + k1 θ² + k2 θ⁴)` de-fished to a rectilinear view (`EmbeddedWarp::fisheye`, CPU +
GPU, interval-bounded coverage). Lightroom's output scale, fitted on six R6 Sigma 15 mm renders
(fx 0.4185, scale 0.7641, corr 0.996 on all six), is "as little zoom as fills the frame": the
short side's edges land on the photo's edges (our rule: 0.7636). The profile's Vignetting amount
scales the model (`1 + a·(V − 1)`), not the gain: on the Voigtländer set (amount 156) corners went
−2.4 → −0.2 L* and the mean 4.51 → 4.37, on both halves of the set (linear 4.79 / 4.12, gain^a
4.74 / 4.03, model 4.68 / 3.92). Amount 100 is unchanged.

| set (held out) | before | after |
|---|---|---|
| Sigma 15 mm fisheye, R6 (`fe`, 19, not the 6 fitted) | 6.23 / 20.9 / 0 | **2.28 / 5.3 / 7** |
| r6d (37 R6 defaults) — its 6 fisheye photos | 4.01 / 11.9 / 4 — 11.01 / 34.4 / 0 | **2.52 / 7.0 / 9** — 1.77 / 4.1 / 5 |
| Voigtländer 50 mm on M262 (`vg`, 49, files read from the LOT48 backup) | 6.73 / 18.2 / 0 | **4.37 / 13.7 / 0** |
| w9 (103) | 4.97 / 12.8 / 8 | 4.96 / 12.7 / 8 (R6 5.74 → 5.73, X100F 4.16 → 4.07) |

**Export profiles.** sRGB and Adobe RGB exports embed ICC **v2.1** profiles generated from the
published specs (`icc::write_v2`: "sRGB IEC61966-2.1" with a 1024-entry curve, "Adobe RGB
(1998)" with gamma 0x0233, D50 Bradford colorants, D65 `wtpt`; nothing copied from HP's / Adobe's
files); Display P3 / ProPhoto / Rec.2020 stay v4. Exif `ColorSpace` = 1 for sRGB, 0xFFFF
otherwise; Make and Model are written apart (`export::split_camera`). `export_check.py`: 20 files,
0 failures (16-bit ΔE ≤ 0.004, LittleCMS / ColorSync agree); `sips -g profile` reads the names.
TIFF exports still carry no Exif IFD (Make / Model only through XMP).

**Adobe Color default.** A raw imported fresh (no sidecar) gets `lc.color`, which on a camera
with an Adobe Standard DCP renders as Adobe Standard + the Adobe Color look: pixel-identical to
setting `adobe:Adobe Color` (R6 CR3 checked). The profile browser names it "Color", not "Adobe
Color"; lens corrections start off unless the file has embedded ones.

## Recipes, curves and the process version (R01–R23, 2026-11-21)

### The acceptance set and its gate (R01)

`lr_compare.py gate --work W --tag T [--base T0] [--split B]` reports every *recipe* — look ×
camera × Lightroom process version — plus edited / default and PV5 / PV6, with mean / p95 ΔE,
`dE128`, mean ΔL* and photos passing (ΔE ≤ 2 and p95 ≤ 5); with `--base` it exits 1 when a cell of
≥ 3 photos gets worse by more than 0.15 ΔE, the overall mean by more than 0.03, or fewer photos
pass. Every sampled photo now carries its process version, a fixed fit / held-out split (`A` / `B`,
a hash of its catalog id), its top-level settings and point curves (`prepare` and the new
`assemble`). `tools/lr-compare/acceptance.json` is the recipe spec for `prepare` (Summer Fields /
Nautica / Adobe Color × M262 / R6 PV5 + PV6 / GR III / iPhone JPEG + ProRAW, plus Monochrome, Q2,
X-T2, X100F). `lr_compare.py hues` breaks a run's colour error down by CIELAB hue (ΔL*, ΔC*, Δh°).

On 2026-11-21 the catalog's `Previews.lrdata` was no longer next to the catalog, and most
originals of the earlier sets live on the `HEMPSTEAD` drive (not mounted; its backup on `LOT48`
holds most of them). `assemble` builds a work folder from earlier prepared sets (their stored
Lightroom previews) and relocates originals (`--relocate OLD=NEW`): the acceptance set used below
is w9 + r6d + fe + vg, the 183 photos whose original is readable (137 edited; Summer Fields 106,
Adobe Color 72, Nautica 4; split A 91 / B 92). Only 7 of them are PV5, so PV5 vs PV6 is measured
with the oracle instead (below).

Baseline (main at ce413b5, CPU):

| recipe | n | ΔE | p95 | pass |
|---|---|---|---|---|
| all | 183 | 4.01 | 10.9 | 23 |
| edited / default | 137 / 46 | 4.59 / 2.29 | 12.4 / 6.3 | 5 / 18 |
| Summer Fields · M262 | 64 | 4.45 | 12.9 | 0 |
| Summer Fields · R6 | 20 | 5.82 | 13.7 | 0 |
| Summer Fields · GR III | 6 | 3.95 | 9.7 | 0 |
| Summer Fields · iPhone JPEG | 4 | 6.99 | 19.3 | 0 |
| Adobe Color · R6 | 64 | 2.96 | 8.0 | 16 |
| Adobe Color · M262 | 7 | 1.27 | 3.1 | 7 |
| Nautica · R6 | 4 | 5.46 | 17.2 | 0 |

### Process Version 5 vs 6 (R12)

Bryan's catalog holds 6,384 photos at `ProcessVersion` 11.0 (Process Version 5) and 10,439 at 15.4
(Version 6); LightCraft reads neither. The previews can't separate them (PV5 is nearly all older R6
work: 7 of the 183 acceptance photos), so the oracle rendered the same synthetic DNGs under both
(`oracle/make_pv.py`, `pvcmp.py`, `pvgrid.py`; colour grid + spatial scene; base, every Basic-tone
slider, Clarity, Texture, Dehaze, Vibrance, Saturation and his whole Summer Fields recipe):

- **Tone is identical**: on the neutral spatial scene PV6 vs PV5 is ΔE ≤ 0.01 for every slider,
  Highlights / Shadows / Clarity / Texture / Dehaze included.
- **Colour differs only for saturated, bright colours**: grid mean 0.27 (base) … 0.57 (recipe); by
  grid saturation 0.00 (neutral) → 0.13 (0.35) → 0.84–1.13 (0.8–0.9), by level 0.01 (−9 EV) → 0.56
  (0 EV); worst at violet-blue (hue 250–270°: PV6 up to +10 L*, less chroma — a highlight gamut
  change).
- LightCraft's base on the grid: 0.35 against PV5, 0.42 against PV6. Not worth a per-PV path; the
  photos' PV is kept in the acceptance set's cells so a PV-dependent change can be gated.

### Point curve (R09)

Camera Raw's point curve, read off a neutral ramp and a dense colour grid rendered by the oracle
(`oracle/make_curve.py`, `curvefit.py`, `curvecolor.py`; Bryan's curve 0,6 62,39 98,84 181,167
255,211, a second custom curve, an S-curve, a steep two-point curve, red and blue channel curves):

- **Interpolation**: the natural cubic spline through the points (C2, zero second derivative at the
  ends — the DNG SDK's `dng_spline_solver`), flat outside the end points and **clamped to 0..1**
  (the steep curve overshoots to 1.26 and below 0: Camera Raw clips). In sRGB-curve encoding of
  ProPhoto values it matches Camera Raw to 0.01–0.02 of a level on all four curves; LightCraft's
  monotone spline was 0.8–1.3 levels off on average (2.9 at worst on Bryan's), gamma 1.8 3–4.
- **Colour**: the master curve is applied **hue-preserving** (largest and smallest channel through
  the curve, the middle one interpolated — the DNG reference RGB tone), not per channel: on the
  grid 0.13 vs 0.65 ΔE (Bryan's curve), 0.15 vs 0.74 (S-curve); the R / G / B curves are per
  channel (0.00). The parametric curve comes first and joins the master curve (0.30 vs 0.92
  levels for the other order; colour 0.19 vs 0.67).

LightCraft now does exactly this (`spline::NaturalCurve`, `finish::Curves`, CPU and GPU; the
curve editor draws the same spline). Oracle renders of LightCraft against Camera Raw: Bryan's
curve on the grid 1.17 → 0.43 ΔE (the base itself is 0.42), with his parametric preset 1.26 →
0.46, ramps 0.24 → 0.05. Acceptance set: 4.01 / 10.9 / 23 → 4.02 / 10.9 / **25** (gate passes;
Summer Fields M262 4.45 → 4.45, X100F −0.2, R6 +0.06).

### Calibration (R10)

The oracle chart (`make_color.py`, 135 renders; `chartcmp.py` by hue): LightCraft's base is 1.58
ΔE per patch, Bryan's Summer Fields calibration (Red Hue +3, Green Hue / Sat +44, Blue Hue −19 /
Sat +38) 4.23: teals (180–210°) come out 18.5 C* too dull, blues (210–270°) 20° off in hue and
4–7 L* dark. Real Summer Fields photos (`lr_compare hues`, acceptance set) show the same: greens
to blues −3…−8.5° and −1…−4 C*, while Adobe Color photos are within ±1.5° everywhere. Fitting
the best white-keeping primaries matrix *at LightCraft's position* (scene-linear, after the
profile's hue/sat map) with LightCraft in the loop (`calfit_lc.py`) only reaches 3.27: a matrix
there can't reproduce it. Camera Raw's calibration edits the camera profile, i.e. acts before the
DCP hue/sat map and look table; moving it there (decode stage, `engine::adobe`) is the fix to try
next.

### Basic tone on edited photos (R02): what Camera Raw does, measured

Single sliders on synthetic scenes, LightCraft against Camera Raw (`make_pv.py`, `lccmp.py`,
`hsana.py`; R6 colour, ΔE2000): the global sliders are close (Exposure 0.56–0.65, Whites / Blacks
0.16–0.63, Contrast 0.6–1.05), the local ones are not — Shadows +69 4.56 (grid) / 1.88 (spatial),
Highlights −60 2.44 / 1.58, Texture +40 3.18 / 0.61, Clarity +40 1.55 / 1.69, Dehaze +30 0.89 /
1.31 — and Bryan's whole tone + colour recipe is 3.69 (grid) / 3.19 (spatial).
`make_hs.py` + `hsprobe.py` then took Highlights / Shadows apart (uniform fields, two-level fields
whose dark share is 10 / 50 / 90 %, the same field 1–2 EV brighter / darker, a thin probe ramp
across, and combinations with Exposure / Contrast / his tone sliders):

- **A uniform image is never changed** (Shadows +100, Highlights −100 at −8…0 EV: 0.000);
  LightCraft moves it by up to +0.76 / −0.41 EV.
- **The size of the move depends on the whole image, not on the level alone.** Highlights −100
  pulls a −1.5 EV region by −2.08 EV when it is 90 % of the frame, −3.86 at 50 %, −3.51 at 10 %,
  and by −2.56 / −4.55 / −4.62 when the whole field is 1 EV brighter / 1 / 2 EV darker; it also
  pulls the −6 EV half down (−1.07). Shadows +100 lifts a −6 EV region +0.94 / +0.74 / +0.61
  (10 / 50 / 90 % of the frame) and the bright half by ≤ 0.05. LightCraft: Shadows 2–3× too strong
  (+1.89 on the dark half, +0.26 on the bright one), Highlights −100 3–5× too weak (−0.83) and
  lifting deep shadows (+1.5 EV) where Camera Raw darkens them; Highlights +30 / +50 lifts darks too
  in Camera Raw (+0.34 / +0.57), not in LightCraft.
- **Edges**: the moves stop sharply at a step edge (no spread, no halo) and fine detail keeps its
  contrast to its surroundings (the probe ramp rides on its region's move) — an edge-aware,
  multi-scale operator (consistent with the local Laplacian filters Adobe published for PV2012),
  with a global, image-dependent part on top.
- With his sliders (E −1.1, C −72, H +30, S +69, W −36, B +30) on the 50 % field Camera Raw gives
  dark +1.18 / bright −0.20 EV, LightCraft +1.30 / −0.64; H / S's share: Camera Raw +0.82 / +1.03,
  LightCraft +1.13 / +0.62.

None of the tried closed forms (a function of level relative to the image's log-mean key, to its
white, or "level minus its value at the key") fits all fields; LightCraft's per-slider tables
(level relative to the key) cannot express it. Next: port an edge-aware local Laplacian operator
(darktable's `local_laplacian`, GPL, allowed in the fork) and fit its remapping to these fields,
then refit the Basic-tone tables with the exact point and parametric curves underneath.

### Custom white balance (R05)

Not resolved: the per-camera strength (`WB_STRENGTH`, k = 2, Leica 1) stays. All three cameras'
Adobe Standard DCPs are dual-illuminant (17 / 21), so the Leica's k = 1 isn't a single-illuminant
profile; the M262 and GR III are both DNGs, so it isn't the file format either.

### Parametric curve (R08): measured, not switched on

Camera Raw's parametric curve on the oracle ramp (`make_curve.py`, `parafit.py`, every region at
±50 / ±100, Bryan's preset, two split settings): with the default splits each region is a fixed
bump in sRGB-curve encoding (Shadows 0–128 peak ~48, Darks 0–255 peak ~100 (+55 levels at +100),
Lights peak ~160 (+71), Highlights 128–255 peak ~208), not symmetric in the sign (clipping at 0 /
255), and regions add: per-region tables interpolated in the slider value and summed reproduce
Bryan's preset to 0.31 of a level (max 1.0) and every single-region stop exactly; moving the splits
is only approximated by warping the input (2–6 levels). LightCraft's preview-fitted model
(amount 0.11) is 10.5 levels off on average over the sweep (1.65 on his preset); the table
(`gen_para.py` → a 4 × 4 × 65 table) brings LightCraft's ramps to ΔE 0.05–0.1 against Camera Raw.

But on his photos the exact curve makes things worse: with it the acceptance set went 3.85 →
4.04 (edited 4.62 → 4.89, Summer Fields M262 +0.6, ΔL* +1.4): Summer Fields photos carry
Lights +48 / Darks +20, which Camera Raw applies at full strength, and LightCraft's Basic-tone
tables (`tone::lr::ADOBE_*`, fitted on previews) had absorbed the weak old curve. The Camera Raw
oracle shows those tables are what's wrong (below), so the table stays out until Basic tone is
refitted on top of it; the generator and the measured data are in `tools/lr-compare/oracle`.

### HSL / Saturation / Vibrance on rendered files (R11)

The oracle-measured HSL / Saturation / Vibrance tables (`colortab.bin`, round 4) act on the
finished colour, whatever the profile, but were used only on Adobe camera bases; JPEG / HEIC /
ProRAW / embedded-profile photos kept the older OkLCh approximation. They now use the tables too
(`ColorOps::with_measured` for every source). Summer Fields iPhone JPEGs in the acceptance set
(4): 7.03 → 6.80 ΔE (per photo 10.03 → 9.80, 5.87 → 5.22, 6.72 → 6.65, 5.49 → 5.52); no other
photo changes. Their remaining error is tone (ΔL* +4.3: rendered files' Basic tone, R02) and the
look table.

### Not done this round (R04, R16, R19–R23)

With the catalog's previews gone no new ground truth could be sampled, and the acceptance set has
only 4 Nautica, 1 Monochrome, 3 X-T2 and 7 X100F photos and none with Point Color or masks, so
these stay measured only:

- **Nautica (R04)**: R6 4 photos 5.45 ΔE / p95 17.3; iPhone Nautica not in the set.
- **Radial / linear masks (R16)**: their local Exposure / Shadows run the same local operator as
  R02, which is what's wrong; nothing to fit until R02 is.
- **Point Color (R19)**, **Color Grading (R20)**: no oracle sweep yet (`make_color.py` is the
  template); no acceptance photos.
- **Texture / Clarity / Dehaze (R21)** against the oracle (single slider, R6 colour): Texture +40
  3.18 ΔE on the grid / 0.61 on the spatial scene, Clarity +40 1.55 / 1.69, Dehaze +30 0.89 / 1.31
  (base 0.42 / 0.55): Texture is far too strong on small detail.
- **B&W / Monochrome (R22)**: 1 photo, 5.05 ΔE.
- **Fujifilm (R23)**: Summer Fields X-T2 5.66 ΔE with ΔL* +2.9 (too bright), X100F 5.00 (PV5,
  ΔL* −2.6) / 2.33 (PV6).

Bryan runs: reopening the catalog in Lightroom Classic rebuilds `Previews.lrdata`; then
`lr_compare.py prepare --spec tools/lr-compare/acceptance.json --seed 1122` gives the full recipe
set (Nautica × iPhone, PV5 R6, Monochrome, X-T2) for the next round.

## Round 7 (2026-11-21): ProRAW gain table map

iPhone ProRAW DNGs carry a DNG 1.6 **`ProfileGainTableMap`** (tag 52525, raw IFD, file byte
order; iPhone 13 Pro: 48 × 62 grid × 257-entry tables, ~3 MB): Apple's local tone mapping, gains
~×2 in the shadows down to ~×0.1 far above white. `crates/raw/src/gainmap.rs` reads it (and DNG
1.7's `ProfileGainTableMap2`, tag 52544: u8 / u16 / f16 / f32 data, Gamma) with checked, capped
reads (≤ 16 M gains; NaN / inf / out-of-range values reject the map, the photo then renders
without it) and applies it as Adobe's reference renderer does (`RefBaselineProfileGainTableMap`):
on linear ProPhoto after the hue/sat map and baseline exposure, before the look table; weight =
clamp(w · [R, G, B, min, max], 0, 1)^Gamma, scaled by N (not N − 1); trilinear over (row, column,
table); position = pixel centre relative to the active area, clamped to the grid. Our DNG writer
keeps it.

The map belongs to the file's own profile ("Apple Embedded Color Profile"). Set: 23 iPhone 13 Pro
ProRAW photos with real Lightroom renders (`prepare --ids`, 8 on the embedded profile, 15 on
Adobe Standard with Summer Fields / Adobe Color; mean ΔE2000 / p95 / ΔL*):

| render | embedded (8) | Adobe Standard (15) | all (23) |
|---|---|---|---|
| file's own profile, no map | 10.57 / 20.5 / −7.96 | 21.04 / 44.4 / −23.5 | 17.40 / 36.1 |
| **file's own profile + map** | **3.01 / 6.5 / −0.01** | 21.08 / 45.8 / −22.8 | 14.79 / 32.1 |
| Leica-transferred `iPhone 13 Pro` profile, no map (Bryan's setup, unchanged) | 9.87 / 17.1 / +9.35 | 6.43 / 17.4 / −2.26 | 7.63 / 17.3 |
| transferred profile + map (rejected) | 16.52 / 26.7 / +19.7 | 11.88 / 25.8 / +4.28 | 13.49 / 26.1 |

So the map applies only with the file's own rendering: a Lightroom-matched camera profile (which
stands for Lightroom's Adobe profiles) and the Adobe DCP path render without it. Not done: the
choice per photo. A source is decoded once per file, not per profile, so a library with an
`iPhone 13 Pro` Lightroom-matched profile (Bryan's, transferred from the Leica, `files: 0`)
renders every ProRAW with it, also the 98 of the 150 ProRAW images in his catalog that Lightroom
shows on the embedded profile; without that profile, Adobe Standard photos stay ~2 EV too dark (Lightroom's
Adobe Standard for iPhones is the per-lens `Apple iPhone14,2 back camera` DCP, which
`adobe::standard_for` doesn't match by Exif model). Next: a profile-keyed source (`lc.color` /
embedded → own profile + map; `adobe:*` → the DCP matched by `UniqueCameraModel`).

## Round 8 (2026-11-21): Highlights / Shadows on Bryan's raws, calibration before the hue/sat map

### Highlights / Shadows (R02): what Camera Raw does on real photos

New oracle sets (`oracle/make_rhs.py`: 26 of Bryan's split-A raws — M262, R6, GR III — with
single-slider and combined sidecars, one neutral ramp per camera to read every render back as
scene EV, plus synthetic gradients / three-level fields / a small bright square / a blob):

- **Linear in the slider, and the sliders add**: S+50 is half of S+100, H+50 ≈ −½·H−100, and
  H+30 S+69 is the sum of the two (to the fit noise).
- **Before Exposure, Contrast, Whites and Blacks**: with his tone sliders (E −1.1 … −1.75,
  C −65 / −72, W, B) Camera Raw's render is the H/S render at default tone passed through the
  sliders' own global curve (`oracle/make_hsorder.py` + `hsorder.py`, error 0.02–0.5 L* against a map noise of
  0.03–0.15) — so H/S see the photo before Exposure and are exposure-invariant.
- **Region-flat, edge-sharp, photo-dependent**: the Shadows map is a crisp, smooth "mask" of the
  dark regions (texture rides on it), not a per-level curve and not a local Laplacian halo (a
  full-depth fast local Laplacian, tried in numpy with level-dependent remaps, decays away from
  edges; Camera Raw's two-level fields move as flat regions). How far a region moves depends on the whole photo:
  around a pivot near log2 of the photo's mean *linear* luminance (−0.5 EV), Shadows lifts what
  lies below it by ~0.7 EV per EV of depth, saturating at the darkest ~13 % (S+100: +0.6 … +2.4
  EV on real photos, +0.45 on a flat two-level field); Highlights − pulls what lies above it
  (~0.5 EV per EV), up to the brightest ~5 %. Synthetic flat fields move less than textured photos
  of the same histogram, so a curve alone can't be exact.

LightCraft now has that model (`tone::lr::adobe_hs_lut`, `ToneStats` from every 7th log-luminance
value on CPU and from the same read-back on the GPU; the table runs through the existing
base / halo path, so CPU and GPU stay equivalent and the cost is one O(n) selection per photo:
prepare 7–9 ms at 2.5 MP either way). Fitted with `oracle/hsfit_rhs.py` (mean |ΔEV| of the move
0.385 → 0.104 on the 26 photos). Against Camera Raw on the oracle photos (ΔE2000, LightCraft's old
tables → the model): S+100 5.24 → 2.16–2.98, H−100 2.95 → 2.21–2.38, H+30 S+69 4.06 → 1.99–2.54,
his whole tone recipe 3.88 → 2.67 (tone without H/S 1.48 either way).

**But it is not switched on** (`finish::ADOBE_HS_MEASURED`): on the acceptance set (Lightroom's
previews) every variant is worse — 4.01 / 10.9 / 25 (b0) → 4.33 / 10.9 / 25 with the measured
parametric curve, 4.39 / 10.9 / 27 without it; Summer Fields M262 4.45 → 5.04–5.27, R6 5.88 →
6.56–6.78 — and stronger slopes are worse still (5.04). Why the oracle and the previews disagree:

- The previews include Bryan's **masks** (R16: radial / linear components with local Exposure /
  Shadows on 376 photos), the oracle renders (`oracle/make_full.py`: his full settings and the
  Summer Fields look, no geometry/detail) do not: on 8 Summer Fields photos Camera Raw without
  masks is 10–15 L* brighter in the shadows than the preview, LightCraft (with its mask model)
  is within ~3. Comparing only the H/S *effect* (full − full without H/S), the model is closer
  to Camera Raw on 6 of 8 photos (mean error 4.42 → 3.62 L*), worst on BHM_8476 (H −100, S +65:
  +12 L* vs +3).
- With his negative Exposure the highlights move into the mid-tones, where the model's weak
  spot shows: above the 95th percentile it stops, Camera Raw keeps pulling (L1009426 at 0 EV:
  −0.4 vs −1.0 EV), and S lifts mid-darks ~25 % short.
- The mask model, the look, `WB_STRENGTH` and the old preview-fitted H/S tables were tuned
  together on the previews; replacing one piece moves the balance.

Next: a richer H/S model (the top end; texture-dependent lift), then R16 masks against the oracle
(local Exposure / Shadows sidecars), then switch R02 + R08 on together and refit on split A.

### Parametric curve (R08)

The measured table is now in the code (`parametric_adobe.rs`, `finish::parametric_adobe_lut`) but
off (`finish::PARAMETRIC_MEASURED`): alone it made the acceptance set worse last round (3.85 →
4.04); on top of the measured H/S model it helps (4.39 → 4.33, ΔL* −0.42 → +0.74), so the two go
on together once H/S passes.

### Calibration before the profile's hue/sat map (R10), and custom white balance (R05)

LightCraft applied the camera profile's hue/sat map in the raw decode, before white balance and
Calibration; Camera Raw edits the profile, i.e. Calibration (and the white balance) come first.
The map can now run in the per-pixel stage as the first of the Adobe tables (`adobe::Base::hue_sat`,
one registered base per distinct map with the white on a 0.002 grid; read at its own level,
`2^-(baseline gain + Exposure)`; CPU and GPU, three tables now, a GPU equivalence case with
Calibration). On the oracle chart (R6 colour, `chartcmp.py`) Bryan's Summer Fields calibration
goes **4.22 → 2.20 ΔE** (teals 5.46 → 1.55, blues 210–270° 11.3 / 14.7 → 3.6 / 4.3), every single
Calibration stop is better (e.g. Blue Saturation +100 2.72 → 1.22, Red Saturation −100 3.62 →
1.28 per pixel), and the base is unchanged (1.39).

With the map after white balance, the white balance is used at the DNG colour spec's strength (1)
for those cameras; the factor 2 found in round 4 (`WB_STRENGTH`, R05) looks like compensation for
the old order (an oracle check of custom white balance in the new order is still to do).

On the acceptance set (old H/S tables and parametric curve), with the map after Calibration and
the white balance at strength 1 for every camera: **4.01 / 10.9 / 25 → 3.81 / 10.6 / 24**, edited
4.59 → 4.32, Summer Fields R6 5.88 → 4.51, X100F PV5 5.00 → 4.62, X-T2 5.66 → 5.51, M262 4.45 →
4.41, GR III 4.02 → 3.97, Adobe Color R6 2.96 → 2.90 (the R6 alone: 3.83). It stays **off**
(`engine::adobe::HUE_SAT_IN_FINISH` empty) only because the gate fails on two edges: the X100F
PV6 cell (3 photos) gets 0.17 worse (tolerance 0.15), and one default R6 photo crosses the pass
line (BHM_0433 1.993 → 2.010; default photos move by a few hundredths, most of them down, since
the map is now read after the resize instead of per sensor pixel). Switching it on is one line.

## Round 9 (2026-11-21): Looks on the GPU, Detail and Effects measured with Camera Raw

### Look profiles on the GPU (R03)

Creative profiles whose look is an RGB table (`lut:` profiles: Bryan's Nomad "Summer Fields" on
6,581 photos and "Nautica" on 876, 44 % of his library) used to force the whole render onto the
CPU. The finish kernel now has the table (`aux` at `LUT_OFF`, size³ RGB, red fastest) and does
what `Lut3d::apply_linear` does — into the table's primaries, its encoding (linear / sRGB /
gamma), trilinear lookup, blended by the amount, keeping the offset of colours outside the
table's range unless the table clips — after the curves in the style block; `.cube` profiles act
on the encoded output as on the CPU. `gpu/tests/equivalence.rs::lut_profiles_match`: three table
spaces / encodings / clip modes, with curves and grain, and a `.cube`: mean 0.0001 LSB, max 1.

Summer Fields on an M262 DNG (2.5 MP preview of the 24 MP file, Apple M3 Pro,
`LOOK=… render_bench`, min of 7, wall-clock; before = the CPU path these photos took):

| scenario | before (CPU) | after (GPU) | budget |
|---|---|---|---|
| loupe 1920×1280, exposure drag (warm) | 103.7 ms | 2.5 ms | 60 ms |
| draft 1152×768, Highlights drag | 42.1 ms | 2.2 ms | 16 ms |
| draft, Clarity drag | 37.0 ms | 2.1 ms | 16 ms |
| draft, NR drag | 97.7 ms | 3.1 ms | 16 ms |
| loupe 1920×1280 cold (new photo) | 435.8 ms | 28.0 ms | — |

GPU vs CPU on that photo: max 1 LSB, mean 0.0002 LSB. Exports keep the CPU path (16-bit /
float); the acceptance gate (CPU) is unchanged by construction.

### The Camera Raw detail oracle (R06, R07, R24)

`oracle/make_detail.py SET grain|grain6000|sharp|nr` writes synthetic LinearRaw DNGs (R6 colour):
grey flats at five levels (1,024, 3,072 and 6,000 px wide), an edge / line / grating / texture
chart, and noisy flats, each with one Detail or Effects setting in its XMP; `ps.sh` renders them
with Camera Raw, `oracle_render` with LightCraft (full size), and `grain_ana.py` /
`sharp_ana.py` / `nr_ana.py` compare grain strength / correlation, sharpening overshoot / halo
width / gains and the noise left. (The DNG SDK must be built in: without it LightCraft's tone
on these files differs by up to 8 L*.)

### Grain (R06)

Camera Raw's grain, measured (L* standard deviation of the grain field, mid grey, 6,000 px):
its cells are sized in the frame and follow the long edge less than in proportion (×
(long / 6000)^0.7: at Size 25 lag-1 autocorrelation −0.12 / 0.18 / 0.49 at 1,024 / 3,072 / 6,000
px); finer grain is *stronger* per pixel (Size 0 / 11 / 25 / 50 / 100: 9.3 / 6.4 / 3.7 / 2.8 /
2.4 at Amount 50); Roughness 0 and 100 are both stronger than 50 (×1.51 / ×1.32) and finer /
coarser; Amount is linear; and in L* the grain stays nearly flat from L* 20 to 95, fading only in
the deep shadows (0.42× at L* 2). The fork's value noise had the opposite size trend (too strong
when coarse, too weak when fine), a lattice aligned with the pixels (its statistics jumped at
whole-pixel cells) and a `lum(1 − lum)` tone weight that left highlights at 0.37×. Upstream's
crystal grain matches Lightroom at mid grey but fades towards black and white, unlike Camera
Raw, and needs emulsion tiles drawn per setting on a thread pool, so it was not merged.

Now (`finish::grain_params`, `grain_tone`, CPU + GPU): one octave of value noise on a lattice
turned by atan ½, cells and strength by Size from the 6,000 px measurements (`GRAIN_CELL_REF`,
`GRAIN_SD_REF`), Roughness as a cell factor `1 + 0.58d + 0.4d²` and a strength factor
`1 − 0.19d + 1.66d²` (d = Roughness/100 − ½), strength by the cell's size when the long edge
differs, and Camera Raw's tone weight. LightCraft / Camera Raw grain strength (per setting and
band; mean |log ratio|; the 1,024 / 3,072 px sizes were not used to fit):

| set | before | after |
|---|---|---|
| 6,000 px, 10 settings × 5 bands | 0.467 (median ratio 0.94, 0.37–2.6) | **0.046** (0.94–1.05) |
| Bryan's 31 / 11 / 43 at 6,000 px, by band | 0.97 / 0.65 / 0.83 / 0.71 / 0.48 | 0.96 / 1.01 / 1.02 / 1.03 / 1.00 |
| 1,024 + 3,072 px (held out) | 0.502 (lag-1 autocorrelation +0.33 too high) | **0.109** (+0.08) |

Previews keep the downscaled full-size grain (fewer, weaker cells per pixel), so the acceptance
gate (Lightroom's previews, ≤ 2,560 px) is unchanged: 4.00 ΔE, 25 pass, every cell within 0.01.

### Sharpening at 1:1 (R07)

Measured on the edge chart (6,000 px source scale: the chart is rendered at its own size):
Camera Raw's capture sharpening was ~10× stronger than LightCraft's at 1:1 (Amount 40 / Radius 1 /
Detail 25: step-edge overshoot 0.38 of the step vs 0.04; Bryan's 126 / 1.0 / 25 / Masking 94:
0.19–0.27 vs 0.05–0.12). Round 2 had tuned it on Lightroom's ≤ 2,560 px previews, where a
full-size halo is mostly averaged away. The structure was right (an unsharp mask of log
luminance whose large differences Detail damps: overshoot ratio across 0.6 / 2 / 6 EV edges
1 : 0.42 : 0.08 here, 1 : 0.35 : 0.05 in Camera Raw), so it was refitted on the chart
(`oracle/sharpfit.py`: the operator emulated on Camera Raw's own tone, 12 settings fitted, 3
held out; `GSIG=1.6`): gain `8.14 · (Amount/150)^0.82`, sigma 0.82 × Radius source px, Detail damping
`0.006 + 0.405 · Detail/100` EV; and Edge Masking now weighs the log-luminance gradient smoothed
by a 1.6 px Gaussian (`c = 0.0075 · e^(3.52 · Masking/100)`, fading in from −0.275c to 2.275c),
like Camera Raw, which already masks a 3 px grating at Masking 30 — the old 3×3 range kept it.
CPU and GPU (`finish::Sharpen`, the kernel's gradient loop); the mask's wider window only runs
when Masking > 0.

`sharp_ana.py` (edges, lines, gratings, textures; mean |log ratio| LightCraft / Camera Raw over
15 settings): **1.755 → 0.417**. Bryan's 126 / 1.0 / 25 / 94: 0.6 EV edge 0.19 vs 0.28 (was
0.12), 2 EV edge 0.27 vs 0.30 (was 0.05), 6 px grating gain 1.21 vs 1.15 (was 1.12), low-contrast
texture 1.00–1.05 vs 1.10–1.13 (was 1.17–1.31). Left: Masking 100 keeps more than Camera Raw
(0.6 EV edge 0.22 vs 0.09), Radius 3 kernels are truncated at 4 taps, and strong unmasked amounts
stay ~20 % under. No Alt-drag mask / detail preview yet.

At preview size the two rows barely move the ratios to Lightroom's previews (`lr_compare.py
detail`, 28 photos ≥ 1,800 px, 1 = Lightroom; before → after R06 + R07): fine texture 0.29 →
0.36, edge sharpness 0.31 → 0.33, halos 0.66 → 0.66, chroma noise 0.54 → 0.55, mid-scale mottle
1.21 → 1.31 (grain on the M262 files: Lightroom's previews are its 1:1 render scaled down, ours
renders at preview size).

### ProRAW: the photo's profile picks the source (round 7 follow-up)

Round 7 left the choice per photo open: a source is decoded once per file, so in Bryan's library
the Lightroom-matched `iPhone 13 Pro` camera profile (transferred from the Leica) rendered every
ProRAW and the file's own `ProfileGainTableMap` never applied. Lightroom renders a ProRAW on
"Embedded" / "Apple ProRAW" with the file's profile and its map, and on Adobe Standard without
them (round 7's measurements). Now the photo's profile decides how its source is decoded: a DNG
developed with `lc.color` (what those profiles migrate to) loads with the file's own colour model
and map (`media::own_profile_source` → `SourceRef::File { own_profile }` → `files::load_raw`;
only files that carry a map are affected), every other profile keeps the matched one; a cached
source decoded the other way is not reused. On the 23 ProRAW photos with Lightroom renders,
Bryan's setup (mean ΔE2000 / p95):

| | embedded (8) | Adobe Standard (15) | all (23) |
|---|---|---|---|
| before (matched profile for all) | 9.87 / 17.1 | 6.43 / 17.4 | 7.63 / 17.3 |
| **now** | **3.00 / 6.5** | 6.64 / 17.8 | **5.37 / 13.8** |

(The Adobe Standard photos decode exactly as before; their +0.2, on the Summer Fields photos
only, comes from other render changes since round 7, not isolated here.) Left: those 15 still need
Lightroom's per-lens `Apple iPhone14,2 back camera` Adobe Standard DCP (not matched by Exif model).

### Lens-profile chromatic aberration and Remove CA (R13): measured, nothing to change

`oracle/make_ca.py` + `ca_ana.py`: two of Bryan's R6 photos whose Lightroom lens profile carries
`ChromaticRedGreenModel` / `ChromaticBlueGreenModel` (Canon EF 24 mm f/1.4L II, EF 50 mm f/1.2L;
the 5D Mark II profiles Lightroom picks for them), rendered by Camera Raw at full size with profile
corrections off / on × Remove Chromatic Aberration off / on; red and blue registration against
green measured per 96 px tile and fitted radially (the measurement finds a synthetic 0.05 %
magnification, 1.48 px at r = 0.9, as 1.32 px).

- Uncorrected, both photos are already registered (|shift| ≤ 0.02 px at r = 0.9), although the
  EF 24 mm profile's model predicts +0.45 px red and +1.24 px blue there.
- **Enable Profile Corrections leaves the colour planes alone** (identical to profile off, both
  photos): Camera Raw does not apply the profile's chromatic model on its own.
- Remove CA is image-based with or without the profile: it moves red by −0.10 to −0.32 px and
  blue by ≤ 0.3 px; LightCraft's estimator (`optics::estimate_lateral_ca`) moves them by
  ≤ 0.06 px on the same files.

So applying the profile's model would have added the 1.2 px the model predicts; LightCraft keeps
ignoring it, and "Remove CA" stays its own estimator (both within 0.3 px of Camera Raw here).
Not settled: lenses with real lateral CA (none of Bryan's profiled lenses showed any).

### Fringe Color Selector (R25)

`develop.defringePick {x, y}` (the eyedropper next to Defringe in Lens Corrections ▸ Manual): the
most saturated pixel within 3 px of the click on the preview-level source, its OkLab hue as a
position on the Purple (240–360°) or Green (90–210°) Hue slider, and a 24-wide hue range centred
on it; Amount becomes 5 when it was 0; a neutral or other hue is refused with a message. The
defringe itself (`optics::defringe`, LightCraft's own edge-gated desaturation) was not measured
against Camera Raw: Bryan uses Purple Amount on 473 photos, at one preset value.

### Noise reduction at 1:1 (R24)

Synthetic flats are no use here: they carry no noise profile and Camera Raw's luminance NR barely
acts on them. `oracle/make_nr_real.py` + `nr_real_ana.py`: two of Bryan's ISO 6400 R6 raws
(Iceland, `BHM_6642`, `BHM_9406`) at full size with no NR, Color 25 / 50 and Luminance 15 / 30 /
60 (+ Color 25); the high-pass noise (pixel − 3×3 mean) in L* and in a*b* over the flattest
third of 32 px tiles. With NR off LightCraft starts noisier (L* 1.59 / 2.62 vs 1.29 / 2.11;
chroma 3× — AHD vs Adobe's demosaic). The fork's NR was weak at 1:1: Luminance blended its
filter by √Luminance (55 % at 30) and Color mixed 75 % of the blurred chromaticity at 25. Now
(`local::nr_params`, CPU + GPU through the same numbers): Luminance blends `min(5 × L/100, 1)`,
radius `1 + 3.5 L/100`, edge threshold ×2.4; Color mixes `min(4 × C/100, 1) × (1 − 0.05 ×
Detail/100)` with a `1.5 + 14 C/100` px blur. Renders below 40 % of the source keep the
preview-fitted round-2 numbers and blend to these by full size (the first try, at every scale,
cost the acceptance gate 2 passing photos: 4.00 → 4.02 ΔE). Noise left at 1:1, LightCraft /
Camera Raw (L* · chroma):

| | before | after |
|---|---|---|
| Color 25, BHM_6642 | 1.57 / 1.35 · 1.24 / 0.54 | 1.57 / 1.34 · 0.59 / 0.54 |
| Color 25, BHM_9406 | 2.57 / 2.05 · 1.78 / 0.28 | 2.53 / 2.05 · 0.77 / 0.28 |
| Luminance 30, BHM_6642 | 1.17 / 0.63 · 1.14 / 0.27 | 0.61 / 0.63 · 0.21 / 0.27 |
| Luminance 30, BHM_9406 | 2.19 / 0.97 · 1.75 / 0.15 | 1.42 / 0.97 · 0.46 / 0.15 |
| mean \|log ratio\| (10 settings × 2 photos) | 1.099 | **0.399** |

Left: the darker photo keeps ~1.5× Camera Raw's luminance noise and ~3× its chroma noise (our
demosaic starts 3× noisier in colour); Color 50 and 25 leave the same fine chroma in both
renderers (the metric sees only the finest scale). Upstream's process V6 "joint" colour NR was
not merged (its own process version, a larger change than these constants).

## Round 10 (2026-11-22): Texture, Dehaze, Color Grading, Point Color, B&W and the masks' tone sliders

The rows round 6 left measured only (R04, R16, R19–R23), against the Camera Raw oracle. New
experiments in `oracle/make_fx.py SET fx|fx2|grade|bw|pc2|look|mask|mask2` (frequency scenes —
bands of sine gratings, and single full-frame gratings at 1,024 and 3,072 px with amplitude and
level variants —, the colour grid, the spatial scene, and full-frame / half / radial masks) and
`make_fuji.py` (Bryan's X-T2 / X100F raws); analysis `fx_ana.py` / `fx2_ana.py` (grating gain
and mean shift), `grade_ana.py` (OkLab shifts of the grid's neutrals), `bw_ana.py`, `selfcmp.py`
(one renderer against itself: does a setting do anything, is a local slider the global one),
`local_ana.py` + `gen_local.py` (the masks' tone curves), `fuji_ana.py`; `lcrun.sh` renders the
LightCraft side with this checkout's `oracle_render`, which now also reads masks, spots and Point
Color from the XMP (before, every mask experiment rendered without its mask).

**Reused, not rebuilt.** RAWmakase (<https://github.com/pch/rawmakase>, MIT) is a Rust raw
editor whose develop operators were each measured against Camera Raw 18.7 on synthetic charts.
Its Texture, Color Grading, B&W mix, Point Color and negative Dehaze agree with what our oracle
measured, so they are ported (NOTICE; each file says so) and checked on our renders: `texture_cr`,
`grading_cr` (+ `grading_cr.bin`), `bw_cr` (+ `bw_cr.bin`), `point_color_cr`, `dehaze_cr`. The
masks' Contrast / Whites / Blacks are our own measurement (`local_tone_cr`). Every one has a
`MEASURED` switch, runs on the CPU and the GPU (equivalence tests pass), and only changes
renders that use its slider.

ΔE2000 against Camera Raw (grid: valid patch means; other scenes: every pixel). Base renders, for
scale: grid 0.42, spatial 0.55, frequency scene 0.72, single gratings 0.70.

| row | experiment | before | after |
|---|---|---|---|
| R21 Texture | single gratings, ±50 / +100 (21 scenes) | 1.07 | **0.79** |
| | grid +25 / +50 / +100 | 2.24 / 3.68 / 5.31 | 1.92 / 3.15 / 4.50 |
| | grid −25 / −50 / −100 | 1.65 / 2.67 / 3.98 | 1.44 / 2.26 / 3.39 |
| | spatial / frequency scene (1,024 + 3,072 px), ±25…100 | 0.58–0.77 / 0.81–1.69 | 0.56–0.64 / 0.73–0.84 |
| R21 Dehaze (−) | grid −25 / −50 | 7.13 / 11.76 | **1.39 / 3.19** |
| | spatial −25 / −50 | 4.36 / 7.01 | **1.21 / 1.96** |
| | frequency scene −25 / −50 | 2.76 / 5.42 | **1.11 / 1.97** |
| | single gratings −50, periods ≥ 21 px / 3–13 px | 2.0–5.3 / 0.7–3.2 | 0.8–2.0 / 4.9–7.8 |
| R21 Dehaze (+), Clarity | grid / spatial | unchanged (below) | — |
| R20 Color Grading | Global wheels (5) | 2.99–12.20 | **0.43–0.47** |
| | Midtones wheels (5) | 1.34–4.93 | **0.43–0.47** |
| | Highlights wheels (5) | 1.68–6.34 | **0.42–0.62** |
| | Shadows wheels (5) | 3.08–14.23 | 0.44–3.13 |
| | Luminance ±50 (8) | 1.08–3.42 | **0.39–0.74** |
| | split toning, Blending 0 / 50 / 100 | 4.19 / 3.94 / 4.56 | **0.50 / 0.47 / 0.72** |
| | split toning, Balance −50 / +50 | 5.07 / 4.23 | **0.76 / 0.66** |
| R19 Point Color | 22 swatch settings (hue, shifts, Range, Variance, sample) | 0.50–1.09 (ignored) | **0.43–0.50** (Variance −100: 0.67) |
| R22 B&W | default mix | 0.74 | **0.33** |
| | single bands ±60 (16) | 1.02–2.99 | **0.35–0.72** |
| | Adobe Monochrome; + Blue −40 | 1.32; 1.58 | 1.20; 1.28 |
| R16 masks | local Blacks ±50 / ±100 (spatial) | 6.35–12.14 | **0.08–0.32** |
| | local Whites ±50 / ±100 | 4.47–7.86 | **0.10–0.92** |
| | local Contrast ±50 / ±100 (frequency scene +50) | 1.16–3.90 (3.23) | **0.11–0.33** (0.44) |
| | the same on the scene 2 EV darker / 1 EV brighter | 0.64–9.89 | 0.25–1.43 |
| | local Exposure ±1, Texture +40 | 0.52–0.55, 0.61 | 0.52–0.55, 0.60 |
| | local Highlights −60 / Shadows +60 / Clarity +40 / Dehaze +30 | 1.26 / 1.39 / 2.14 / 1.90 | unchanged |
| R04 Nautica | grid, Amount 100 / 50 | 0.41 / 0.42 | (already matched) |

### Texture (R21)

Camera Raw's Texture is a broad band of log-luminance contrast: at +100 on 0.4 EV gratings the
gain is 1.78 at a 3 px period, 1.54 at 21–34 px, 1.25 at 89 px and 1.05 at 233 px, in pixels of
the full-resolution photo (a 3,072 px image renders the same per pixel up to ~40 px periods);
faint detail is lifted most (×1.78 at 0.1 EV) and strong contrast hardly (×1.13 at 1.2 EV, 13 px).
LightCraft's single band (σ 0.18 % of the long edge, ×1.1) was 2.07 at 3 px and 1.02 at 55 px —
far too strong on the finest detail and absent at the scales Camera Raw lifts most. Now
RAWmakase's operator: a 6-level Laplacian pyramid of each channel's log2 (5-tap binomial),
each level compressed (`band / (1 + (band / t)²)`) and weighted, the sum scaled by a measured
strength; the finest level follows the render's scale against the photo (`texture_cr::level_offset`).
Its gratings (Camera Raw / LightCraft at +100): 3 px 1.79 / 1.80, 13 px 1.56 / 1.49, 34 px
1.53 / 1.49, 89 px 1.25 / 1.22, 0.1 EV 1.78 / 1.84, 2.5 EV 1.13 / 1.13. Per channel, because
Camera Raw's Texture also moves colour at colour edges (the grid's neutral patches gain 0.02 OkLab
chroma beside coloured ones; luminance-only left the grid at 3.71, per channel 3.15). The rest of
the grid's error is that colour effect: ours gains a quarter of Camera Raw's. In a mask Texture is
the global operator (Camera Raw: local +40 = global +40 to 0.00 ΔE). The GPU path reads the
detail made on the CPU once per image and level.

### Clarity and Dehaze (R21): measured, Clarity left

**Clarity** is not a fixed-radius band in Camera Raw. On single full-frame gratings at −3.5 EV,
+50 / +100 barely move them (×1.02–1.08), while the same gratings as bands next to brighter
ones were ×1.59 / ×2.15 at every period up to 233 px; −50 works on both (×0.68, with a cut-off
near 55 px at 1,024 px and 165 px at 3,072 px: relative to the image). So positive Clarity is
weighted by the level relative to the photo's brightest levels, at scales up to ~15 % of the long
edge — RAWmakase's 4-scale bilateral model with weights by base level against a highlight key,
part of its scene-tone stage. LightCraft's (one 1.2 % edge-aware band, absolute level weight) is
1.04 / 1.90 / 3.77 on the grid at +25 / +50 / +100 and 0.47–0.78 at −25…−100. Not ported this
round (it needs RAWmakase's local-tone map); next.

**Negative Dehaze** is a tone curve per linear-ProPhoto channel by its level relative to the
photo's 99th luminance percentile (RAWmakase's measured family at −100 / −40 / −20 with its
monotone projection, `dehaze_cr`): on gratings Camera Raw's −25 / −50 keep ×0.75 / ×0.50 of the
contrast at every scale and lift the darks; LightCraft's mix towards the airlight crushed the
grid (7.1 / 11.8 ΔE). Masks' negative Dehaze takes the same curve. On single full-frame
gratings −50 is 2.0–5.3 → 0.8–2.0 ΔE at periods of 21 px and up, but 0.7–3.2 → 4.9–7.8 at 3–13 px
(mean over the 21 gratings 2.88 → 2.89): there Camera Raw keeps the mean level (+0.00 EV at 13 px,
+0.28 at 21 px for the same statistics), a spatial (dark-channel window?) part the per-channel
curve does not have. **Positive Dehaze** stays
LightCraft's dark-channel removal (grid +15 / +30 / +60 / +100: 0.56 / 0.89 / 1.58 / 2.73; spatial
0.70 / 1.31 / 3.29 / 6.67 — it drives dark regions to black where Camera Raw lifts them);
RAWmakase's measured haze model (`scene_tone/haze.rs`) is the next port.

### Color Grading (R20)

Camera Raw grades each channel of linear ProPhoto with its own curve (Global at 210° / 100: green
exactly unchanged, red ×0.565 and blue ×1.774 in the shadows, both fading to ×0.88 / ×1.08 at
L 0.9); LightCraft added OkLab offsets with a constant strength and a hue map 20–45° off. Now
RAWmakase's measured curves (profiles by Blending and Balance, per-channel coefficients by hue
and saturation, Luminance curves), collapsed per render into three 64-sample gain curves (CPU;
192 floats in the kernel's aux table on the GPU). One more finding: **Split Toning without
`ColorGradeBlending` (settings from before Color Grading) renders as Blending 100** — a
shadows-only tint spreads over the midtones exactly like Blending 100 (Shadows 210° / 20 with
the key missing: 3.08 → 1.34 with the curves, → 0.44 once read as 100); `crs::to_partial` now maps
it so. Left: Shadows at saturation 100 in deep cyan / green (3.13 / 1.76: Camera Raw drives red to
×0.01 there).

### Point Color (R19)

Camera Raw ignored both encodings round 6 guessed. Its XMP form is 19 numbers per swatch — the
sample as HSV of linear ProPhoto (hue in sixths of a turn), the three shifts (−1..1), Range, then
the hue / saturation / luminance ranges as trapezoids (outer, inner, inner, outer) — with one
`crs:ColorVariance` per swatch; Camera Raw drops swatches whose ranges don't hold the sample
(Lightroom's empty selection is all −1). The catalog keeps the same fields by name. LightCraft
read neither the XMP form nor the units (`SrcHue` as radians of an sRGB HSL colour). Both forms
now keep Lightroom's own sample (`PointColor.lr`) and render it with Camera Raw's model
(RAWmakase's fit: HSV of linear ProPhoto after the mixer, smoothed trapezoids scaled by Range,
±35° at Hue ±100, Saturation ×/÷1.9, Variance spreading around the sample; CPU and GPU).
Swatches picked in LightCraft keep its own OkLCh model. (Camera Raw ignores swatches written as
named XMP fields; only the catalog stores them so, and those now render with the Camera Raw
model too.)

### B&W (R22)

Camera Raw's bands are narrower than LightCraft's were and its darkening is stronger than its
lightening (Aqua −60 −0.18 OkLab L on aquas, +60 +0.09; ours ∓0.27–0.38, centred 30° off). Now
RAWmakase's measured grey chart: a colour becomes `Y · 2^g`, `g` looked up in the colour mixer's
36 × 6 × 6 grid of linear-ProPhoto HSV from the zero-mix table plus each band's table at
−100 / −50 / +50 / +100. Adobe Monochrome (its look table under the mix) 1.32 → 1.20.

### The masks' tone sliders (R16)

In Camera Raw a full-frame mask's Exposure, Highlights, Shadows, Texture and Clarity are within
0.0–0.8 ΔE of the global sliders, but Contrast (1.7), Dehaze (1.4–1.8) and Blacks (5.0) are not.
Read off the spatial scene's gradient (scene-referred change LightCraft's own base would need, by
level l1 = log2(Y / 0.18)): local Blacks +50 lifts l1 −5 by +2.2 EV and l1 0 by +0.27 (global
Blacks +50: +0.5 / +0.1), −50 crushes below l1 −2.5; local Whites ±50 moves the mid tones by
±0.3 EV and l1 +0.5 by +0.73 / −0.47; local Contrast +50 pivots near l1 +0.4 (−0.48 at −2.5,
+0.14 at +1.5). The curves are the same on the scene 2 EV darker or 1 EV brighter — they follow
the scene level, not the photo. They replace LightCraft's guessed formulas as 16-knot tables at
±50 / ±100 (`local_tone_cr`, from `gen_local.py`). Left: local Highlights / Shadows still use
LightCraft's older local formula (Camera Raw: local = global, which uses the measured tables
only globally), Clarity and positive Dehaze as above.

### Nautica (R04)

Bryan's imported Nautica look (an Archipelago RGB table, `look_from_file`) on the grid: 0.41 /
0.42 ΔE at Amount 100 / 50, the base itself 0.42 — LightCraft renders the look as Camera Raw does.
The 5.45 ΔE of the four Nautica R6 photos in the acceptance set comes from the rest of their
edits (Basic tone, R02), not the look.

### Fujifilm brightness (R23): measured, not changed

`make_fuji.py` on four of Bryan's split-A raws (X-T2 `DSCF6043`, X100F `_DSF4109` PV5,
`_DSF4145`, `_DSF4241`), Camera Raw's minimum size, Adobe Standard: at defaults LightCraft is
+0.12 to +0.20 EV too bright in the mid tones (+1.3 to +2.1 L*; +0.06 to +0.15 at Exposure +1),
about the same on both bodies and at ISO 100–400 (RawExposureBias +0.3 … −1.7; RAWmakase's camera
table gives both bodies a constant 0.15 EV baseline that ignores it). With the
photos' own tone and colour settings (no look) LightCraft is 0.3–0.8 EV too dark — that is Basic
tone (R02: their Highlights −74…−90 / Shadows +43…+79), and it dominates what the acceptance set
sees. Lowering `MEASURED_EXPOSURE` by ~0.16 EV would fix the defaults but move the edited
photos the wrong way until R02 is refitted, so it is left.
