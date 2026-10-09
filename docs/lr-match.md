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
