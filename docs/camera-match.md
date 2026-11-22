# Match camera & lens color

Beyond Lightroom: sync a finished look from one camera (say a Leica M Typ 262) onto photos from
another (a Ricoh GR III, a Canon R6, a phone) and have the colour land where the reference is,
not where the other camera's profile puts the same settings.

- **Photo ▸ Match Camera & Lens Color** (`develop.matchCameraColor {ids?, scene?}`): the active
  photo is the reference, every other selected photo is matched to it.
- **Sync Settings ▸ "Match camera & lens color"** (`develop.sync {matchCameraColor: true}`): the
  same, right after the settings are copied and before Match exposure. Off by default; the dialog
  remembers it with its other checkboxes.
- **`develop.buildLensCasts {catalog?}`**: learns the lens / phone casts (below) from a Lightroom
  catalog (read from a copy) or, without `catalog`, from the open library's edits.

Everything it changes is ordinary sliders (Temp / Tint, the 8 × 3 HSL sliders, Calibration,
Saturation / Vibrance), in one undo step. The result reports, per photo, the chart ΔE before and
after and every slider moved.

## How

1. **Stage 0 — each camera renders like Lightroom.** Raws render on the camera's own Adobe
   Standard DCP through Adobe's DNG SDK (`docs/lr-match.md`, rounds 3–6). Measured against
   Lightroom's own previews of the same edits (set `w9`, 103 photos, current build) the per-camera
   colour bias is small and no camera stands out: mean Δa\*/Δb\* GR III −0.3 / −0.0, M262 −0.2 /
   −0.3 (Summer Fields), R6 −1.0 / 0.0; dE128 GR III 3.58, M262 3.81, R6 4.86 (what remains is
   mostly Lightroom's Basic-tone operator on heavy edits, shared by every camera). iPhone ProRAW:
   the file's own `ProfileGainTableMap` is now applied (round 7 in `docs/lr-match.md`).
2. **Chart (raw ↔ raw of different cameras).** 40 patches: the 24 ColorChecker Classic patches
   (published L\*a\*b\* D50), five skin tones, a neutral ramp and six saturated primaries, as
   scene colours in the working space. Each photo's raw decode (`files::chart_source`) takes them
   through its own colour path with the camera matrix inverted, so what differs between cameras is
   what follows the matrix: the DCP's hue/sat map and look table, the profile's tone curve, how
   Temp / Tint move that camera's neutral (Leica M: as the DNG spec says; others: twice as far,
   `adobe::WB_STRENGTH`). Both charts are lit by the target photo's light (the reference decode
   takes the target's as-shot white) and the reference is rendered with the target's white balance
   and exposure, so only the cameras differ, not the scenes. Spatial and noisy settings (grain,
   sharpening, NR, vignettes, crop, lens corrections, masks) are left out of the chart renders.
   A damped Gauss-Newton search (finite differences in slider units, run in parallel; a prior that
   prefers small moves; clamped to the slider ranges) moves the target's sliders until the two
   renders agree in CIELAB; the result is rounded to whole slider values and kept only if it lowers
   the mean CIEDE2000.
3. **Lens and phone casts**, learned from the user's own edits (`lens_casts`): at events (same
   camera, same day) where he shot two or more lenses, each lens's median finished white balance
   (mireds, tint) minus the event's median; per lens, the median over events, kept with ≥ 3 events
   and ≥ 10 photos (counts and spread stored). Manual lenses (Exif "not selected") go by the lens
   profile Lightroom picked. Phones' rendered files (JPEG / HEIC) also get, at events shot with a
   camera too, their median Saturation / Vibrance relative to the camera's. Applied as the
   difference between the target's and the reference's lens on a Custom white balance (an As Shot
   one already follows the camera's own reading of that lens). Stored in
   `~/Library/Application Support/LightCraft/lens-casts.json` (`LIGHTCRAFT_LENS_CASTS` names
   another file): derived from his catalog, so it is never committed.
4. **Scene pass** (photos shot within 30 minutes of each other, or `scene: true`): small renders
   of both; the mean a\*/b\* of near-neutral mid-tone pixels evens out the remaining cast (measured
   Temp / Tint response, 60 % of the step, ≤ ±25 mireds / ±10 tint per step, two steps), and each
   hue band holding ≥ 2 % of both photos nudges its HSL saturation toward the reference's chroma
   (half the gap, ≤ ±10). Rendered files (phone JPEG / HEIC: sRGB or Display P3, decoded to the
   working space through their own profile) have no sensor path and get only 3 and 4.

## Limits

- The chart can only show what the profiles do: without the sensors' spectral sensitivities it
  can't model metamerism (two cameras seeing the same paint differently). The scene pass covers
  some of that when the photos share a scene.
- Lens casts are white balance only (his HSL rarely differs by lens); evidence is thin for
  lenses he uses at few events.

A chart match is kept only when it lowers the mean ΔE to ≤ 60 % of where it started and ≤ 3.5,
and no slider moves more than 2.7× its small-move scale (±40 HSL / Calibration, ±27 Saturation /
Vibrance, ±160 mireds): a partial fix of a huge difference (an extreme Custom white balance synced
onto a photo in very different light) looked worse than none.

When the two photos show the same scene (shot within 30 min, or `scene: true`), the chart is lit by
the light the reference camera read and the target camera keeps its own as-shot reading, so the
two cameras' auto white balance reading one light differently is matched as well. The scene pass
runs only when the two small renders' colour content overlaps ≥ 85 % (hue-band and neutral
shares): different framings at one event (a wide hillside vs a close-up) otherwise pull the white
balance toward their content.

## Measured on his photos (2026-11-21, scratch library migrated from his catalog)

Pairs shot seconds apart, Summer Fields recipe, synced from the reference with "Match camera &
lens color"; mean / max chart CIEDE2000 before → after:

| reference → target | chart ΔE mean | max | moved |
|---|---|---|---|
| M262 → GR III (5 raw pairs, same scene) | 4.9–12.1 → **1.37–2.12** (avg 1.80) | 19.6–28.2 → 4.5–6.2 | Temp +190…+950 K warmer (the GR III renders a synced Custom Temp cooler: WB model ×2 vs the Leica's ×1, and its own as-shot reading), Calibration green, magenta / aqua HSL |
| M262 → R6 (different scenes, Custom 2850 / 3425 K synced onto daylight) | 14.0 / 15.0 → not kept (best 5.7) | | nothing |

Side by side (reference | synced | synced + match): the GR III photos come out warmer and closer to
the Leica's look, whites cream instead of grey-green, skin natural, no new cast. Lens casts in his
catalog are small: R6 Sigma 15 mm +10.3 mireds (9 events, 143 photos, spread ±8.9), EF 24 mm
+1.5, 50 / 70–200 / 85 mm 0; M262 Elmarit 21 mm +6.6 mireds / −2 tint (7 events, spread ±15);
iPhone 13 Pro Max lenses 0 (18 events). The scene pass ran on 3 of 12 R6 lens / phone pairs (the
others framed different content) and moved white balance ≤ 25 mireds; on the iPhone JPEG next to
an R6 at a glacier it took out the synced blue cast.
