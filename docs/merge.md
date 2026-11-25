# Photo Merge (HDR, Panorama, HDR Panorama)

`crates/merge` (layer L3, no UI dependencies) implements the algorithms; `crates/engine/src/merge.rs`
plans, runs and finishes merges in the library; the desktop UI shows a dialog with a live preview.
All algorithms are our own implementations from published papers (cited in the module docs).

## Using it

- **App:** select the photos, then *Photo Merge ▸ HDR…* (`Ctrl+H`), *Panorama…* (`Ctrl+M`) or
  *HDR Panorama…* from the photo context menu (or the `dialog.merge*` commands). The dialog previews
  the merge at ≤ 1024 px and re-runs the preview when an option changes; *Merge* runs the full merge
  in the background (progress in a toast). The result is written next to the first photo as
  `<name>-HDR.dng`, `-Pano.dng` or `-HDR-Pano.dng`, imported and selected.
- **Commands** (CLI, MCP, control channel):
  - `merge.hdr {ids?, align=true, deghost=none|low|medium|high, autoSettings=true, stack=false, preview=false, showOverlay=false, previewPath?}`
  - `merge.panorama {ids?, projection=auto|spherical|cylindrical|perspective, boundaryWarp=0..100, autoCrop=false, crop=[x0,y0,x1,y1] (normalized; replaces the auto-crop rectangle — the dialog's draggable crop frame), fillEdges=false, autoSettings=true, maxMegapixels=40, preview, previewPath?}`
  - `merge.hdrPanorama {ids?, bracket=0 (from EXIF), …both sets of options}`
- **CLI:** `lightcraft-cli merge hdr|panorama|hdr-panorama [options] FILES…`;
  `lightcraft-cli synth-merge hdr|panorama -o DIR` writes synthetic inputs (procedural scene).

## What it does

**HDR** (`hdr.rs`): frames ordered by EXIF exposure (or median luminance); the middle one is the
reference. Auto Align: features on exposure-normalised luminance, RANSAC translation / similarity /
homography (the simplest model that explains ≥ 97 % of the matches). Exposure ratios are measured
(median log ratio of well-exposed pixels between brightness neighbours), EXIF is the fallback.
Radiance is a weighted mean in linear light, weight `k · (1 − smoothstep(0.8·clip, 0.96·clip, max
channel))`. Deghost: a per-pixel consistency test against the reference radiance (with a noise
floor), dilated and feathered masks; the union is the deghost overlay. Output: 16-bit float DNG.

**Panorama** (`pano/`): MOPS-style features, pairwise RANSAC homographies with Brown & Lowe's
verification, focal from homographies (or EXIF 35 mm), rotations chained along a maximum spanning
tree, Levenberg–Marquardt bundle adjustment of rotations + focal lengths, straightening.
Projections: spherical, cylindrical, perspective; *Auto* picks perspective for compact fields of
view < 110°, cylindrical when the vertical field is < 90°, else spherical. Exposure: EXIF, then
gain compensation from the overlaps. Seams: each pixel goes to the image with the highest
centre-weighted feather weight; multi-band blending (Laplacian pyramids) in log space. Boundary
Warp: a separable stretch of the smoothed valid-area edges to the canvas edges. Auto Crop: the
largest rectangle of image data (set as the develop crop, so it stays adjustable). Fill Edges:
push–pull diffusion. Output: 16-bit integer DNG (16-bit float for HDR panoramas).

**DNG output** (`output.rs`): LinearRaw, 3 samples, Deflate tiles; values scaled below 1.0 with the
scale in `BaselineExposure`; raw sources keep their DNG colour tags (camera RGB), other sources get
`ColorMatrix1`/`ForwardMatrix1` of their RGB space.

## Measured (synthetic tests, `cargo test -p lightcraft-merge`)

- HDR, 3 brackets ±2 EV with 14-bit noise and handheld shifts: alignment error < 0.3 px, exposure
  error < 0.02 EV, radiance median log error < 0.03 (noise-limited, bias < 0.01), recovered
  highlights < 0.05; moving object: ghost error 0.2–2.0 without deghosting, < 0.06 with High.
- Panorama, 5 views over 182° with a 1.3× exposure drift: relative rotations < 0.1°, focal < 1 %,
  BA RMS ≈ 0.35 px; median log error vs ground truth < 0.02; no seam steps.

## Limits (known gaps)

- No lens-distortion model in the panorama bundle adjustment (rectilinear lenses assumed); no
  360° wrap-around; flat (translating-camera) scans are fitted with the rotation model.
- No camera response calibration for JPEG brackets (sRGB decoding is taken as linear light).
- Boundary Warp is a separable stretch, not a content-preserving mesh warp; Fill Edges is diffusion,
  not patch synthesis.
- *Create Stack*: the result goes on top of a collapsed stack of its sources (`Catalog::stack_with_ops`); its
  caption also lists them.
- Panorama output is limited to `maxMegapixels` (default 40) — blending memory is ~25 bytes/pixel.
- The browser build has no file access for merges yet.

## merges. — found merges (Bryan's request, 2026-11-23; 2026-11-24)

LightCraft looks for photos that belong together and lists them under a `merges.` row beneath
their shoot folder and their collections; nothing is merged until he says so. Most of what he
shoots is panoramas (handheld, aperture priority), so finding them readily comes first.

- **Finding** (`crates/engine/src/merge_find.rs`, `merge.find`) — three steps:
  1. *Time* (`plan`): per folder and camera, in capture order, frames from one body / lens / focal
     length taken a few seconds apart (5 s plus the exposure) form a run. Exposure never splits a
     run: a handheld panorama's shutter drifts.
  2. *Similarity* (`decide`, `seen` / `verify`; rebuilt 11-24 on Bryan's real shoots, where the
     old 64 px thumbnail correlation had matched a canyon with a person on a path): every pair of
     neighbouring frames is verified by feature matching on the 512 px source the finder loads —
     the merge crate's own detector (`features::detect`, exposure-normalised grey), matches,
     a RANSAC homography, then: ≥ 20 inliers passing Brown & Lowe's test, spread over the
     overlap, no zoom (± 12 %) or tilt (≤ 6°) to speak of, ≥ 15 % overlap, and the two frames'
     fine detail correlating ≥ 0.35 once aligned (people who moved, or another scene, fail).
     Anything it can't confirm is unrelated — never a merge at any confidence. Verified frames
     of one framing (shift < 6 %) make a *position*: a bracket when the exposures step (≥ 0.9 EV
     in all) and the frames were fired back to back (≤ 3 s plus the exposure) aligned within
     3 % with their detail agreeing ≥ 0.5 — two shots of one scene seconds apart are two photos;
     a focus stack when a recorded focus distance steps (three or more), or — no focus distance
     recorded (a manual Leica lens) — four or more tripod-steady frames (≤ 1 % apart) whose sharp
     4 × 4 tiles sweep one way; else a burst (nothing). Positions linked by verified shifts
     (15–75 % overlap) moving one way are a `panorama.` (`hdr.panorama.` when most are
     brackets); a chain that turns back is cut into its one-way stretches. Two frames alone are a
     panorama only with ≥ 50 inliers, ≤ 50 % overlap, and no other view of the scene beside them
     in the run (a wedding's reframed sequence). Where there are no pixels at all (no source), a
     bracket fired quickly with distinct exposures still counts, as `maybe.` / `likely.`.
  3. *Classify + score*: each group gets `confidence` — `sure.` (three positions or more, every
     link ≥ 40 inliers and detail ≥ 0.6; a bracket so matched whose camera stepped the
     compensation or that spans ≥ 1.5 EV), `likely.` (verified; two-frame panoramas are never
     sure), `maybe.` — and its `reasons` in plain words ("5 frames in 2.1 s", "each frame
     overlaps the last by ~40 %", "frames move left → right", "120+ matched points between
     neighbours", "exposure drifts by 0.7 stop (auto exposure): still one panorama",
     "exposures −2 / 0 / +2", "same framing", "focus moves near → far").
  Evaluation on real photos: `cargo run --release -p lightcraft-engine --example merge_eval --
  <copy of catalog.snap> <truth.json> [--lr-panos]` (read-only; prints truth vs verdict per
  group and every pair's match numbers; `--lr-panos` checks Lightroom's own `-Pano.dng` merges).
  Metadata alone settles two cases without pixels: a lone bracket whose camera stepped the
  exposure compensation (fired back to back), and one framing at three or more recorded focus distances.
- **One pool per shoot**: a shoot folder with camera subfolders (M262, GR3, X-T2 …) is one
  merges. pool (`Scope::Folder {deep: true}`, the shoots of `lightcraft_catalog::shoots`); the
  sidebar's `merges.` row sits on the shoot folder and its subfolders get none. A group never
  mixes two cameras; each shows its camera and subfolder.
- **When**: Settings ▸ General ▸ find merges: `auto.` (default) looks in the background at folders
  whose photos changed (fingerprints in the `merge.scanned` record; all folders the first time);
  `find.` only when find. is clicked in a merges. view.
- **Groups**: `merge.group.<id>` library records (kind, frames in the group, every frame found
  with it (`foundFrames`), open / merged / dismissed, merged photo, confidence, reasons). Records
  from before 2026-11-24 still load (likely., reasons worked out from the frames' metadata). A
  group sharing a frame with an earlier one — sent-back ones too — is never suggested again.
  `merge.setFrames` takes frames out of a group or puts them back; `merge.dismiss` (send.back.)
  marks it not a merge, `merge.restore` brings it back.
- **The view** (`crates/ui-egui/src/merges.rs`): not the grid. Each group is a panel — kind
  (a menu), confidence, camera and subfolder chips, frame count, reasons, merge. / send.back. — with
  its frames as overlapping cards fanned on a gentle wave. Hover spreads them side by side
  (animated); a click on the group's empty area keeps them spread; the triangle folds the group.
  A click on a card takes that frame out (dimmed, with a mark, still clickable) or puts it back; a
  double-click opens it in the loupe. Groups are picked by their header (click, ⌘-click, ⇧-click,
  all. / none., ⌘A). Then the `merged.` photos, then a foldable `sent back.` section with
  restore. Bar: find. all. none. merge. send.back. both. merges.only. unmerge. smaller. bigger.
- **Merging** (`merge.mergeGroups`; in the app one group at a time on the merge worker): the frames
  in the group are merged into a DNG, imported as usual; its frames get `Photo.merge.into` and
  leave every view and count (catalog v12; never deleted, files untouched); the merged photo goes
  into each collection they were in, at the first frame's place, keeps their capture time, and
  shows its kind on the thumbnail. `merge.showIn` both. / merges.only., `merge.unmerge` brings the
  frames back.
- **Second Brain** reads and acts through the same commands (`merge.candidates`, `merge.thumbs`,
  …; see `docs/control-protocol.md` → merges.), so both apps always show the same groups.
