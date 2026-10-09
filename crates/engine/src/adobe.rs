//! Adobe's own camera profiles and looks, read at run time from the Camera Raw / Lightroom data
//! installed on this Mac with Adobe's DNG SDK (Bryan's fork, AGENTS.md → *Fork rules*).
//!
//! - **Camera base**: the camera's `Adobe Standard` DCP (`…/CameraRaw/CameraProfiles/Adobe
//!   Standard/<camera> Adobe Standard.dcp`). [`decode_color`] gives the raw decode its exact
//!   camera → working-space matrix (`dng_color_spec`: dual-illuminant interpolation, forward
//!   matrices), the hue/sat map interpolated for the white, the white balance as Temp / Tint
//!   (`dng_temperature`) and a white-balance model fitted to the SDK's neutrals; the profile's
//!   look table and tone curve (`ProfileToneCurve`, else the ACR3 default) are registered with
//!   [`lightcraft_pipeline::adobe`] for the per-pixel stage.
//! - **Looks** (`Adobe Color`, `Adobe Monochrome`, …: Camera Raw XMP "Look" profiles under
//!   `…/CameraRaw/Settings`): look table (decoded by the SDK), tone curves and hidden settings,
//!   registered as profile ids `adobe:<name>`.
//!
//! Nothing Adobe ships is copied into LightCraft or its repository. Without the SDK, without an
//! installed profile for the camera, or with a profile the SDK rejects, everything falls back to
//! LightCraft's own colour (a warning is logged once per camera), never a panic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use lightcraft_color::{D50, D65, Mat3, REC2020, bradford};
use lightcraft_dng_sdk_sys as sdk;
use lightcraft_raw::profile::HsvTable;

/// Bumped when what [`decode_color`] produces changes (part of the source cache key).
pub const VERSION: u64 = 7;

/// Largest look-profile XMP read (Adobe's are ≤ ~1 MB).
const MAX_XMP: u64 = 8 << 20;
/// Largest DCP read (Adobe's are ≤ ~2 MB).
const MAX_DCP: u64 = 16 << 20;

/// Camera Raw's data folders: `LIGHTCRAFT_ADOBE_CAMERARAW` (one folder; `none` = don't use
/// Adobe's data), else the system and the user folder.
pub fn camera_raw_dirs() -> Vec<PathBuf> {
    if let Some(v) = std::env::var_os("LIGHTCRAFT_ADOBE_CAMERARAW") {
        if v == "none" || v.is_empty() {
            return Vec::new();
        }
        return vec![PathBuf::from(v)];
    }
    let mut v = Vec::new();
    if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/Library/Application Support/Adobe/CameraRaw"));
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join("Library/Application Support/Adobe/CameraRaw"));
        }
    }
    v
}

/// Whether Adobe's base can be used at all here (SDK built in and Camera Raw data present).
pub fn enabled() -> bool {
    sdk::available() && camera_raw_dirs().iter().any(|d| d.join("CameraProfiles").is_dir())
}

/// A camera's Adobe Standard profile, parsed, with what the pipeline needs registered.
pub struct Camera {
    /// Key of the registered [`lightcraft_pipeline::adobe::Base`].
    pub key: u64,
    pub path: PathBuf,
    pub info: sdk::ProfileInfo,
    profile: sdk::Profile,
}

impl std::fmt::Debug for Camera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Camera").field("path", &self.path).field("model", &self.info.unique_model).finish()
    }
}

fn lower(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// `Adobe Standard` DCPs by lower-cased camera name (the file name before ` Adobe Standard.dcp`).
fn standard_index() -> &'static HashMap<String, PathBuf> {
    static I: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    I.get_or_init(|| {
        let mut m = HashMap::new();
        for d in camera_raw_dirs() {
            let dir = d.join("CameraProfiles/Adobe Standard");
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten().take(20_000) {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(cam) = name.strip_suffix(" Adobe Standard.dcp") {
                    // the user's folder (later) wins over the system's
                    m.insert(lower(cam), e.path());
                }
            }
        }
        m
    })
}

/// Names Camera Raw may file a camera under, from the file's Exif make and model.
fn candidates(make: Option<&str>, model: &str) -> Vec<String> {
    let model = model.trim();
    let mut v = vec![lower(model)];
    if let Some(make) = make.map(str::trim).filter(|m| !m.is_empty()) {
        let first = make.split(|c: char| c.is_whitespace() || c == ',').next().unwrap_or(make);
        let short: String = first.chars().enumerate().map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).collect();
        for m in [first.to_string(), short] {
            v.push(lower(&format!("{m} {model}")));
            if let Some(rest) = model.get(m.len()..).filter(|_| model.to_lowercase().starts_with(&m.to_lowercase())) {
                v.push(lower(&format!("{m} {}", rest.trim())));
            }
        }
    }
    v.dedup();
    v
}

/// The camera's Adobe Standard profile (`None`: no SDK, no installed profile, or unreadable).
/// Parsed once per camera per process.
pub fn standard_for(make: Option<&str>, model: &str) -> Option<Arc<Camera>> {
    if !sdk::available() || model.trim().is_empty() {
        return None;
    }
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<Camera>>>>> = OnceLock::new();
    let key = format!("{}\u{0}{}", make.unwrap_or(""), model);
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return hit.clone();
    }
    let found = candidates(make, model).iter().find_map(|c| standard_index().get(c).cloned()).and_then(|path| match load_camera(&path) {
        Ok(c) => Some(Arc::new(c)),
        Err(e) => {
            log::warn!("adobe: {}: {e}; using LightCraft's own colour for {model}", path.display());
            None
        }
    });
    cache.lock().unwrap_or_else(PoisonError::into_inner).insert(key, found.clone());
    found
}

fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > cap {
        return Err(format!("{len} bytes is too large"));
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

fn hsv_table(m: &sdk::HueSatMap) -> Option<HsvTable> {
    let data: Vec<f64> = m.deltas.iter().flat_map(|d| d.map(f64::from)).collect();
    HsvTable::from_tags(&[m.dims[0] as u64, m.dims[1] as u64, m.dims[2].max(1) as u64], &data, m.encoding as u64)
}

fn load_camera(path: &Path) -> Result<Camera, String> {
    let bytes = read_capped(path, MAX_DCP)?;
    let profile = sdk::Profile::parse(&bytes).map_err(|e| e.to_string())?;
    let info = profile.info().map_err(|e| e.to_string())?;
    let xs: Vec<f64> = (0..=lightcraft_pipeline::adobe::CURVE_N).map(|i| i as f64 / lightcraft_pipeline::adobe::CURVE_N as f64).collect();
    let (curve, _own) = profile.tone_curve(&xs).map_err(|e| e.to_string())?;
    let look = profile.look_table().map_err(|e| e.to_string())?.as_ref().and_then(hsv_table);
    let base = lightcraft_pipeline::adobe::Base {
        look,
        curve: curve.iter().map(|v| *v as f32).collect(),
        black: if info.default_black_render == 0 { black_level() } else { 0.0 },
    };
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut h);
        bytes.len().hash(&mut h);
        VERSION.hash(&mut h);
        h.finish()
    };
    if !lightcraft_pipeline::adobe::register_base(key, base) {
        return Err("the profile's tone curve is not usable".into());
    }
    register_standard_looks();
    Ok(Camera { key, path: path.to_path_buf(), info, profile })
}

/// The exposure ramp's black for `DefaultBlackRender` auto: 0 — Lightroom's PV2012 default tone,
/// measured with Camera Raw, is in `tone_adobe::BASE` instead of the DNG reference's Shadows 5;
/// `LIGHTCRAFT_ADOBE_BLACK` overrides (measurement).
fn black_level() -> f32 {
    std::env::var("LIGHTCRAFT_ADOBE_BLACK").ok().and_then(|v| v.parse::<f32>().ok()).filter(|v| (0.0..0.05).contains(v)).unwrap_or(0.0)
}

/// What the raw decode applies for a camera with an Adobe base.
#[derive(Clone, Debug)]
pub struct DecodeColor {
    /// White-balanced camera RGB (`wb ⊙ camera`) → linear Rec.2020 D65.
    pub matrix: Mat3,
    /// Per-channel multipliers (minimum 1), for highlight reconstruction and the matrix.
    pub wb: [f32; 3],
    /// The hue/sat map for the as-shot white (applied in linear ProPhoto, before exposure).
    pub hue_sat: Option<HsvTable>,
    /// EV gain after the hue/sat map: the file's baseline exposure + the profile's offset + the
    /// camera's measured offset ([`exposure_offset`]).
    pub gain_ev: f64,
    pub as_shot_temp: f64,
    pub as_shot_tint: f64,
    /// Temp / Tint → camera neutral, fitted to the SDK's colour spec.
    pub camera_wb: lightcraft_pipeline::CameraWb,
}

/// The as-shot camera neutral (raw RGB of white, green-normalised) of a raw file.
fn shot_neutral(raw: &lightcraft_raw::RawImage) -> Option<[f64; 3]> {
    if let Some(n) = raw.color.as_shot_neutral.filter(|n| n.iter().all(|v| v.is_finite() && *v > 0.0)) {
        return Some(n);
    }
    let m = raw.wb_multipliers.filter(|m| m.iter().all(|v| v.is_finite() && *v > 0.0))?;
    let n = [1.0 / m[0] as f64, 1.0 / m[1] as f64, 1.0 / m[2] as f64];
    Some(n.map(|v| v / n[1]))
}

/// D50 XYZ → linear Rec.2020 D65 (Bradford), as LightCraft's working space is built.
fn pcs_to_working() -> Mat3 {
    REC2020.from_xyz().mul(&bradford(D50, D65))
}

/// The colour the raw decode applies for `cam` (`None` → LightCraft's own path).
pub fn decode_color(cam: &Arc<Camera>, raw: &lightcraft_raw::RawImage) -> Option<DecodeColor> {
    let analog = raw.color.analog_balance.filter(|a| a.iter().all(|v| v.is_finite() && *v > 0.0));
    let white = match (shot_neutral(raw), raw.color.as_shot_white_xy) {
        (_, Some(xy)) => sdk::White::Xy(xy.x, xy.y),
        (Some(n), None) => sdk::White::Neutral(n),
        // no white balance recorded: Camera Raw falls back to D55-ish daylight
        (None, None) => sdk::White::Xy(0.3324, 0.3474),
    };
    let spec = match cam.profile.color_spec(white, analog) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("adobe: colour spec for {}: {e}", cam.info.unique_model);
            return None;
        }
    };
    let n = spec.camera_white;
    if !n.iter().all(|v| v.is_finite() && *v > 0.0) {
        return None;
    }
    let nmax = n.iter().copied().fold(0.0f64, f64::max);
    let wb = n.map(|v| (nmax / v) as f32);
    let c2p = Mat3(spec.camera_to_pcs);
    let matrix = pcs_to_working().mul(&c2p).mul(&Mat3::diag(1.0 / wb[0] as f64, 1.0 / wb[1] as f64, 1.0 / wb[2] as f64));
    let hue_sat = cam.profile.hue_sat_map(spec.white_xy).ok().flatten().as_ref().and_then(hsv_table);
    let [temp, tint] = sdk::xy_to_temp_tint(spec.white_xy[0], spec.white_xy[1]).ok()?;
    // the file's baseline exposure (DNG tag; Fujifilm's raw exposure bias), the profile's offset
    // (a DNG carries its own baseline exposure: the measured offsets are for the camera's own format)
    let measured = if raw.format == lightcraft_raw::RawFormat::Dng { 0.0 } else { exposure_offset(&cam.info.unique_model, raw.metadata.iso) };
    let gain_ev = raw.color.baseline_exposure + cam.info.baseline_exposure_offset + measured;
    let camera_wb = wb_model(cam, analog, n, &matrix)?;
    Some(DecodeColor { matrix, wb, hue_sat, gain_ev, as_shot_temp: temp, as_shot_tint: tint, camera_wb })
}

/// The SDK's camera neutral (green = 1) for Temp / Tint.
pub fn neutral_for(cam: &Camera, analog: Option<[f64; 3]>, temp: f64, tint: f64) -> Option<[f64; 3]> {
    let xy = sdk::temp_tint_to_xy(temp, tint).ok()?;
    let n = cam.profile.color_spec(sdk::White::Xy(xy[0], xy[1]), analog).ok()?.camera_white;
    (n[1] > 0.0 && n.iter().all(|v| v.is_finite() && *v > 0.0)).then(|| n.map(|v| v / n[1]))
}

/// How far Lightroom's Temp / Tint move the white, as a power of the SDK's camera-neutral change
/// from the as-shot white (measured with Camera Raw on real raws and synthetic DNGs: most cameras
/// move twice as far as the DNG colour spec says; the Leica M (Typ 262)'s own DNGs move as specified).
/// Fitted per camera on split A of Bryan's edits (`docs/lr-match.md`, round 4).
pub const WB_STRENGTH: &[(&str, f64)] = &[("LEICA M (Typ 262)", 1.0)];
pub const WB_STRENGTH_DEFAULT: f64 = 2.0;

pub fn wb_strength(model: &str) -> f64 {
    WB_STRENGTH.iter().find(|(m, _)| *m == model).map_or(WB_STRENGTH_DEFAULT, |e| e.1)
}

/// The neutral Lightroom renders for the SDK's neutral `n` (green = 1), as-shot `sh` (green = 1).
fn strengthen(n: [f64; 3], sh: [f64; 3], k: f64) -> [f64; 3] {
    let e: [f64; 3] = std::array::from_fn(|i| sh[i] * (n[i] / sh[i]).powf(k));
    if e.iter().all(|v| v.is_finite() && *v > 0.0) { e.map(|v| v / e[1]) } else { n }
}

/// [`lightcraft_pipeline::CameraWb`] from the SDK: exact neutrals through a registered function
/// (the colour spec for each Temp / Tint), plus the polynomial (least squares in log space) as a
/// fallback; `shot` = the as-shot neutral exactly.
fn wb_model(cam: &Arc<Camera>, analog: Option<[f64; 3]>, shot: [f64; 3], matrix: &Mat3) -> Option<lightcraft_pipeline::CameraWb> {
    let k = wb_strength(&cam.info.unique_model);
    let sh = shot.map(|v| v / shot[1]);
    let mut a = [[0.0f64; 5]; 5];
    let (mut br, mut bb) = ([0.0f64; 5], [0.0f64; 5]);
    for mi in 0..=24 {
        let temp = 1e6 / (33.0 + (500.0 - 33.0) * mi as f64 / 24.0);
        for ti in -5..=5 {
            let tint = ti as f64 * 20.0;
            let Some(n) = neutral_for(cam, analog, temp, tint).map(|n| strengthen(n, sh, k)) else { continue };
            let (yr, yb) = (n[0].ln(), n[2].ln());
            let (m, t) = (1000.0 / temp, tint / 100.0);
            let f = [1.0, m, m * m, t, m * t];
            for r in 0..5 {
                br[r] += f[r] * yr;
                bb[r] += f[r] * yb;
                for c in 0..5 {
                    a[r][c] += f[r] * f[c];
                }
            }
        }
    }
    for (r, row) in a.iter_mut().enumerate() {
        row[r] += 1e-9;
    }
    let cr = solve5(a, br)?;
    let cb = solve5(a, bb)?;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        cam.key.hash(&mut h);
        analog.map(|a| a.map(f64::to_bits)).hash(&mut h);
        h.finish()
    };
    // decoded colour = `matrix` · (wb ⊙ camera) = R · C2P(shot) · camera: back to camera RGB, then
    // the profile's camera → PCS for the new white
    let shot_inverse =
        (pcs_to_working().mul(&cam.profile.color_spec(sdk::White::Neutral(shot), analog).ok().map(|s| Mat3(s.camera_to_pcs))?)).inverse()?;
    let (c1, c2) = (Arc::clone(cam), Arc::clone(cam));
    lightcraft_pipeline::adobe::register_wb(
        key,
        lightcraft_pipeline::adobe::WbFns {
            neutral: Box::new(move |t, ti| neutral_for(&c1, analog, t, ti).map(|n| strengthen(n, sh, k))),
            matrix: Box::new(move |t, ti| {
                let n = strengthen(neutral_for(&c2, analog, t, ti)?, sh, k);
                let spec = c2.profile.color_spec(sdk::White::Neutral(n), analog).ok()?;
                Some(pcs_to_working().mul(&Mat3(spec.camera_to_pcs)).mul(&shot_inverse).0)
            }),
        },
    );
    // white-balanced camera RGB (as shot) → working, and back
    let from = matrix.inverse()?;
    let s = shot.map(|v| v / shot[1]);
    Some(lightcraft_pipeline::CameraWb {
        r: cr.map(|v| v as f32),
        b: cb.map(|v| v as f32),
        to_working: matrix.to_f32(),
        from_working: from.to_f32(),
        shot: s.map(|v| v as f32),
        exact: Some(key),
    })
}

/// Gauss-Jordan with partial pivoting; `None` if singular or not finite.
pub(crate) fn solve5(a: [[f64; 5]; 5], b: [f64; 5]) -> Option<[f64; 5]> {
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

/// Measured exposure offsets (EV) per camera from an ISO up (`(model, from ISO, EV)`): Camera
/// Raw's own per-camera baseline exposure and white levels live in its binaries, so they were
/// measured against Lightroom's renders of Bryan's photos (`tools/lr-compare`, `docs/lr-match.md`
/// → Round 3). Leica M (Typ 262) is the reference (its DNGs carry their baseline exposure).
pub const MEASURED_EXPOSURE: &[(&str, u32, f64)] = &[
    ("Canon EOS R6", 0, 0.127),
    ("Canon EOS R6", 125, 0.216),
    ("Fujifilm X-T2", 0, -0.533),
    ("Fujifilm X-T2", 500, -1.062),
    ("Fujifilm X100F", 0, -0.37),
];

/// A camera's exposure offset (EV) at `iso`: `adobe-exposure.json` in LightCraft's config folder
/// (`{"Canon EOS R6": 0.3}`, or by ISO `{"Canon EOS R6": {"0": 0.19, "125": 0.3}}` — each value
/// from that ISO up; `LIGHTCRAFT_ADOBE_EXPOSURE` names another file) over
/// [`MEASURED_EXPOSURE`]; 0 when neither knows the camera.
pub fn exposure_offset(unique_model: &str, iso: Option<u32>) -> f64 {
    use std::collections::BTreeMap;
    static M: OnceLock<HashMap<String, BTreeMap<u32, f64>>> = OnceLock::new();
    let m = M.get_or_init(|| {
        let mut m: HashMap<String, BTreeMap<u32, f64>> = HashMap::new();
        for (model, from, ev) in MEASURED_EXPOSURE {
            m.entry(lower(model)).or_default().insert(*from, *ev);
        }
        let path = std::env::var_os("LIGHTCRAFT_ADOBE_EXPOSURE")
            .map(PathBuf::from)
            .or_else(|| crate::camera_profiles::config_dir().map(|d| d.join("adobe-exposure.json")));
        let Some(path) = path else { return m };
        let Ok(bytes) = read_capped(&path, 1 << 20) else { return m };
        let ok = |v: f64| v.is_finite() && v.abs() <= 3.0;
        match serde_json::from_slice::<HashMap<String, serde_json::Value>>(&bytes) {
            Ok(file) => {
                for (k, v) in file {
                    let mut by = BTreeMap::new();
                    match v {
                        serde_json::Value::Number(n) => {
                            if let Some(v) = n.as_f64().filter(|v| ok(*v)) {
                                by.insert(0, v);
                            }
                        }
                        serde_json::Value::Object(o) => {
                            for (ik, iv) in o {
                                if let (Ok(from), Some(v)) = (ik.trim().parse::<u32>(), iv.as_f64().filter(|v| ok(*v))) {
                                    by.insert(from, v);
                                }
                            }
                        }
                        _ => {}
                    }
                    if !by.is_empty() {
                        m.insert(lower(&k), by);
                    }
                }
            }
            Err(e) => log::warn!("adobe: {}: {e}", path.display()),
        }
        m
    });
    let Some(by) = m.get(&lower(unique_model)) else { return 0.0 };
    by.range(..=iso.unwrap_or(0)).next_back().or_else(|| by.iter().next()).map_or(0.0, |(_, v)| *v)
}

// ------------------------------------------------------------------------------------- looks

/// The Adobe Raw looks (`…/Settings/Adobe/Profiles/Adobe Raw/<name>.xmp`) registered whenever a
/// camera base is.
pub const ADOBE_RAW_LOOKS: &[&str] = &["Adobe Color", "Adobe Monochrome", "Adobe Neutral", "Adobe Portrait", "Adobe Landscape", "Adobe Vivid"];

fn register_standard_looks() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        for name in ADOBE_RAW_LOOKS {
            if let Err(e) = register_look(name) {
                log::warn!("adobe: look {name}: {e}");
            }
        }
    });
}

/// Find the look profile `name` among Camera Raw's settings folders.
pub fn find_look(name: &str) -> Option<PathBuf> {
    for d in camera_raw_dirs() {
        let p = d.join("Settings/Adobe/Profiles/Adobe Raw").join(format!("{name}.xmp"));
        if p.is_file() {
            return Some(p);
        }
    }
    // a bounded walk of every settings folder for a Look profile of that name
    let needle = format!(">{name}<");
    for d in camera_raw_dirs() {
        let mut stack = vec![(d.join("Settings"), 0usize)];
        let mut seen = 0usize;
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                seen += 1;
                if seen > 20_000 {
                    break;
                }
                let p = e.path();
                if p.is_dir() && depth < 6 {
                    stack.push((p, depth + 1));
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xmp")) {
                    let Ok(bytes) = read_capped(&p, MAX_XMP) else { continue };
                    let text = String::from_utf8_lossy(&bytes);
                    if text.contains("PresetType=\"Look\"") && text.contains(&needle) {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

/// Read look profile `name` and register it as `adobe:<name>`; returns the id.
pub fn register_look(name: &str) -> Result<String, String> {
    let path = find_look(name).ok_or("not installed")?;
    let bytes = read_capped(&path, MAX_XMP)?;
    let look = parse_look(&String::from_utf8_lossy(&bytes))?;
    let id = format!("adobe:{name}");
    lightcraft_pipeline::adobe::register_look(&id, look);
    Ok(id)
}

/// A Camera Raw look profile (XMP) as a pipeline look.
pub fn parse_look(xmp: &str) -> Result<lightcraft_pipeline::adobe::Look, String> {
    let props = lightcraft_meta::parse_xmp(xmp).map_err(|e| e.to_string())?.properties;
    let get = |k: &str| props.get(k).and_then(|v| v.first()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if get("crs:PresetType").as_deref() != Some("Look") {
        return Err("not a look profile".into());
    }
    let name = get("crs:Name").unwrap_or_default();
    let (table, amount_range) = match get("crs:LookTable") {
        Some(digest) => {
            let text = get(&format!("crs:Table_{digest}")).ok_or_else(|| format!("look table {digest} is referenced but not included"))?;
            let t = sdk::decode_look_table(&text).map_err(|e| format!("look table: {e}"))?;
            (hsv_table(&t.map), [t.amount_range[0] as f32, t.amount_range[1] as f32])
        }
        None => (None, [0.0, 2.0]),
    };
    let supports_amount = get("crs:SupportsAmount").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let grayscale = get("crs:ConvertToGrayscale").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let partial = crate::crs::to_partial(&props, Some(true));
    let curve = |k: &str| -> Vec<lightcraft_geom::Point> {
        partial["curve"][k]
            .as_array()
            .map(|a| a.iter().filter_map(|p| Some(lightcraft_geom::Point::new(p["x"].as_f64()?, p["y"].as_f64()?))).collect())
            .unwrap_or_default()
    };
    let curves = [curve("master"), curve("red"), curve("green"), curve("blue")];
    let mut deltas = Vec::new();
    for (section, keys) in [
        ("light", &["exposure", "contrast", "highlights", "shadows", "whites", "blacks"][..]),
        ("effects", &["clarity", "texture", "dehaze"][..]),
        ("color", &["vibrance", "saturation"][..]),
    ] {
        for k in keys {
            if let Some(v) = partial[section][*k].as_f64().filter(|v| v.is_finite() && *v != 0.0) {
                deltas.push((format!("{section}.{k}"), v));
            }
        }
    }
    Ok(lightcraft_pipeline::adobe::Look { name, table, amount_range, supports_amount, curves, deltas, grayscale })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_names_follow_camera_raw() {
        assert!(candidates(Some("FUJIFILM"), "X-T2").contains(&"fujifilm x-t2".to_string()));
        assert!(candidates(Some("Canon"), "Canon EOS R6").contains(&"canon eos r6".to_string()));
        assert!(candidates(Some("NIKON CORPORATION"), "NIKON Z 6").contains(&"nikon z 6".to_string()));
        assert!(candidates(Some("SONY"), "ILCE-7M4").contains(&"sony ilce-7m4".to_string()));
        assert!(candidates(Some("LEICA CAMERA AG"), "LEICA M (Typ 262)").contains(&"leica m (typ 262)".to_string()));
        assert!(candidates(None, "  ").iter().all(|c| c.is_empty()));
    }

    #[test]
    fn hostile_look_profiles_are_errors() {
        assert!(parse_look("").is_err());
        assert!(parse_look("<x:xmpmeta>garbage").is_err());
        let not_look = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:PresetType="Normal"/></rdf:RDF></x:xmpmeta>"#;
        assert!(parse_look(not_look).is_err());
        let missing = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:PresetType="Look" crs:LookTable="ABCD"/></rdf:RDF></x:xmpmeta>"#;
        assert!(parse_look(missing).is_err());
        let bad_table = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:PresetType="Look" crs:LookTable="ABCD" crs:Table_ABCD="!!!!"/></rdf:RDF></x:xmpmeta>"#;
        assert!(parse_look(bad_table).is_err());
    }

    /// The installed Adobe data on this Mac (skipped where it isn't there).
    fn r6() -> Option<Arc<Camera>> {
        enabled().then(|| standard_for(Some("Canon"), "Canon EOS R6")).flatten()
    }

    #[test]
    fn installed_adobe_standard_profile_resolves_and_its_wb_model_tracks_the_sdk() {
        let Some(cam) = r6() else { return };
        assert_eq!(cam.info.unique_model, "Canon EOS R6");
        let base = lightcraft_pipeline::adobe::base(cam.key).expect("registered");
        assert!(base.valid());
        // the ACR3 default curve: grey 0.18 → ~0.39
        assert!((base.curve_at(0.18) - 0.388).abs() < 0.01, "{}", base.curve_at(0.18));
        // the fitted white-balance model stays within 1.5 % of the SDK's neutrals (moved by the
        // camera's measured Lightroom strength)
        let shot = neutral_for(&cam, None, 5200.0, 5.0).unwrap();
        let m = wb_model(&cam, None, shot, &Mat3::IDENTITY).unwrap();
        assert_eq!(m.neutral(5200.0, 5.0).map(|v| (v * 1e6).round()), shot.map(|v| (v * 1e6).round()));
        for (t, ti) in [(2600.0, 0.0), (3200.0, 10.0), (4500.0, -15.0), (5500.0, 5.0), (6500.0, 20.0), (9000.0, -30.0), (15000.0, 0.0)] {
            let sdk_n = strengthen(neutral_for(&cam, None, t, ti).unwrap(), shot, wb_strength("Canon EOS R6"));
            let ours = m.neutral(t, ti);
            for c in [0, 2] {
                let e = (ours[c] / sdk_n[c]).ln().abs();
                assert!(e < 0.004, "{t} K / {ti}: channel {c} {ours:?} vs {sdk_n:?}");
            }
        }
        // the Adobe Raw looks registered with it
        let color = lightcraft_pipeline::adobe::look("adobe:Adobe Color").expect("Adobe Color");
        assert!(color.table.is_some() && !color.supports_amount);
        assert!(!color.curves[0].is_empty());
        let mono = lightcraft_pipeline::adobe::look("adobe:Adobe Monochrome").expect("Adobe Monochrome");
        assert!(mono.grayscale && mono.deltas.iter().any(|(k, v)| k == "effects.clarity" && (*v - 8.0).abs() < 1e-9));
    }

    #[test]
    fn our_hue_sat_lookup_matches_the_sdks_reference() {
        let Some(cam) = r6() else { return };
        let map = cam.profile.hue_sat_map([0.3457, 0.3585]).unwrap().unwrap();
        let ours = hsv_table(&map).unwrap();
        let applier = sdk::HsmApplier::new(&map).unwrap();
        let look = cam.profile.look_table().unwrap().unwrap();
        let ours_look = hsv_table(&look).unwrap();
        let look_applier = sdk::HsmApplier::new(&look).unwrap();
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let px: Vec<[f32; 3]> = (0..2000).map(|_| [rnd(), rnd(), rnd()].map(|v| v * 0.98 + 0.01)).collect();
        for (o, a) in [(&ours, &applier), (&ours_look, &look_applier)] {
            let (mut r, mut g, mut b): (Vec<f32>, Vec<f32>, Vec<f32>) =
                (px.iter().map(|p| p[0]).collect(), px.iter().map(|p| p[1]).collect(), px.iter().map(|p| p[2]).collect());
            a.apply(&mut r, &mut g, &mut b, false).unwrap();
            let mut worst = 0.0f32;
            for (i, p) in px.iter().enumerate() {
                let q = o.apply(*p);
                worst = worst.max((q[0] - r[i]).abs()).max((q[1] - g[i]).abs()).max((q[2] - b[i]).abs());
            }
            assert!(worst < 2e-4, "max difference {worst}");
        }
    }
}

#[cfg(test)]
mod rgb_table_tests {
    /// LightCraft's own decode + application of a creative profile's RGB table against the DNG
    /// SDK's (`dng_rgb_to_rgb_table_data`), on the user's installed Summer Fields profile.
    #[test]
    fn creative_rgb_table_matches_the_sdk() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let path = std::path::PathBuf::from(home).join("Library/Application Support/Adobe/CameraRaw/ImportedSettings/Summer Fields.xmp");
        let (true, Ok(xmp)) = (lightcraft_dng_sdk_sys::available(), std::fs::read_to_string(&path)) else { return };
        let props = lightcraft_meta::parse_xmp(&xmp).unwrap().properties;
        let get = |k: &str| props.get(k).and_then(|v| v.first()).cloned();
        let digest = get("crs:Look/crs:Parameters/crs:RGBTable").or_else(|| get("crs:RGBTable")).unwrap();
        let text = get(&format!("crs:Look/crs:Parameters/crs:Table_{digest}")).or_else(|| get(&format!("crs:Table_{digest}"))).unwrap();
        let sdk = lightcraft_dng_sdk_sys::RgbTable::decode(&text).unwrap();
        eprintln!("info {:?} range {:?}", sdk.info, sdk.amount_range);
        let ours = crate::crs_table::profile_from_xmp(&xmp).unwrap().unwrap().lut;
        let m = ours.matrices();
        let (to_pp, from_pp) = lightcraft_pipeline::adobe::prophoto_matrices();
        let mul = |m: &[[f32; 3]; 3], v: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]) };
        let mut seed = 777u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        // colours inside the table's gamut (sRGB), as linear ProPhoto
        let srgb_to_pp: [[f32; 3]; 3] = lightcraft_color::SRGB.to_space(&lightcraft_color::PROPHOTO).0.map(|r| r.map(|v| v as f32));
        let px: Vec<[f32; 3]> = (0..3000).map(|_| mul(&srgb_to_pp, [rnd(), rnd(), rnd()].map(|v| v * v))).collect();
        for amount in [0.5f64, 1.0] {
            let (mut r, mut g, mut b): (Vec<f32>, Vec<f32>, Vec<f32>) =
                (px.iter().map(|p| p[0]).collect(), px.iter().map(|p| p[1]).collect(), px.iter().map(|p| p[2]).collect());
            sdk.apply(amount, &mut r, &mut g, &mut b, false).unwrap();
            let (mut worst, mut sum) = (0.0f32, 0.0f32);
            for (i, p) in px.iter().enumerate() {
                let q = mul(&to_pp, ours.apply_linear(mul(&from_pp, *p), amount as f32, &m));
                let d = (q[0] - r[i]).abs().max((q[1] - g[i]).abs()).max((q[2] - b[i]).abs());
                worst = worst.max(d);
                sum += d;
            }
            eprintln!("amount {amount}: max {worst:.5} mean {:.6}", sum / px.len() as f32);
            assert!(worst < 0.01, "amount {amount}: max difference {worst}");
        }
    }
}

#[cfg(test)]
mod exposure_tests {
    #[test]
    fn measured_offsets_follow_iso_ranges() {
        if std::env::var_os("LIGHTCRAFT_ADOBE_EXPOSURE").is_some() {
            return;
        }
        let r6 = |iso| super::exposure_offset("Canon EOS R6", Some(iso));
        assert_eq!(r6(100), 0.127);
        assert_eq!(r6(125), 0.216);
        assert_eq!(r6(6400), 0.216);
        assert_eq!(super::exposure_offset("canon eos r6", None), 0.127);
        assert_eq!(super::exposure_offset("Fujifilm X-T2", Some(1250)), -1.062);
        assert_eq!(super::exposure_offset("Some Camera", Some(100)), 0.0);
    }
}
