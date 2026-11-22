# Camera coverage, per model (Bryan's library)

What LightCraft does with every camera and raw format in Bryan's real library, checked model by
model (LR-IMP-CAMERA-COVERAGE, LR-IMP-FORMATS). Measured on 2026-11-22 from read-only copies; none of
his media, catalogs or previews is in the repository.

## How it was measured

- **Which cameras.** A copy of the Lightroom Classic catalog (`LR-Cat2.lrcat-v13-3.lrcat`, 13,798
  photos) grouped by camera model and file type, plus a walk of the mounted drives (`BONUSBOY`,
  `LOT48`, `hempstead.studio`, `~/Pictures`) for every raw extension: about 1.55 million raw files,
  almost all of them Blackmagic CinemaDNG video frames. One or more files per model and per folder
  were decoded headless.
- **Three decoders.** `crates/raw/examples/decodecheck.rs` (which decoder read the file, sizes,
  orientation, CFA, black and white level) with LightCraft's native decoder, then forced through
  LibRaw (`LIGHTCRAFT_RAW_DECODER=libraw`); for DNG files also the DNG SDK's own reference render
  (`crates/dng-sdk-sys/examples/sdk_render.rs`). Renders at default settings went through
  `lightcraft-cli render` both ways and were compared (CIEDE2000, mean ΔL*/Δa*/Δb*, after the SDK
  render was turned to the file's orientation).
- **Against Lightroom.** Lightroom's `Previews.lrdata` is gone from the catalog folder, so the
  colour comparison uses the 183 previews stored by earlier `lr_compare.py prepare` runs (the
  acceptance set: Leica M262, R6, GR III, X-T2, X100F, iPhone 13 Pro Max) rendered with each photo's
  Lightroom settings (`lr_compare.py migrate / render / compare`, Summer Fields and Nautica looks
  imported). **Lightroom's geometry** comes from its smart previews (`Smart Previews.lrdata`, 4,633
  DNGs): each records `OriginalDefaultFinalSize`, the size Lightroom crops the original to by
  default, so every model's default crop is checked even where no rendered preview exists.

## The table

15 raw bodies; every one decodes with LightCraft's native decoder. Columns:

- **Photos**: in the Lightroom catalog / found on the drives (originals reachable).
- **Sensor → crop**: stored sensor array → LightCraft's default crop. **Lightroom** is the
  `OriginalDefaultFinalSize` its smart previews record (`SP` = how many); ✓ = identical.
- **Levels**: black / white as the native decoder reads them; LibRaw reads the same unless noted.
- **LibRaw path**: ΔE (mean / p95) of our default render with the samples forced through LibRaw
  against the native render, before → after this round's fallback fix (0.00 = identical render).
- **DNG SDK**: ΔE of the SDK's reference render (`dng_render`, the file's own profile, no Lightroom
  tone) against ours at defaults, on files without a crop in their XMP. It differs by design (no
  Adobe Color look, no PV2012 tone); it is a sanity check of geometry and of gross colour errors.
- **ΔE vs Lightroom**: against Lightroom's preview with the photo's edits (`n` photos; main at
  a888dbac5, CPU). Photos flagged `moved` by the alignment all have a rotated crop, Upright or a lens
  profile on (develop geometry, not the decoder); none without one is flagged.

| Model | Format | Photos | Sensor → crop | Lightroom | Levels | LibRaw path | DNG SDK | ΔE vs Lightroom | Notes |
|---|---|---|---|---|---|---|---|---|---|
| Leica M (Typ 262) | DNG | 8,071 / 6,442 | 5984×4000 → 5976×3992 | — (no SP) | 0 / 15000 | 0.00 → 0.00 | 1.8 / 3.6 | **4.09** (72) | |
| Canon EOS R6 | CR3 lossless, incl. 1.6× crop mode | 2,756 / 2,251 | 5568×3708 → 5472×3648; crop mode 3584×2386 → 3408×2272 | 5472×3648 ✓ (2,724 SP); 3408×2272 ✓ (10 SP) | 512 or 2048 / 14008–14888 (ColorData, per ISO) | 0.00 → 0.00 | — | **3.74** (91) | Lightroom pano / HDR DNGs (linear, 41): LibRaw path 0.8–4.4 → 0.00 |
| Ricoh GR III | DNG | 874 / 787 | 6112×4064 → 6000×4000 | — | 64–1024 / 14316–16315 (per file) | 2.9–5.3 (ΔL* ±6) → 0.00 | 1.6–2.7 | **4.02** (6) | BaselineExposure was lost on the LibRaw path |
| Leica Q2 | DNG (+ FixBadPixels, WarpRectilinear opcodes) | 226 / catalog path offline; 7 folders elsewhere | 8424×5632 → 8368×5584 | 8368×5584 ✓ (224 SP) | 512 / 16383 | 1.8–5.9, p95 ≤ 33 → 0.00 | 1.6–3.2 | no preview | opcodes were lost on the LibRaw path |
| Canon EOS 5D Mark III | CR2 | 175 / catalog path offline; 2 folders on LOT48 | 5920×3950 → 5760×3840 | 5760×3840 ✓ (185 SP) | 2095 → **2048** / 15094–15553 (SpecularWhiteLevel) | 0.1–0.4 | — | no preview | black level fixed; no sRAW in his files |
| Fujifilm X-T2 | RAF compressed | 153 / 153 | 6160×4032 → 6000×4000 (LibRaw 5998×3998) | — | 1018–1023 / 16319–16383 | 3.7–7.3 → 0.5–1.4 | — | **5.68** (3) | ΔL* +2.9 vs Lightroom (colour, not decode) |
| Fujifilm X100F | RAF | 106 / 101 | 6160×4032 → 6000×4000 | 6000×4000 ✓ (106 SP) | 1022 / 16319–16383 | 10–22 (DR400 bias lost) → 1.0–1.5 | — | **3.86** (7) | |
| Apple iPhone 13 Pro | ProRAW DNG (linear, gain table map) | 80 / 80 | 4032×3024 | 4032×3024 ✓ (93 SP) | 0 / 65535 | 20–33 (ΔL* −21…−40) → 0.00 | 8.4–9.1 | — | iPhone 13 Pro Max JPEGs: 6.80 (4) |
| Apple iPhone 17 Pro | ProRAW DNG, 10-bit lossless JPEG, 12 / 48 MP | 0 / 461 (`~/Pictures`) | 4032×3024, 8064×6048 | — | 0 / 65535 | 20–24 → 0.00 | 5.8–10.3 | no preview | not in the catalog yet |
| Sony A7 III (ILCE-7M3) | ARW | 0 / ~1,950 | 6048×4024 → 6000×4000 | — | 512 / 16383 | 0.00 → 0.00 | — | no preview | also Lightroom pano DNGs: 3.8–5.5 → 0.00 |
| Sony A7S III (ILCE-7SM3) | ARW | 0 / 1 folder | 4288×2848 → 4240×2832 | — | 512 / 16383 | 0.00 | — | no preview | |
| Sony A7 (ILCE-7) | ARW (pre-2017) | 0 / 1 folder | 6048×4024 → 6000×4000 (LibRaw 6024×4024) | — | 512 / 16383 | 4.45 | — | no preview | LibRaw's WB is bluer (sky b* −5.1; camera JPEG −2.4, native +0.3) |
| Canon EOS Rebel T3 (1100D) | CR2 | 0 / 38 | 4352×2874 → 4272×2848 (LibRaw 4272×2847) | — | 2053–2282 → **2048** / 16383 → **15531** | 10.8–11.0 → 2.7–2.9 (LibRaw: white 13584 from its table) | — | no preview | black, white and Adobe profile fixed |
| Nikon D3400 | NEF | 0 / 3 (+ 3 copies) | 6016×4016 → 6000×4000 | — | 150 / 4095 | 0.00 | — | no preview | |
| Blackmagic Pocket Cinema Camera | CinemaDNG (12-bit lossless JPEG tiles, log linearization table) | 0 / ~1.5 M frames, 181 folders | 1952×1112 → 1920×1080 | — | 256 / 51080 | 10–15 (ΔL* −13…−19) → 0.00 | 8.9–10.4 | — | video frames; make read from UniqueCameraModel |
| Lightroom smart previews | DNG 1.7 JPEG XL (2560 px linear RGB) | 3,745 (+ 888 lossy-JPEG ones) | 2560 px | — | 0 / 65535 | — | (SDK built without JXL) | 0.64 vs its CR3 original | was "unsupported raw variant" |

## What was wrong, and fixed

- **Canon CR2 black level** (all Canon bodies with masked columns): `SensorInfo`'s left border can
  lie past the first image columns, so the "masked" columns included image data. EOS Rebel T3:
  black 2053–2282 depending on the scene (true 2048: shadows crushed); CC0 files: 6D 2591 for
  2047, 5DS R 2178 for 2047, 40D 1165 for 1024, 5D Mark II 1095 for 1024, 5D Mark III 2095 for
  2048. The black level now comes from the longest run of masked columns at the reference level
  (`crates/raw/src/vendor/mod.rs` `black_columns`); every one matches LibRaw within 1.
- **White level hidden by hot pixels** (Rebel T3): saturation at 15698 with a few samples up to
  16353; the plateau search started at the absolute maximum, missed it, left white at 16383, and
  clipped lamps rendered pink. It now starts at the highest value held by more than a few samples.
- **JPEG XL DNG** (DNG 1.7, Compression 52546): 3,745 of Lightroom's smart previews (and Lightroom's
  JPEG XL DNG exports) were "unsupported raw variant". Decoded natively now with `jxl-oxide`; a
  smart preview's render matches its CR3 original within ΔE 0.64.
- **LibRaw fallback dropped the file's own tags**: a DNG or RAF that only LibRaw could unpack lost
  `BaselineExposure`, forward matrices, profile tables, the ProRAW gain table map, opcode lists
  (Leica Q2's `FixBadPixelsConstant` and `WarpRectilinear`) and Fujifilm's DR exposure bias, and
  rendered up to ~3 EV too dark. The native header pass now supplies them.
- **CinemaDNG make**: Blackmagic frames carry only `UniqueCameraModel`; the make was unknown.
- **Canon CR2 white level** came from the data (the 14-bit maximum when nothing clipped, the
  plateau when something did): frames of one shoot differed by up to ~0.1 EV. It is now the
  camera's `SpecularWhiteLevel` for the photo's ISO from `ColorData` (as LibRaw reads it; CC0
  550D, 5D Mark II/III, 5DS R, 6D, 7D, 80D identical to LibRaw).
- **Canon regional names**: an EOS Rebel T3 (or any Rebel / Kiss) found no Adobe profile, because
  Camera Raw files it under the European name (`Canon EOS 1100D`), and rendered on the neutral
  fallback colour, washed out next to its camera JPEG. `crates/engine/src/adobe.rs` maps the
  Rebel / Kiss names to Camera Raw's.

## Still open

- **Specular or normal white?** Canon's `ColorData` carries a `NormalWhiteLevel` ~512 below the
  `SpecularWhiteLevel` LightCraft (like LibRaw) uses; Adobe's own 5D Mark III DNGs say 15000, close
  to the normal level (15041 at ISO 100). Which one Lightroom clips at is unmeasured (no 5D Mark III
  previews stored).
- **LibRaw's X-Trans crop** is 5998×3998 (Lightroom 6000×4000); only matters when LibRaw is forced.
- **The DNG SDK build here has no JPEG XL** (`qDNGSupportJXL` off): the SDK can't render JXL
  smart previews, so they're checked against their originals instead.
- **5D Mark III and Q2 originals** in the catalog point at a drive path that isn't mounted
  (`//lot48/lot48/MY MEDIA/…`); Q2 and 5D Mark III files found in other folders were checked
  instead, plus the 185 5D Mark III smart previews' geometry and a CC0 5D Mark III file.
- Lightroom previews for Q2, 5D Mark III, iPhone 17 Pro, the Sony and Blackmagic cameras: none
  stored, so no ΔE vs Lightroom for them. Reopening the catalog in Lightroom Classic rebuilds
  `Previews.lrdata` (`lr_compare.py prepare` then covers them).
- Natively missing but absent from Bryan's library (LibRaw reads them in this fork): Canon
  sRAW/mRAW (his 5D Mark III shot full raw: 185 smart previews at 5760×3840), CR3 variants other
  than lossless / C-RAW 0x100/0x200, compressed ORF, NEF "lossy after split".

## Reproducing

Scripts used (kept in the session scratchpad, not the repo): `survey.py` (native / LibRaw / SDK
renders and ΔE per file), the `lr_compare.py` acceptance run (`LIGHTCRAFT_CAMERAS` pointed at a
scratch copy so `lr-compare.json` isn't touched), `exiftool -T -Model -OriginalRawFileName
-OriginalDefaultFinalSize` over the smart previews, and `decodecheck` over one file per folder.
