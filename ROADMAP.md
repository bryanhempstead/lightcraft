# LightCraft roadmap

Milestones toward full Adobe Lightroom parity (cloud Lightroom first, then every Lightroom Classic module), with wall-clock
estimates for continuous (24/7) agent-driven development with 4–6 parallel agents. Estimates are calibrated on the sibling
projects (DrawCraft reached its first four milestones in ≈ 4½ h) and are revised as milestones land.

## Where we stand

*Honest assessment, 2026-10-08. Agents: read this before picking work. The checklist in
[`docs/parity.md`](docs/parity.md) counts features that **exist**; this section is about whether a photographer can
**switch** from Lightroom. Update it when a gap below closes.*

**In one line:** the checklist says **79.7%** (P0 98.2%, P1 95.6%, P2 41.6%), but measured by whether a working
photographer could replace Lightroom without noticing, we are at roughly **60–70%**. The remaining gap is mostly
**quality of results and camera coverage**, not missing buttons.

Caveat on the checklist: ✅ is set by whoever lands a feature, and nobody has systematically checked rows against
Lightroom's behaviour or output. Bugs keep turning up in ✅ areas (CR2 colour-filter phase on some Canon models #85,
duplicate Local entries #22, black GPU exports on an Intel iGPU #78).

### By dimension

| Dimension | Estimate | What's true today | Biggest gaps |
|---|---:|---|---|
| **Feature checklist** | 79.7% | P0 core and P1 nearly complete: import (Add / Copy / Move, templates, devices), library, grid/loupe/compare/survey, every Edit slider, curves, colour grading, masking tools, crop/Upright, heal/clone, presets/profiles, versions/history, sync, export, menus, shortcuts | P0: full-resolution zoom detail; P1: lens-profile database, content-aware fill (patch synthesis), video playback/trim |
| **RAW coverage** (formats people shoot) | ~80% | DNG (all kinds), CR2, CR3 lossless Bayer and version 0x100/0x200 C-RAW (M50/R100/R8 pixel-exact corpus; 2,451 of 2,451 of Bryan's R6 files), ARW, NEF (uncompressed + Huffman lossless/lossy), RAF (uncompressed, lossless/lossy compressed Bayer and X-Trans), uncompressed ORF, RW2 / RWL / Panasonic RAW (every raw format, checked on 178 files from 118 Panasonic and Leica bodies), PEF; every container's embedded preview (incl. CR3). **This fork: LibRaw 0.22.3 as the fallback decoder** (`crates/libraw-sys`, `tools/fetch-libraw.sh`; native decoder first, then LibRaw, then the embedded preview): every camera LibRaw lists (1,258 models) decodes from sensor data, developed by LightCraft's own pipeline (Adobe profile via the DNG SDK): checked on Canon sRAW, R6 crop-mode C-RAW, CRW, Minolta MRW, Samsung SRW | Per-model verification is thin (~55 corpus files vs >1,000 models); LibRaw-decoded files have no camera-JPEG colour fit (Adobe matrix / profile instead); JPEG XL DNG, Foveon X3F; HEIC/AVIF in pure Rust; without `vendor/` (and on the web) the old gaps return: compressed ORF variants, Nikon lossy-after-split, Canon sRAW, unverified CRX variants |
| **Colour & image quality** | ~55–65% | Pipeline is complete and fast; GPU path CPU-exact within 1/255 | **No measured camera calibration database**: ARW, NEF, RW2, RAF and CR3 have guarded camera-JPEG colour estimates, with bundled ILCE-7M4, X-H2S and X-T4 profiles (docs/camera-preview-colour.md); other non-DNG raws and rejected estimates use a neutral matrix. Colour fidelity remains incomplete. No lens-profile database. Fidelity against Lightroom measured per slider with the Camera Raw oracle in this fork (`docs/lr-match.md`): Texture, Dehaze, Clarity (positive), Color Grading, Point Color, B&W and the masks' Contrast / Whites / Blacks now render as Camera Raw does or close to it (rounds 10–11); Basic tone (Highlights / Shadows: Camera Raw's rule measured, off) and Clarity's regional moves still differ |
| **AI & computational** | ~45% | Assisted culling (focus, bursts, blinks, face focus, smiles / expressions; at import too), auto tone, HDR/panorama merge; AI masks in pure Rust with optional downloaded models: Sky, Landscape classes and People parts (SegFormer), Object / Describe / Subject (SAM 3), Update AI Settings; Super Resolution (~43 s per 24 MP photo); Lightroom's adaptive presets with their AI masks; faces / People (YuNet + SFace, found by themselves in the background or as needed, drawn and moved in the loupe, named and unnamed people, bulk confirm / reject, person keywords kept in step, XMP and Lightroom face regions), Select Subject following the camera's focus point, and natural-language search (CLIP) — `docs/people.md` | Adaptive profiles' per-photo part (Camera Raw's model is encrypted), diffusion-grade generative remove (LaMa fills and DSRNet reflection removal are in: `docs/remove.md`), pets in People, the expression network checked on a labelled set of real photos. **Blocked on a model strategy** (licensable weights or our own training; pure-Rust inference is feasible) |
| **Workflow & library** | ~85% (single machine) | Robust catalog (journal + snapshots, background compaction, crash-tested), 85k-photo libraries stay responsive, Local browsing with automatic cleanup, XMP interop, keywords, smart albums, Move import; Settings ▸ cams. (fork) shows per camera the decoder, the profile that colours it, lens profiles and the last Lightroom ΔE, and an import recognises a camera it hasn't seen | Opening an 85k library takes 1.7–4.7 s (catalog backups, Trash deletes and Lightroom naming landed 2026-11-21); no cloud sync (out of scope), no tablet companion (#74, roadmap), shared albums, online publish services (Hard Drive / Apple Photos publishing landed 2026-11-22), tethering |
| **Classic modules** | ~30% | Geotagging from GPX track logs, the Map module (OpenStreetMap, drag to geotag, saved locations), soft proofing (partial), slideshow (basic) | **Book, Print, Slideshow module, Web, online publish services**: ~40 tracker rows ⬜ |
| **HDR & video** | HDR ~85%, video ~55% | HDR editing (Lightroom's HDR button, HDR Limit, SDR rendition with SDRBlend, Visualize HDR with beyond-the-display marks, Lightroom's split HDR histogram, `crs:` round trip), HDR export (gain map JPEG, gain map AVIF on macOS, PQ / HLG AVIF, PNG, JPEG XL, float TIFF; open as HDR in macOS), EDR loupe / Compare / Survey / Second Window on macOS, HDR renders on the GPU (draft drag ~13 ms at 2.5 MP); video (fork, 2026-11-22): import, Loupe playback with sound, trim, poster frame, Capture Frame, the whole Edit panel on frames (one GPU render for playback and export), trimmed and edited H.264 export (ffmpeg when installed, else macOS AVFoundation / avconvert) | HDR tone measured against Lightroom (and its SDR rendition is a local tone map, ours the HDR-off render), Windows / Linux HDR display; pure-Rust video decoding |
| **Platform & robustness** | ~70% | macOS native; Windows/Linux builds; web via WASM; no-panic lints workspace-wide, `unsafe` confined to `crates/sysmem`; failed saves are reported; GPU errors fall back to CPU | Windows installer UI unverified on Windows (PR #79); GPU path proven only on Apple + user reports; English, Simplified/Traditional Chinese, Japanese, Brazilian Portuguese and German UI (see docs/localization.md); remaining technical errors; Traditional Chinese uses the Simplified Chinese font until craft-fonts has a TC face; accessibility partial; headless UI tests time out under machine load |

### By kind of user

| User | Readiness | What blocks them |
|---|---:|---|
| JPEG / DNG shooter, single machine | ~85% | Fidelity polish, AI masks |
| Nikon / Sony / Panasonic / older-Canon raw shooter | ~65% | Camera colour fidelity and coverage (ARW, NEF and RW2 preview estimates are only a starting point); Panasonic compacts and kit zooms also miss their embedded distortion correction |
| Fujifilm raw shooter | ~65% | Uncompressed and lossless/lossy compressed RAF decoded; camera colour calibration remains missing. Verified on 13 bodies, including 14/16-bit GFX; see `docs/raf-compression.md` |
| Canon CR3 / Olympus compressed-raw shooter | ~65% | CR3 lossless Bayer and version 0x100/0x200 C-RAW develop from sensor data (M50, R100, R8 verified pixel for pixel; all 2,451 of Bryan's R6 CR3s); other CRX variants (e.g. R6 crop-mode C-RAW) and compressed ORF go through LibRaw in this fork instead of the embedded JPEG. Camera colour and broader model verification remain |
| Lightroom Classic power user | ~45% | Print, Book, publish, tethering (catalogs migrate with history and snapshots, and re-sync) |
| Relies on AI (masks, denoise) | ~40% | Sky, Landscape, People parts, Object / Describe / Subject masks via optional downloaded models (SegFormer, SAM 3); AI Denoise / Raw Details / Super Resolution / Lens Blur; Lightroom's adaptive presets |

## Where we're going

Priorities, in order. Each points at tracker rows in [`docs/parity.md`](docs/parity.md) → *Top gaps*.

1. **Camera colour calibration of our own** (LR-PROF-CAMERACOLOR, P0): fit each camera to its own embedded JPEG, use
   matrices the files carry themselves, then chart shots. Sony ARW, Nikon NEF, Panasonic RW2 and Fujifilm RAF have the file-local fit
   (matrix + tone curve from their own JPEG); generalise it to the other makes' raws (PEF, ORF…), then validate fidelity.
2. **Raw formats** (LR-IMP-FORMATS, P0): this fork falls back to LibRaw (2026-11-21; GPL/LGPL allowed for Bryan's
   personal build), so unverified CR3 CRX variants, compressed ORF, NEF lossy-after-split, sRAW, CRW/MRW/SRW and the
   long tail decode when `tools/fetch-libraw.sh` has run. Left: JPEG XL DNG, Foveon, pure-Rust HEIC/AVIF, and native
   (faster, wasm-capable) decoders for the variants that now only LibRaw reads.
3. **Verified camera coverage** (LR-IMP-CAMERA-COVERAGE, P0): a CC0 sample per model in the corpus, each decoded and
   checked for plausible colour; fix per-model bugs (#85).
4. **Render fidelity suite** (LR-BEHAV-RENDER-FIDELITY, P1): measure our output against Lightroom on the same CC0 raws
   (references stay in the local `plan/`), then tune against the numbers.
5. **Lens profiles of our own** (LR-EDIT-OPTICS-PROFILE, P1).
6. **AI model strategy** (maintainer decision): which permissively licensed models (or our own training) for
   segmentation masks and denoise; then pure-Rust inference. Masks are done this way in Bryan's fork: SAM 3
   (Object / Describe / Subject) and three public SegFormer checkpoints (Sky, Landscape, People parts), each an
   optional, consented, hash-pinned download (docs/ai-masks.md). Enhance (Denoise, Raw Details, Super
   Resolution), Lens Blur and Depth Range run open models too (RawNIND GPL-3.0, Real-ESRGAN BSD-3,
   Depth Anything V2 Apache-2.0; `docs/enhance.md`); Super Resolution makes a 24 MP photo in ~43 s on an M3 Pro. Generative Remove
   runs LaMa big-lama (Apache-2.0), Remove Reflections DSRNet (Apache-2.0), and Remove ▸ Detect offers people / wires /
   dust to pick (`docs/remove.md`). People and AI culling are done the same way (YuNet, SFace, Intel's eye-state and
   OpenCV Zoo's expression networks on our own ONNX interpreter; `docs/people.md`): faces are found by themselves,
   culling runs at import, smiles count.
7. **Then:** HDR tone measured against Lightroom, the Classic output modules (Print first, then Book,
   Slideshow; Map, Hard Drive publishing and video are in), localisation and accessibility.

Already closed in the week of 2026-10-03: compressed NEF (#10), Move import (#29), per-library smart-preview folder,
Local roots / cleanup, grid and catalog performance at 85k photos (#35, #37), failed-save reporting, GPU export
hardening (#78), copyright metadata (#51), GPX geotagging (#60), import tag help and folder templates (#31, #32).

Image navigation now supports native pinch zoom and two-finger pan in Detail, Compare and Reference views.
The whole-image preview still has a configurable size cap (2,560 px default), so 100% on larger photos does not
yet guarantee original-pixel detail; visible-region rendering remains a gap (LR-VIEW-ZOOM).

## Milestones

**Status legend:** ✅ done · 🚧 in progress · ⬜ not started

| # | Milestone | Scope (summary) | Estimate (h) | Status |
|---|---|---|---|---|
| M0 | Skeleton + visual shell | workspace, xtask CI + layering, geom/color/raster, develop model, pipeline v0, catalog v0, engine commands, Lightroom-look UI (grid, loupe, filmstrip, Edit panel), control channel, MCP, web build | 3–5 | ✅ |
| M1 | Library core | import (JPEG/PNG/TIFF/WebP), EXIF/XMP, persistent catalog (op log + snapshots), albums, ratings/flags/labels, filter/search/sort, thumbnail cache, 100k-photo grid | 6–10 | ✅ (100k-photo grid scale test pending) |
| M2 | Pipeline v1 (quality) | WB temp/tint, profiles, local tone mapping (highlights/shadows), curves, HSL, point colour, colour grading, texture/clarity/dehaze, vignette, grain, B&W, auto tone/WB, histogram, before/after | 10–15 | ✅ (look tuning vs our references ongoing) |
| M3 | RAW I | TIFF/DNG (LJ92, deflate, tiles, opcodes), demosaic (AHD/PPG/bilinear), highlight recovery, DNG colour model, CR2, NEF, ARW, embedded previews | 10–15 | 🚧 (DNG, CR2, ARW, NEF uncompressed + Huffman lossless/lossy, embedded previews ✅; NEF lossy-after-split ⬜) |
| M4 | Crop, geometry, optics | crop tool + overlays, straighten, Upright (auto/level/vertical/full/guided), manual transforms, CA, defringe, manual lens corrections | 6–10 | ✅ |
| M5 | Performance | source pyramids, wgpu compute pipeline (CPU oracle), draft/full renders, prefetch, budgets (16 ms slider updates on 24 MP) | 10–15 | 🚧 (stage cache, source pyramid, wgpu pipeline, prefetch, memory budget ✅; colour NR at half resolution, GPU histogram ⬜) |
| M6 | Masking | brush, linear/radial gradients, colour/luminance/depth range, add/subtract/intersect/invert, all local adjustments, masks panel | 8–12 | 🚧 (brush/linear/radial/colour/luminance range, add/subtract/intersect, masks panel ✅; depth range, AI masks ⬜) |
| M7 | Detail | sharpening + masking preview, luminance/colour NR, Denoise, Raw Details, Super Resolution | 6–10 | 🚧 (sharpening, luminance/colour NR ✅ — sharpening, grain and NR measured against Camera Raw at 1:1 in this fork, `docs/lr-match.md` round 9; AI Denoise, Raw Details ✅, Super Resolution ✅ ~43 s / 24 MP, `docs/enhance.md`; mask preview ⬜) |
| M8 | Heal / Remove | content-aware remove (PatchMatch), heal, clone, brush spots, visualize spots, red/pet eye | 6–10 | 🚧 (heal, clone, auto source, visualize spots, red/pet eye ✅; PatchMatch remove ⬜) |
| M9 | Presets, profiles, versions, sync | preset browser + amount, create/import presets, profile browser, versions, history, copy/paste/sync settings | 5–8 | ✅ |
| M10 | Export & share | export dialog (JPEG/PNG/TIFF/DNG/AVIF/JXL/original), sizing, sharpening, metadata, watermark, naming, batch jobs, XMP sidecars, HDR export | 6–10 | 🚧 (all formats incl. DNG/original, sizing, presets, background jobs, JXL encode, HDR export ✅) |
| M11 | RAW II | CR3, RAF (X-Trans), ORF, RW2, PEF, SRW, 3FR, IIQ + long tail; camera calibration DB; HEIC/AVIF/JXL import | 20–35 | 🚧 (CR3 lossless CRX and C-RAW, RAF uncompressed and lossless/lossy compressed, RW2 (every raw format), PEF, ORF uncompressed ✅; **camera colour calibration** 🚧 (guarded ARW, NEF, RW2 and RAF preview fitting; measured database still missing), CR3, compressed ORF ⬜) |
| M12 | AI & smart features | subject/sky/background/people/object masks, semantic search, faces/People (permissively licensed models, pure-Rust inference) | 20–40 | 🟡 masks, faces/People, AI culling, semantic search done (`docs/people.md`) |
| M13 | Merge | HDR merge (deghost), panorama (projections, boundary warp, fill edges), HDR panorama | 10–15 | ✅ |
| M14 | Video | import/playback/trim via FilmCraft crates, global edits + presets on video, video export | 6–10 | 🟡 (fork: ffmpeg / AVFoundation, the whole Edit panel on video; pure-Rust decode open) |
| M15 | Classic modules | Map, Book, Slideshow, Print, Web; smart collections, stacks, virtual copies, publish services, tethering | 25–40 | 🚧 (smart albums, stacks, virtual copies, compare/survey ✅; Map ✅; Book/Slideshow/Print/Web ⬜) |
| M16 | 1.0 polish | preferences, shortcut editor, accessibility, localization, packaging (dmg/msi/AppImage/web), hardening | 10–20 | 🚧 (settings, keyboard shortcuts sheet, packaging basics ✅; English/Chinese (Simplified, Traditional)/Japanese/Brazilian Portuguese/German localisation 🟡; accessibility and further locales ⬜) |

## Parity estimate (feature count updated 2026-10-08; effort estimate from 2026-10-02)

**By feature count** — from `docs/parity.md` (one row per Lightroom feature, menu item and shortcut; `cargo xtask
parity` prints this line on every run, so it stays current):

| Scope | Weighted completion | Rows | 2026-10-02 |
|---|---:|---:|---:|
| P0 (core) | **98.2%** | 200 | 98.7% |
| P1 (important parity) | **95.6%** | 149 | 94.4% |
| P2 (later / AI / niche, incl. Classic modules) | **41.6%** | 160 | 29.4% |
| **All in-scope rows** | **79.7%** | 509 | 75.6% |

The 2026-10-05 count includes three new 🟡 rows that make quality gaps visible (camera colour calibration, verified
camera coverage, render fidelity), which is why P0 dipped slightly.

✅ counts 1, 🟡 ½, ⬜ 0; out-of-scope rows (cloud sharing, Adobe accounts…) are left out.

**By remaining effort** — rows are not equal: a shortcut and the whole Book module are one row each, and what is
left is the heavy part (AI, video, Classic output modules, undocumented raw codecs). Remaining work in **Opus agent-hours**
(one agent working continuously; calibrated on this project — a single lead agent closed ≈ 45 tracker rows of
UI/feature work in ≈ 5 h on 2026-10-01, and ≈ 60 more (preset import incl. `.lrtemplate` / DNG / zip and masks, smart-album
rule editor, auto sync, keyword and label sets, import options and DNG conversion, smart previews, external-editor round
trip, slideshow, auto import…) in ≈ 12 h on 2026-10-02; four to six parallel agents built M0–M13 in ≈ 25 active hours):

| Work package | Tracker rows | Agent-hours | Risk |
|---|---|---:|---|
| Remaining P0/P1 UI and library features (folder rename/move, keyword painter, people view…) | ≈ 8 | 5–10 | low |
| Raw codecs: JPEG XL DNG, HEIC/AVIF decode (the CRX variants, compressed ORF, NEF lossy-after-split and sRAW are LibRaw-decoded in this fork) | LR-IMP-FORMATS | 15–30 | medium |
| Lens-profile database of our own (calibration targets, fitting, data) | LR-EDIT-OPTICS-PROFILE | 15–30 | data collection |
| Video: playback, trim, edits, export (pure-Rust decode, ideally shared with FilmCraft) | R. Video | 20–40 | medium |
| AI: subject / sky / background / people / object masks, object-aware remove, AI denoise, super resolution, lens blur, people & faces, natural-language search, culling | ≈ 30 | 80–150 | **high** — permissively licensed weights, pure-Rust inference, maybe training |
| HDR leftovers: Windows / Linux HDR display, HDR tone (and the SDR rendition's local tone map) vs Lightroom | Q. HDR, LR-VIEW-HDR-DISPLAY | 4–8 | medium |
| Classic modules: Map, Book, Slideshow module, Print, publish services, tethering, soft proofing | ≈ 50 | 60–100 | medium (large, well-understood) |
| Smaller P2 items: Enhance dialog, export to Photos, help / what's new, accessibility, localisation, sidecar variants | ≈ 15 | 12–25 | low |
| Look tuning, performance budgets (colour NR at half resolution on CPU + GPU, 100k-photo library), packaging, hardening | — | 25–45 | medium |
| **Total remaining** | | **≈ 270–505** | |

Spent so far ≈ 105–145 agent-hours, so **by effort the project is ≈ 20–35% of the way to complete Lightroom + Classic
parity**, and ≈ 45–60% of the way for cloud-Lightroom parity without AI and the Classic modules (remaining ≈ 130–255 h,
most of it the raw codecs, lens data, video and HDR).

**Wall clock:** ≈ 270–505 h for one agent working alone; with 4–6 parallel agents (≈ 70% parallel efficiency, merges and
CI under load cost the rest) **≈ 65–125 h of continuous work**. The AI package and the raw codecs carry most of the
uncertainty: they can finish faster if suitable permissive models / documentation turn up, or stall on licensing.

## Totals

Remaining from 2026-10-02 evening (see *Parity estimate* above for the breakdown):

| Target | Remaining agent-hours | Wall clock, 4–6 parallel agents |
|---|---:|---:|
| Cloud-Lightroom parity without AI or Classic modules | ≈ 130–255 h | ≈ 35–65 h |
| Full parity incl. AI, video and the Classic modules | ≈ 270–505 h | ≈ 65–125 h |

The milestone estimates in the table above were made before work started and are kept for calibration (M0–M13 took
≈ 25 active hours with parallel agents against an estimate of ≈ 110–170 h for those milestones).

## Risks that coding hours alone don't retire

- **AI features** (subject/sky/people masks, generative remove) need model weights with licences we can ship; classical
  fallbacks first. No permissively licensed sky-segmentation model was found — we may need to train our own (raw
  denoise: this fork uses RawNIND's GPL-3.0 weights, allowed here, `docs/enhance.md`).
- **Camera colour and lens data** is a data problem: we never use Adobe's matrices, DCPs or LCPs. DNG-embedded data first,
  then our own calibration; long-tail camera/lens coverage grows over time.
- **Raw-format sources:** decided 2026-10-05: decoders are written from *prose* format descriptions (even ones
  published alongside GPL code); decoder source is never read. Still open: freedom-to-operate review for local
  Laplacian filters, PatchMatch and HEVC (HEIC).
- **Look parity** with Adobe's default rendering is tuned by eye today; the planned fidelity suite (LR-BEHAV-RENDER-FIDELITY)
  turns it into measured comparisons against local-only Lightroom references.

## Raw format coverage and known gaps

Sony ILCE-7M4 downsized lossless ARW (subsampled YCbCr 4:2:0 / 4:2:2 tiles) now decodes to linear RGB;
private 3:2 & 4:3 examples verified through native rendering & full-resolution 16-bit export.

CR3 lossless Bayer and version 0x100/0x200 C-RAW sensor decoding is verified pixel-exactly on Canon M50, R100 and R8 (EOS R6 RAW checked on private files); any other CR3, or one that fails to decode, opens from its embedded JPEG as before. See [`docs/cr3.md`](docs/cr3.md) for coding limits, reference provenance and colour limitations.

Decoded (CC0 corpus from raw.pixls.us, `cargo xtask corpus --download`, `crates/raw/tests/corpus.rs`): DNG (uncompressed,
LJ92, lossy JPEG / Smart Previews, Deflate, float, linear), CR2, ARW (uncompressed, ARW2, LJ92; as-shot white balance and black level of pre-2017 bodies from the enciphered
maker-note `Tag2010` and the encrypted `SR2SubIFD`, both recovered by black-box analysis, `crates/raw/src/vendor/arw.rs`), NEF/NRW uncompressed and Huffman-compressed (lossless, lossy type 1/2, 12/14-bit), RAF uncompressed and lossless/lossy compressed (Bayer and
X-Trans, 14/16-bit real-file verification), RW2 / Leica RWL / Panasonic RAW in every raw format (compressed formats 4 and 6, the prefix-coded strips of format 8,
packed formats 2, 5 and 7, the 16-bit words of the 2005–2007 bodies; all recovered by black-box analysis of 178 CC0 files from
118 bodies, `crates/raw/src/vendor/rw2.rs`), PEF (uncompressed and Huffman), ORF uncompressed (16-bit and 12-bit packed). Supported containers yield their embedded JPEG preview when present (including CR3). The engine uses it for RAW variants it cannot decode yet; HEVC-only CR3 previews are not decoded.

**LibRaw fallback (this fork, 2026-11-21).** `crates/libraw-sys` builds LibRaw 0.22.3 (LGPL-2.1 / CDDL-1.0, fetched
and checksum-pinned by `tools/fetch-libraw.sh` into the gitignored `vendor/`, never committed) behind a C ABI shim that
only opens and unpacks: the sensor samples, black/white levels (Canon's SpecularWhiteLevel), CFA, as-shot white balance,
Adobe's XYZ→camera matrix, geometry, default crop, orientation and names become an ordinary `RawImage`, which
LightCraft demosaics and colours itself (the camera's Adobe profile through the DNG SDK when installed). Order: the
native decoder, then LibRaw (a data error LibRaw reports counts as a failure), then the embedded preview with the native
decoder's reason. CRW, MRW and Phase One IIQ are recognised only when LibRaw is built in. `LIGHTCRAFT_RAW_DECODER`
(`libraw`, `nolibraw`, `none`) changes the order for comparisons; `crates/raw/examples/decodecheck.rs` reports which
decoder read each file. Checked: all 2,451 of Bryan's R6 CR3s still decode natively (identical levels, crop and samples
when forced through LibRaw, 156 ms vs 86 ms per 20 MP; against Lightroom's previews of 37 default-setting R6 photos
mean CIEDE2000 2.52 (dE128 1.49) through LibRaw = native 2.52, vs 3.46 (2.72) from the embedded JPEG); CC0 samples that were preview-only now decode with matching
framing and orientation: Canon 5D Mark III sRAW2, R6 crop-mode C-RAW, EOS 10D CRW, Minolta DiMAGE A2 MRW, Samsung NX300 SRW.
Without `vendor/` (and on the web) the list below still applies.

Not decoded natively — LibRaw in this fork, else preview only (no permissively licensed description; black-box analysis incomplete):
- **Nikon "lossy after split" NEF** (non-zero split row in maker note `0x0096`, e.g. some D3400/D5000/D5200/D5500/D5600
  files): rows above the split decode with the regular lossy table; from the split row on a different code is used
  that our black-box analysis has not recovered yet (none of the four regular tables fits, also not with byte
  alignment or reset predictors at the split row). The other Nikon Huffman variants are decoded (`crates/raw/src/vendor/nefc.rs` documents the analysis).
- **Olympus compressed ORF**, **Canon CR3 unverified CRX variants** (M11.1), **Canon sRAW/mRAW**, lossy DNG.

**Camera colour matrices:** ARW, NEF, RW2, RAF and CR3 files can use guarded, separate chromaticity and tone estimates from their own embedded JPEG (see `docs/camera-preview-colour.md`); this is a per-file camera-look estimate with relative WB, not measured calibration or absolute-Kelvin WB. Other non-DNG raws and rejected fits use the documented neutral fallback (camera RGB ≈ linear sRGB, flagged
`matrix_is_fallback`) with the file's as-shot white-balance multipliers. Clean sources to evaluate next: manufacturer
matrices stored in the files themselves (Olympus ImageProcessing `ColorMatrix`, Pentax equivalents; Panasonic RW2 files carry none) and our
own chart-based calibration (M11.4). Adobe matrices are never used.

## Log
- 2026-09-30: roadmap created; M0 in progress; research docs (Lightroom reference, Rust imaging ecosystem) complete.
- 2026-09-30 (later): app running with the full Lightroom-style UI; pipeline v0; DNG/CR2/ARW; README showcase. ≈ 8 h elapsed.
- 2026-10-02: parity estimate added (xtask parity prints weighted completion); milestone statuses refreshed.
- 2026-10-08: Fujifilm camera colour: guarded JPEG matching and relative WB, plus bundled X-H2S/X-T4 profiles from 201/545 photos. On 150 withheld files the pooled profiles improve mean ΔE slightly over per-file fitting (X-H2S 1.70 → 1.62, X-T4 2.43 → 2.34); dark X-H2S samples 8.74 → 2.25. Measured calibration remains open.
- 2026-10-08: Fujifilm lossless/lossy compressed RAF: pure-Rust striped decoder; full sensor arrays verified against a black-box reference on 17 CC0 files and 7 supplied X-H2S / X-T4 files (13 camera bodies, 14/16-bit Bayer and X-Trans). Camera colour calibration remains a separate gap.
- 2026-10-01: RAW II formats: RAF (uncompressed Bayer + X-Trans), RW2 (packed), PEF (incl. Huffman), ORF (uncompressed); embedded previews for every container incl. CR3; raw corpus test with 37 CC0 samples.
- 2026-10-03 – 10-05: community PRs (#42–#51, #60) and 25+ issues landed: compressed NEF, Move import, folder templates,
  Local roots and cleanup, 85k-photo grid and catalog performance, background compaction, failed-save errors, GPU
  export hardening, clippy 1.99. Added the honest *Where we stand* assessment and *Where we're going* priorities; added
  tracker rows for camera colour, camera coverage and render fidelity.
- 2026-10-07: Panasonic RW2 / Leica RWL / Panasonic RAW decode in every raw format (compressed 4/6/8, packed 2/5/7,
  16-bit words), black levels, in-camera aspect crops and mapped-out defects established on 178 CC0 files from 118
  bodies; RW2 gets the file-local camera look (accepted on 140 of 174 files with a preview).
- 2026-10-08: independent Rust CR3 lossless and version 0x100/0x200 C-RAW decoding, six exact full-sensor regressions on M50/R100/R8; any CR3 decode error falls back to the embedded JPEG; guarded CR3 camera-JPEG colour fitting with independent framing validation (`docs/cr3.md`). Unverified CRX variants and broader model verification remain.
- 2026-11-21 (fork): LibRaw 0.22.3 as the fallback raw decoder (`crates/libraw-sys`): native first, then LibRaw, then the embedded preview; 1,258 camera models; sRAW, CRX variants, compressed ORF, CRW/MRW/SRW and the long tail decode from sensor data.
- 2026-11-21 (fork): the Lightroom Classic migration keeps going after the move: `library.syncLightroom` (File ▸ Sync
  from Lightroom Classic…, dry run first) brings photos, edits, metadata, collections, history and snapshots changed in
  Lightroom since the migration / last sync, without overwriting edits made here unless asked; migrated photos keep
  their Lightroom history (side files, History panel) and snapshots (versions); Upright uses the transform Lightroom
  stored.
- 2026-11-21 (fork): Library safety and Lightroom habits — dated, tested catalog backups with a schedule asked at quit,
  keep-N pruning and an undoable restore (LRC-LIB-BACKUP); Delete from Disk to the system Trash / Remove / Delete
  Rejected Photos behind Lightroom's question (LRC-LIB-DELETE); `[` `]` rating and flag-cycle keys; Lightroom's ten
  filename templates and its naming tokens; Export Post-Processing (Show in Finder, open in an app, Export Actions);
  Set Copy as Original; Library Filter extras (No Label, copy status, stacks) and the Metadata column browser; 63
  smart-collection fields; selection set operations, Refine Photos, capture time from the file; raw imports start with
  lens corrections; automatic versions off by default, like Lightroom. Then: export status and the Exported filter,
  Export To the original's folder, Add to This Catalog (stacked), export metadata options, named watermarks, tokens in
  watermark text, several presets per export; collection extras, import presets, Eject after import, Quick Develop
  crop / treatment, more sort orders, Painter modes, Text filter targets and rules, Auto-Stack by similarity, Find
  Duplicates, Import from another library, Grid View Options (⌘J).
- 2026-11-21 (fork): Copy / Sync / Paste Selected / New Preset use Lightroom's per-setting checklist (43 settings
  under 14 headings, choose masks) with all. / none. / def. / mod. and saved subsets; older coarse group ids still work.
- 2026-11-21 (fork): relative paste (changes since the first / a chosen history state) and Merge / Replace masks
  on paste, as Lightroom asks.
- 2026-11-21 (fork): Alt/Option-drag previews (clipping on the Light sliders, greyscale / detail band / sharpening
  mask on Detail), ported from upstream.
- 2026-11-21 (fork): local adjustment ("Effect") presets ported from upstream; his Lightroom local presets import in
  place (Masking ▸ Effect ▸ import.).
- 2026-11-21 (fork): crop extras: As Shot aspect, Aspect Ratios / Center / Fifths overlays, Choose Overlays to Cycle;
  the Before & After button cycles the layouts.
- 2026-11-22 (fork): Clarity and positive Dehaze render as Camera Raw does (RAWmakase's scene map: four-scale
  Clarity refit on our oracle, its haze model; Bryan's raws Clarity +100 3.75 → 2.51 ΔE, Dehaze +100 10.1 → 4.7);
  Camera Raw's Highlights / Shadows rule is in the code but off until it passes the acceptance gate on R6 photos
  (`docs/lr-match.md` → Round 11).
- 2026-11-22 (fork): Texture, negative Dehaze, Color Grading, Point Color, the B&W mix and the masks' Contrast /
  Whites / Blacks render as Camera Raw does (RAWmakase's measured operators, checked on our Camera Raw oracle; the
  masks' curves our own measurement), CPU and GPU (`docs/lr-match.md` → Round 10).
- 2026-11-21 (fork): content-aware Fill spots (PatchMatch, `crates/inpaint` from upstream); Lightroom's content-aware
  remove spots map to it.
- 2026-11-21 (fork): photos on a P3 display (Studio Display) are shown in Display P3 values, so the loupe matches the
  export instead of looking more saturated (Settings ▸ Interface ▸ Photo color).
- 2026-11-21 (fork): point curve Snap to Grid, copy / paste a channel, Show All Curves.
- 2026-11-21 (fork): Remove tool ⌥-drag marquee deletes the spots inside it.

## Chinese and Japanese interfaces, and text watermarks

English, Simplified Chinese, Traditional Chinese (Taiwan) and Japanese interface languages are persisted in UI state, and the
language table (`crates/ui-egui/src/i18n.rs`) drives the menus, settings and fonts, so another
language is a table entry plus two catalogs (docs/localization.md). Core menus and panels are
translated; untranslated text retains English. CJK glyphs (Chinese: Noto Sans CJK SC; Japanese UI:
BIZ UDPGothic; watermarks: BIZ UDMincho) come from storytold/craft-fonts, embedded by builds made
with the optional `CRAFT_FONTS_DIR` input (all releases), so no system fonts are needed. Text watermarks now accept
`vertical: true` in export JSON/presets and expose an orientation selector. Japanese
characters stay upright in top-to-bottom columns, with newlines starting columns to the
left. This is basic lettering, without tate-chu-yoko, ruby, kinsoku, or general vertical
OpenType shaping. The same coverage renderer serves 8/16/32-bit exports.
