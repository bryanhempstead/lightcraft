# Matching Lightroom Classic's renders

For someone moving a Lightroom Classic catalog over, "right" means "looks like it did in Lightroom".
This page is how LightCraft measures that, what was changed because of the measurements, and what
still differs. Everything here is black-box: Lightroom's *output* (the previews it keeps of the
user's own photos) is compared with ours; no Adobe code, profile, curve or table is read or copied,
and everything fitted from a user's catalog stays on that user's machine.

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
