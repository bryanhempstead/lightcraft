//! The LightCraft develop pipeline (CPU reference implementation).
//!
//! Input: a scene-referred, linear Rec.2020 source image (already EXIF-oriented) at any resolution
//! (full size or a proxy), plus [`DevelopSettings`]. Output: a display-encoded sRGB image at the
//! requested size, and its histogram.
//!
//! Stage order (see `docs/pipeline.md`):
//! 1. geometry — user orientation, lens corrections (distortion, CA, vignetting), perspective, crop +
//!    straighten, flips; one resample at output resolution; then defringe
//! 2. scene-linear — white balance, exposure, dehaze, local tone (highlights/shadows), texture,
//!    clarity, local adjustments (masks)
//! 3. tone map — contrast / whites / blacks filmic curve on luminance, highlight desaturation
//! 4. colour — vibrance, saturation, colour mixer, colour grading, B&W (OkLCh)
//! 5. display — gamut map to the output space (sRGB unless [`RenderRequest::space`] says otherwise), encode, tone curves (parametric + point), vignette, grain
//!
//! Spatial parameters are specified relative to the image's long edge, so a 400 px preview and a
//! 60 MP export look alike.
//!
//! Exposure is a gain, so the spatial stages run on the un-exposed image and the per-pixel stage
//! applies it (filters on log luminance are shift-equivariant: identical result). With
//! [`render_cached`] each stage's output is reused while its inputs are unchanged ([`StageCache`]):
//! dragging a tone, colour or exposure slider re-runs only the per-pixel stage.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod adobe;
pub mod auto;
pub mod black_cr;
pub mod bw_cr;
pub mod colorops;
pub mod colortab;
pub mod cull;
pub mod dehaze_cr;
pub mod dump;
pub mod dust;
pub mod finish;
pub mod geometry;
pub mod grading_cr;
pub mod hdr_histogram;
pub mod lens_blur;
pub mod local;
pub mod local_tone_cr;
pub mod lut;
pub mod masks;
pub mod optics;
pub mod output;
pub mod parametric_adobe;
pub mod point_color_cr;
pub mod profiles;
pub mod redeye;
pub mod reflection;
pub mod region;
pub mod scene_cr;
pub mod spots;
pub mod texture_cr;
pub mod tone;
pub mod tone_adobe;
pub mod tone_hdr;
pub mod tone_hdr_cr;
pub mod transform;
pub mod upright;
pub mod visualize;

pub use output::{DeepImage, DeepSamples, OutputDepth, OutputSpace, OutputTrc, Proof};
pub use region::{Globals, Region, RegionPx};
pub use visualize::{MaskView, Overlay};

use lightcraft_develop::{DevelopSettings, Treatment};
use std::any::Any;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use lightcraft_raster::{Histogram, Plane, Rgb32f, Rgba8, par_rows};

pub use tone::ToneMap;

/// Facts about the source the settings are interpreted against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceInfo {
    /// Lens corrections embedded in the file (DNG opcodes), relative to the EXIF-oriented source.
    pub lens: Option<lightcraft_develop::EmbeddedLens>,
    /// Raw sources use absolute Kelvin white balance; rendered sources use a relative scale around
    /// their as-shot white.
    pub raw: bool,
    pub as_shot_temp: f64,
    pub as_shot_tint: f64,
    /// No measured camera illuminant: WB adjustments are relative to the camera's rendered look.
    pub relative_wb: bool,
    pub camera_tone: Option<tone::CameraTone>,
    /// A camera white-balance model matched to Lightroom ([`CameraWb`]): Temp / Tint move the
    /// camera's neutral as Lightroom's do, instead of a Bradford adaptation of the as-shot white.
    pub camera_wb: Option<CameraWb>,
    /// The camera's Lightroom white-balance scale (set white balances only, see [`WbMap`]).
    pub wb_map: Option<WbMap>,
    /// The source's own long edge in pixels (the file's full resolution; 0 = unknown): sharpening
    /// radii are in these pixels, as in Lightroom.
    pub native_long: f32,
    /// An Adobe camera base registered in [`adobe`] (Bryan's fork): the decoded pixels are the
    /// profile's linear rendering, and the per-pixel stage applies its look tables and tone curve.
    pub adobe: Option<u64>,
}

/// How a camera's neutral (raw RGB of a white surface, green = 1) follows Lightroom's Temp /
/// Tint: `ln(r/g)` and `ln(b/g)` as `c0 + c1·m + c2·m² + c3·t + c4·m·t` of `m` = 1000 / K
/// (mireds / 1000) and `t` = tint / 100, fitted per camera model to as-shot photos and the
/// temperatures Lightroom gave them (`calibrate --lightroom`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraWb {
    pub r: [f32; 5],
    pub b: [f32; 5],
    /// White-balanced camera RGB (the as-shot balance) → linear Rec.2020, and back.
    pub to_working: [[f32; 3]; 3],
    pub from_working: [[f32; 3]; 3],
    /// The as-shot neutral (green = 1).
    pub shot: [f32; 3],
    /// Exact neutrals (Bryan's fork): a white-balance function registered with
    /// [`adobe::register_wb`] (Adobe's DNG SDK colour spec), used instead of the polynomial.
    pub exact: Option<u64>,
}

impl CameraWb {
    /// The camera neutral for `temp` (K) and `tint`.
    pub fn neutral(&self, temp: f64, tint: f64) -> [f64; 3] {
        if let Some(n) = self.exact.and_then(|k| adobe::wb_neutral(k, temp, tint)) {
            return n;
        }
        let m = 1000.0 / temp.clamp(1500.0, 50000.0);
        let t = tint.clamp(-150.0, 150.0) / 100.0;
        let f = |c: &[f32; 5]| (c[0] as f64 + c[1] as f64 * m + c[2] as f64 * m * m + c[3] as f64 * t + c[4] as f64 * m * t).clamp(-6.0, 6.0).exp();
        [f(&self.r), 1.0, f(&self.b)]
    }

    /// Temp / Tint whose neutral is `n` (Newton's method on the model; `None` if it doesn't
    /// converge).
    pub fn temp_tint(&self, n: [f64; 3]) -> Option<(f64, f64)> {
        if !(n.iter().all(|v| v.is_finite() && *v > 0.0)) {
            return None;
        }
        let target = [(n[0] / n[1]).ln(), (n[2] / n[1]).ln()];
        let (mut m, mut t) = (1000.0 / 5000.0, 0.0);
        let g = |c: &[f32; 5], m: f64, t: f64| c[0] as f64 + c[1] as f64 * m + c[2] as f64 * m * m + c[3] as f64 * t + c[4] as f64 * m * t;
        for _ in 0..30 {
            let (fr, fb) = (g(&self.r, m, t) - target[0], g(&self.b, m, t) - target[1]);
            let (rm, rt) = (self.r[1] as f64 + 2.0 * self.r[2] as f64 * m + self.r[4] as f64 * t, self.r[3] as f64 + self.r[4] as f64 * m);
            let (bm, bt) = (self.b[1] as f64 + 2.0 * self.b[2] as f64 * m + self.b[4] as f64 * t, self.b[3] as f64 + self.b[4] as f64 * m);
            let det = rm * bt - rt * bm;
            if !det.is_finite() || det.abs() < 1e-12 {
                return None;
            }
            let dm = (fr * bt - rt * fb) / det;
            let dt = (rm * fb - fr * bm) / det;
            m = (m - dm).clamp(0.02, 0.67);
            t = (t - dt).clamp(-1.5, 1.5);
            if dm.abs() < 1e-9 && dt.abs() < 1e-9 {
                break;
            }
        }
        let k = 1000.0 / m;
        (k.is_finite() && (1500.0..=50000.0).contains(&k)).then_some((k, t * 100.0))
    }
}

impl Default for SourceInfo {
    fn default() -> Self {
        Self {
            raw: false,
            as_shot_temp: 6500.0,
            as_shot_tint: 0.0,
            lens: None,
            relative_wb: false,
            camera_tone: None,
            camera_wb: None,
            wb_map: None,
            native_long: 0.0,
            adobe: None,
        }
    }
}

/// How one camera's Temp / Tint in Lightroom read in LightCraft's white-balance model: a set
/// white balance (any mode but As Shot) of `m` mired (10⁶ / K) and tint `t` renders as
/// `m + mired · [1, m/100, t/100]`, `t + tint · [1, m/100, t/100]`. Fitted per camera from the
/// user's own edits (`tools/lr-compare` `wbmap`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WbMap {
    pub mired: [f32; 3],
    pub tint: [f32; 3],
}

impl WbMap {
    /// The white balance to render for Lightroom's `temp` (K) and `tint`.
    pub fn apply(&self, temp: f64, tint: f64) -> (f64, f64) {
        if !(temp.is_finite() && temp > 0.0 && tint.is_finite()) {
            return (temp, tint);
        }
        let m = 1e6 / temp.clamp(1500.0, 50000.0);
        let x = [1.0, m / 100.0, tint / 100.0];
        let dot = |c: &[f32; 3]| c.iter().zip(x).map(|(a, b)| f64::from(*a) * b).sum::<f64>();
        let (dm, dt) = (dot(&self.mired), dot(&self.tint));
        if !(dm.is_finite() && dt.is_finite()) {
            return (temp, tint);
        }
        // a bounded correction: never a negative or absurd temperature
        let m2 = (m + dm.clamp(-150.0, 150.0)).clamp(20.0, 667.0);
        ((1e6 / m2).clamp(1500.0, 50000.0), (tint + dt.clamp(-80.0, 80.0)).clamp(-150.0, 150.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Quality {
    /// Interactive drags: skips the most expensive refinements.
    Draft,
    #[default]
    Full,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderRequest {
    /// Output must fit in `max_w × max_h` (aspect preserved).
    pub max_w: usize,
    pub max_h: usize,
    pub quality: Quality,
    /// Show the crop frame's content only (true) or the whole uncropped image (crop tool active).
    pub apply_crop: bool,
    /// A diagnostic view drawn over the result (e.g. Point Color's "visualize range").
    pub overlay: Overlay,
    /// Colour space of the result (exports; previews stay sRGB).
    pub space: OutputSpace,
    /// Sample format: 8-bit only, or also a 16-bit / linear float [`Rendered::deep`] (exports).
    pub depth: OutputDepth,
    /// Soft proofing (CPU only; see [`Proof`]).
    pub proof: Option<Proof>,
    /// Render only this rectangle of the whole `max_w × max_h` fit, at its scale ([`region`]).
    pub region: Option<Region>,
}

impl RenderRequest {
    pub fn fit(max_w: usize, max_h: usize) -> Self {
        Self {
            max_w,
            max_h,
            quality: Quality::Full,
            apply_crop: true,
            overlay: Overlay::None,
            space: OutputSpace::Srgb,
            depth: OutputDepth::U8,
            proof: None,
            region: None,
        }
    }
}

pub struct Rendered {
    /// The result, 8-bit (for deep renders: `deep` reduced to 8 bits, display-encoded).
    pub image: Rgba8,
    pub histogram: Histogram,
    /// The high-bit-depth result when [`RenderRequest::depth`] asks for one.
    pub deep: Option<DeepImage>,
    /// A region render ([`RenderRequest::region`]): where `image` sits in the whole render.
    pub region: Option<RegionPx>,
}

/// Everything the per-pixel stage needs, precomputed at output resolution.
///
/// The image and planes are computed *before exposure* (so they can be reused while exposure is
/// dragged): the per-pixel stage multiplies the image by `gain` and shifts log planes by `ev`.
/// Spatial filters on log luminance are shift-equivariant, so this equals filtering after exposure.
pub(crate) struct Prepared {
    pub img: Arc<Rgb32f>,
    /// Log2 luminance relative to middle grey (before exposure: add `ev`).
    pub log_l: Arc<Plane>,
    pub base: Arc<Plane>,
    /// The photo's tone statistics ([`tone::lr::image_stats`], before exposure) when the base
    /// plane was needed.
    pub tone_stats: Option<tone::lr::ToneStats>,
    pub clarity_blur: Option<Arc<Plane>>,
    pub texture_blur: Option<Arc<Plane>>,
    /// Camera Raw's Texture detail ([`texture_cr`]): each channel's log2 change per unit of
    /// strength.
    pub texture_detail: Option<Arc<Rgb32f>>,
    pub dark: Option<Arc<Plane>>,
    /// The photo's 99th log2 luminance percentile before exposure ([`dehaze_cr::p99_of`]), when
    /// negative Dehaze may apply.
    pub p99: Option<f32>,
    /// Blurred chromaticity (`rgb / Y`) for local Moiré / Noise.
    pub chroma_blur: Option<Arc<Rgb32f>>,
    /// Airlight of `dark` (before exposure).
    pub air: f32,
    pub masks: Vec<masks::Evaluated>,
    /// Output pixels per unit of the source long edge.
    pub px_per_long: f64,
    /// Camera Raw's scene tone map ([`scene_cr::MapBase`]: Clarity, Highlights / Shadows) and
    /// haze (positive Dehaze), when the settings use them.
    pub scene_map: Option<Arc<scene_cr::MapBase>>,
    pub haze: Option<Arc<scene_cr::Haze>>,
}

/// Output size for a source of `src_w × src_h` under `s`, fitting `max_w × max_h`.
pub fn output_size(src_w: usize, src_h: usize, s: &DevelopSettings, req: &RenderRequest) -> (usize, usize) {
    geometry::Frame::new(src_w, src_h, s, req.apply_crop).fit(req.max_w, req.max_h)
}

/// Size of the cropped output of a `src_w × src_h` source under `s` at the source's own resolution
/// (what a full-size export produces).
pub fn native_output_size(src_w: usize, src_h: usize, s: &DevelopSettings) -> (f64, f64) {
    geometry::Frame::new(src_w, src_h, s, true).native_size()
}

/// The geometric frame `render` uses for `src` (lens data, automatic CA estimate included).
pub fn frame_for(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, apply_crop: bool) -> geometry::Frame {
    let mut frame = geometry::Frame::with_lens(src.width, src.height, s, apply_crop, info.lens.as_ref());
    if s.optics.remove_ca && s.section_enabled("optics") {
        frame.add_lateral_ca(optics::estimate_lateral_ca(src));
    }
    frame
}

/// Intermediate results of recent renders of one view, reused by [`render_cached`].
///
/// Each stage is keyed by exactly the inputs it depends on: the resampled source by the source
/// buffer, framing and output size; the white-balanced, retouched, denoised image by those plus
/// white balance, spots and noise reduction; each spatial plane by that plus its own radius. So a
/// tone or colour slider drag (or exposure) re-runs only the per-pixel stage, a clarity drag skips
/// resampling and noise reduction, and so on. Holds the last few output sizes (a draft-size and a
/// full-size render of the same view both stay warm).
pub struct StageCache {
    entries: Mutex<Vec<CacheEntry>>,
    capacity: usize,
    /// Another renderer's per-view state (the GPU renderer keeps its device-resident stages here).
    ext: Mutex<Option<Arc<dyn Any + Send + Sync>>>,
    /// A region view's whole-photo values ([`region::Globals`]) and their key.
    globals: Mutex<Option<(u64, Arc<Globals>)>>,
}

impl Default for StageCache {
    fn default() -> Self {
        StageCache { entries: Mutex::new(Vec::new()), capacity: 2, ext: Mutex::new(None), globals: Mutex::new(None) }
    }
}

impl StageCache {
    /// Drop everything (e.g. when memory is needed).
    pub fn clear(&self) {
        self.lock().clear();
        *self.ext.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.globals.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The whole-photo values kept for `key` ([`Self::set_globals`]), if any.
    pub fn globals(&self, key: u64) -> Option<Arc<Globals>> {
        self.globals.lock().unwrap_or_else(|e| e.into_inner()).as_ref().filter(|(k, _)| *k == key).map(|(_, g)| g.clone())
    }

    /// Keep a region view's whole-photo values (one set: the latest).
    pub fn set_globals(&self, key: u64, g: Arc<Globals>) {
        *self.globals.lock().unwrap_or_else(|e| e.into_inner()) = Some((key, g));
    }

    /// Per-view state of type `T` kept alongside this cache (created on first use). Lets another
    /// renderer of the same view (e.g. `lightcraft-gpu`) keep its own stage cache with this one.
    pub fn extension<T: Any + Send + Sync + Default>(&self) -> Arc<T> {
        let mut g = self.ext.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = g.as_ref()
            && let Ok(t) = e.clone().downcast::<T>()
        {
            return t;
        }
        let t = Arc::new(T::default());
        *g = Some(t.clone());
        t
    }

    /// The per-view state of type `T` if there is one (unlike [`Self::extension`], never creates it).
    pub fn peek_extension<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.ext.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|e| e.clone().downcast::<T>().ok())
    }

    /// Bytes of the intermediate images held (each buffer counted once; the sources they were
    /// made from belong to their owner and are not counted).
    pub fn bytes(&self) -> usize {
        fn size<T>(i: &lightcraft_raster::Image<T>) -> usize {
            i.data.len() * std::mem::size_of::<T>()
        }
        let mut seen: Vec<usize> = Vec::new();
        let mut total = 0;
        let mut add = |p: usize, n: usize| {
            if !seen.contains(&p) {
                seen.push(p);
                total += n;
            }
        };
        for e in self.lock().iter() {
            add(Arc::as_ptr(&e.sampled) as usize, size(&e.sampled));
            if let Some((_, l)) = &e.lin {
                add(Arc::as_ptr(l) as usize, size(l));
            }
            let pl = &e.planes;
            let planes = pl.log_l.iter().chain(pl.base.iter().map(|x| &x.1)).chain(pl.clarity.iter().map(|x| &x.1));
            for p in planes.chain(pl.texture.iter().map(|x| &x.1)).chain(pl.dark.iter().map(|x| &x.1)) {
                add(Arc::as_ptr(p) as usize, size(p));
            }
        }
        total
    }

    /// Number of cached output sizes.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<CacheEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn get(&self, src: &Arc<Rgb32f>, geo: u64) -> Option<CacheEntry> {
        self.lock().iter().find(|e| e.geo == geo && Arc::ptr_eq(&e.src, src)).cloned()
    }

    fn put(&self, e: CacheEntry) {
        let mut v = self.lock();
        v.retain(|o| !(o.geo == e.geo && Arc::ptr_eq(&o.src, &e.src)));
        v.push(e);
        while v.len() > self.capacity {
            v.remove(0);
        }
    }
}

#[derive(Clone)]
struct CacheEntry {
    src: Arc<Rgb32f>,
    geo: u64,
    sampled: Arc<Rgb32f>,
    lin: Option<(u64, Arc<Rgb32f>)>,
    planes: local::Planes,
}

fn hash_of(parts: impl std::hash::Hash) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(parts)
}

/// What a render resolves to before any pixel work: the effective settings (profile and Upright
/// applied), the geometric frame, the output size and the stage-cache keys. Shared by the CPU
/// renderer and the GPU renderer (`lightcraft-gpu`), so both key their caches identically.
pub struct Plan<'a> {
    pub settings: Cow<'a, DevelopSettings>,
    pub frame: geometry::Frame,
    pub w: usize,
    pub h: usize,
    /// Output pixels per unit of the oriented source's long edge.
    pub px_per_long: f64,
    /// Long edge of the source buffer (px).
    pub src_long: usize,
    /// Key of the resampled source (together with the source buffer's identity).
    pub geo: u64,
    /// Key of the white-balanced, retouched, denoised image (and of the planes computed from it).
    pub lin_key: u64,
    /// Red eye / pet eye corrections with their detected pupils, in output pixels.
    pub eyes: Vec<redeye::EyeK>,
    /// A region render: the requested pixels of the whole render (the `w × h` buffer is the
    /// frame's [`geometry::Frame::window`] around them, or the whole render).
    pub region: Option<RegionPx>,
}

/// Resolve `s` against `src` for `req` (see [`Plan`]).
pub fn plan<'a>(src: &Rgb32f, info: &SourceInfo, s: &'a DevelopSettings, req: &RenderRequest) -> Plan<'a> {
    let mut settings: Cow<'a, DevelopSettings> = match profiles::effective_for(s, info.adobe) {
        Cow::Borrowed(b) => upright::resolve(src, info, b),
        Cow::Owned(o) => Cow::Owned(upright::resolve(src, info, &o).into_owned()),
    };
    visualize::adjust_settings(req.overlay, &mut settings);
    let s = &*settings;
    let mut frame = frame_for(src, info, s, req.apply_crop);
    let (w, h) = frame.fit(req.max_w, req.max_h);
    let px_per_long = frame.px_per_long(w);
    let src_long = src.width.max(src.height);
    let (region, w, h) = match req.region.and_then(|r| region::place(r, &mut frame, s, w, h, px_per_long, req.quality, src_long)) {
        Some((px, ww, wh)) => (Some(px), ww, wh),
        None => (None, w, h),
    };
    let geo = hash_of((format!("{frame:?}"), w, h));
    let (wb_t, wb_tint) = local::effective_wb(info, s);
    let eyes = redeye::resolve(src, &s.red_eye, s.orientation, &frame, w, h, px_per_long);
    let d = &s.detail;
    let lin_key = hash_of((
        geo,
        [wb_t, wb_tint, info.as_shot_temp, info.as_shot_tint].map(f64::to_bits),
        format!("{:?}", s.spots),
        format!("{eyes:?}"),
        // defringe runs in this stage
        format!("{:?}", s.optics),
        [d.nr_luminance, d.nr_detail, d.nr_color, d.nr_color_detail, d.nr_color_smoothness].map(f64::to_bits),
        src_long,
        lens_blur::key(s),
        spots::gen_state(&s.spots),
        reflection::key(s),
    ));
    Plan { settings, frame, w, h, px_per_long, src_long, geo, lin_key, eyes, region }
}

/// Whether the scene-linear stage needs work only the CPU does (defringe, spot removal).
pub fn lin_needs_cpu(s: &DevelopSettings) -> bool {
    let o = &s.optics;
    let defringe = s.section_enabled("optics") && (o.defringe_purple_amount > 0.0 || o.defringe_green_amount > 0.0);
    defringe || !s.spots.is_empty() || lens_blur::active(s) || s.reflection.active()
}

/// The white-balanced, defringed, retouched image (before noise reduction): the CPU part of the
/// scene-linear stage, in place.
pub fn lin_cpu(img: &mut Rgb32f, info: &SourceInfo, p: &Plan<'_>) {
    let s = &*p.settings;
    local::white_balance(img, info, s);
    let wb = local::wb_matrix_for(info, s);
    reflection::apply(img, s, &p.frame, wb);
    optics::defringe(img, s, p.px_per_long / optics::DEFRINGE_REF_LONG);
    spots::apply_with_wb(img, &s.spots, &p.frame, p.px_per_long, wb);
    redeye::apply(img, &p.eyes);
    lens_blur::apply(img, s, &p.frame, p.px_per_long);
}

/// The scene-linear image of `s`'s (cropped) frame fitting `req` as the retouch stage sees it — white balance,
/// defringe, spots, red eye, lens blur; no noise reduction or tone — and the white-balance matrix applied (`None`:
/// the as-shot white). Generative Remove looks at this (with a crop around the spot and the spots before it).
pub fn linear_stage(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest) -> (Rgb32f, Option<[[f32; 3]; 3]>) {
    let effective = s.effective();
    let p = plan(src, info, &effective, req);
    let mut img = p.frame.sample(src, p.w, p.h);
    lin_cpu(&mut img, info, &p);
    let wb = local::wb_matrix_for(info, &p.settings);
    (img, wb)
}

/// The settings a render of `s` for `req` uses: the HDR edit for an [`OutputDepth::F32Hdr`]
/// render, otherwise its SDR rendition (see [`DevelopSettings::sdr_rendition`]). Every renderer
/// (CPU and GPU) goes through this, so an SDR render of an HDR edit looks the same everywhere.
pub fn settings_for<'a>(s: &'a DevelopSettings, req: &RenderRequest) -> std::borrow::Cow<'a, DevelopSettings> {
    if req.depth == OutputDepth::F32Hdr { std::borrow::Cow::Borrowed(s) } else { s.sdr_rendition() }
}

/// Render `src` with settings `s`.
pub fn render(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest) -> Rendered {
    render_impl(Src::Borrowed(src), info, s, req, None, None)
}

/// [`render`], reusing (and refreshing) the intermediate results in `cache`. The output is
/// identical to [`render`]'s.
pub fn render_cached(src: &Arc<Rgb32f>, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest, cache: &StageCache) -> Rendered {
    render_impl(Src::Shared(src), info, s, req, Some(cache), None)
}

/// A region render ([`RenderRequest::region`]) taking what depends on the whole photo from
/// `globals` (see [`region`]); `cache` as in [`render_cached`].
pub fn render_region(
    src: &Arc<Rgb32f>,
    info: &SourceInfo,
    s: &DevelopSettings,
    req: &RenderRequest,
    globals: &Globals,
    cache: Option<&StageCache>,
) -> Rendered {
    render_impl(Src::Shared(src), info, s, req, cache, Some(globals))
}

/// What a whole render of `req` (its region ignored) learns from the whole photo, for
/// [`render_region`]: usually a preview-size render's.
pub fn globals(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest) -> Globals {
    let effective = s.effective();
    let req = RenderRequest { region: None, ..*req };
    let rendition = settings_for(&effective, &req);
    let plan = plan(src, info, &rendition, &req);
    let s = &*plan.settings;
    let mut img = plan.frame.sample(src, plan.w, plan.h);
    lin_cpu(&mut img, info, &plan);
    local::denoise(&mut img, s, plan.src_long, plan.w.max(plan.h));
    let mut planes = local::Planes { key: plan.lin_key, ..Default::default() };
    let tex_space = texture_cr::space(info);
    let prep = local::prepare(
        Arc::new(img),
        s,
        &plan.frame,
        plan.px_per_long,
        info.native_long,
        req.quality,
        &mut planes,
        tex_space.as_ref().map(|m| &m.0),
        info.adobe.is_some(),
    );
    Globals { tone_stats: prep.tone_stats, p99: prep.p99, air: prep.dark.is_some().then_some(prep.air), scene_map: prep.scene_map, haze: prep.haze }
}

enum Src<'a> {
    Borrowed(&'a Rgb32f),
    Shared(&'a Arc<Rgb32f>),
}

fn render_impl(
    src: Src<'_>,
    info: &SourceInfo,
    s: &DevelopSettings,
    req: &RenderRequest,
    cache: Option<&StageCache>,
    globals: Option<&Globals>,
) -> Rendered {
    // sections switched off with their eye render as if at their defaults (issue #316)
    let effective = s.effective();
    let edit: &DevelopSettings = &effective;
    let rendition = settings_for(edit, req);
    let s: &DevelopSettings = &rendition;
    // `Instant::now()` panics on wasm32-unknown-unknown: only read the clock when profiling.
    let lap = |what: &str, t: &mut Option<std::time::Instant>| {
        if let Some(t) = t {
            eprintln!("  {what}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
            *t = std::time::Instant::now();
        }
    };
    let mut t = profiling().then(std::time::Instant::now);
    let src_img: &Rgb32f = match &src {
        Src::Borrowed(r) => r,
        Src::Shared(a) => a,
    };
    let plan = plan(src_img, info, s, req);
    let Plan { ref frame, w, h, px_per_long, src_long, geo, lin_key, .. } = plan;
    let s = &*plan.settings;

    let shared = match (&src, cache) {
        (Src::Shared(a), Some(c)) => Some((*a, c)),
        _ => None,
    };
    let cached = shared.and_then(|(a, c)| c.get(a, geo));
    let sampled = match &cached {
        Some(e) => e.sampled.clone(),
        None => Arc::new(frame.sample(src_img, w, h)),
    };
    lap("sample", &mut t);

    let lin = match cached.as_ref().and_then(|e| e.lin.clone()).filter(|(k, _)| *k == lin_key) {
        Some((_, img)) => img,
        None => {
            // Without a cache the resampled buffer is ours: work on it in place.
            let mut img = if shared.is_some() { (*sampled).clone() } else { Arc::unwrap_or_clone(sampled.clone()) };
            lin_cpu(&mut img, info, &plan);
            let (fw, fh) = frame.full_size(w, h);
            local::denoise(&mut img, s, src_long, fw.max(fh));
            Arc::new(img)
        }
    };
    lap("wb/spots/nr", &mut t);
    let mut planes = match cached.map(|e| e.planes) {
        Some(p) if p.key == lin_key => p,
        _ => local::Planes { key: lin_key, ..Default::default() },
    };
    if let Some(g) = globals {
        seed_planes(&mut planes, g);
    }
    let tex_space = texture_cr::space(info);
    let mut prep = local::prepare(
        lin.clone(),
        s,
        frame,
        px_per_long,
        info.native_long,
        req.quality,
        &mut planes,
        tex_space.as_ref().map(|m| &m.0),
        info.adobe.is_some(),
    );
    if let Some(a) = globals.and_then(|g| g.air).filter(|_| prep.dark.is_some()) {
        prep.air = a;
    }
    lap("prepare", &mut t);
    if let Some((a, c)) = shared {
        c.put(CacheEntry { src: a.clone(), geo, sampled, lin: Some((lin_key, lin)), planes });
    }
    if req.depth != OutputDepth::U8 {
        let deep = finish::finish_deep(&prep, s, frame, info, req.space, req.depth, req.proof);
        let image = deep.to_rgba8();
        let histogram = Histogram::of_srgb8(&image);
        lap("finish (deep)", &mut t);
        return cut_region(Rendered { image, histogram, deep: Some(deep), region: None }, &plan);
    }
    let image = finish::finish(&prep, s, frame, info, req.space, req.proof);
    lap("finish", &mut t);
    let histogram = Histogram::of_srgb8(&image);
    lap("histogram", &mut t);
    let mut image = image;
    let mask = overlay_alpha(req.overlay, &plan, &prep);
    visualize::apply(&mut image, req.overlay, &plan, mask.as_ref());
    let mut out = cut_region(Rendered { image, histogram, deep: None, region: None }, &plan);
    if let Overlay::HdrRange(display) = req.overlay
        && edit.hdr.enabled
    {
        // the HDR rendition of the same request says how far above SDR white each pixel goes
        // (a region's: the same region)
        let hreq = RenderRequest { depth: OutputDepth::F32Hdr, overlay: Overlay::None, proof: None, ..*req };
        if let Some(hdr) = render_impl(Src::Borrowed(src_img), info, edit, &hreq, None, globals).deep {
            visualize::hdr_range(&mut out.image, &hdr, display);
        }
        lap("visualize hdr", &mut t);
    }
    out
}

/// A region render's globals in its planes (replacing what the window alone would give).
fn seed_planes(planes: &mut local::Planes, g: &Globals) {
    if g.tone_stats.is_some() {
        planes.tone_stats = g.tone_stats;
    }
    if g.p99.is_some() {
        planes.p99 = g.p99;
    }
    if let Some(m) = &g.scene_map {
        planes.scene_map = Some(Some(m.clone()));
    }
    if let Some(h) = &g.haze {
        planes.haze = Some(Some(h.clone()));
    }
}

/// The requested pixels of a region render (the buffer is the padded window around them) with
/// their histogram; any other render unchanged.
pub fn cut_region(r: Rendered, plan: &Plan<'_>) -> Rendered {
    let Some(px) = plan.region else { return r };
    let (ox, oy) = plan.frame.offset();
    let rect = (px.x.saturating_sub(ox), px.y.saturating_sub(oy), px.w, px.h);
    let (bw, bh) = (r.image.width, r.image.height);
    if rect.0 == 0 && rect.1 == 0 && px.w == bw && px.h == bh {
        return Rendered { region: Some(px), ..r };
    }
    let data = region::crop_samples(&r.image.data, bw, 1, rect);
    let image = Rgba8 { width: px.w, height: px.h, data };
    let histogram = Histogram::of_srgb8(&image);
    let deep = r.deep.map(|d| d.crop(bw, rect));
    Rendered { image, histogram, deep, region: Some(px) }
}

/// The colour a colour-range mask would sample at normalized image point `p` (OkLab of the
/// exposed scene colours, as [`MaskShape::ColorRange`](lightcraft_develop::MaskShape) compares
/// them), averaged over 3 × 3 pixels of a render fitting `req`. `None` outside the image.
pub fn color_range_sample(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest, p: lightcraft_geom::Point) -> Option<[f64; 3]> {
    let plan = plan(src, info, s, req);
    let mut img = plan.frame.sample(src, plan.w, plan.h);
    lin_cpu(&mut img, info, &plan);
    let o = plan.frame.norm_to_out(plan.w, plan.h).apply(p);
    let (cx, cy) = (o.x.floor() as i64, o.y.floor() as i64);
    if cx < 0 || cy < 0 || cx >= plan.w as i64 || cy >= plan.h as i64 {
        return None;
    }
    let gain = (s.light.exposure as f32).exp2();
    let mut acc = [0f64; 3];
    let mut n = 0.0;
    for y in (cy - 1).max(0)..=(cy + 1).min(plan.h as i64 - 1) {
        for x in (cx - 1).max(0)..=(cx + 1).min(plan.w as i64 - 1) {
            let c = img.data[y as usize * plan.w + x as usize].map(|v| v * gain);
            let lab = lightcraft_color::perceptual::oklab_from_2020(masks::tonemap_for_select(c));
            for k in 0..3 {
                acc[k] += lab[k] as f64;
            }
            n += 1.0;
        }
    }
    Some(acc.map(|v| v / n))
}

/// The alpha plane a mask overlay shows: the one the render evaluated, or (for a hidden mask) a
/// fresh evaluation.
fn overlay_alpha(o: Overlay, plan: &Plan<'_>, prep: &Prepared) -> Option<Plane> {
    let m = o.mask(&plan.settings)?;
    if let Some(e) = prep.masks.iter().find(|e| e.id == m.id) {
        return Some(e.alpha.clone());
    }
    let ev = plan.settings.light.exposure as f32;
    Some(masks::evaluate_one(m, &plan.frame, plan.w, plan.h, &prep.img, &prep.log_l, ev))
}

/// Convenience: render a before/after pair side by side is up to the UI; this renders "before"
/// (default look, keeping the crop so framing matches).
pub fn before_settings(s: &DevelopSettings) -> DevelopSettings {
    let mut b = DevelopSettings { crop: s.crop, orientation: s.orientation, ..DevelopSettings::default() };
    b.wb = lightcraft_develop::WhiteBalance { mode: lightcraft_develop::WbMode::AsShot, ..b.wb };
    b
}

/// Settings the GPU kernel doesn't render yet: such renders take the CPU pipeline (none at the
/// moment; round 10's measured operators all have a kernel path).
pub fn cpu_only(_s: &DevelopSettings) -> bool {
    false
}

pub(crate) fn is_bw(s: &DevelopSettings) -> bool {
    s.treatment == Treatment::Bw || s.profile.id == "lc.mono" || s.profile.id.starts_with("lc.bw.")
}

/// `LIGHTCRAFT_PROFILE` is set: print per-stage timings to stderr.
pub fn profiling() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LIGHTCRAFT_PROFILE").is_some())
}

/// Run `f`, printing its duration under `LIGHTCRAFT_PROFILE`.
pub(crate) fn timed<R>(what: &str, f: impl FnOnce() -> R) -> R {
    if !profiling() {
        return f();
    }
    let t = std::time::Instant::now();
    let r = f();
    eprintln!("    {what}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    r
}

pub(crate) fn for_rows<T: Send>(data: &mut [T], w: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    par_rows(data, w, f)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_geometry;
#[cfg(test)]
mod tests_hdr;
#[cfg(test)]
mod tests_local;
#[cfg(test)]
mod tests_region;
