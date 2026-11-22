# People, AI culling and AI search

Lightroom Classic finds the faces in a catalog, groups the faces of the same person, and lets you name people
(a name is a **person keyword**); its Assisted Culling looks for closed eyes and soft faces; Lightroom's AI search
finds photos by what is in them. LightCraft does the same with open models run on our own code
(`crates/vision`), downloaded only when you ask for them.

## What runs

| Step | Model (downloaded on request, never bundled) | How |
|---|---|---|
| Find faces | YuNet 2023mar (OpenCV Zoo, MIT, 0.2 MB) | boxes + five landmarks on a 1280 px picture of the photo (a raw's embedded preview, else its source decoded at that size) |
| Recognise faces | SFace 2021dec (OpenCV Zoo, Apache-2.0, 39 MB) | the face aligned on its landmarks (ArcFace template, 112 × 112) → 128 numbers; same person ≈ cosine ≥ 0.36 |
| Eyes open / closed | open-closed-eye-0001 (Intel Open Model Zoo, Apache-2.0, 46 kB) | a grey crop around each eye landmark |
| Face focus | none | the culling focus measure (Laplacian energy) on the face alone |
| AI search | CLIP ViT-B/32 trained on LAION-2B (LAION, MIT, 607 MB) | one 512-number embedding per photo; a phrase's embedding ranks them |

The three ONNX models run on a small ONNX interpreter of our own (`crates/vision/src/graph.rs`, CPU, candle's matrix
product): no ONNX Runtime, no protoc. It was checked against OpenCV 4.12's `FaceDetectorYN` / `FaceRecognizerSF`:
all 29 faces of the 1927 Solvay conference photo, boxes within a pixel, and the same-person similarity of two
portraits 0.808 (OpenCV: 0.824). CLIP is ported from candle-transformers and runs on Metal on macOS.

Models live in `~/Library/Application Support/LightCraft/Models/Vision/{faces,search}/` (override:
`LIGHTCRAFT_VISION_MODELS`). The People view's `get.` and the search field's AI mode offer them with their size and
licence; `vision.model.download {model: faces|search, acknowledged: true}` does the same. Every file is size- and
SHA-256-pinned, tried from two mirrors (Hugging Face at a pinned commit, then GitHub / OpenVINO's storage).

## The indexes

Results are kept beside the library in `vision/faces.lcidx` and `vision/search.lcidx`: append-only, one
checksummed record per photo (keyed by its content key), so a quit, a crash or `stop.` loses at most the photos in
flight and the next run goes on from there. A model change starts an index over (the old file is kept as `.old`).
~14,000 photos take ≈ 10 MB (faces) and ≈ 30 MB (search). Indexing runs on worker threads (2–4 decoders, the
analysis itself parallel) as background work, so the grid and loupe always go first.

## People view

View ▸ People (or the toolbar): **Named People** and **Unnamed People**, for the photos the current source and
filters show.

- `find.` finds faces in the photos shown (those not analysed yet) in the background; the header shows the run
  (`finding faces 120 / 900 · 6.2/s`, `stop.`). Found faces become unnamed face regions (catalog format v9,
  `Op::SetRegions`; LightCraft's bookkeeping, not an undo step).
- Unnamed faces are grouped into stacks of the same person (re-grouped on a worker thread whenever the index
  grows). Type a name under a stack and press Return to name all of its faces. A stack whose faces look like a named
  person shows `Ann?` with ✓ / ✗.
- Stacks are picked with click / ⌘-click / ⇧-click / `all.` / `none.`; dragging picked stacks onto a named card names
  them all.
- A named card opens the person's page: **Similar** (unnamed faces suggested as them, each with ✓ / ✗, `yes.` / `no.`
  for the picked ones) and **Confirmed** (their faces; `unname.`, `rm.`), `rename.`, `photos.` (their photos in the
  grid). ⌥-click on a card goes straight to their photos.
- A rejected suggestion is remembered on the face (`Region.rejected`) and never suggested again.

Naming a face sets the region's name and puts the person keyword on the photo (an existing person keyword of that
name anywhere in the keyword list is reused, e.g. `People|Ann`; otherwise a top-level keyword marked as a person).
Person keywords follow their own Include on Export option and leave exports with Remove Person Info. Renaming a
person renames every face (onto a name someone already has, the two merge). Everything is undoable.

In the loupe (View ▸ Face Boxes) every unnamed box has a `?` under it: click, type a name, Return.
`photo.addFace {rect, name?}` draws a face region (its embedding is computed right away when the models are there).

## Lightroom and XMP

- XMP: named face regions are written to the sidecar as MWG regions (`mwg-rs:Regions`, with
  `AppliedToDimensions`), as Lightroom Classic writes them; unnamed faces stay in the catalog. Reading a sidecar
  keeps the catalog's unnamed faces the file doesn't cover, and rejected suggestions.
- Lightroom catalogs (Library ▸ Migrate from Lightroom): `AgLibraryFace` boxes become face regions (turned from the
  frame Lightroom shows into the file's upright frame), `AgLibraryKeywordFace` rows with `userPick` name them (a
  person keyword), rows with `userReject` become rejected suggestions. The catalog is read from a copy, never opened.

## Assisted Culling

Photo ▸ Assisted Culling (`photo.analyze`) also records, when the face models are installed, the faces in each
photo, how many of the main faces (at least a quarter the size of the largest) blink, and the focus of the largest
face. A burst's best is the one where nobody blinks and the main face is sharpest. Options reject photos with
closed eyes (`rejectEyesClosed`) or a soft face (`rejectFaceBelow`). Smart albums / rules: Face Focus, Eyes Closed,
Faces Found.

## AI search

The search field's `ai.` switch: describe the photo ("dog on the beach", "red car at night") and press Return. The
grid shows the best matches, best first, with every other filter still applied (`AI search: “…”` chip to clear).
The first time it downloads the model (the field says the size and licence) and indexes the library in the
background. `search.ai {query}` / `library.filter {semantic}` do the same from the CLI, MCP and control channel.

A blink is both eyes of a main face clearly closed (each ≥ 0.7, together ≥ 0.85): a squint, a wink or glasses
that confuse the eye network don't reject a photo. The network was trained on infrared eye crops; it is right on the
public-domain test portraits and agrees with OpenCV + ONNX Runtime on Bryan's photos, but has not been checked on a
large labelled set.

## Measured

On an M-series Mac (12 cores), per model call on one core: YuNet 45 ms on a 1280 px picture, SFace 15 ms a face, the
eye network < 1 ms; CLIP ViT-B/32 about 50 ms a photo in batches of 16 on Metal; a text query 7–60 ms (the first
one loads the model: ~1 s, more on first Metal use). Indexing throughput was measured only while other builds kept
the load average at 140–200, so the numbers are a floor: faces 0.5–1 photo/s (40 iPhone JPEGs, 30 Canon R6 CR3s
from the embedded preview), search 3–12 photos/s. Without that load, expect several photos a second for faces (2–4
photos at once, each decode ≈ 0.1 s) — ~14,000 photos in under an hour, in the background — and faster for search.
Memory: the face models ≈ 40 MB, CLIP ≈ 600 MB while indexing or after a search (kept for the next query), the
indexes ≈ 1 kB (faces) and 2 kB (search) a photo.

The benchmark test: `cargo test -p lightcraft-engine --release --features enhance tests_people::bench -- --ignored
--nocapture` (`LC_VISION_MODELS`, `LC_VISION_BENCH`); `LIGHTCRAFT_PROFILE=1` prints per-photo and per-operator
timings.
