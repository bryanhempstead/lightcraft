# Enhance, Lens Blur and depth

Photo ▸ **Enhance…** (⌃⌥I) works like Lightroom Classic's Enhance: pick **Denoise** (with an Amount), **Raw
Details** and/or **Super Resolution**, look at the before / after preview of the middle of the photo at 1:1, and
choose where the result goes:

- **new.dng.** (Lightroom Classic's way): `<name>-Enhanced-NR.dng` (`-RD`, `-SR`, `-NR-SR`) is written next to the
  raw as a linear DNG (16-bit float), imported with the original's develop settings (luminance and colour noise
  reduction set to 0 after Denoise) and stacked on top of the original.
- **in.place.**: the photo itself renders from the enhanced picture (kept in `~/Library/Caches/LightCraft/Enhance`,
  never next to your files). Detail ▸ Denoise then shows the in-place amount; changing it makes the result again in
  the background. Undo or **rem.** goes back to the plain raw. If the cache file is gone (cleared caches, another
  Mac), the photo renders from the raw until it is made again (automatically for the photo being edited;
  `enhance.status` reports it).

Everything is a command, so the CLI, MCP and the control channel do the same: `photo.enhance {denoise: 60,
rawDetails, superResolution, inPlace, stack}`, `enhance.preview {…, beforePath, afterPath}`, `enhance.status`,
`enhance.remove`.

## What runs

| Step | How | Model (downloaded on request, never bundled) |
|---|---|---|
| Denoise | On the raw Bayer mosaic, denoising and demosaicing at once, tile by tile (`crates/denoise-core`, from storytold/lightcraft) | RawNIND UtNet2 as published by darktable-ai (GPL-3.0, 58 MB), run on candle (Metal on macOS). Without it, the built-in block-matching denoiser (BM3D-style, no download) |
| Raw Details | RCD demosaic (Luis Sanz Rodríguez, ported from RawTherapee, GPL-3.0+: `crates/rcd`) | none |
| Super Resolution | Real-ESRGAN general x4v3 (SRVGGNetCompact) on the demosaiced picture, ×4 then box-averaged to 2× width and height (`RealESRGAN_x2plus`, an RRDBNet, also loads) | Real-ESRGAN general x4v3 (BSD-3-Clause, 4.9 MB) |
| Depth | Depth Anything V2 Small on the photo as developed (uncropped, no masks), kept with the photo's settings | Depth Anything V2 Small (Apache-2.0, 99 MB) |

The models live in `~/Library/Application Support/LightCraft/Models/Enhance/<model>/` (override:
`LIGHTCRAFT_ENHANCE_MODELS`). The dialog and the Lens Blur panel offer each missing model with its size and licence
(**dl.**); `enhance.model.download {model, acknowledged: true}` does the same. Each download is size- and
SHA-256-pinned (`crates/enhance/src/models.rs`).

Denoise and Raw Details need a Bayer raw (X-Trans, Foveon and already-demosaiced files such as iPhone ProRAW are
refused with a message; RawNIND's linear variant could cover them later). Super Resolution works on any raw.

## Measured (M3 Pro, 18 GB)

On a Ricoh GR III DNG at ISO 4000 (6112 × 4064):

| | Time |
|---|---|
| AHD demosaic (the normal path) | 0.91 s |
| Raw Details (RCD) | 0.34 s |
| Denoise, RawNIND on Metal | 11.2 s (the first model use in a process adds ~30 s while candle compiles its Metal kernels; the app warms them up in the background at start when a model is installed) |
| Denoise, built-in (CPU, 4 threads) | 37.6 s |
| Super Resolution (x4v3) | 217 s for the whole 24.8 MP photo → 12000 × 8000 (incl. ~30 s first-use warm-up) |
| Depth map | 0.6 s warm (518 px model input; plus a 768 px render of the photo, ~2 s) |

`cargo test -p lightcraft-enhance --release --test real_models -- --ignored --nocapture` with `LC_MODELS=<folder of
the downloaded files>` and `LC_RAW=<a raw>` measures it again and writes crops (`LC_OUT`).

candle 0.9.2's own convolution is wrong on the CPU when a layer's channel count equals its width, and slow on Metal;
`lightcraft_enhance::netrun::conv2d` (one matrix product per kernel tap over a flattened, padded picture) is used
instead (`conv2d_is_right_when_the_channels_equal_the_width`).

## Lens Blur

Develop ▸ **Lens Blur**: **Apply** estimates the depth map when the photo has none and starts at Lightroom's
defaults (Blur Amount 50, focused on the nearest subject). Blur Amount, **Bokeh** (circ. / bubl. / 5.bld. / ring.),
Boost (bright highlights become bright discs), the **Focal Range** over the depth histogram (Focal Distance and
Range sliders, or **focus.** and click the photo), **depth.** (Visualize Depth: near warm, far cool), **redo.**
(estimate the depth again), and **Refine** brushes (**focus.brush.** paints sharp, **blur.brush.** paints fully
out of focus; **clr.** removes them). It renders in the scene-linear stage at output resolution with a circle of
confusion relative to the long edge, so previews and exports match.

Masking ▸ **Depth** adds a **Depth Range** mask (Near, Far, Smoothness over 0 nearest … 100 farthest), estimating
the depth when needed; it also intersects with other masks like Lightroom's.

Not yet: Cat Eye bokeh, a live depth overlay on the photo (the map is shown in the panel), and a painted-stroke
preview while dragging a Refine brush.
