//! Adobe camera-profile rendering (Bryan's fork, AGENTS.md → *Fork rules*).
//!
//! The engine reads the Adobe profiles installed on the Mac with Adobe's DNG SDK and registers
//! here what the per-pixel stage needs:
//! - a camera **base** ([`Base`], keyed by [`SourceInfo::adobe`](crate::SourceInfo)): the camera
//!   profile's own look table (`ProfileLookTableData` of the `.dcp`), its tone curve
//!   (`ProfileToneCurve`, else Camera Raw's ACR3 default) and the exposure ramp's black level
//!   (`DefaultBlackRender`); the matrices and hue/sat map are applied when the raw is decoded;
//! - **looks** ([`Look`], keyed by profile id `adobe:<name>`, e.g. `adobe:Adobe Color`): the look
//!   profile's own look table, tone curve and hidden slider settings (added to the user's).
//!
//! The per-pixel order follows the DNG SDK's reference renderer (`dng_render`): linear ProPhoto,
//! exposure ramp, look tables, then the tone curve applied as `RefBaselineRGBTone` (on the
//! largest and smallest channel, the middle one interpolated: hue-preserving). Without a
//! registered base everything falls back to LightCraft's own camera tone.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use lightcraft_color::{D50, D65, Mat3, PROPHOTO, REC2020, bradford};
use lightcraft_geom::Point;
use lightcraft_raw::profile::HsvTable;

/// Samples of a base tone curve over 0..=1 (`CURVE_N + 1` values).
pub const CURVE_N: usize = 1024;

/// A camera profile's rendering after the raw decode.
#[derive(Clone, Debug, PartialEq)]
pub struct Base {
    /// The profile's look table (applied after the exposure ramp, before the tone curve).
    pub look: Option<HsvTable>,
    /// The tone curve sampled at `i / CURVE_N` (linear in, linear out, 0..=1).
    pub curve: Vec<f32>,
    /// Exposure-ramp black level (linear, 0..0.1): Camera Raw's default "Shadows 5" = 0.005 for
    /// `DefaultBlackRender` auto, 0 for none.
    pub black: f32,
}

impl Base {
    /// The tone curve at `x` (clamped to 0..=1, linear interpolation of the samples).
    #[inline]
    pub fn curve_at(&self, x: f32) -> f32 {
        curve_at(&self.curve, x)
    }

    /// Whether the base is usable (a full curve, finite and non-decreasing).
    pub fn valid(&self) -> bool {
        self.curve.len() == CURVE_N + 1
            && self.curve.iter().all(|v| v.is_finite())
            && self.curve.windows(2).all(|w| w[1] >= w[0])
            && (0.0..0.1).contains(&self.black)
    }
}

/// `curve` (samples over 0..=1) at `x`.
#[inline]
pub fn curve_at(curve: &[f32], x: f32) -> f32 {
    let n = curve.len();
    if n < 2 || !x.is_finite() {
        return 0.0;
    }
    let f = x.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (f as usize).min(n - 2);
    let t = f - i as f32;
    let (a, b) = (curve.get(i).copied().unwrap_or(0.0), curve.get(i + 1).copied().unwrap_or(1.0));
    a + (b - a) * t
}

/// A look profile (`adobe:<name>`): its look table, curve and the settings it adds.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Look {
    pub name: String,
    /// The look table and what Amount 0 / 2 mean as table strengths.
    pub table: Option<HsvTable>,
    pub amount_range: [f32; 2],
    /// The Amount slider scales the look (Adobe Color and the other Adobe Raw looks: no).
    pub supports_amount: bool,
    /// `crs:ToneCurvePV2012` (master, red, green, blue; empty = identity), composed under the
    /// user's own curves.
    pub curves: [Vec<Point>; 4],
    /// Hidden slider settings added to the user's: (settings path, value), e.g.
    /// `("light.highlights", -12.0)`.
    pub deltas: Vec<(String, f64)>,
    pub grayscale: bool,
}

impl Look {
    /// The look-table strength for profile amount `amount` (0..=2, 1 = 100 %).
    pub fn strength(&self, amount: f64) -> f32 {
        if !self.supports_amount {
            return 1.0;
        }
        let a = amount.clamp(0.0, 2.0) as f32;
        let [lo, hi] = self.amount_range;
        if a <= 1.0 { lo + (1.0 - lo) * a } else { 1.0 + (hi - 1.0) * (a - 1.0) }
    }
}

type Registry<K, V> = RwLock<HashMap<K, Arc<V>>>;

fn bases() -> &'static Registry<u64, Base> {
    static R: OnceLock<Registry<u64, Base>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn looks() -> &'static Registry<String, Look> {
    static R: OnceLock<Registry<String, Look>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Make a camera base available under `key` (an invalid one is not registered).
pub fn register_base(key: u64, base: Base) -> bool {
    if !base.valid() {
        return false;
    }
    bases().write().unwrap_or_else(|e| e.into_inner()).insert(key, Arc::new(base));
    true
}

pub fn base(key: u64) -> Option<Arc<Base>> {
    bases().read().unwrap_or_else(|e| e.into_inner()).get(&key).cloned()
}

/// Profile ids of Adobe look profiles.
pub fn is_adobe_look(id: &str) -> bool {
    id.starts_with("adobe:")
}

/// Make a look available under its profile id (`adobe:<name>`).
pub fn register_look(id: &str, look: Look) {
    if is_adobe_look(id) {
        looks().write().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), Arc::new(look));
    }
}

pub fn look(id: &str) -> Option<Arc<Look>> {
    if !is_adobe_look(id) {
        return None;
    }
    looks().read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}

/// The look an Adobe base renders profile `id` with: our default (`lc.color`, Lightroom's
/// default for raw files being Adobe Color) is Adobe Color; `adobe:<name>` that look.
pub fn look_for(id: &str) -> Option<Arc<Look>> {
    match id {
        "" | "lc.color" => look("adobe:Adobe Color"),
        id => look(id),
    }
}

/// Ids of the registered looks, sorted.
pub fn look_ids() -> Vec<String> {
    let mut v: Vec<String> = looks().read().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect();
    v.sort();
    v
}

/// A camera's white balance from Adobe's colour spec: for Temp (K) / Tint, the camera neutral
/// (green = 1) and the matrix taking the decoded (as-shot) working colour to the colour the
/// camera profile gives at that white (linear Rec.2020, row-major).
pub struct WbFns {
    pub neutral: Box<dyn Fn(f64, f64) -> Option<[f64; 3]> + Send + Sync>,
    pub matrix: Box<dyn Fn(f64, f64) -> Option<[[f64; 3]; 3]> + Send + Sync>,
}

fn wbs() -> &'static RwLock<HashMap<u64, Arc<WbFns>>> {
    static R: OnceLock<RwLock<HashMap<u64, Arc<WbFns>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Make a white-balance model available under `key` ([`crate::CameraWb::exact`]).
pub fn register_wb(key: u64, f: WbFns) {
    wbs().write().unwrap_or_else(|e| e.into_inner()).insert(key, Arc::new(f));
}

fn wb_fns(key: u64) -> Option<Arc<WbFns>> {
    wbs().read().unwrap_or_else(|e| e.into_inner()).get(&key).cloned()
}

/// The registered neutral for `temp` / `tint` (`None`: not registered, or not finite/positive).
pub fn wb_neutral(key: u64, temp: f64, tint: f64) -> Option<[f64; 3]> {
    let f = wb_fns(key)?;
    (f.neutral)(temp.clamp(1500.0, 50000.0), tint.clamp(-150.0, 150.0)).filter(|n| n.iter().all(|v| v.is_finite() && *v > 0.0))
}

/// The registered white-balance matrix for `temp` / `tint` (`None`: not registered or not finite).
pub fn wb_matrix(key: u64, temp: f64, tint: f64) -> Option<[[f64; 3]; 3]> {
    let f = wb_fns(key)?;
    (f.matrix)(temp.clamp(1500.0, 50000.0), tint.clamp(-150.0, 150.0)).filter(|m| m.iter().flatten().all(|v| v.is_finite() && v.abs() < 1e3))
}

/// The DNG SDK's exposure ramp at white 1 (`dng_function_exposure_ramp`): `x − black` rescaled,
/// with a quadratic toe of radius `black / 2` instead of a hard cut.
#[inline]
pub fn ramp(x: f32, black: f32) -> f32 {
    if black <= 0.0 {
        return x;
    }
    let slope = 1.0 / (1.0 - black);
    let radius = (0.5 * black).min(1.0 / 16.0 / slope);
    if x <= black - radius {
        return 0.0;
    }
    if x >= black + radius {
        return (x - black) * slope;
    }
    let y = x - (black - radius);
    slope / (4.0 * radius) * y * y
}

/// `RefBaselineRGBTone`: the tone function on the largest and smallest channel (each clipped to
/// 0..=1), the middle one placed between them in proportion, so hue is kept.
#[inline]
pub fn rgb_tone(c: [f32; 3], f: impl Fn(f32) -> f32) -> [f32; 3] {
    let [r, g, b] = c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    // (hi, mid, lo) → (f(hi), interpolated mid, f(lo))
    let tone = |hi: f32, mid: f32, lo: f32| {
        let (th, tl) = (f(hi), f(lo));
        (th, tl + (th - tl) * (mid - lo) / (hi - lo), tl)
    };
    if r >= g {
        if g > b {
            let (rr, gg, bb) = tone(r, g, b);
            [rr, gg, bb]
        } else if b > r {
            let (bb, rr, gg) = tone(b, r, g);
            [rr, gg, bb]
        } else if b > g {
            let (rr, bb, gg) = tone(r, b, g);
            [rr, gg, bb]
        } else {
            let gg = f(g);
            [f(r), gg, gg]
        }
    } else if r >= b {
        let (gg, rr, bb) = tone(g, r, b);
        [rr, gg, bb]
    } else if b > g {
        let (bb, gg, rr) = tone(b, g, r);
        [rr, gg, bb]
    } else {
        let (gg, bb, rr) = tone(g, b, r);
        [rr, gg, bb]
    }
}

/// Linear Rec.2020 (D65) → linear ProPhoto (D50, Bradford) and back, row-major.
pub fn prophoto_matrices() -> ([[f32; 3]; 3], [[f32; 3]; 3]) {
    let to: Mat3 = PROPHOTO.from_xyz().mul(&bradford(D65, D50)).mul(&REC2020.to_xyz());
    let from = to.inverse().unwrap_or(Mat3::IDENTITY);
    (to.to_f32(), from.to_f32())
}

/// A look table at strength `k` (1 = as stored): hue shifts and the scales' distances from 1
/// scaled by `k`.
pub fn scaled_table(t: &HsvTable, k: f32) -> HsvTable {
    if (k - 1.0).abs() < 1e-6 {
        return t.clone();
    }
    let data = t.data.iter().map(|d| [d[0] * k, (1.0 + (d[1] - 1.0) * k).max(0.0), (1.0 + (d[2] - 1.0) * k).max(0.0)]).collect();
    HsvTable { data, ..t.clone() }
}

/// What the per-pixel stage applies for an Adobe base (+ look), resolved for one render.
#[derive(Clone, Debug)]
pub struct Finish {
    pub base: Arc<Base>,
    /// Look tables in order (the camera profile's, then the look profile's at its strength).
    pub tables: Vec<Arc<HsvTable>>,
    pub to_pp: [[f32; 3]; 3],
    pub from_pp: [[f32; 3]; 3],
}

impl Finish {
    /// For source base `key` and profile `id` at `amount` (0..=2); `None` without a registered base.
    pub fn new(key: Option<u64>, id: &str, amount: f64) -> Option<Finish> {
        let base = base(key?)?;
        let mut tables = Vec::new();
        if let Some(t) = &base.look {
            tables.push(Arc::new(t.clone()));
        }
        if let Some(l) = look_for(id)
            && let Some(t) = &l.table
        {
            let k = l.strength(amount);
            if k > 0.0 {
                tables.push(Arc::new(scaled_table(t, k)));
            }
        }
        let (to_pp, from_pp) = prophoto_matrices();
        Some(Finish { base, tables, to_pp, from_pp })
    }

    /// Scene-linear Rec.2020 (after exposure) → ProPhoto after the ramp and look tables, ready
    /// for the tone curve.
    #[inline]
    pub fn pre_tone(&self, c: [f32; 3]) -> [f32; 3] {
        let m = &self.to_pp;
        let mut p: [f32; 3] = std::array::from_fn(|i| m[i][0] * c[0] + m[i][1] * c[1] + m[i][2] * c[2]);
        let black = self.base.black;
        if black > 0.0 {
            p = p.map(|v| ramp(v, black));
        }
        for t in &self.tables {
            p = t.apply(p);
        }
        p
    }

    /// ProPhoto (display linear, after the tone curve) → linear Rec.2020.
    #[inline]
    pub fn post_tone(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.from_pp;
        std::array::from_fn(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Base {
        Base { look: None, curve: (0..=CURVE_N).map(|i| (i as f32 / CURVE_N as f32).sqrt()).collect(), black: 0.005 }
    }

    #[test]
    fn rgb_tone_keeps_neutrals_and_hue_order() {
        let f = |x: f32| x.sqrt();
        let g = rgb_tone([0.25, 0.25, 0.25], f);
        assert!(g.iter().all(|v| (v - 0.5).abs() < 1e-6));
        let c = rgb_tone([0.36, 0.16, 0.04], f);
        assert!((c[0] - 0.6).abs() < 1e-6 && (c[2] - 0.2).abs() < 1e-6);
        // the middle channel keeps its place between the others
        assert!((c[1] - (0.2 + 0.4 * (0.12 / 0.32))).abs() < 1e-6, "{c:?}");
        for p in [[0.1, 0.5, 0.3], [0.9, 0.2, 0.6], [0.3, 0.3, 0.8], [0.5, 0.1, 0.1], [2.0, -1.0, f32::NAN]] {
            let o = rgb_tone(p, f);
            assert!(o.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{p:?} → {o:?}");
        }
    }

    #[test]
    fn ramp_matches_the_sdk_shape() {
        assert_eq!(ramp(0.0, 0.005), 0.0);
        assert_eq!(ramp(0.002, 0.005), 0.0);
        assert!((ramp(1.0, 0.005) - 1.0).abs() < 1e-6);
        // continuous at both ends of the toe
        let (lo, hi) = (0.005 - 0.0025, 0.005 + 0.0025);
        assert!(ramp(lo + 1e-7, 0.005) < 1e-6);
        assert!((ramp(hi - 1e-7, 0.005) - (hi - 0.005) / 0.995).abs() < 1e-5);
        assert_eq!(ramp(0.3, 0.0), 0.3);
    }

    #[test]
    fn registry_rejects_bad_bases_and_resolves_looks() {
        let mut bad = base();
        bad.curve[10] = f32::NAN;
        assert!(!register_base(0xbad, bad));
        assert!(Finish::new(Some(0xbad), "adobe:Adobe Color", 1.0).is_none());
        assert!(register_base(0x600d, base()));
        register_look(
            "adobe:Test Look",
            Look {
                name: "Test Look".into(),
                table: Some(HsvTable { hue_divisions: 6, sat_divisions: 2, val_divisions: 1, data: vec![[10.0, 1.2, 0.9]; 12], srgb_value: false }),
                amount_range: [0.0, 2.0],
                supports_amount: true,
                ..Default::default()
            },
        );
        let f = Finish::new(Some(0x600d), "adobe:Test Look", 1.0).unwrap();
        assert_eq!(f.tables.len(), 1);
        assert!(Finish::new(Some(0x600d), "adobe:Test Look", 0.0).unwrap().tables.is_empty(), "amount 0 with min 0: no table");
        let half = Finish::new(Some(0x600d), "adobe:Test Look", 0.5).unwrap();
        assert!((half.tables[0].data[0][1] - 1.1).abs() < 1e-6);
        // grey stays grey through the ramp-free path, and round-trips through ProPhoto
        let g = f.post_tone(f.pre_tone([0.18, 0.18, 0.18]));
        assert!((g[0] - g[2]).abs() < 1e-3, "{g:?}");
        assert!(look("lut:Test Look").is_none());
    }

    #[test]
    fn curve_lookup_interpolates_and_clamps() {
        let c: Vec<f32> = (0..=CURVE_N).map(|i| i as f32 / CURVE_N as f32).collect();
        assert!((curve_at(&c, 0.3337) - 0.3337).abs() < 1e-5);
        assert_eq!(curve_at(&c, -1.0), 0.0);
        assert_eq!(curve_at(&c, 7.0), 1.0);
        assert_eq!(curve_at(&c, f32::NAN), 0.0);
    }
}
