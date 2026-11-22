# Remove tool: Generative Remove and Detect

The Remove panel (Develop ▸ Remove) has Lightroom Classic's Remove / Fill / Heal / Clone spots (`docs/parity.md`,
section I). This page is about its AI side.

## Generative Remove (Use Generative AI)

**gen.ai.** in the Remove panel (Lightroom: *Use Generative AI*, ⌥⇧G) makes every new Remove or Fill stroke a
*generative* spot: the area under the stroke is filled by **LaMa** ("Resolution-robust Large Mask Inpainting with
Fourier Convolutions", Samsung Research, Apache-2.0; the `big-lama` checkpoint), which continues the surroundings —
grass, water, a path, a wall — through the hole instead of copying patches.

- **Variations:** each spot gets three. Pick with **<** / **>** in the panel, ⌥← / ⌥→ (Lightroom) or Shift+, /
  Shift+. ; **more.** adds three more (up to eight). LaMa itself is deterministic, so the variations come from
  showing it the spot differently: as it is, mirrored, with more of the surroundings, and at a coarser scale.
- **Resolution independent:** the spot is stored in normalized coordinates like every spot. The model looks at a crop
  around it (the spot plus about its own size of surroundings on each side) at the photo's resolution, at most 1024
  pixels across (`LIGHTCRAFT_GENREMOVE_EDGE` changes it, 64…4096); renders resample that fill at their own size, so
  previews and exports match (measured: < 1 level of 255 between a 600 px preview and a 2400 px render). A fill much
  larger than 1024 px in the export is softer than the photo around it.
- **Where the pixels live:** not in the catalog. The spot names them (a key over the photo's content, the
  pixel-moving settings and the spots before it, and the spot's place and size); they are kept in
  `~/Library/Caches/LightCraft/Remove` (`LIGHTCRAFT_REMOVE_CACHE`) in scene-linear light *before* white balance, so
  later white balance and tone edits apply to them like to the rest of the photo. Missing pixels (cleared caches,
  another Mac) are made again from the photo: LaMa gives the same result every time. Moving or resizing the spot, or
  editing a spot before it, makes new ones.
- **Without the model** a generative spot is filled content-aware (PatchMatch, like Fill) and the panel says so,
  with the model's download (**dl.**, 204 MB, Apache-2.0). It is never part of LightCraft: it is downloaded on
  request from a pinned Hugging Face copy, size- and SHA-256-checked (`crates/enhance/src/models.rs`, model
  `inpaint`), into the Enhance models folder (`docs/enhance.md`).
- **Speed (M3 Pro, Metal, measured):** the network takes ~1.4 s for a 512 × 512 crop and ~5 s for 1024 × 1024
  (`lama_speed`); three variations of a person-sized spot in a 10 MP photo took 14 s, model load and GPU kernel
  compile included. In the app it runs in the background (**making…**), and the spot shows the content-aware fill
  until its variation arrives. Faster convolutions (half precision, fewer passes over the 7 × 7 layers) are left.

Commands: `spot.add {generative: true, points, size}`, `spot.update {generative: bool}`, `spot.variation {delta |
variation}`, `spot.moreVariations`, `remove.generate` (make missing fills now), `remove.status`.

The network is implemented in `crates/enhance/src/lama.rs` on candle (Metal on macOS): candle has no FFT, so the
Fourier units' real 2-D FFT is two matrix products with cosine/sine tables; stride-2 and transposed convolutions are
split into their four phases. It matches the PyTorch reference (advimman/lama's `ffc.py` on the same weights) to
3 × 10⁻⁶ on the CPU and on Metal (`lama_matches_the_reference`).

## Detect (⌥⇧O)

**det.** (Lightroom: *Detect*) finds **people** (SAM 3, else the Scene parser of `docs/ai-masks.md`), **wires and
cables** (SAM 3) and **sensor dust**, and offers them: each person is a chip (**1.**, **2.**, …), the wires one
(**wires.**), all the dust one (**dust.N**). Hovering a chip or the object on the photo highlights it; click a chip
or the object to pick it, ⌘-click to add or remove one, ⇧-click for the run from the last one, **all.** / **none.**.
**rem.N** removes the picked ones in one undo step — each object as one Generative Remove spot when the model is
installed (else content-aware Fill rows), dust with heal spots — and **clr.** forgets what was found.

Commands: `remove.detect {people?, wires?, dust?}` → candidates, `remove.candidates`, `remove.apply {indices | all,
generative?}`, `remove.clearCandidates`; `remove.detect {add: true}` keeps the old behaviour (every object at once).

## Remove Reflections

**refl.** in the Remove panel (Lightroom Classic's Reflection Removal) splits the photo into the scene behind a
window and the reflection on it with **DSRNet** ("Single Image Reflection Separation via Component Synergy", Hu & Guo,
ICCV 2023, Apache-2.0; the authors' `dsrnet_l` checkpoint trained with real-world pairs), on top of VGG-19's
ImageNet features (torchvision's weights, BSD-3-Clause). **rmv.** takes the reflection away, **keep.** shows only the
reflection; **Amount** (a develop control, `reflection.amount`) sets how much.

- The model looks at the whole photo (uncropped, before spots) at up to 512 pixels across
  (`LIGHTCRAFT_REFLECTION_EDGE`): DSRNet was trained on small crops, and on a 1024-pixel view of a test photo it took
  a graffiti wall behind the glass for a reflection that it left alone at 512 (~20 s on an M3 Pro, model load
  included; 82 s at 1024). What it took away is kept
  as a layer, scene-linear and before white balance, in the same cache as Generative Remove's fills; renders resample
  it at their own size and subtract it right after white balance, so previews and exports match and later edits
  (white balance, tone, crop, spots) apply on top. The scene's own detail stays at full resolution; the reflection
  layer is as sharp as the 512-pixel analysis.
- The layers are named by a key over the photo's content and what moves its pixels (orientation, lens and geometry
  corrections, Enhance); a change makes them again (`remove.generate`), like Lightroom's Update AI Settings.
- Models: `enhance.model.download {model: "reflection"}` (495 MB, from the authors' Google Drive, SHA-256 pinned) and
  `{model: "vgg19"}` (575 MB, the `timm/vgg19.tv_in1k` copy on Hugging Face, pinned). Without them the photo renders
  as it is and the panel offers them.
- Accuracy: the Rust network matches DSRNet's own PyTorch code on the same weights to 1.4 × 10⁻⁶
  (`dsrnet_matches_the_reference`).

Commands: `reflection.set {enabled?, keep?, amount?}`, `reflection.status`, `develop.set {control:
"reflection.amount"}`.
