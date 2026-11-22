//! Photo ▸ Enhance… (Lightroom Classic's Enhance: Denoise, Raw Details, Super Resolution) and the depth map.
//!
//! An Enhance is planned from the session ([`Session::plan_enhance`] → [`EnhanceJob`], `Send` and self-contained),
//! run anywhere (a worker thread in the app, inline for the CLI/MCP: [`EnhanceJob::run`], with progress and
//! cancellation) and finished on the session ([`Session::finish_enhance`]):
//!
//! - **New DNG** (Lightroom Classic's way): the enhanced picture is written as a linear DNG next to the original
//!   (`<name>-Enhanced-NR.dng`, `-RD`, `-SR`, `-NR-SR`), imported with the original's develop settings (luminance
//!   and colour noise reduction off after Denoise) and stacked on top of it.
//! - **In place**: the picture is kept in the Enhance cache ([`cache_dir`]; half floats,
//!   `lightcraft_denoise_core::product`) and the photo's `enhance` settings say to use it: every render (previews,
//!   1:1, export) decodes the photo from it instead of the raw mosaic ([`with_enhanced`], read in
//!   `files::load_raw`). Turning it off is an ordinary, undoable develop change. A missing cache file (cleared
//!   caches, another computer) renders the plain raw and `enhance.status` says to run Enhance again.
//!
//! The models (RawNIND raw denoise, Real-ESRGAN x2, Depth Anything V2) are never part of LightCraft; they are
//! downloaded with consent into [`models_root`] (`enhance.model.download`). Without the RawNIND model Denoise uses
//! the built-in block-matching denoiser (no download); Super Resolution and depth need theirs.

use std::path::{Path, PathBuf};
#[cfg(feature = "enhance")]
use std::sync::Arc;

use lightcraft_catalog::{Photo, PhotoId, Source};
use lightcraft_develop::DevelopSettings;
pub use lightcraft_enhance::Options;
use lightcraft_raster::Rgb32f;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// Version of the in-place product format and algorithms (part of every cache key).
const PRODUCT_VERSION: u32 = 1;

/// Where the Enhance models live: `LIGHTCRAFT_ENHANCE_MODELS`, else
/// `~/Library/Application Support/LightCraft/Models/Enhance`.
pub fn models_root() -> PathBuf {
    if let Some(d) = std::env::var_os("LIGHTCRAFT_ENHANCE_MODELS") {
        return PathBuf::from(d);
    }
    match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h).join("Library/Application Support/LightCraft/Models/Enhance"),
        None => std::env::temp_dir().join("LightCraft Enhance Models"),
    }
}

/// Where in-place Enhance results are kept: `LIGHTCRAFT_ENHANCE_CACHE`, else `~/Library/Caches/LightCraft/Enhance`.
pub fn cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("LIGHTCRAFT_ENHANCE_CACHE") {
        return PathBuf::from(d);
    }
    if cfg!(test) {
        return std::env::temp_dir().join(format!("lc-enhance-cache-test-{}", std::process::id()));
    }
    match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h).join("Library/Caches/LightCraft/Enhance"),
        None => std::env::temp_dir().join("LightCraft Enhance"),
    }
}

/// The Enhance options a photo's settings ask for in place.
pub fn options_of(s: &DevelopSettings) -> Options {
    let e = &s.enhance;
    let denoise = if e.denoise.is_finite() { e.denoise.clamp(0.0, 100.0) as f32 } else { 0.0 };
    Options { denoise, raw_details: e.raw_details, super_resolution: e.super_resolution }
}

/// An in-place Enhance result a source is decoded from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enhanced {
    pub product: PathBuf,
    pub key: String,
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(96).collect()
}

/// The in-place result `p` would be decoded from with `opts` (whether or not it has been made yet).
pub fn enhanced_for(p: &Photo, opts: &Options) -> Option<Enhanced> {
    if !opts.any() || !matches!(p.source, Source::File { .. }) {
        return None;
    }
    let content = crate::media::content_key(p);
    let key = format!("lightcraft-enhance v{PRODUCT_VERSION} {content} {}", opts.key());
    // a short hash keeps names unique when the readable part is cut
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let product = cache_dir().join(format!("{}-{}-{h:016x}.lcdn", safe_name(&content), opts.key()));
    Some(Enhanced { product, key })
}

/// The in-place result a photo's source is decoded from, per its settings.
pub fn enhanced_source(p: &Photo) -> Option<Enhanced> {
    enhanced_for(p, &options_of(&p.develop))
}

thread_local! {
    static ENHANCED: std::cell::RefCell<Option<Enhanced>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` (a [`crate::media::FileLoader`] call on this thread) decoding a raw from its in-place Enhance result.
pub fn with_enhanced<R>(e: Option<Enhanced>, f: impl FnOnce() -> R) -> R {
    let before = ENHANCED.with(|c| c.replace(e));
    let r = f();
    ENHANCED.with(|c| *c.borrow_mut() = before);
    r
}

/// The in-place result this thread's load asked for ([`with_enhanced`]).
pub fn current_enhanced() -> Option<Enhanced> {
    ENHANCED.with(|c| c.borrow().clone())
}

/// The in-place result to decode from, when this thread's load asked for one and it is there: camera RGB as
/// `RawImage::develop` gives it, no larger than needed for `max_edge` (whole reduction factors).
pub(crate) fn product_for_load(max_edge: usize) -> Option<Rgb32f> {
    let e = ENHANCED.with(|c| c.borrow().clone())?;
    if !lightcraft_denoise_core::product::is_current(&e.product, &e.key) {
        return None;
    }
    let h = lightcraft_denoise_core::product::read_header_at(&e.product).ok()?;
    let long = (h.width as usize).max(h.height as usize);
    let need = (max_edge.saturating_mul(9) / 10).max(1);
    let factor = [8usize, 6, 4, 3, 2].into_iter().find(|&k| long / k >= need).unwrap_or(1);
    match lightcraft_denoise_core::product::read(&e.product, &e.key, factor) {
        Ok(img) => Some(img),
        Err(err) => {
            log::warn!("enhance: {}: {err}", e.product.display());
            None
        }
    }
}

/// Reads a source photo's bytes (by catalog path).
pub type ByteReader = crate::merge::ByteReader;

/// A planned Enhance, detached from the session.
#[derive(Clone)]
pub struct EnhanceJob {
    pub options: Options,
    /// Keep the result in place (the photo's settings) instead of a new DNG.
    pub in_place: bool,
    /// New DNG: stack it on top of its original (Lightroom does).
    pub stack: bool,
    pub sources: Vec<(PhotoId, String)>,
    /// In place: where each source's result goes.
    pub products: Vec<Option<Enhanced>>,
    /// A preview: the part `[x0, y0, x1, y1]` (normalised, unrotated sensor) of the first photo, rendered at most
    /// `side` pixels with the photo's settings, before and after.
    pub preview: Option<PreviewSpec>,
    pub read: ByteReader,
    pub models_root: PathBuf,
}

#[derive(Clone)]
pub struct PreviewSpec {
    pub rect: [f64; 4],
    pub side: usize,
    pub settings: DevelopSettings,
}

/// What one photo's Enhance made.
pub struct EnhancedPhoto {
    pub id: PhotoId,
    pub image: Rgb32f,
    pub color: lightcraft_raw::ColorData,
    pub orientation: lightcraft_geom::Orientation,
    pub metadata: lightcraft_meta::Metadata,
    pub info: Value,
}

/// What a job made: each photo's result or why it failed, and for a preview the before / after pictures.
pub struct EnhanceOutput {
    pub photos: Vec<std::result::Result<EnhancedPhoto, (PhotoId, String)>>,
    pub preview: Option<(lightcraft_raster::Rgba8, lightcraft_raster::Rgba8)>,
}

pub type ProgressFn<'a> = dyn Fn(f32, &str) -> bool + Sync + 'a;

/// The models this build can run (see [`Enhancer`]).
pub const AVAILABLE: bool = cfg!(feature = "enhance");

#[cfg(feature = "enhance")]
mod loaded {
    use std::sync::{Arc, Mutex, PoisonError};

    use lightcraft_enhance::depth::DepthModel;
    use lightcraft_enhance::esrgan::SuperResolution;
    use lightcraft_enhance::job::DenoiseModel;
    use lightcraft_enhance::models::{self, ModelId};

    /// Models stay loaded for the process (loading Real-ESRGAN or the depth model takes about a second).
    static DENOISE: Mutex<Option<(std::path::PathBuf, Arc<DenoiseModel>)>> = Mutex::new(None);
    static SR: Mutex<Option<(std::path::PathBuf, Arc<SuperResolution>)>> = Mutex::new(None);

    fn get<T>(
        slot: &Mutex<Option<(std::path::PathBuf, Arc<T>)>>,
        root: &std::path::Path,
        id: ModelId,
        load: impl FnOnce(&std::path::Path) -> Result<T, String>,
    ) -> Result<Option<Arc<T>>, String> {
        let Some(path) = models::installed(root, id) else { return Ok(None) };
        let mut g = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((p, m)) = g.as_ref()
            && *p == path
        {
            return Ok(Some(m.clone()));
        }
        let m = Arc::new(load(&path)?);
        *g = Some((path, m.clone()));
        Ok(Some(m))
    }

    pub fn denoise(root: &std::path::Path) -> Result<Option<Arc<DenoiseModel>>, String> {
        get(&DENOISE, root, ModelId::RawDenoise, |p| DenoiseModel::load(p, lightcraft_enhance::best_device()).map_err(|e| e.to_string()))
    }
    pub fn super_resolution(root: &std::path::Path) -> Result<Option<Arc<SuperResolution>>, String> {
        get(&SR, root, ModelId::SuperResolution, |p| SuperResolution::load(p, lightcraft_enhance::best_device()).map_err(|e| e.to_string()))
    }
    /// (not kept: its layers aren't `Send`; loading takes about a second)
    pub fn depth(root: &std::path::Path) -> Result<Option<DepthModel>, String> {
        match models::installed(root, ModelId::Depth) {
            Some(p) => DepthModel::load(&p, lightcraft_enhance::best_device()).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }
}

impl EnhanceJob {
    /// Run the job. `progress(fraction, stage)` returns false to cancel.
    pub fn run(&self, progress: &ProgressFn) -> std::result::Result<EnhanceOutput, String> {
        #[cfg(not(feature = "enhance"))]
        {
            let _ = progress;
            Err("Enhance is not available in this build".into())
        }
        #[cfg(feature = "enhance")]
        {
            let models = lightcraft_enhance::job::Models {
                denoise: if self.options.denoise > 0.0 { loaded::denoise(&self.models_root)? } else { None },
                super_resolution: if self.options.super_resolution {
                    Some(loaded::super_resolution(&self.models_root)?.ok_or_else(|| {
                        lightcraft_enhance::models::missing_message(&self.models_root, lightcraft_enhance::models::ModelId::SuperResolution)
                    })?)
                } else {
                    None
                },
            };
            let n = self.sources.len().max(1);
            let mut photos = Vec::new();
            let mut preview = None;
            for (i, (id, path)) in self.sources.iter().enumerate() {
                let (a, b) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
                let sub = |f: f32, s: &str| progress(a + (b - a) * f, s);
                if !sub(0.0, "Reading") {
                    return Err("cancelled".into());
                }
                let r = self.one(*id, path, &models, &sub);
                match r {
                    Ok((p, pv)) => {
                        preview = preview.or(pv);
                        photos.push(Ok(p));
                    }
                    Err(e) if e == "cancelled" => return Err(e),
                    Err(e) => photos.push(Err((*id, e))),
                }
                if self.preview.is_some() {
                    break;
                }
            }
            progress(1.0, "Done");
            Ok(EnhanceOutput { photos, preview })
        }
    }

    #[cfg(feature = "enhance")]
    #[allow(clippy::type_complexity)]
    fn one(
        &self,
        id: PhotoId,
        path: &str,
        models: &lightcraft_enhance::job::Models,
        progress: &ProgressFn,
    ) -> std::result::Result<(EnhancedPhoto, Option<(lightcraft_raster::Rgba8, lightcraft_raster::Rgba8)>), String> {
        let bytes = (self.read)(path)?;
        if lightcraft_raw::probe(&bytes).is_none() {
            return Err(format!("{path}: Enhance works on raw files"));
        }
        let metadata = lightcraft_meta::extract(&bytes);
        let mut raw = lightcraft_raw::decode(&bytes).map_err(|e| format!("{path}: {e}"))?;
        drop(bytes);
        // geometric lens corrections stay with the develop settings ("Enable Profile Corrections"), as on load
        raw.opcodes.list3.retain(|op| !op.is_lens_correction());
        if let Some(pv) = &self.preview {
            raw = crop_raw(&raw, pv.rect)?;
        }
        let out = lightcraft_enhance::job::run(&raw, &self.options, models, progress).map_err(|e| e.to_string())?;
        let mut info = json!({
            "id": id.0,
            "width": out.image.width,
            "height": out.image.height,
            "denoiser": out.denoiser,
            "seconds": out.timings.iter().map(|(k, v)| (k.to_string(), json!((v * 100.0).round() / 100.0))).collect::<serde_json::Map<_, _>>(),
        });
        let metadata = if raw.metadata == lightcraft_meta::Metadata::default() { metadata } else { raw.metadata.clone() };
        let photo = EnhancedPhoto { id, image: out.image, color: raw.color.clone(), orientation: raw.orientation, metadata, info: Value::Null };
        let preview = match &self.preview {
            Some(pv) => {
                let before = raw.develop(lightcraft_raw::Method::Ahd).map_err(|e| e.to_string())?;
                let render = |img: &Rgb32f| render_crop(img, &raw, &photo.metadata, &pv.settings, pv.side);
                Some((render(&before)?, render(&photo.image)?))
            }
            None => None,
        };
        info["ok"] = json!(true);
        Ok((EnhancedPhoto { info, ..photo }, preview))
    }
}

/// The part `rect` (normalised, unrotated) of a raw as a raw of its own (on the colour filter's phase), for
/// previews.
#[cfg(feature = "enhance")]
fn crop_raw(raw: &lightcraft_raw::RawImage, rect: [f64; 4]) -> std::result::Result<lightcraft_raw::RawImage, String> {
    let c = raw.crop.clipped(raw.active_area.width, raw.active_area.height);
    let (ox, oy, w, h) = if c.width > 1 && c.height > 1 {
        (raw.active_area.x + c.x, raw.active_area.y + c.y, c.width, c.height)
    } else {
        (raw.active_area.x, raw.active_area.y, raw.active_area.width, raw.active_area.height)
    };
    let [x0, y0, x1, y1] = rect.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    let even = |v: f64, n: usize| ((v * n as f64) as usize).min(n) & !1;
    let (cx0, cy0, cx1, cy1) = (even(x0, w), even(y0, h), even(x1, w).max(even(x0, w) + 32), even(y1, h).max(even(y0, h) + 32));
    let (cw, ch) = (cx1.min(w).saturating_sub(cx0) & !1, cy1.min(h).saturating_sub(cy0) & !1);
    if cw < 16 || ch < 16 || raw.cpp == 0 {
        return Err("the preview area is too small".into());
    }
    let mut out = raw.clone();
    out.active_area = lightcraft_raw::Rect { x: ox + cx0, y: oy + cy0, width: cw, height: ch };
    out.crop = lightcraft_raw::Rect { x: 0, y: 0, width: cw, height: ch };
    out.opcodes = Default::default();
    Ok(out)
}

/// Render camera RGB (from `raw`'s decode) with `settings` as the library shows it, at most `side` pixels: through
/// a linear DNG, so the colour model is exactly the photo's own.
#[cfg(feature = "enhance")]
fn render_crop(
    img: &Rgb32f,
    raw: &lightcraft_raw::RawImage,
    metadata: &lightcraft_meta::Metadata,
    settings: &DevelopSettings,
    side: usize,
) -> std::result::Result<lightcraft_raster::Rgba8, String> {
    let small = lightcraft_raster::resample::fit(img, side, side, lightcraft_raster::resample::Filter::Box);
    let color = lightcraft_merge::FrameColor::Camera(Box::new(raw.color.clone()));
    let dng = lightcraft_merge::write_linear_dng(
        &small,
        &color,
        raw.orientation,
        metadata,
        raw.color.baseline_exposure,
        lightcraft_merge::DngSamples::Half,
    )
    .map_err(|e| e.to_string())?;
    let (src, info) = crate::files::load_bytes(&dng, side)?;
    let mut s = settings.clone();
    // the crop is the whole picture now; the geometry belongs to the full photo
    s.crop = Default::default();
    s.enhance = Default::default();
    let req = lightcraft_pipeline::RenderRequest {
        quality: lightcraft_pipeline::Quality::Full,
        apply_crop: false,
        ..lightcraft_pipeline::RenderRequest::fit(side, side)
    };
    Ok(lightcraft_pipeline::render(&src, &info, &s, &req).image)
}

/// `dir/<stem>-<suffix>.dng`, then `-2`, `-3`…
fn output_names(first: &str, suffix: &str) -> impl Iterator<Item = PathBuf> {
    let p = Path::new(first);
    let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Enhanced".into());
    let suffix = suffix.to_string();
    (1u64..).map(move |k| dir.join(if k == 1 { format!("{stem}-{suffix}.dng") } else { format!("{stem}-{suffix}-{k}.dng") }))
}

/// Parse `photo.enhance` / `enhance.preview` parameters.
pub fn parse_options(cmd: &str, p: &Value) -> Result<Options> {
    let bad = |m: &str| EngineError::BadParams { cmd: cmd.into(), msg: m.into() };
    let denoise = match p.get("denoise") {
        None | Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => {
            if *b {
                50.0
            } else {
                0.0
            }
        }
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("denoise: a number 0..100 (or true for 50)"))?.clamp(0.0, 100.0),
    };
    let o = Options {
        denoise: denoise as f32,
        raw_details: p.get("rawDetails").and_then(Value::as_bool).unwrap_or(false),
        super_resolution: p.get("superResolution").and_then(Value::as_bool).unwrap_or(false),
    };
    if !o.any() {
        return Err(bad("choose denoise (1..100), rawDetails or superResolution"));
    }
    Ok(o)
}

impl Session {
    /// Plan an Enhance of `ids` (raw files). `preview`: the part of the first photo to preview, and its size.
    pub fn plan_enhance(
        &self,
        options: Options,
        ids: &[PhotoId],
        in_place: bool,
        stack: bool,
        preview: Option<([f64; 4], usize)>,
    ) -> Result<EnhanceJob> {
        if !options.any() {
            return Err(EngineError::Other("choose Denoise, Raw Details or Super Resolution".into()));
        }
        let mut sources = Vec::new();
        let mut products = Vec::new();
        for id in ids {
            let p = self.catalog.photo(*id).ok_or(lightcraft_catalog::CatalogError::NoPhoto(*id))?;
            match &p.source {
                Source::File { path } => {
                    sources.push((*id, path.clone()));
                    products.push(if in_place { enhanced_for(p, &options) } else { None });
                }
                Source::Demo { .. } => return Err(EngineError::Other(format!("{} is a demo photo; Enhance needs a raw file", p.file_name))),
            }
        }
        if sources.is_empty() {
            return Err(EngineError::Other("select a raw photo to enhance".into()));
        }
        let preview = match preview {
            Some((rect, side)) => {
                let first = sources.first().map(|s| s.0).ok_or_else(|| EngineError::Other("nothing to preview".into()))?;
                let settings = self.develop_of(first).map(|d| (*d).clone()).unwrap_or_default();
                Some(PreviewSpec { rect, side: side.clamp(64, 2048), settings })
            }
            None => None,
        };
        let read: ByteReader = match &self.media.file_bytes {
            Some(r) => r.clone(),
            None => std::sync::Arc::new(|path: &str| std::fs::read(path).map_err(|e| format!("{path}: {e}"))),
        };
        Ok(EnhanceJob { options, in_place, stack, sources, products, preview, read, models_root: self.enhancer.models_root.clone() })
    }

    /// Write a finished Enhance: new DNGs next to their originals (imported, stacked, given the originals'
    /// settings), or in-place results (cache file + the photos' `enhance` settings). Returns `{photos: [...]}`.
    pub fn finish_enhance(&mut self, job: &EnhanceJob, out: EnhanceOutput) -> Result<Value> {
        let mut done = Vec::new();
        let mut new_ids = Vec::new();
        for r in out.photos {
            let photo = match r {
                Ok(p) => p,
                Err((id, e)) => {
                    done.push(json!({"id": id.0, "ok": false, "error": e}));
                    continue;
                }
            };
            let Some(idx) = job.sources.iter().position(|(id, _)| *id == photo.id) else { continue };
            let mut info = photo.info.clone();
            if job.in_place {
                let Some(Some(e)) = job.products.get(idx) else { continue };
                if let Some(dir) = e.product.parent() {
                    std::fs::create_dir_all(dir).map_err(|err| EngineError::Other(format!("{}: {err}", dir.display())))?;
                }
                lightcraft_denoise_core::product::write(&e.product, &photo.image, &e.key)
                    .map_err(|err| EngineError::Other(format!("could not keep the enhanced picture: {err}")))?;
                if let Some(d) = self.develop_of(photo.id) {
                    let mut d = (*d).clone();
                    d.enhance.denoise = f64::from(job.options.denoise);
                    d.enhance.raw_details = job.options.raw_details;
                    d.enhance.super_resolution = job.options.super_resolution;
                    if job.options.denoise > 0.0 {
                        d.detail.nr_luminance = 0.0;
                        d.detail.nr_color = 0.0;
                    }
                    self.media.forget(photo.id);
                    self.set_develop(photo.id, d, "Enhance")?;
                }
                info["inPlace"] = json!(true);
                info["product"] = json!(e.product.to_string_lossy());
            } else {
                let first = &job.sources[idx].1;
                let mut metadata = photo.metadata.clone();
                metadata.software = Some("LightCraft Enhance".into());
                let color = lightcraft_merge::FrameColor::Camera(Box::new(photo.color.clone()));
                let dng = lightcraft_merge::write_linear_dng(
                    &photo.image,
                    &color,
                    photo.orientation,
                    &metadata,
                    photo.color.baseline_exposure,
                    lightcraft_merge::DngSamples::Half,
                )
                .map_err(|e| EngineError::Other(e.to_string()))?;
                let path = lightcraft_catalog::safe_file::write_new_unique(&mut output_names(first, &job.options.suffix()), &dng)
                    .map_err(|e| EngineError::Other(format!("could not write the enhanced DNG next to {first}: {e}")))?
                    .to_string_lossy()
                    .to_string();
                let report = crate::import::import(self, std::slice::from_ref(&path), crate::import::ImportMode::Add)?;
                let Some(id) = report.imported.first().copied().map(PhotoId) else {
                    // the very same picture is in the library already (the same Enhance run again): keep that one
                    let _ = std::fs::remove_file(&path);
                    let why = if report.failed.is_empty() {
                        "the same enhanced picture is already in the library".to_string()
                    } else {
                        format!("{:?}", report.failed)
                    };
                    done.push(json!({"id": photo.id.0, "ok": false, "error": format!("{path} was not added: {why}")}));
                    continue;
                };
                // the original's edit carries over (Lightroom does); its noise reduction is the model's job now
                if let Some(d) = self.develop_of(photo.id) {
                    let mut d = (*d).clone();
                    d.enhance = Default::default();
                    if job.options.denoise > 0.0 {
                        d.detail.nr_luminance = 0.0;
                        d.detail.nr_color = 0.0;
                    }
                    self.set_develop(id, d, "Enhance")?;
                }
                if job.stack
                    && let Some(op) = self.catalog.stack_with_ops(id, &[photo.id])
                {
                    self.commit("Stack with Original", op)?;
                }
                new_ids.push(id);
                info["newId"] = json!(id.0);
                info["path"] = json!(path);
                info["bytes"] = json!(dng.len());
            }
            done.push(info);
        }
        if !new_ids.is_empty() {
            self.selection = crate::Selection { active: new_ids.first().copied(), ids: new_ids };
        }
        Ok(json!({"photos": done}))
    }

    /// Plan, run and finish synchronously (CLI, MCP, control channel).
    pub fn enhance_now(&mut self, options: Options, ids: &[PhotoId], in_place: bool, stack: bool) -> Result<Value> {
        let job = self.plan_enhance(options, ids, in_place, stack, None)?;
        let out = job.run(&|_, _| true).map_err(EngineError::Other)?;
        self.finish_enhance(&job, out)
    }

    /// Whether each photo's in-place Enhance result is there.
    pub fn enhance_status(&self, ids: &[PhotoId]) -> Value {
        let photos: Vec<Value> = ids
            .iter()
            .filter_map(|id| self.catalog.photo(*id))
            .map(|p| {
                let o = options_of(&p.develop);
                let e = enhanced_source(p);
                let ready = e.as_ref().is_some_and(|e| lightcraft_denoise_core::product::is_current(&e.product, &e.key));
                json!({"id": p.id.0, "options": o, "inPlace": o.any(), "ready": ready, "missing": o.any() && !ready})
            })
            .collect();
        json!({"photos": photos, "models": self.enhancer.models_status()})
    }
}

/// Long edge of the picture the depth model looks at (it works at 518 px).
#[cfg(feature = "enhance")]
const DEPTH_INPUT_EDGE: usize = 768;

impl Session {
    /// Estimate photo `id`'s depth map (Depth Anything V2 Small on the photo as developed, uncropped, without masks
    /// or lens blur, as AI masks see it) and keep it in its settings ("Estimate Depth", undoable).
    pub fn estimate_depth(&mut self, id: PhotoId) -> Result<Value> {
        #[cfg(not(feature = "enhance"))]
        {
            let _ = id;
            Err(EngineError::Other("depth estimation is not available in this build".into()))
        }
        #[cfg(feature = "enhance")]
        {
            let model = self.enhancer.depth_model().map_err(EngineError::Other)?;
            let cur = self.develop_of(id).ok_or(lightcraft_catalog::CatalogError::NoPhoto(id))?;
            let mut d = (*cur).clone();
            d.masks.clear();
            d.lens_blur.enabled = false;
            d.depth = None;
            let job = self.preview_job(id, DEPTH_INPUT_EDGE, DEPTH_INPUT_EDGE, false, &d).ok_or(lightcraft_catalog::CatalogError::NoPhoto(id))?;
            let t0 = std::time::Instant::now();
            let r = job.run();
            let img = r.rendered.map_err(EngineError::Other)?.image;
            let mut rgb = Rgb32f::new(img.width, img.height);
            for (o, p) in rgb.data.iter_mut().zip(&img.data) {
                *o = [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0);
            }
            let t1 = std::time::Instant::now();
            let depth = model.estimate(&rgb).map_err(|e| EngineError::Other(e.to_string()))?;
            let estimate = t1.elapsed().as_secs_f64();
            let mut next = (*self.develop_of(id).ok_or(lightcraft_catalog::CatalogError::NoPhoto(id))?).clone();
            next.depth = Some(lightcraft_develop::DepthMap::from_values(depth.width, depth.height, &depth.data));
            self.set_develop(id, next, "Estimate Depth")?;
            Ok(json!({
                "id": id.0, "width": depth.width, "height": depth.height,
                "seconds": {"render": (t1 - t0).as_secs_f64(), "depth": estimate},
            }))
        }
    }

    /// Estimate photo `id`'s depth unless it has a depth map.
    pub fn ensure_depth(&mut self, id: PhotoId) -> Result<()> {
        if self.develop_of(id).is_some_and(|d| d.depth.is_none()) {
            self.estimate_depth(id)?;
        }
        Ok(())
    }
}

/// Compile the GPU kernels the models use on a background thread (once per process), when a model is installed:
/// the first Enhance, preview or depth map then doesn't wait ~30 s for them.
pub fn warm_up_in_background(models_root: &Path) {
    #[cfg(feature = "enhance")]
    {
        static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let any = lightcraft_enhance::models::MODELS.iter().any(|m| lightcraft_enhance::models::installed(models_root, m.id).is_some());
        if any && !STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let _ = std::thread::Builder::new().name("enhance-warm-up".into()).spawn(|| {
                let _ = std::panic::catch_unwind(lightcraft_enhance::warm_up);
            });
        }
    }
    #[cfg(not(feature = "enhance"))]
    let _ = models_root;
}

/// Enhance's state in a session: where the models are, and their downloads.
pub struct Enhancer {
    pub models_root: PathBuf,
    #[cfg(feature = "enhance")]
    downloads: [Arc<download::Shared>; 3],
}

impl Default for Enhancer {
    fn default() -> Self {
        Enhancer {
            models_root: models_root(),
            #[cfg(feature = "enhance")]
            downloads: Default::default(),
        }
    }
}

impl Enhancer {
    /// Each model: installed, its size and licence, and its download.
    pub fn models_status(&self) -> Value {
        let list: Vec<Value> = lightcraft_enhance::models::MODELS
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let installed = lightcraft_enhance::models::installed(&self.models_root, m.id).is_some();
                #[cfg(feature = "enhance")]
                let dl = self.downloads.get(i).map(|d| d.status()).unwrap_or(Value::Null);
                #[cfg(not(feature = "enhance"))]
                let dl = {
                    let _ = i;
                    Value::Null
                };
                json!({
                    "model": m.key, "name": m.name, "licence": m.licence, "licenceUrl": m.licence_url, "source": m.source,
                    "bytes": m.size, "installed": installed, "download": dl,
                    "folder": lightcraft_enhance::models::dir(&self.models_root, m.id).to_string_lossy(),
                })
            })
            .collect();
        json!({"available": AVAILABLE, "models": list})
    }

    /// Start downloading a model (background thread). `Ok(false)` when installed or already downloading.
    pub fn start_download(&self, key: &str) -> std::result::Result<bool, String> {
        let id = lightcraft_enhance::models::by_key(key).ok_or_else(|| format!("unknown model `{key}` (raw-denoise, super-resolution, depth)"))?;
        if lightcraft_enhance::models::installed(&self.models_root, id).is_some() {
            return Ok(false);
        }
        #[cfg(feature = "enhance")]
        {
            let i = lightcraft_enhance::models::MODELS.iter().position(|m| m.id == id).unwrap_or(0);
            let d = self.downloads.get(i).ok_or("no download slot")?;
            download::start(d.clone(), self.models_root.clone(), id)
        }
        #[cfg(not(feature = "enhance"))]
        Err("Enhance is not available in this build".into())
    }

    /// Stop a running download (its partial file stays, to resume).
    pub fn cancel_download(&self, key: &str) -> bool {
        #[cfg(feature = "enhance")]
        if let Some(i) = lightcraft_enhance::models::MODELS.iter().position(|m| m.key == key)
            && let Some(d) = self.downloads.get(i)
        {
            return d.cancel();
        }
        let _ = key;
        false
    }

    /// The depth model, loaded (an actionable error when it isn't installed).
    #[cfg(feature = "enhance")]
    pub fn depth_model(&self) -> std::result::Result<lightcraft_enhance::depth::DepthModel, String> {
        loaded::depth(&self.models_root)?
            .ok_or_else(|| lightcraft_enhance::models::missing_message(&self.models_root, lightcraft_enhance::models::ModelId::Depth))
    }
}

#[cfg(feature = "enhance")]
mod download {
    //! A model download on a background thread: progress in atomics, never blocking the session.
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use serde_json::{Value, json};

    #[derive(Default)]
    pub struct Shared {
        running: AtomicBool,
        cancel: AtomicBool,
        finished: AtomicBool,
        done: AtomicU64,
        total: AtomicU64,
        error: Mutex<Option<String>>,
    }

    impl Shared {
        pub fn status(&self) -> Value {
            let error = self.error.try_lock().ok().and_then(|e| e.clone());
            json!({
                "running": self.running.load(Ordering::SeqCst), "finished": self.finished.load(Ordering::SeqCst),
                "done": self.done.load(Ordering::SeqCst), "total": self.total.load(Ordering::SeqCst), "error": error,
            })
        }
        pub fn cancel(&self) -> bool {
            let r = self.running.load(Ordering::SeqCst);
            if r {
                self.cancel.store(true, Ordering::SeqCst);
            }
            r
        }
    }

    struct Running(Arc<Shared>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.running.store(false, Ordering::SeqCst);
        }
    }

    pub fn start(s: Arc<Shared>, root: PathBuf, id: lightcraft_enhance::models::ModelId) -> Result<bool, String> {
        if s.running.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        s.cancel.store(false, Ordering::SeqCst);
        s.finished.store(false, Ordering::SeqCst);
        s.done.store(0, Ordering::SeqCst);
        s.total.store(lightcraft_enhance::models::info(id).size, Ordering::SeqCst);
        *s.error.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let guard = Running(s.clone());
        std::thread::Builder::new()
            .name("enhance-download".into())
            .spawn(move || {
                let s = guard.0.clone();
                let mut on = |p: &lightcraft_enhance::fetch::Progress| {
                    s.done.store(p.done, Ordering::SeqCst);
                    s.total.store(p.total, Ordering::SeqCst);
                };
                let r =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lightcraft_enhance::models::download(&root, id, &s.cancel, &mut on)));
                let error = match r {
                    Ok(Ok(_)) => {
                        s.finished.store(true, Ordering::SeqCst);
                        None
                    }
                    Ok(Err(e)) => Some(e),
                    Err(_) => Some("the download failed unexpectedly".into()),
                };
                if let Some(e) = &error {
                    log::warn!("enhance model download: {e}");
                }
                *s.error.lock().unwrap_or_else(PoisonError::into_inner) = error;
                drop(guard);
            })
            .map(|_| true)
            .map_err(|e| format!("could not start the download: {e}"))
    }
}
