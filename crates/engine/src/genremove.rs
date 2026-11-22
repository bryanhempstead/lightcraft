//! Generative Remove (Lightroom Classic's Remove tool with "Use Generative AI"): a Fill spot whose pixels LaMa
//! (`lightcraft_enhance::lama`, Apache-2.0, downloaded with consent as the Enhance model `inpaint`) makes, in a
//! few variations to pick from.
//!
//! - A generative spot names its pixels ([`lightcraft_develop::Generative`]: a key, the variation shown and how
//!   many there are). The key is a hash of what the model looks at: the photo's content, the pixel-moving settings
//!   and the spots before it ([`lightcraft_develop::ai::basis`]) and the spot's place and size. The pixels live in
//!   the cache ([`cache_dir`], half floats), never in the catalog: LaMa is deterministic, so missing pixels (cleared
//!   caches, another computer) are made again from the photo ([`Session::gen_schedule`]); until they are there the
//!   spot is filled content-aware (PatchMatch), and without the model it stays that way, with a notice.
//! - What the model sees: the scene-linear retouch stage (white balance, the spots before this one) of a crop around
//!   the spot at up to [`MAX_EDGE`] pixels, encoded like a display picture; what it makes is decoded back and kept
//!   *before* white balance, so later white-balance or tone changes apply to it like to the rest of the photo.
//!   Renders resample it from normalized coordinates at their own size (previews and exports match).
//! - Variations: the model is run on the crop as it is, mirrored, with more context around the spot, at a coarser
//!   scale ([`RECIPES`]). `spot.variation` cycles them (`<` / `>`), `spot.moreVariations` adds three more.
//! - In the app the work runs on a worker thread ([`GenRemover::background`]) and [`Session::gen_poll`] picks up
//!   the results; the CLI and MCP wait for them.


use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use lightcraft_catalog::PhotoId;
use lightcraft_develop::{DevelopSettings, Generative, Spot, SpotMode};
use lightcraft_pipeline::spots::{GenFill, gen_geometry};
use lightcraft_raster::Rgb32f;
use serde_json::{Value, json};

use crate::Session;

/// Version of the generation (part of every key: a change remakes the pixels).
const GEN_VERSION: u32 = 1;
/// Longest edge of the crop the model looks at (`LIGHTCRAFT_GENREMOVE_EDGE` overrides, 64..4096).
pub const MAX_EDGE: usize = 1024;
/// Variations a new generative spot gets (Lightroom: three).
pub const VARIATIONS: u8 = 3;

/// How variation `i` is made: (context around the spot, in spot sizes; mirrored; scale of the model's input).
pub const RECIPES: [(f64, bool, f64); 8] =
    [(1.0, false, 1.0), (1.0, true, 1.0), (1.8, false, 1.0), (1.8, true, 1.0), (1.0, false, 0.6), (1.0, true, 0.6), (2.6, false, 1.0), (2.6, true, 1.0)];

/// Where generated pixels are kept: `LIGHTCRAFT_REMOVE_CACHE`, else `~/Library/Caches/LightCraft/Remove`.
pub fn cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("LIGHTCRAFT_REMOVE_CACHE") {
        return PathBuf::from(d);
    }
    if cfg!(test) {
        return std::env::temp_dir().join(format!("lc-remove-cache-test-{}", std::process::id()));
    }
    match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h).join("Library/Caches/LightCraft/Remove"),
        None => std::env::temp_dir().join("LightCraft Remove"),
    }
}

fn max_edge() -> usize {
    std::env::var("LIGHTCRAFT_GENREMOVE_EDGE").ok().and_then(|v| v.parse::<usize>().ok()).map_or(MAX_EDGE, |v| v.clamp(64, 4096))
}

/// A cache name is `<16 hex>-<digit>`: nothing else is ever turned into a path.
fn valid_name(name: &str) -> bool {
    let mut parts = name.split('-');
    let (Some(k), Some(v), None) = (parts.next(), parts.next(), parts.next()) else { return false };
    k.len() == 16 && k.bytes().all(|b| b.is_ascii_hexdigit()) && v.len() == 1 && v.bytes().all(|b| b.is_ascii_digit())
}

fn path_of(dir: &Path, name: &str) -> Option<PathBuf> {
    valid_name(name).then(|| dir.join(format!("{name}.lcgen")))
}

fn header_key(geometry: u64, rect: [f64; 4]) -> String {
    format!("lcgen{GEN_VERSION} {geometry:016x} {:?} {:?} {:?} {:?}", rect[0], rect[1], rect[2], rect[3])
}

fn parse_header_key(k: &str) -> Option<(u64, [f64; 4])> {
    let mut it = k.split(' ');
    if it.next()? != format!("lcgen{GEN_VERSION}") {
        return None;
    }
    let g = u64::from_str_radix(it.next()?, 16).ok()?;
    let mut r = [0f64; 4];
    for v in &mut r {
        *v = it.next()?.parse().ok()?;
    }
    (it.next().is_none() && r.iter().all(|v| v.is_finite())).then_some((g, r))
}

/// Write generated pixels.
pub fn save(dir: &Path, name: &str, fill: &GenFill) -> Result<(), String> {
    let path = path_of(dir, name).ok_or("not a generated-pixels name")?;
    lightcraft_denoise_core::product::write(&path, &fill.image, &header_key(fill.geometry, fill.rect)).map_err(|e| format!("{}: {e}", path.display()))?;
    loaded_forget();
    Ok(())
}

/// Read generated pixels (any damage is `None`: the spot is then filled content-aware and made again).
pub fn load(dir: &Path, name: &str) -> Option<GenFill> {
    let path = path_of(dir, name)?;
    let header = lightcraft_denoise_core::product::read_header_at(&path).ok()?;
    let (geometry, rect) = parse_header_key(&header.key)?;
    let image = lightcraft_denoise_core::product::read(&path, &header.key, 1).ok()?;
    Some(GenFill { geometry, rect, image })
}

/// Whether the pixels `name` names are in the cache.
pub fn made(name: &str) -> bool {
    path_of(&cache_dir(), name).is_some_and(|p| p.is_file())
}

/// Recently used pixels (and names known to be missing), so renders don't read the disk each time.
struct Loaded {
    hits: Vec<(String, Arc<GenFill>)>,
    misses: Vec<String>,
}

static LOADED: Mutex<Loaded> = Mutex::new(Loaded { hits: Vec::new(), misses: Vec::new() });
const LOADED_MAX: usize = 24;

fn loaded_forget() {
    LOADED.lock().unwrap_or_else(PoisonError::into_inner).misses.clear();
}

fn lookup(name: &str) -> Option<Arc<GenFill>> {
    {
        let g = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, f)) = g.hits.iter().find(|(n, _)| n == name) {
            return Some(f.clone());
        }
        if g.misses.iter().any(|n| n == name) {
            return None;
        }
    }
    let found = load(&cache_dir(), name).map(Arc::new);
    let mut g = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
    match &found {
        Some(f) => {
            g.hits.retain(|(n, _)| n != name);
            g.hits.push((name.to_string(), f.clone()));
            if g.hits.len() > LOADED_MAX {
                g.hits.remove(0);
            }
        }
        None => {
            if g.misses.len() > 512 {
                g.misses.clear();
            }
            g.misses.push(name.to_string());
        }
    }
    found
}

/// Let renders find generated pixels (once per process).
pub fn install_loader() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| lightcraft_pipeline::spots::set_gen_loader(Some(Arc::new(lookup))));
}

/// The part of a render's cache key that follows generated pixels arriving (0 without generative spots).
pub fn render_key(s: &DevelopSettings) -> u64 {
    if s.spots.iter().all(|sp| sp.generative.is_none()) {
        return 0;
    }
    install_loader();
    lightcraft_pipeline::spots::gen_state(&s.spots).wrapping_mul(0x2545_f491_4f6c_dd1d) | 1
}

/// The key of spot `i` of `d` on a photo whose content is `content`.
pub fn key_of(content: &str, d: &DevelopSettings, i: usize) -> Option<String> {
    let spot = d.spots.get(i)?;
    let mut before = d.clone();
    before.spots.truncate(i);
    let text = format!("v{GEN_VERSION} {content} {} {:016x}", lightcraft_develop::ai::basis(&before), gen_geometry(spot));
    let h = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    Some(format!("{h:016x}"))
}

/// Give every generative spot of `d` the key its pixels have now (in order: a spot's key follows the ones
/// before it). Variations stay picked when the key is unchanged.
pub fn rekey(content: &str, d: &mut DevelopSettings) {
    for i in 0..d.spots.len() {
        if d.spots.get(i).is_some_and(|s| s.generative.is_some())
            && let Some(k) = key_of(content, d, i)
            && let Some(g) = d.spots.get_mut(i).and_then(|s| s.generative.as_mut())
            && g.key != k
        {
            g.key = k;
        }
    }
}

/// One spot's generation, detached from the session.
pub struct GenJob {
    pub photo: PhotoId,
    /// Where the model is (the session's Enhance models folder).
    pub models_root: PathBuf,
    pub key: String,
    pub geometry: u64,
    /// The spot (normalized points, size in long-edge units).
    pub spot: Spot,
    /// The photo's oriented size (px) and its exposure (to look at the crop like a display picture).
    pub oriented: (f64, f64),
    pub exposure: f64,
    /// Variations to make: (index, the scene-linear crop for its context, that crop's rectangle).
    pub variations: Vec<(u8, crate::media::RenderJob, [f64; 4])>,
}

/// What a finished job made.
pub struct GenDone {
    pub photo: PhotoId,
    pub key: String,
    pub made: Vec<String>,
    pub error: Option<String>,
    pub seconds: f64,
}

/// Generative Remove's state in a session.
pub struct GenRemover {
    /// Run generations on a worker thread (the app); `false` waits for them (CLI, MCP, tests).
    pub background: bool,
    jobs: Option<Sender<GenJob>>,
    done: Option<Receiver<GenDone>>,
    busy: Arc<AtomicUsize>,
    /// Names queued or being made.
    pending: Vec<String>,
    /// The model was missing when a generative spot was added (said once per session).
    pub missing_said: bool,
}

impl Default for GenRemover {
    fn default() -> Self {
        GenRemover { background: false, jobs: None, done: None, busy: Arc::new(AtomicUsize::new(0)), pending: Vec::new(), missing_said: false }
    }
}

impl GenRemover {
    /// Spots being generated now.
    pub fn busy(&self) -> usize {
        self.busy.load(Ordering::SeqCst)
    }

    fn sender(&mut self) -> Option<Sender<GenJob>> {
        if self.jobs.is_none() {
            let (tx, rx) = channel::<GenJob>();
            let (dtx, drx) = channel::<GenDone>();
            let busy = self.busy.clone();
            let spawned = std::thread::Builder::new().name("generative-remove".into()).spawn(move || {
                while let Ok(job) = rx.recv() {
                    let photo = job.photo;
                    let key = job.key.clone();
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&job)))
                        .unwrap_or_else(|_| GenDone { photo, key: key.clone(), made: Vec::new(), error: Some("generating failed unexpectedly".into()), seconds: 0.0 });
                    busy.fetch_sub(1, Ordering::SeqCst);
                    if dtx.send(r).is_err() {
                        break;
                    }
                }
            });
            if spawned.is_err() {
                return None;
            }
            self.jobs = Some(tx);
            self.done = Some(drx);
        }
        self.jobs.clone()
    }
}

/// Whether the inpainting model is installed.
pub fn model_installed(models_root: &Path) -> bool {
    lightcraft_enhance::models::installed(models_root, lightcraft_enhance::models::ModelId::Inpaint).is_some()
}

/// What to tell the user when the model is missing.
pub fn missing_message(models_root: &Path) -> String {
    format!(
        "Generative Remove fills content-aware until its model is there: {}",
        lightcraft_enhance::models::missing_message(models_root, lightcraft_enhance::models::ModelId::Inpaint)
    )
}

#[cfg(feature = "enhance")]
mod model {
    use std::sync::{Arc, Mutex, PoisonError};

    use lightcraft_enhance::lama::Inpainter;
    use lightcraft_enhance::models::{self, ModelId};

    static LAMA: Mutex<Option<(std::path::PathBuf, Arc<Inpainter>)>> = Mutex::new(None);

    pub fn get(root: &std::path::Path) -> Result<Arc<Inpainter>, String> {
        let path = models::installed(root, ModelId::Inpaint).ok_or_else(|| super::missing_message(root))?;
        let mut g = LAMA.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((p, m)) = g.as_ref()
            && *p == path
        {
            return Ok(m.clone());
        }
        let m = Arc::new(Inpainter::load(&path, lightcraft_enhance::best_device()).map_err(|e| e.to_string())?);
        *g = Some((path, m.clone()));
        Ok(m)
    }
}

/// The photo's colour → what the model sees (0..1, display-encoded) and back.
#[cfg_attr(not(feature = "enhance"), allow(dead_code))]
struct Encoding {
    scale: f32,
}

#[cfg_attr(not(feature = "enhance"), allow(dead_code))]
impl Encoding {
    fn of(img: &Rgb32f, hole: &[bool], exposure: f64) -> Encoding {
        let gain = (exposure.clamp(-10.0, 10.0) as f32).exp2();
        let mut lum: Vec<f32> = img
            .data
            .iter()
            .zip(hole)
            .filter(|(_, h)| !**h)
            .map(|(p, _)| p[0].max(p[1]).max(p[2]) * gain)
            .filter(|v| v.is_finite())
            .collect();
        let top = if lum.is_empty() {
            1.0
        } else {
            let k = (lum.len() * 995 / 1000).min(lum.len() - 1);
            *lum.select_nth_unstable_by(k, f32::total_cmp).1
        };
        Encoding { scale: gain / top.max(1.0) }
    }
    fn encode(&self, c: [f32; 3]) -> [f32; 3] {
        c.map(|v| lightcraft_color::transfer::linear_to_srgb((v * self.scale).clamp(0.0, 1.0)))
    }
    fn decode(&self, c: [f32; 3]) -> [f32; 3] {
        c.map(|v| lightcraft_color::transfer::srgb_to_linear(v.clamp(0.0, 1.0)) / self.scale.max(1e-12))
    }
}

/// The pixels of a `w × h` crop of `rect` that the spot covers (grown by about two pixels).
#[cfg_attr(not(feature = "enhance"), allow(dead_code))]
fn hole_of(spot: &Spot, rect: [f64; 4], w: usize, h: usize, oriented: (f64, f64)) -> Vec<bool> {
    let mut hole = vec![false; w * h];
    let (ow, oh) = oriented;
    let long = ow.max(oh).max(1.0);
    // crop pixels per long-edge unit
    let ppl = w as f64 / ((rect[2] - rect[0]) * ow / long).max(1e-9);
    let r = spot.size * ppl + 2.0;
    if !r.is_finite() {
        return hole;
    }
    for p in &spot.points {
        if !(p.x.is_finite() && p.y.is_finite()) {
            continue;
        }
        let cx = (p.x - rect[0]) / (rect[2] - rect[0]) * w as f64;
        let cy = (p.y - rect[1]) / (rect[3] - rect[1]) * h as f64;
        let (x0, x1) = ((cx - r).floor().max(0.0) as usize, ((cx + r).ceil().max(0.0) as usize).min(w));
        let (y0, y1) = ((cy - r).floor().max(0.0) as usize, ((cy + r).ceil().max(0.0) as usize).min(h));
        for y in y0..y1 {
            for x in x0..x1 {
                if (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy) < r
                    && let Some(m) = hole.get_mut(y * w + x)
                {
                    *m = true;
                }
            }
        }
    }
    hole
}

#[cfg_attr(not(feature = "enhance"), allow(dead_code))]
fn invert3(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let inv = lightcraft_color::Mat3(m.map(|r| r.map(f64::from))).inverse()?;
    Some(inv.0.map(|r| r.map(|v| v as f32)))
}

#[cfg_attr(not(feature = "enhance"), allow(dead_code))]
fn resize(img: &Rgb32f, w: usize, h: usize) -> Rgb32f {
    lightcraft_raster::resample::resize(img, w.max(1), h.max(1), lightcraft_raster::resample::Filter::Mitchell)
}

/// Make one variation: the model on `img` (the encoded crop), mirrored and / or scaled per the recipe.
#[cfg(feature = "enhance")]
fn inpaint_variant(m: &lightcraft_enhance::lama::Inpainter, img: &Rgb32f, hole: &[bool], mirror: bool, scale: f64) -> Result<Rgb32f, String> {
    let (w, h) = (img.width, img.height);
    let flip = |v: &Rgb32f| if mirror { v.flip_h() } else { v.clone() };
    let flip_mask = |m: &[bool]| -> Vec<bool> {
        if !mirror {
            return m.to_vec();
        }
        (0..w * h).map(|i| m.get((i / w) * w + (w - 1 - i % w)).copied().unwrap_or(false)).collect()
    };
    let (sw, sh) = (((w as f64 * scale).round() as usize).max(16), ((h as f64 * scale).round() as usize).max(16));
    let src = flip(img);
    let mask = flip_mask(hole);
    let out = if (sw, sh) == (w, h) {
        m.inpaint(&src, &mask).map_err(|e| e.to_string())?
    } else {
        // coarser: the model's fill at the smaller size, scaled back up into the hole
        let small = resize(&src, sw, sh);
        let smask: Vec<bool> = (0..sw * sh)
            .map(|i| {
                let (x, y) = (i % sw, i / sw);
                let (x0, x1) = (x * w / sw, ((x + 1) * w).div_ceil(sw).min(w));
                let (y0, y1) = (y * h / sh, ((y + 1) * h).div_ceil(sh).min(h));
                (y0..y1).any(|yy| (x0..x1).any(|xx| mask.get(yy * w + xx).copied().unwrap_or(false)))
            })
            .collect();
        let filled = resize(&m.inpaint(&small, &smask).map_err(|e| e.to_string())?, w, h);
        let mut o = src.clone();
        for (i, d) in o.data.iter_mut().enumerate() {
            if mask.get(i).copied().unwrap_or(false)
                && let Some(v) = filled.data.get(i)
            {
                *d = *v;
            }
        }
        o
    };
    Ok(flip(&out))
}

/// Run a job: each variation's crop through the model, saved to the cache.
fn run(job: &GenJob) -> GenDone {
    let t0 = std::time::Instant::now();
    let (made, error) = make(job);
    GenDone { photo: job.photo, key: job.key.clone(), made, error, seconds: t0.elapsed().as_secs_f64() }
}

#[cfg(not(feature = "enhance"))]
fn make(_job: &GenJob) -> (Vec<String>, Option<String>) {
    (Vec::new(), Some("Generative Remove is not available in this build".into()))
}

/// One variation: its crop through the model, decoded and taken back before white balance.
#[cfg(feature = "enhance")]
fn make_one(m: &lightcraft_enhance::lama::Inpainter, job: &GenJob, render: &crate::media::RenderJob, rect: [f64; 4], recipe: (f64, bool, f64)) -> Result<GenFill, String> {
    let (lin, wb) = render.clone().linear()?;
    let hole = hole_of(&job.spot, rect, lin.width, lin.height, job.oriented);
    if !hole.iter().any(|h| *h) {
        return Err("the spot is outside the photo".into());
    }
    let enc = Encoding::of(&lin, &hole, job.exposure);
    let mut shown = lin.clone();
    shown.data.iter_mut().for_each(|p| *p = enc.encode(*p));
    let out = inpaint_variant(m, &shown, &hole, recipe.1, recipe.2)?;
    let back = wb.and_then(invert3);
    let mut image = lin;
    for ((d, o), h) in image.data.iter_mut().zip(&out.data).zip(&hole) {
        // outside the hole the crop stays as it was (never shown: the spot's alpha is 0 there)
        let c = if *h { enc.decode(*o) } else { *d };
        *d = match &back {
            Some(m) => std::array::from_fn(|r| m[r][0] * c[0] + m[r][1] * c[1] + m[r][2] * c[2]),
            None => c,
        };
    }
    Ok(GenFill { geometry: job.geometry, rect, image })
}

#[cfg(feature = "enhance")]
fn make(job: &GenJob) -> (Vec<String>, Option<String>) {
    let m = match model::get(&job.models_root) {
        Ok(m) => m,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let dir = cache_dir();
    let mut made = Vec::new();
    for (v, render, rect) in &job.variations {
        let name = format!("{}-{v}", job.key);
        let recipe = RECIPES.get(*v as usize).copied().unwrap_or(RECIPES[0]);
        match make_one(&m, job, render, *rect, recipe).and_then(|fill| save(&dir, &name, &fill)) {
            Ok(()) => made.push(name),
            Err(e) => return (made, Some(e)),
        }
    }
    (made, None)
}

/// The crop around `spot` with `context` spot sizes of room on each side (normalized, clamped to the photo).
fn context_rect(spot: &Spot, oriented: (f64, f64), context: f64) -> Option<[f64; 4]> {
    let (ow, oh) = oriented;
    let long = ow.max(oh).max(1.0);
    let (rx, ry) = (spot.size * long / ow.max(1.0), spot.size * long / oh.max(1.0));
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in spot.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()) {
        b = [b[0].min(p.x - rx), b[1].min(p.y - ry), b[2].max(p.x + rx), b[3].max(p.y + ry)];
    }
    if b[2] <= b[0] || b[3] <= b[1] || !b.iter().all(|v| v.is_finite()) {
        return None;
    }
    // room in pixels, the same on both axes: `context` × the larger side of the spot
    let side = ((b[2] - b[0]) * ow).max((b[3] - b[1]) * oh);
    let (mx, my) = (context * side / ow.max(1.0), context * side / oh.max(1.0));
    let r = [(b[0] - mx).max(0.0), (b[1] - my).max(0.0), (b[2] + mx).min(1.0), (b[3] + my).min(1.0)];
    (r[2] - r[0] > 1e-6 && r[3] - r[1] > 1e-6).then_some(r)
}

impl Session {
    /// The content key generated pixels of photo `id` follow.
    fn gen_content(&self, id: PhotoId) -> String {
        self.catalog.photo(id).map(|p| crate::media::content_key(p)).unwrap_or_default()
    }

    /// Recompute the keys of `d`'s generative spots (before committing an edit of photo `id`'s spots).
    pub fn gen_rekey(&self, id: PhotoId, d: &mut DevelopSettings) {
        rekey(&self.gen_content(id), d);
    }

    /// The generation job for spot `i` of photo `id` (its missing variations), when it needs one.
    fn gen_job(&mut self, id: PhotoId, i: usize) -> Option<GenJob> {
        let d = self.develop_of(id)?;
        let spot = d.spots.get(i)?.clone();
        let g = spot.generative.clone()?;
        let p = self.catalog.photo(id)?.clone();
        let (w, h) = (f64::from(p.width.max(1)), f64::from(p.height.max(1)));
        let oriented = if d.orientation.swaps_axes() { (h, w) } else { (w, h) };
        let edge = max_edge();
        let mut variations = Vec::new();
        for v in 0..g.count.min(Generative::MAX) {
            let name = format!("{}-{v}", g.key);
            if made(&name) || self.genremove.pending.contains(&name) {
                continue;
            }
            let recipe = RECIPES.get(v as usize).copied().unwrap_or(RECIPES[0]);
            let rect = context_rect(&spot, oriented, recipe.0)?;
            // the crop at the photo's own resolution, at most `edge` across, and big enough for the model
            let native = ((rect[2] - rect[0]) * oriented.0).max((rect[3] - rect[1]) * oriented.1);
            let long = native.clamp(64.0, edge as f64).round() as usize;
            let mut s = (*d).clone();
            s.spots.truncate(i);
            s.masks.clear();
            s.lens_blur.enabled = false;
            s.crop = lightcraft_develop::Crop {
                geometry: lightcraft_geom::CropGeometry { rect: lightcraft_geom::Rect::new(rect[0], rect[1], rect[2], rect[3]), angle: 0.0 },
                flip_h: false,
                flip_v: false,
                ..s.crop.clone()
            };
            s.geometry.constrain_crop = false;
            let job = self.crop_job(id, long, long, &s)?;
            variations.push((v, job, rect));
        }
        if variations.is_empty() {
            return None;
        }
        Some(GenJob { photo: id, models_root: self.enhancer.models_root.clone(), key: g.key.clone(), geometry: gen_geometry(&spot), spot, oriented, exposure: d.light.exposure, variations })
    }

    /// Make the missing pixels of photo `id`'s generative spots: queued on the worker in the app
    /// ([`GenRemover::background`]), else now. Returns (spots queued or made, errors).
    pub fn gen_schedule(&mut self, id: PhotoId) -> (usize, Vec<String>) {
        install_loader();
        let n = self.develop_of(id).map_or(0, |d| d.spots.len());
        let mut count = 0;
        let mut errors = Vec::new();
        if !model_installed(&self.enhancer.models_root) {
            let any = self.develop_of(id).is_some_and(|d| d.spots.iter().any(|s| s.generative.as_ref().is_some_and(|g| !made(&g.name()))));
            if any {
                errors.push(missing_message(&self.enhancer.models_root));
            }
            return (0, errors);
        }
        for i in 0..n {
            let Some(job) = self.gen_job(id, i) else { continue };
            count += 1;
            let names: Vec<String> = job.variations.iter().map(|(v, _, _)| format!("{}-{v}", job.key)).collect();
            if self.genremove.background
                && let Some(tx) = self.genremove.sender()
            {
                self.genremove.busy.fetch_add(1, Ordering::SeqCst);
                self.genremove.pending.extend(names);
                if tx.send(job).is_err() {
                    self.genremove.busy.fetch_sub(1, Ordering::SeqCst);
                    errors.push("the Generative Remove worker stopped".into());
                }
                continue;
            }
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&job)))
                .unwrap_or_else(|_| GenDone { photo: id, key: job.key.clone(), made: Vec::new(), error: Some("generating failed unexpectedly".into()), seconds: 0.0 });
            if let Some(e) = done.error {
                errors.push(e);
            }
        }
        (count, errors)
    }

    /// Pick up finished background generations: (photos that got pixels, errors).
    pub fn gen_poll(&mut self) -> (Vec<PhotoId>, Vec<String>) {
        let mut photos = Vec::new();
        let mut errors = Vec::new();
        let Some(rx) = &self.genremove.done else { return (photos, errors) };
        while let Ok(d) = rx.try_recv() {
            let prefix = format!("{}-", d.key);
            self.genremove.pending.retain(|n| !n.starts_with(&prefix));
            if let Some(e) = d.error {
                errors.push(e);
            }
            log::info!("generative remove: {} variation(s) in {:.1} s", d.made.len(), d.seconds);
            if !d.made.is_empty() && !photos.contains(&d.photo) {
                photos.push(d.photo);
            }
        }
        if self.genremove.busy() == 0 {
            self.genremove.pending.clear();
        }
        (photos, errors)
    }

    /// `remove.status`: photo `id`'s generative spots and whether their pixels are there.
    pub fn gen_status(&self, id: PhotoId) -> Value {
        let spots: Vec<Value> = self
            .develop_of(id)
            .map(|d| {
                d.spots
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| {
                        let g = s.generative.as_ref()?;
                        let made: Vec<bool> = (0..g.count).map(|v| made(&format!("{}-{v}", g.key))).collect();
                        Some(json!({"index": i, "variation": g.variation, "count": g.count, "made": made, "key": g.key}))
                    })
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "model": model_installed(&self.enhancer.models_root),
            "busy": self.genremove.busy(),
            "spots": spots,
            "available": crate::enhance::AVAILABLE,
        })
    }

    /// Make spot `i` of `d` generative (a Fill spot with [`VARIATIONS`] variations, the first shown).
    pub fn gen_make(&self, id: PhotoId, d: &mut DevelopSettings, i: usize) {
        if let Some(s) = d.spots.get_mut(i) {
            s.mode = SpotMode::Fill;
            s.source_offset = None;
            s.generative = Some(Generative { variation: 0, count: VARIATIONS, key: String::new() });
        }
        self.gen_rekey(id, d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_geom::Point;

    #[test]
    fn names_and_headers_round_trip_and_reject_paths() {
        assert!(valid_name("0123456789abcdef-2"));
        for bad in ["../etc/passwd-1", "0123456789abcdef", "0123456789abcdef-12", "0123456789abcdeg-1", "0123456789abcdef-1-1"] {
            assert!(!valid_name(bad), "{bad}");
        }
        let k = header_key(0xdead_beef, [0.1, 0.2, 0.30000000000000004, 1.0]);
        assert_eq!(parse_header_key(&k), Some((0xdead_beef, [0.1, 0.2, 0.30000000000000004, 1.0])));
        assert_eq!(parse_header_key("lcgen1 zz 0 0 1 1"), None);
        assert_eq!(parse_header_key("lcgen1 00 0 0 1 NaN"), None);
    }

    #[test]
    fn pixels_survive_the_cache_and_damage_is_none() {
        let dir = std::env::temp_dir().join(format!("lc-genremove-{}", std::process::id()));
        let image = Rgb32f::from_fn(40, 30, |x, y| [x as f32 / 40.0, y as f32 / 30.0, 0.5]);
        let fill = GenFill { geometry: 7, rect: [0.25, 0.5, 0.75, 1.0], image };
        save(&dir, "00000000000000aa-0", &fill).unwrap();
        let back = load(&dir, "00000000000000aa-0").unwrap();
        assert_eq!((back.geometry, back.rect), (7, [0.25, 0.5, 0.75, 1.0]));
        assert!((back.image.get(20, 15)[0] - 0.5).abs() < 1e-3);
        let p = path_of(&dir, "00000000000000aa-0").unwrap();
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, &bytes[..bytes.len() / 2]).unwrap();
        assert!(load(&dir, "00000000000000aa-0").is_none());
        assert!(load(&dir, "../x-0").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_follow_the_spot_and_the_spots_before_it_not_the_tone() {
        let mut d = DevelopSettings::default();
        d.spots.push(Spot { mode: SpotMode::Fill, points: vec![Point::new(0.3, 0.3)], size: 0.05, ..Default::default() });
        d.spots.push(Spot { mode: SpotMode::Fill, points: vec![Point::new(0.6, 0.6)], size: 0.05, ..Default::default() });
        let k1 = key_of("c", &d, 1).unwrap();
        d.light.exposure = 1.5;
        assert_eq!(key_of("c", &d, 1).unwrap(), k1);
        assert_ne!(key_of("other photo", &d, 1).unwrap(), k1);
        d.spots[0].size = 0.06;
        assert_ne!(key_of("c", &d, 1).unwrap(), k1);
        let k0 = key_of("c", &d, 0).unwrap();
        d.spots[1].points[0].x = 0.61;
        assert_eq!(key_of("c", &d, 0).unwrap(), k0);
        assert!(key_of("c", &d, 9).is_none());
    }

    #[test]
    fn the_crop_holds_the_spot_with_room_and_the_hole_covers_it() {
        let spot = Spot { points: vec![Point::new(0.5, 0.5), Point::new(0.52, 0.5)], size: 0.02, ..Default::default() };
        let r = context_rect(&spot, (3000.0, 2000.0), 1.0).unwrap();
        assert!(r[0] < 0.48 && r[2] > 0.54 && r[1] < 0.47 && r[3] > 0.53, "{r:?}");
        // the crop's own aspect
        let w = 200;
        let h = ((r[3] - r[1]) * 2000.0 / ((r[2] - r[0]) * 3000.0) * w as f64).round() as usize;
        let hole = hole_of(&spot, r, w, h, (3000.0, 2000.0));
        let at = |p: Point| {
            let (x, y) = (((p.x - r[0]) / (r[2] - r[0]) * w as f64) as usize, ((p.y - r[1]) / (r[3] - r[1]) * h as f64) as usize);
            hole[y * w + x]
        };
        assert!(at(Point::new(0.5, 0.5)) && at(Point::new(0.51, 0.5)) && at(Point::new(0.535, 0.5)));
        assert!(!at(Point::new(r[0] + 0.001, r[1] + 0.001)));
        // hostile spots never panic
        for size in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
            let s = Spot { points: vec![Point::new(0.5, 0.5)], size, ..Default::default() };
            let _ = context_rect(&s, (100.0, 100.0), 1.0).map(|r| hole_of(&s, r, 10, 10, (100.0, 100.0)));
        }
        assert!(context_rect(&Spot::default(), (100.0, 100.0), 1.0).is_none());
    }

    #[test]
    fn encoding_round_trips_inside_the_range() {
        let img = Rgb32f::from_fn(10, 10, |x, _| [x as f32 * 0.3; 3]);
        let hole = vec![false; 100];
        let e = Encoding::of(&img, &hole, 0.0);
        for v in [0.0f32, 0.01, 0.5, 2.0] {
            let c = e.decode(e.encode([v; 3]));
            assert!((c[0] - v).abs() < 1e-3 * v.max(1.0), "{v} → {c:?}");
        }
    }
}
