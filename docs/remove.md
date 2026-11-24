# Remove tool: Generative Remove and Detect

The Remove panel (Develop ▸ Remove) has Lightroom Classic's Remove / Fill / Heal / Clone spots (`docs/parity.md`,
section I). This page is about its AI side.

## Generative Remove (Use Generative AI)

**generative.ai.** in the Remove panel (Lightroom: *Use Generative AI*, ⌥⇧G) makes every new Remove or Fill stroke a
*generative* spot. Like Lightroom's Generative Remove it is a **diffusion model** that repaints the area under the
stroke with what belongs there — grass with blades, water with ripples, a path that carries on — in a few different
ways to pick from. Two engines, both open models downloaded on request, never part of LightCraft:

| engine | model | licence | download | a variation (M3 Pro, measured) |
|---|---|---|---|---|
| **generative.ai.** (default) | Stable Diffusion 1.5 Inpainting (RunwayML), on LaMa's first guess | CreativeML OpenRAIL-M | `gen-fill`, 2.13 GB | ~10 s for a crop of 512 px, ~18 s for 1024 px |
| **quick.** | LaMa big-lama ("Resolution-robust Large Mask Inpainting with Fourier Convolutions", Samsung Research) | Apache-2.0 | `inpaint`, 204 MB | ~1 s for 512 px, ~4.5 s for 1024 px |

**quick.** (beside generative.ai.) makes new spots LaMa-only; on a selected spot **quick.** switches that spot's engine
(`spot.update {quick}`; each engine keeps its own fills, so switching back finds them again).

### How generative.ai. fills (`crates/enhance/src/genfill.rs`)

1. **The crop.** The model looks at the spot plus its own size of surroundings on each side, rendered at the
   photo's own resolution up to 3072 px across (`LIGHTCRAFT_GENAI_EDGE`, 256…8192; a larger crop is scaled down).
2. **Structure first.** LaMa fills the hole (at most 1024 px across, scaled back up): the horizon, a path's edges
   and a wall's lines continue through it.
3. **Texture.** Stable Diffusion repaints LaMa's fill from a partly noised start (strength 0.75 of a 16-step DDIM
   schedule: 12 UNet passes) on the crop scaled to 512 px across. The masked picture and the mask go in as the
   inpainting UNet's extra channels, so it sees exactly what is around the hole. The prompt is fixed ("empty
   background, seamless natural texture, high quality photo"); without classifier-free guidance by default (with
   LaMa's start the negative prompt added little on test photos and doubled the time).
4. **Large regions.** When the crop is more than 1.5× the model's 512 px, the hole is repainted again in
   overlapping 512 px tiles at up to 1536 px across (strength 0.3, feathered into each other), so a big fill gets
   detail close to the photo's resolution; beyond that it is resampled and given the photo's grain (the
   high-frequency energy of the pixels around the hole, deterministic noise). A 24 MP photo works: what the model
   ever holds is one 512 px picture.
5. **Seamless edge.** The difference between the photo and the model's picture on the ring just outside the hole
   is spread smoothly over the hole (pull–push interpolation: a fast Poisson-style membrane) and added to the fill,
   so the colour and brightness meet the photo exactly at the mask's edge; the spot's feather does the rest.
6. **Deterministic.** Every random number comes from the variation's seed (a hash of the spot's key and its
   number; SplitMix64 + Box–Muller on the CPU, so the same on every Mac) and DDIM adds none (eta 0): a cleared
   cache remakes the same fill.

### Variations, cache, fallbacks

- **Variations:** each spot gets three. Pick with **<** / **>** in the panel, ⌥← / ⌥→ (Lightroom) or Shift+, /
  Shift+. ; **more.** adds three more (up to eight). generative.ai.'s variations are seeds (truly different fills); LaMa
  (quick.) is deterministic, so its variations come from showing it the spot differently: as it is, mirrored, with
  more of the surroundings, and at a coarser scale.
- **Progress and stop:** in the app fills are made in the background (**making… 40%**, **stop.** = `remove.cancel`,
  which keeps what is made); the spot shows the content-aware fill until its variation arrives. The CLI and MCP
  wait for them.
- **Resolution independent:** the spot is stored in normalized coordinates like every spot; renders resample its
  fill at their own size, so previews and exports match (measured for LaMa: < 1 level of 255 between a 600 px
  preview and a 2400 px render).
- **Where the pixels live:** not in the catalog. The spot names them (a key over the photo's content, the
  pixel-moving settings and the spots before it, the spot's place and size, and the engine); they are kept in
  `~/Library/Caches/LightCraft/Remove` (`LIGHTCRAFT_REMOVE_CACHE`) in scene-linear light *before* white balance, so
  later white balance and tone edits apply to them like to the rest of the photo. Missing pixels (cleared caches,
  another Mac) are made again from the photo. Moving or resizing the spot, or editing a spot before it, makes new
  ones.
- **Never a crash on model or memory failure.** Without the diffusion model a generative.ai. spot is filled by LaMa and
  the panel says so (with the model's **download.**); the fill is marked, and remade by the diffusion model once it is
  installed (when the download finishes, for the photo on screen). If the diffusion model fails (memory, a damaged
  file) LaMa fills the spot with a notice, and **remake.** (`remove.generate {remake: "fallbacks"}`) tries again.
  Without any model the spot is filled content-aware (PatchMatch, like Fill).
- **Downloads:** each model is downloaded on request (**download.**, whose hover states size and licence; or
  `enhance.model.download {model: "gen-fill" | "inpaint", acknowledged: true}`) from a pinned Hugging Face commit,
  size- and SHA-256-checked (`crates/enhance/src/models.rs`), into the Enhance models folder (`docs/enhance.md`).
  gen-fill is three files (the half-precision UNet, VAE and CLIP text encoder of
  `stable-diffusion-v1-5/stable-diffusion-inpainting`, the community re-upload of RunwayML's weights). Its licence,
  CreativeML OpenRAIL-M, allows personal and commercial use with use-based restrictions (no illegal or harmful
  content, …); not LightCraft's licence.

### Speed (M3 Pro 18 GB, Metal, half precision, measured)

`genfill_speed` on a public-domain photo (two hikers in a meadow, the hole over them; release build, model loaded):

| crop (px) | LaMa | generative.ai. variation | where the time goes |
|---|---|---|---|
| 391 × 512 | 1.1 s | 9.3–10.6 s | VAE encode 2.1 s, 12 UNet passes 5.0 s, decode 2.0 s |
| 782 × 1024 | 4.1 s | 17.4–18.5 s | the 512 px pass ~9.5 s + one 512 px refining tile ~8 s |

Through the app's code path (headless `lightcraft-cli run --library … --import photo spot.add {generative: true, …}`
on a 1920 × 1422 photo, a stroke over the two hikers, a ~1000 px crop with one refining tile), while four other
builds were running on the same Mac: three variations in 203 s from a cold start (LaMa, model load and Metal kernel
compile included), ~25 s a variation once warm; under heavier load (the GPU shared with other jobs) 577 s. So the
≤ 20 s target holds for a 512–1024 px crop on an idle Mac, not under load, and not yet for the very first fill.

Loading the model takes ~2 s (plus candle's one-time Metal kernel compile, ~30 s, shared with every model). With
classifier-free guidance (`Params::guidance` 4) a 512 px variation took 18–21 s. What was made fast: the UNet's
and VAE's stride-1 convolutions use LightCraft's tap-wise convolution (`netrun::conv2d_taps`, ~3× candle 0.9.2's
Metal one), attention runs in half precision (upstream casts to single), the VAE too. Left: the VAE is now ~40 %
of a variation (a tiny autoencoder such as TAESD would cut it), and a 2× larger crop costs a refining tile per
512 px of hole.

Commands: `spot.add {generative: true, quick?, points, size}`, `spot.update {generative?: bool, quick?: bool}`,
`spot.variation {delta | variation}`, `spot.moreVariations`, `remove.generate {remake?: true | "fallbacks"}` (make
missing fills now), `remove.cancel`, `remove.status` (per spot: engine, made and fallback per variation; the
models; progress).

The networks: Stable Diffusion's UNet, VAE, CLIP text encoder and DDIM come from Hugging Face candle-transformers
0.9.2 (MIT OR Apache-2.0, `crates/enhance/src/sd/`, see NOTICE). LaMa is implemented in `crates/enhance/src/lama.rs`
on candle (Metal on macOS): candle has no FFT, so the Fourier units' real 2-D FFT is two matrix products with
cosine/sine tables; stride-2 and transposed convolutions are split into their four phases. It matches the PyTorch
reference (advimman/lama's `ffc.py` on the same weights) to 3 × 10⁻⁶ on the CPU and on Metal
(`lama_matches_the_reference`).

### Why Stable Diffusion 1.5 Inpainting

Candidates weighed for a local model on an M3 Pro (18 GB) whose licence Bryan's fork allows:

- **FLUX.1 Fill [dev]** (best quality; FLUX.1 [dev] Non-Commercial licence): 12 B parameters, ~24 GB in half
  precision — does not fit, and minutes a variation.
- **SDXL Inpainting** (diffusers 0.1, OpenRAIL++): 1024 px native, better detail, but a 5 GB UNet and ~4× SD 1.5's
  work per step: about a minute a variation here.
- **PowerPaint / BrushNet** (SD 1.5 + a second UNet-sized branch; removal-specific prompts): better at "remove,
  don't add", but twice the network per step and an architecture candle doesn't have.
- **MAT** (CC BY-NC) and LaMa refiners: GANs, no variety.
- **SD 1.5 / 2 Inpainting**: the inpainting UNet runs on candle's existing SD code, 2.1 GB, ~0.4 s a step at
  512 px here. SD 1.5 Inpainting is the better-tested checkpoint; starting it from LaMa's fill keeps it from
  inventing new objects (SD's usual failing for removal).
- Core ML (Apple's ml-stable-diffusion, faster on the Neural Engine) would need Swift / Objective-C FFI (an
  `unsafe` helper crate) and a converted model; candle needed none.


## Detect (⌥⇧O)

**detect.** (Lightroom: *Detect*) finds **people** (SAM 3, else the Scene parser of `docs/ai-masks.md`), **wires and
cables** (SAM 3) and **sensor dust**, and offers them: each person is a chip (**1.**, **2.**, …), the wires one
(**wires.**), all the dust one (**dust.N**). Hovering a chip or the object on the photo highlights it; click a chip
or the object to pick it, ⌘-click to add or remove one, ⇧-click for the run from the last one, **all.** / **none.**.
**remove.N** removes the picked ones in one undo step — each object as one Generative Remove spot when the model is
installed (else content-aware Fill rows), dust with heal spots — and **clear.** forgets what was found.

Commands: `remove.detect {people?, wires?, dust?}` → candidates, `remove.candidates`, `remove.apply {indices | all,
generative?}`, `remove.clearCandidates`; `remove.detect {add: true}` keeps the old behaviour (every object at once).

## Remove Reflections

**reflections.** in the Remove panel (Lightroom Classic's Reflection Removal) splits the photo into the scene behind a
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
