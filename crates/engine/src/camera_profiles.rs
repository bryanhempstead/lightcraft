//! Camera colour profiles of our own, pooled from many photos of one camera model.
//!
//! A raw file without colour matrices (Sony ARW, Nikon NEF, Fujifilm RAF) gets its look fitted to its own embedded camera
//! JPEG (`camera_preview`), but one photo shows too little of some colours: a lime shirt covering a
//! few dozen proxy pixels next to a hillside of foliage at the same hue. `lightcraft-cli calibrate`
//! pools the colour pairs of many photos per model and fits one matrix and hue/saturation/value
//! table; photos of that model then only fit their tone and chroma curves.
//!
//! Profiles are JSON files (`<model>.json`) in [`dir`]: `LIGHTCRAFT_CAMERA_PROFILES`, else
//! `<config>/camera-profiles`; a local profile replaces the one built in ([`BUNDLED`], from
//! `assets/camera-profiles/`). They are read once per process; a damaged or hostile file is
//! ignored with a warning. They hold aggregate colour statistics only, never image content.
use lightcraft_color::Mat3;
use lightcraft_raw::profile::HsvTable;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Format version of the profile files.
const VERSION: u32 = 1;
/// Largest profile file read (a 5 × 72 × 5 table is ~60 KB of JSON).
const MAX_FILE: u64 = 4 << 20;

/// Profiles built into LightCraft (`assets/camera-profiles/`, see `assets/ATTRIBUTION.md`):
/// `(model, JSON)`.
pub const BUNDLED: &[(&str, &str)] = &[
    ("ILCE-7M4", include_str!("../../../assets/camera-profiles/ILCE-7M4.json")),
    ("X-H2S", include_str!("../../../assets/camera-profiles/X-H2S.json")),
    ("X-T4", include_str!("../../../assets/camera-profiles/X-T4.json")),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraProfile {
    pub version: u32,
    /// The camera model as the files report it (Exif `Model`), e.g. `ILCE-7M4`.
    pub model: String,
    /// Photos and colour pairs the profile was fitted on.
    pub files: usize,
    pub samples: usize,
    /// White-balanced camera RGB (with the baseline exposure) → linear Rec.2020, rows.
    matrix: [[f64; 3]; 3],
    /// Hue/saturation/value correction after `matrix` (linear ProPhoto RGB).
    pub hue_sat: Option<HsvTable>,
    /// Tone and chroma curve, for a profile fitted to Lightroom's renders
    /// (`calibrate --lightroom`): the camera's starting look then comes wholly from the profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<lightcraft_pipeline::tone::CameraTone>,
    /// What the profile was fitted to: `"lightroom"` (the user's Lightroom previews), else the
    /// camera's own JPEGs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// How the camera's neutral follows Lightroom's Temp / Tint ([`lightcraft_pipeline::CameraWb`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wb: Option<WbFit>,
    /// How Lightroom's Temp / Tint read in LightCraft for this camera ([`lightcraft_pipeline::WbMap`]),
    /// fitted from the user's edits after the profile (`tools/lr-compare` `wbmap`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wb_map: Option<WbMapFit>,
    /// Per lens (the files' `LensModel`): Lightroom's lens-profile vignetting correction for this
    /// camera, fitted from the user's photos with lens corrections on (`tools/lr-compare`
    /// `lensfit`). Used when the file carries no vignetting data of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lenses: Option<std::collections::BTreeMap<String, LensFit>>,
}

/// A lens's vignetting correction: EV added at radius r (half-diagonals from the centre) as
/// `ev[0] r² + ev[1] r⁴ + ev[2] r⁶`, and the photos it was fitted on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensFit {
    pub vignette_ev: [f64; 3],
    #[serde(default)]
    pub photos: usize,
}

impl LensFit {
    /// As a DNG `FixVignetteRadial` (gain `1 + k0 r² + … + k4 r¹⁰`) centred on a `w × h` image
    /// (r in half-diagonals); `None` when out of range.
    pub fn vignette(&self, w: f64, h: f64) -> Option<lightcraft_develop::EmbeddedVignette> {
        let long = w.max(h);
        if !(long >= 1.0 && w.min(h) >= 1.0) || !self.vignette_ev.iter().all(|v| v.is_finite() && v.abs() <= 4.0) {
            return None;
        }
        // least squares of the gain polynomial on r² ∈ [0, 1]
        let mut a = [[0.0f64; 5]; 5];
        let mut b = [0.0f64; 5];
        for i in 0..=40 {
            let t = i as f64 / 40.0;
            let g = (self.vignette_ev[0] * t + self.vignette_ev[1] * t * t + self.vignette_ev[2] * t * t * t).exp2() - 1.0;
            let f: [f64; 5] = std::array::from_fn(|j| t.powi(j as i32 + 1));
            for r in 0..5 {
                b[r] += f[r] * g;
                for c in 0..5 {
                    a[r][c] += f[r] * f[c];
                }
            }
        }
        for (r, row) in a.iter_mut().enumerate() {
            row[r] += 1e-9;
        }
        let k = solve5(a, b)?;
        Some(lightcraft_develop::EmbeddedVignette { k, center: lightcraft_geom::Point::new(0.5, 0.5), radius: (w * w + h * h).sqrt() / 2.0 / long })
    }
}

/// Gauss-Jordan with partial pivoting; `None` if singular or not finite.
fn solve5(a: [[f64; 5]; 5], b: [f64; 5]) -> Option<[f64; 5]> {
    let mut m = [[0.0f64; 6]; 5];
    for i in 0..5 {
        m[i][..5].copy_from_slice(&a[i]);
        m[i][5] = b[i];
    }
    for c in 0..5 {
        let p = (c..5).max_by(|x, y| m[*x][c].abs().total_cmp(&m[*y][c].abs()))?;
        if m[p][c].abs() < 1e-15 {
            return None;
        }
        m.swap(c, p);
        let d = m[c][c];
        for k in 0..6 {
            m[c][k] /= d;
        }
        for r in 0..5 {
            if r != c {
                let f = m[r][c];
                for k in 0..6 {
                    m[r][k] -= f * m[c][k];
                }
            }
        }
    }
    let x: [f64; 5] = std::array::from_fn(|i| m[i][5]);
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Coefficients of [`lightcraft_pipeline::WbMap`] and the photos they were fitted on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WbMapFit {
    pub mired: [f64; 3],
    pub tint: [f64; 3],
    #[serde(default)]
    pub photos: usize,
}

impl WbMapFit {
    pub fn map(&self) -> lightcraft_pipeline::WbMap {
        lightcraft_pipeline::WbMap { mired: self.mired.map(|v| v as f32), tint: self.tint.map(|v| v as f32) }
    }
}

/// Coefficients of [`lightcraft_pipeline::CameraWb`] and the photos they were fitted on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WbFit {
    pub r: [f64; 5],
    pub b: [f64; 5],
    pub samples: usize,
    /// RMS residual of the fit (ln units).
    pub rms: f64,
}

impl WbFit {
    /// The pipeline's model for a photo: `to_working` maps its white-balanced camera RGB to the
    /// working space, `shot` is its as-shot neutral.
    pub fn camera_wb(&self, to_working: &Mat3, shot: [f64; 3]) -> Option<lightcraft_pipeline::CameraWb> {
        let from = to_working.inverse()?;
        Some(lightcraft_pipeline::CameraWb {
            r: self.r.map(|v| v as f32),
            b: self.b.map(|v| v as f32),
            to_working: to_working.to_f32(),
            from_working: from.to_f32(),
            shot: shot.map(|v| v as f32),
        })
    }

    /// Least-squares fit of `(temp K, tint, camera neutral)` samples; `None` with too few or
    /// too uniform samples.
    pub fn fit(samples: &[(f64, f64, [f64; 3])]) -> Option<WbFit> {
        let rows: Vec<([f64; 5], f64, f64)> = samples
            .iter()
            .filter(|(k, t, n)| k.is_finite() && (1500.0..=50000.0).contains(k) && t.is_finite() && n.iter().all(|v| v.is_finite() && *v > 0.0))
            .map(|(k, t, n)| {
                let (m, t) = (1000.0 / k, t / 100.0);
                ([1.0, m, m * m, t, m * t], (n[0] / n[1]).ln(), (n[2] / n[1]).ln())
            })
            .collect();
        if rows.len() < 8 {
            return None;
        }
        let ms: Vec<f64> = rows.iter().map(|r| r.0[1]).collect();
        let (lo, hi) = ms.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
        if hi - lo < 0.02 {
            return None;
        }
        // normal equations with a small ridge (more on the curvature and tint terms, which a
        // narrow spread of as-shot whites can't pin down)
        let solve = |pick: fn(&([f64; 5], f64, f64)) -> f64| -> Option<[f64; 5]> {
            let mut a = [[0.0f64; 5]; 5];
            let mut b = [0.0f64; 5];
            for r in &rows {
                for i in 0..5 {
                    b[i] += r.0[i] * pick(r);
                    for j in 0..5 {
                        a[i][j] += r.0[i] * r.0[j];
                    }
                }
            }
            let n = rows.len() as f64;
            for (i, ridge) in [1e-6, 1e-4, 1e-2, 1e-2, 1e-2].iter().enumerate() {
                a[i][i] += ridge * n;
            }
            // Gauss-Jordan
            let mut m = [[0.0f64; 6]; 5];
            for i in 0..5 {
                m[i][..5].copy_from_slice(&a[i]);
                m[i][5] = b[i];
            }
            for c in 0..5 {
                let p = (c..5).max_by(|x, y| m[*x][c].abs().total_cmp(&m[*y][c].abs()))?;
                if m[p][c].abs() < 1e-12 {
                    return None;
                }
                m.swap(c, p);
                let d = m[c][c];
                for k in 0..6 {
                    m[c][k] /= d;
                }
                for r in 0..5 {
                    if r != c {
                        let f = m[r][c];
                        for k in 0..6 {
                            m[r][k] -= f * m[c][k];
                        }
                    }
                }
            }
            let x: [f64; 5] = std::array::from_fn(|i| m[i][5]);
            x.iter().all(|v| v.is_finite()).then_some(x)
        };
        let r = solve(|r| r.1)?;
        let b = solve(|r| r.2)?;
        let dot = |c: &[f64; 5], f: &[f64; 5]| c.iter().zip(f).map(|(a, b)| a * b).sum::<f64>();
        let rms =
            (rows.iter().map(|x| (dot(&r, &x.0) - x.1).powi(2) + (dot(&b, &x.0) - x.2).powi(2)).sum::<f64>() / (2.0 * rows.len() as f64)).sqrt();
        Some(WbFit { r, b, samples: rows.len(), rms })
    }
}

impl CameraProfile {
    pub fn new(model: &str, files: usize, samples: usize, matrix: Mat3, hue_sat: Option<HsvTable>) -> CameraProfile {
        CameraProfile {
            version: VERSION,
            model: model.to_owned(),
            files,
            samples,
            matrix: matrix.0,
            hue_sat,
            tone: None,
            source: None,
            wb: None,
            wb_map: None,
            lenses: None,
        }
    }

    pub fn matrix(&self) -> Mat3 {
        Mat3(self.matrix)
    }

    /// Whether the data is usable (bounded matrix, table shape matching its data).
    fn valid(&self) -> bool {
        let matrix = self.matrix.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0) && Mat3(self.matrix).inverse().is_some();
        let wb = self.wb.as_ref().is_none_or(|w| w.r.iter().chain(&w.b).all(|v| v.is_finite() && v.abs() < 1e3))
            && self.wb_map.as_ref().is_none_or(|m| m.mired.iter().chain(&m.tint).all(|v| v.is_finite() && v.abs() < 1e3));
        let table = self.hue_sat.as_ref().is_none_or(|t| {
            let dims = (1..=4096).contains(&t.hue_divisions) && (2..=4096).contains(&t.sat_divisions) && (1..=4096).contains(&t.val_divisions);
            let len = t.hue_divisions.checked_mul(t.sat_divisions).and_then(|n| n.checked_mul(t.val_divisions));
            dims && len == Some(t.data.len()) && t.data.iter().flatten().all(|v| v.is_finite()) && t.data.iter().all(|e| e[1] >= 0.0 && e[2] >= 0.0)
        });
        self.version == VERSION && !self.model.is_empty() && matrix && table && wb
    }
}

/// LightCraft's configuration folder (settings, GPU marker, camera profiles).
pub fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/LightCraft"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("LightCraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("lightcraft"))
    }
}

/// Where camera profiles are read from and written to.
pub fn dir() -> Option<PathBuf> {
    std::env::var_os("LIGHTCRAFT_CAMERA_PROFILES").map(PathBuf::from).or_else(|| config_dir().map(|d| d.join("camera-profiles")))
}

/// File name of `model`'s profile: letters, digits, `-` and `_` kept, anything else `_`.
pub fn file_name(model: &str) -> String {
    let name: String = model.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("{name}.json")
}

/// Read and validate a profile file.
pub fn load(path: &Path) -> Result<CameraProfile, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if size > MAX_FILE {
        return Err(format!("{}: {size} bytes is too large for a camera profile", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&bytes, &path.display().to_string())
}

/// Parse and validate a profile's JSON (`origin` names it in errors).
fn parse(json: &[u8], origin: &str) -> Result<CameraProfile, String> {
    let profile: CameraProfile = serde_json::from_slice(json).map_err(|e| format!("{origin}: {e}"))?;
    if !profile.valid() {
        return Err(format!("{origin}: not a usable camera profile (version {}, model {:?})", profile.version, profile.model));
    }
    Ok(profile)
}

/// The built-in profile for `model`, if any.
fn bundled(model: &str) -> Option<CameraProfile> {
    let (_, json) = BUNDLED.iter().find(|(m, _)| *m == model)?;
    parse(json.as_bytes(), &format!("built-in profile {model}")).inspect_err(|e| eprintln!("lightcraft: ignoring camera profile {e}")).ok()
}

/// Write `profile` to `dir` (created if needed) as `<model>.json`; returns the path.
pub fn save(profile: &CameraProfile, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(file_name(&profile.model));
    let json = serde_json::to_vec_pretty(profile).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn cache() -> &'static Mutex<HashMap<String, Option<Arc<CameraProfile>>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<CameraProfile>>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The profile for camera `model`: from [`dir`], else built in; read once per process.
pub fn get(model: &str) -> Option<Arc<CameraProfile>> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(hit) = cache.get(model) {
        return hit.clone();
    }
    let path = dir().map(|d| d.join(file_name(model)));
    let profile = match path {
        Some(path) if path.is_file() => match load(&path) {
            Ok(p) if p.model.trim() == model => Some(Arc::new(p)),
            Ok(p) => {
                eprintln!("lightcraft: ignoring camera profile {}: it is for {:?}, not {model:?}", path.display(), p.model);
                None
            }
            Err(e) => {
                eprintln!("lightcraft: ignoring camera profile {e}");
                None
            }
        },
        _ => None,
    };
    let profile = profile.or_else(|| bundled(model).map(Arc::new));
    cache.insert(model.to_owned(), profile.clone());
    profile
}

/// Changes when the profiles folder's contents change (per process): part of the render cache
/// keys, so thumbnails rendered before a profile existed are not reused after.
pub fn cache_key() -> u64 {
    static KEY: OnceLock<u64> = OnceLock::new();
    *KEY.get_or_init(|| {
        let entries = dir().and_then(|d| std::fs::read_dir(d).ok());
        let mut files: Vec<(String, u64, u64)> = entries
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
                Some((e.file_name().to_string_lossy().into_owned(), meta.len(), modified))
            })
            .collect();
        files.sort();
        let mut h = lightcraft_preview::Hasher128::new();
        for (name, len, modified) in &files {
            h.str(name).u64(*len).u64(*modified);
        }
        for (model, json) in BUNDLED {
            h.str(model).str(json);
        }
        h.finish().0 as u64
    })
}

type Pairs = Vec<([f64; 3], [f64; 3])>;

/// Per camera model: the photos read and their pooled colour pairs (and, for Lightroom
/// calibration, the pairs including highlights).
#[derive(Default)]
pub struct Pool {
    models: HashMap<String, (usize, Pairs)>,
    lightroom: HashMap<String, (usize, Pairs, Pairs)>,
    /// Per model: (Lightroom's as-shot temp, tint, the raw's as-shot camera neutral).
    wb: HashMap<String, Vec<(f64, f64, [f64; 3])>>,
    /// Per model: the files' own colorimetric transforms (white-balanced camera RGB → linear
    /// Rec.2020 at their as-shot white), for [`Pool::transfer`].
    colorimetric: HashMap<String, Vec<Mat3>>,
}

/// The as-shot camera neutral (green = 1) of a raw's header data.
pub(crate) fn shot_neutral(color: &lightcraft_raw::ColorData, wb_multipliers: Option<[f32; 3]>, xy: lightcraft_color::Xy) -> Option<[f64; 3]> {
    let n = match (color.as_shot_neutral, wb_multipliers) {
        (Some(n), _) => n,
        (None, Some(m)) if m.iter().all(|v| v.is_finite() && *v > 0.0) => m.map(|v| 1.0 / v as f64),
        _ => lightcraft_raw::color::wb_multipliers(color, xy).map(|v| 1.0 / v),
    };
    (n.iter().all(|v| v.is_finite() && *v > 0.0)).then(|| [n[0] / n[1], 1.0, n[2] / n[1]])
}

/// Most colour pairs kept per photo, so a few busy photos can't dominate a profile.
const PAIRS_PER_FILE: usize = 4000;

impl Pool {
    /// Add one raw file's colour pairs. `Ok(None)` when the file can't contribute (not an ARW,
    /// NEF or RAF without colour matrices, no usable camera JPEG, too little colour).
    pub fn add(&mut self, bytes: &[u8]) -> Result<Option<String>, String> {
        let mut raw = lightcraft_raw::decode(bytes).map_err(|e| e.to_string())?;
        // colour only: geometric lens corrections would stop the sensor proxy from binning
        raw.opcodes.list3.retain(|op| !op.is_lens_correction());
        let Some(model) = raw.metadata.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) else { return Ok(None) };
        let Some(pairs) = crate::camera_preview::profile_pairs(&raw, bytes) else { return Ok(None) };
        let step = pairs.len().div_ceil(PAIRS_PER_FILE).max(1);
        let entry = self.models.entry(model.to_owned()).or_default();
        entry.0 += 1;
        entry.1.extend(pairs.into_iter().step_by(step));
        Ok(Some(model.to_owned()))
    }

    /// Add one raw file against Lightroom's preview of it (`preview`: the JPEG Lightroom keeps in
    /// its previews cache, Adobe RGB, in the raw's stored orientation) rendered at Lightroom's
    /// default settings. `Ok(None)` when the pair can't contribute (frames differ, too little
    /// colour).
    pub fn add_lightroom(&mut self, raw_bytes: &[u8], preview: &[u8], as_shot: Option<(f64, f64)>) -> Result<Option<String>, String> {
        let mut raw = lightcraft_raw::decode(raw_bytes).map_err(|e| e.to_string())?;
        raw.opcodes.list3.retain(|op| !op.is_lens_correction());
        let Some(model) = raw.metadata.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) else { return Ok(None) };
        if let Some((k, t)) = as_shot
            && let Some(n) = shot_neutral(&raw.color, raw.wb_multipliers, lightcraft_raw::color::as_shot_white_xy(&raw))
        {
            self.wb.entry(model.to_owned()).or_default().push((k, t, n));
        }
        let opts = lightcraft_codecs::DecodeOptions { max_size: Some((1024, 1024)), max_pixels: 64_000_000 };
        let decoded =
            lightcraft_codecs::decode_jpeg_with_fallback(preview, opts, lightcraft_codecs::NamedSpace::AdobeRgb).map_err(|e| e.to_string())?;
        let Some((pairs, bright)) = crate::camera_preview::lightroom_pairs(&raw, decoded.to_working()) else { return Ok(None) };
        let step = pairs.len().div_ceil(PAIRS_PER_FILE).max(1);
        let bstep = bright.len().div_ceil(PAIRS_PER_FILE).max(1);
        let entry = self.lightroom.entry(model.to_owned()).or_default();
        entry.0 += 1;
        entry.1.extend(pairs.into_iter().step_by(step));
        entry.2.extend(bright.into_iter().step_by(bstep));
        Ok(Some(model.to_owned()))
    }

    /// Add a white-balance sample: a raw (read from its headers only) and the Temp / Tint
    /// Lightroom gave it as shot. `Ok(None)` when the file has no model name or as-shot white.
    pub fn add_wb(&mut self, raw_bytes: &[u8], temp: f64, tint: f64) -> Result<Option<String>, String> {
        let info = lightcraft_raw::probe_info(raw_bytes).map_err(|e| e.to_string())?;
        let Some(model) = info.metadata.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) else { return Ok(None) };
        let Some(n) = shot_neutral(&info.color, info.wb_multipliers, lightcraft_raw::color::as_shot_white_xy_of(&info)) else { return Ok(None) };
        self.wb.entry(model.to_owned()).or_default().push((temp, tint, n));
        Ok(Some(model.to_owned()))
    }

    /// Add a raw's own colorimetric transform (headers only) for [`Pool::transfer`]. `Ok(None)`
    /// when the file has no model name or no colour matrices of its own.
    pub fn add_colorimetric(&mut self, raw_bytes: &[u8]) -> Result<Option<String>, String> {
        let info = lightcraft_raw::probe_info(raw_bytes).map_err(|e| e.to_string())?;
        let Some(model) = info.metadata.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) else { return Ok(None) };
        let t = lightcraft_raw::color::camera_transform_of(&info.color, lightcraft_raw::color::as_shot_white_xy_of(&info));
        if t.matrix_is_fallback || !t.matrix.0.iter().flatten().all(|v| v.is_finite()) {
            // no matrices in the file (Fujifilm RAF): nothing colorimetric to carry a look onto
            // (a matrix fitted to the camera's JPEGs measured worse than keeping its profile)
            return Ok(None);
        }
        self.colorimetric.entry(model.to_owned()).or_default().push(t.matrix);
        Ok(Some(model.to_owned()))
    }

    /// The colorimetric transform of `model`: the mean of its files' own.
    fn colorimetric_of(&self, model: &str) -> Result<Mat3, String> {
        let v = self.colorimetric.get(model).filter(|v| !v.is_empty()).ok_or_else(|| format!("{model}: no raw with colour matrices of its own"))?;
        let mut acc = [[0.0; 3]; 3];
        for t in v {
            for (a, b) in acc.iter_mut().flatten().zip(t.0.iter().flatten()) {
                *a += b / v.len() as f64;
            }
        }
        Ok(Mat3(acc))
    }

    /// A Lightroom-matched profile for `model` (a camera with no photos at Lightroom's default
    /// settings to calibrate on) carried over from `from`, another camera's Lightroom-matched
    /// profile: Lightroom's default rendering is one look applied to each camera's colorimetric
    /// colour, so `from`'s look (its matrix relative to its camera's own colorimetric transform,
    /// its hue/saturation table and tone curve) is put on `model`'s colorimetric transform. Both
    /// models need [`Pool::add_colorimetric`] files; `model`'s white-balance model comes from its
    /// [`Pool::add_wb`] photos.
    pub fn transfer(&self, from: &CameraProfile, model: &str) -> Result<CameraProfile, String> {
        let mean = |m: &str| self.colorimetric_of(m);
        let src = mean(&from.model)?.inverse().ok_or_else(|| format!("{}: singular colour transform", from.model))?;
        let look = from.matrix().mul(&src);
        let matrix = look.mul(&mean(model)?);
        let mut p = CameraProfile::new(model, 0, 0, matrix, from.hue_sat.clone());
        p.tone = from.tone;
        p.source = Some(format!("lightroom-transfer:{}", from.model));
        p.wb = self.wb.get(model).and_then(|w| WbFit::fit(w));
        if !p.valid() {
            return Err(format!("{model}: transferred profile out of range"));
        }
        Ok(p)
    }

    /// Fit a Lightroom-matched profile (colour, tone and chroma) per model with at least
    /// `min_files` photos.
    pub fn fit_lightroom(&self, min_files: usize) -> Vec<Result<CameraProfile, String>> {
        let mut models: Vec<_> = self.lightroom.iter().collect();
        models.sort_by(|a, b| a.0.cmp(b.0));
        models
            .into_iter()
            .filter(|(_, (files, _, _))| *files >= min_files)
            .map(|(model, (files, pairs, bright))| {
                let (matrix, hue_sat, tone) =
                    crate::camera_preview::fit_lightroom_profile(pairs, bright).ok_or_else(|| format!("{model}: no usable colour fit"))?;
                let mut p = CameraProfile::new(model, *files, pairs.len(), matrix, hue_sat);
                p.tone = Some(tone);
                p.source = Some("lightroom".into());
                p.wb = self.wb.get(model).and_then(|w| WbFit::fit(w));
                Ok(p)
            })
            .collect()
    }

    /// Fit a profile per model with at least `min_files` photos.
    pub fn fit(&self, min_files: usize) -> Vec<Result<CameraProfile, String>> {
        let mut models: Vec<_> = self.models.iter().collect();
        models.sort_by(|a, b| a.0.cmp(b.0));
        models
            .into_iter()
            .filter(|(_, (files, _))| *files >= min_files)
            .map(|(model, (files, pairs))| {
                let (matrix, hue_sat) = crate::camera_preview::fit_profile(pairs).ok_or_else(|| format!("{model}: no usable colour fit"))?;
                Ok(CameraProfile::new(model, *files, pairs.len(), matrix, hue_sat))
            })
            .collect()
    }

    /// Add another pool's photos and pairs (pools filled on separate threads).
    pub fn merge(&mut self, other: Pool) {
        for (model, (files, pairs)) in other.models {
            let entry = self.models.entry(model).or_default();
            entry.0 += files;
            entry.1.extend(pairs);
        }
        for (model, w) in other.wb {
            self.wb.entry(model).or_default().extend(w);
        }
        for (model, t) in other.colorimetric {
            self.colorimetric.entry(model).or_default().extend(t);
        }
        for (model, (files, pairs, bright)) in other.lightroom {
            let entry = self.lightroom.entry(model).or_default();
            entry.0 += files;
            entry.1.extend(pairs);
            entry.2.extend(bright);
        }
    }

    /// Photos read per model.
    pub fn files(&self) -> Vec<(String, usize)> {
        let mut v: Vec<_> =
            self.models.iter().map(|(m, (n, _))| (m.clone(), *n)).chain(self.lightroom.iter().map(|(m, (n, _, _))| (m.clone(), *n))).collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> CameraProfile {
        let table = HsvTable { hue_divisions: 4, sat_divisions: 2, val_divisions: 1, data: vec![[5.0, 1.2, 1.0]; 8], srgb_value: false };
        CameraProfile::new("ILCE-7M4", 12, 3456, Mat3([[1.6, -0.5, -0.1], [-0.2, 1.3, -0.1], [-0.1, -0.2, 1.3]]), Some(table))
    }

    #[test]
    fn a_lens_fit_becomes_a_centred_radial_vignette() {
        let fit = LensFit { vignette_ev: [0.1, 1.9, -0.9], photos: 26 };
        let v = fit.vignette(6000.0, 4000.0).unwrap();
        assert!((v.radius - (6000f64.hypot(4000.0) / 2.0 / 6000.0)).abs() < 1e-12);
        for t in [0.0f64, 0.3, 0.7, 1.0] {
            let want = (0.1 * t + 1.9 * t * t - 0.9 * t * t * t).exp2();
            let got = 1.0 + v.k.iter().enumerate().map(|(j, k)| k * t.powi(j as i32 + 1)).sum::<f64>();
            assert!((got - want).abs() < 0.01, "r² {t}: {got} vs {want}");
        }
        // out of range or degenerate: no correction rather than a wild one
        assert!(LensFit { vignette_ev: [f64::NAN, 0.0, 0.0], photos: 1 }.vignette(10.0, 10.0).is_none());
        assert!(LensFit { vignette_ev: [9.0, 0.0, 0.0], photos: 1 }.vignette(10.0, 10.0).is_none());
        assert!(fit.vignette(0.0, 4000.0).is_none());
        // and it survives a profile file round trip
        let mut p = profile();
        p.lenses = Some([("EF24mm f/1.4L II USM".to_string(), fit.clone())].into_iter().collect());
        let back: CameraProfile = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.lenses.unwrap()["EF24mm f/1.4L II USM"], fit);
    }

    #[test]
    fn transfer_puts_the_look_on_the_other_cameras_colorimetric_transform() {
        let src = profile();
        let (ts, td) = (Mat3([[1.2, -0.1, -0.1], [-0.1, 1.1, 0.0], [0.0, -0.2, 1.2]]), Mat3([[0.9, 0.2, -0.1], [0.05, 0.9, 0.05], [0.0, 0.1, 0.9]]));
        let mut pool = Pool::default();
        pool.colorimetric.insert("ILCE-7M4".into(), vec![ts, ts]);
        pool.colorimetric.insert("GR".into(), vec![td]);
        pool.wb.insert(
            "GR".into(),
            (0..40).map(|i| (3000.0 + 100.0 * i as f64, (i % 7) as f64, [0.4 + 0.01 * i as f64, 1.0, 0.6 - 0.005 * i as f64])).collect(),
        );
        let p = pool.transfer(&src, "GR").unwrap();
        let want = src.matrix().mul(&ts.inverse().unwrap()).mul(&td);
        for (a, b) in p.matrix().0.iter().flatten().zip(want.0.iter().flatten()) {
            assert!((a - b).abs() < 1e-9);
        }
        // a colour seen through the source camera's transform renders the same on the target
        let x = [0.3, 0.5, 0.2];
        let (via_src, via_dst) = (src.matrix().apply(ts.inverse().unwrap().apply(x)), p.matrix().apply(td.inverse().unwrap().apply(x)));
        assert!(via_src.iter().zip(via_dst).all(|(a, b)| (a - b).abs() < 1e-9));
        assert_eq!((p.hue_sat.clone(), p.tone, p.model.as_str()), (src.hue_sat.clone(), src.tone, "GR"));
        assert_eq!(p.source.as_deref(), Some("lightroom-transfer:ILCE-7M4"));
        assert!(p.wb.is_some());
        // no colour matrices of its own: an error, never a panic or a guess
        assert!(pool.transfer(&src, "X-T2").is_err());
        let mut singular = Pool::default();
        singular.colorimetric.insert("ILCE-7M4".into(), vec![Mat3([[0.0; 3]; 3])]);
        singular.colorimetric.insert("GR".into(), vec![td]);
        assert!(singular.transfer(&src, "GR").is_err());
    }

    #[test]
    fn profiles_round_trip_and_reject_bad_files() {
        let dir = std::env::temp_dir().join(format!("lc-camera-profiles-{}", std::process::id()));
        let path = save(&profile(), &dir).unwrap();
        assert_eq!(path.file_name().unwrap(), "ILCE-7M4.json");
        assert_eq!(load(&path).unwrap(), profile());
        let bad = |p: CameraProfile| {
            let path = dir.join("bad.json");
            std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
            load(&path).is_err()
        };
        let mut p = profile();
        p.matrix[0][0] = f64::NAN;
        assert!(bad(p), "non-finite matrix");
        let mut p = profile();
        p.matrix = [[0.0; 3]; 3];
        assert!(bad(p), "singular matrix");
        let mut p = profile();
        if let Some(t) = p.hue_sat.as_mut() {
            t.data.truncate(3);
        }
        assert!(bad(p), "table data doesn't match its shape");
        // a Lightroom-matched profile keeps its tone curve
        let mut p = profile();
        p.tone = lightcraft_pipeline::tone::CameraTone::new(std::array::from_fn(|i| {
            let x = 2f32.powf(-10.0 + 10.5 * i as f32 / 31.0);
            [x, x / (1.0 + x)]
        }));
        p.source = Some("lightroom".into());
        let path2 = save(&p, &dir).unwrap();
        assert_eq!(load(&path2).unwrap(), p);
        let _ = std::fs::remove_file(path2);
        let mut p = profile();
        p.version = 99;
        assert!(bad(p), "unknown version");
        std::fs::write(dir.join("junk.json"), b"{not json").unwrap();
        assert!(load(&dir.join("junk.json")).is_err());
        assert!(load(&dir.join("missing.json")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundled_profiles_are_valid_and_named_after_their_model() {
        assert!(!BUNDLED.is_empty());
        for (model, json) in BUNDLED {
            let p = parse(json.as_bytes(), model).unwrap();
            assert_eq!(p.model, *model);
            assert!(p.files >= 5 && p.hue_sat.is_some(), "{model}: {} photos", p.files);
            assert_eq!(bundled(model), Some(p));
        }
        assert!(bundled("No Such Camera").is_none());
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("ILCE-7M4"), "ILCE-7M4.json");
        assert_eq!(file_name(" DSC-RX100M3 "), "DSC-RX100M3.json");
        assert_eq!(file_name("../../etc/passwd"), "______etc_passwd.json");
        assert_eq!(file_name("Ω 1/2"), "__1_2.json");
    }

    #[test]
    fn white_balance_fit_recovers_a_camera() {
        let (r, b) = ([-1.1, 1.4, -0.2, 0.05, 0.0], [0.2, -1.6, 0.3, 0.4, -0.1]);
        let f = |c: &[f64; 5], k: f64, t: f64| {
            let (m, t) = (1000.0 / k, t / 100.0);
            (c[0] + c[1] * m + c[2] * m * m + c[3] * t + c[4] * m * t).exp()
        };
        let samples: Vec<(f64, f64, [f64; 3])> = (0..40)
            .map(|i| {
                let k = 2800.0 + 150.0 * i as f64;
                let t = ((i * 7) % 30) as f64 - 10.0;
                (k, t, [f(&r, k, t), 1.0, f(&b, k, t)])
            })
            .collect();
        let fit = WbFit::fit(&samples).unwrap();
        assert!(fit.rms < 0.03, "{fit:?}");
        let cw = fit.camera_wb(&Mat3::IDENTITY, [f(&r, 5000.0, 5.0), 1.0, f(&b, 5000.0, 5.0)]).unwrap();
        let (k, t) = cw.temp_tint([f(&r, 5000.0, 5.0), 1.0, f(&b, 5000.0, 5.0)]).unwrap();
        assert!((k - 5000.0).abs() < 60.0 && (t - 5.0).abs() < 2.0, "{k} {t}");
        // too few / identical / hostile samples: no model
        assert!(WbFit::fit(&samples[..5]).is_none());
        assert!(WbFit::fit(&vec![(5000.0, 0.0, [0.5, 1.0, 0.8]); 50]).is_none());
        assert!(WbFit::fit(&vec![(f64::NAN, 0.0, [0.5, 1.0, 0.8]); 50]).is_none());
    }

    #[test]
    fn pool_skips_files_that_cannot_contribute() {
        let mut pool = Pool::default();
        assert!(pool.add(b"not a raw file").is_err());
        assert!(pool.add_lightroom(b"not a raw file", b"not a jpeg", Some((5000.0, 0.0))).is_err());
        assert!(pool.add_wb(b"not a raw file", 5000.0, 0.0).is_err());
        assert!(pool.fit_lightroom(1).is_empty());
        assert!(pool.fit(1).is_empty());
        assert!(pool.files().is_empty());
    }
}
