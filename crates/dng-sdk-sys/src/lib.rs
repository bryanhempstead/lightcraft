//! Adobe's DNG SDK for Bryan's fork of LightCraft: the exact Camera Raw / Lightroom maths for
//! camera profiles (`.dcp`), colour specs (camera → XYZ D50 with dual-illuminant interpolation and
//! forward matrices), Temp / Tint ↔ xy (`dng_temperature`), hue/saturation maps (interpolated for
//! a white, and applied as `RefBaselineHueSatMap`), the ACR3 default / profile tone curves
//! (applied as `RefBaselineRGBTone`), Camera Raw's XMP look tables, and `dng_render` reference
//! renders of DNG files.
//!
//! The SDK is C++ (`vendor/dng_sdk_1_7_1`, fetched from adobe.com by `tools/fetch-dng-sdk.sh`
//! and never committed). It is reached through a small C ABI shim (`shim/lc_dng.cpp`) that
//! catches every exception. Without the SDK (not fetched, wasm, other OS) every call returns
//! [`Error::Unavailable`] and callers keep their own path.
//!
//! This crate and `lightcraft-sysmem` are the only ones allowed `unsafe` (AGENTS.md → *Fork
//! rules*): each `unsafe` block is an FFI call with checked lengths and owned buffers.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

/// Why an SDK call gave no answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The SDK isn't built into this binary.
    Unavailable,
    BadArgument,
    /// The data isn't a valid profile / table / DNG for the SDK.
    Parse,
    /// The SDK threw (bad data, out of memory, unsupported feature).
    Sdk,
    /// Nothing of that kind (e.g. a profile without a look table).
    None,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Unavailable => "Adobe DNG SDK not built in",
            Error::BadArgument => "bad argument",
            Error::Parse => "not readable by the DNG SDK",
            Error::Sdk => "the DNG SDK failed",
            Error::None => "not present",
        })
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Whether the SDK is built in.
pub fn available() -> bool {
    cfg!(lc_dng_sdk)
}

/// A hue/saturation/value table (`dng_hue_sat_map`): `dims` = hue, saturation, value divisions;
/// `deltas` = (hue shift °, saturation scale, value scale), value-major, hue, saturation-minor;
/// `encoding` 0 = linear, 1 = sRGB value axis.
#[derive(Clone, Debug, PartialEq)]
pub struct HueSatMap {
    pub dims: [u32; 3],
    pub deltas: Vec<[f32; 3]>,
    pub encoding: u32,
}

/// Facts about a camera profile.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileInfo {
    pub name: String,
    pub calibration_signature: String,
    /// `UniqueCameraModelRestriction`: the camera the profile is for (e.g. `Canon EOS R6`).
    pub unique_model: String,
    pub illuminants: [u32; 3],
    pub temperatures: [f64; 2],
    pub has_color_matrix2: bool,
    pub has_forward_matrix: bool,
    pub illuminant_model: i32,
    pub baseline_exposure_offset: f64,
    /// `DefaultBlackRender`: 0 = auto, 1 = none.
    pub default_black_render: u32,
    pub hue_sat_dims: [u32; 3],
    pub hue_sat_encoding: u32,
    pub has_hue_sat2: bool,
    pub look_dims: [u32; 3],
    pub look_encoding: u32,
    /// Points of `ProfileToneCurve` (0 = none: the ACR3 default curve applies).
    pub tone_points: u32,
    pub embed_policy: u32,
    pub has_rgb_tables: bool,
    pub has_gain_table_map: bool,
}

/// A colour spec for one white: `camera_to_pcs` maps camera RGB to XYZ D50 (the camera white to
/// D50), rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorSpec {
    pub white_xy: [f64; 2],
    pub camera_white: [f64; 3],
    pub camera_to_pcs: [[f64; 3]; 3],
}

/// The white a colour spec is computed for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum White {
    /// A camera neutral (raw RGB of a white surface, e.g. `AsShotNeutral`).
    Neutral([f64; 3]),
    Xy(f64, f64),
}

/// A Camera Raw XMP look table (`crs:Table_<digest>` of a `crs:LookTable`).
#[derive(Clone, Debug, PartialEq)]
pub struct LookTable {
    pub map: HueSatMap,
    /// What Amount 0 and 2 (0 % and 200 %) mean, as table strengths.
    pub amount_range: [f64; 2],
}

/// The largest table copied out (Adobe's are ≤ 90 × 30 × 16 entries).
const MAX_DELTAS: usize = 1 << 20;

#[cfg(lc_dng_sdk)]
#[allow(unsafe_code)]
mod ffi {
    use std::ffi::c_char;

    #[repr(C)]
    pub struct LcProfile {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct LcRgbt {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct LcHsm {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct LcProfileInfo {
        pub name: [c_char; 256],
        pub calibration_signature: [c_char; 256],
        pub unique_model: [c_char; 256],
        pub illuminant1: u32,
        pub illuminant2: u32,
        pub illuminant3: u32,
        pub temperature1: f64,
        pub temperature2: f64,
        pub has_color_matrix2: i32,
        pub has_forward_matrix1: i32,
        pub illuminant_model: i32,
        pub baseline_exposure_offset: f64,
        pub default_black_render: u32,
        pub hsm_dims: [u32; 3],
        pub hsm_encoding: u32,
        pub has_hsm2: i32,
        pub look_dims: [u32; 3],
        pub look_encoding: u32,
        pub tone_points: u32,
        pub embed_policy: u32,
        pub has_rgb_tables: i32,
        pub has_gain_table_map: i32,
    }

    unsafe extern "C" {
        pub fn lc_profile_open(data: *const u8, len: usize, out: *mut *mut LcProfile) -> i32;
        pub fn lc_profile_free(p: *mut LcProfile);
        pub fn lc_profile_info_get(p: *const LcProfile, out: *mut LcProfileInfo) -> i32;
        pub fn lc_color_spec(
            p: *const LcProfile,
            analog_balance: *const f64,
            mode: i32,
            input: *const f64,
            out_white_xy: *mut f64,
            out_camera_white: *mut f64,
            out_camera_to_pcs: *mut f64,
        ) -> i32;
        pub fn lc_profile_hue_sat_map(p: *const LcProfile, x: f64, y: f64, dims: *mut u32, buf: *mut f32, cap: usize) -> i32;
        pub fn lc_profile_look_table(p: *const LcProfile, dims: *mut u32, buf: *mut f32, cap: usize) -> i32;
        pub fn lc_profile_tone_curve(p: *const LcProfile, xs: *const f64, ys: *mut f64, n: usize) -> i32;
        pub fn lc_temp_tint_to_xy(temp: f64, tint: f64, out: *mut f64) -> i32;
        pub fn lc_xy_to_temp_tint(x: f64, y: f64, out: *mut f64) -> i32;
        pub fn lc_look_table_decode(
            text: *const c_char,
            len: usize,
            dims: *mut u32,
            encoding: *mut u32,
            range: *mut f64,
            buf: *mut f32,
            cap: usize,
        ) -> i32;
        pub fn lc_hsm_new(dims: *const u32, deltas: *const f32, len: usize, encoding: u32, out: *mut *mut LcHsm) -> i32;
        pub fn lc_hsm_free(h: *mut LcHsm);
        pub fn lc_hsm_apply(h: *const LcHsm, r: *mut f32, g: *mut f32, b: *mut f32, n: usize, overrange: i32) -> i32;
        pub fn lc_rgb_tone(p: *const LcProfile, r: *mut f32, g: *mut f32, b: *mut f32, n: usize) -> i32;
        pub fn lc_rgb_table_new(text: *const c_char, len: usize, info: *mut u32, range: *mut f64, out: *mut *mut LcRgbt) -> i32;
        pub fn lc_rgb_table_free(t: *mut LcRgbt);
        pub fn lc_rgb_table_apply(t: *const LcRgbt, amount: f64, r: *mut f32, g: *mut f32, b: *mut f32, n: usize, overrange: i32) -> i32;
        pub fn lc_render_dng(
            dng: *const u8,
            dng_len: usize,
            dcp: *const u8,
            dcp_len: usize,
            max_size: u32,
            space: i32,
            w: *mut u32,
            h: *mut u32,
            out: *mut *mut f32,
        ) -> i32;
        pub fn lc_free(ptr: *mut std::ffi::c_void);
    }
}

#[cfg(lc_dng_sdk)]
fn status(code: i32) -> Result<i32> {
    match code {
        c if c >= 0 => Ok(c),
        -1 => Err(Error::BadArgument),
        -2 => Err(Error::Parse),
        -5 => Err(Error::None),
        _ => Err(Error::Sdk),
    }
}

#[cfg(lc_dng_sdk)]
fn c_string(chars: &[std::ffi::c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A table's divisions and entries.
#[cfg(lc_dng_sdk)]
type Table = ([u32; 3], Vec<[f32; 3]>);

/// Copy out a table the SDK fills into a caller buffer (sized from its dims on a first call).
#[cfg(lc_dng_sdk)]
fn read_table(mut call: impl FnMut(&mut [u32; 3], &mut [f32]) -> i32) -> Result<Option<Table>> {
    let mut dims = [0u32; 3];
    let first = call(&mut dims, &mut []);
    match first {
        -5 => return Ok(None),
        -4 => {}
        c => {
            status(c)?;
        }
    }
    let count = (dims[0] as usize).saturating_mul(dims[1] as usize).saturating_mul(dims[2].max(1) as usize);
    if count == 0 || count > MAX_DELTAS {
        return Err(Error::Sdk);
    }
    let mut buf = vec![0f32; count * 3];
    status(call(&mut dims, &mut buf))?;
    Ok(Some((dims, buf.as_chunks::<3>().0.to_vec())))
}

/// A camera profile parsed by the SDK. Immutable after parsing (all queries are const in the
/// SDK), so it may be shared between threads.
pub struct Profile {
    #[cfg(lc_dng_sdk)]
    ptr: std::ptr::NonNull<ffi::LcProfile>,
}

// SAFETY (Send/Sync): the profile is only read after parsing; the SDK's const profile queries
// allocate their results and touch no shared mutable state.
#[allow(unsafe_code)]
unsafe impl Send for Profile {}
#[allow(unsafe_code)]
unsafe impl Sync for Profile {}

impl fmt::Debug for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Profile")
    }
}

#[cfg(lc_dng_sdk)]
#[allow(unsafe_code)]
impl Drop for Profile {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `lc_profile_open` and is freed exactly once, here.
        unsafe { ffi::lc_profile_free(self.ptr.as_ptr()) }
    }
}

#[allow(unsafe_code)]
impl Profile {
    /// Parse a DCP (or any extended camera profile stream).
    pub fn parse(bytes: &[u8]) -> Result<Profile> {
        #[cfg(lc_dng_sdk)]
        {
            let mut out = std::ptr::null_mut();
            // SAFETY: `bytes` is a live slice of `len` bytes; the shim copies what it keeps and
            // writes `out` only on success.
            status(unsafe { ffi::lc_profile_open(bytes.as_ptr(), bytes.len(), &mut out) })?;
            let ptr = std::ptr::NonNull::new(out).ok_or(Error::Sdk)?;
            Ok(Profile { ptr })
        }
        #[cfg(not(lc_dng_sdk))]
        {
            let _ = bytes;
            Err(Error::Unavailable)
        }
    }

    pub fn info(&self) -> Result<ProfileInfo> {
        #[cfg(lc_dng_sdk)]
        {
            // SAFETY: an all-zero `LcProfileInfo` is valid (integers and char arrays).
            let mut i: ffi::LcProfileInfo = unsafe { std::mem::zeroed() };
            // SAFETY: `ptr` is a live profile; `i` is a valid out-pointer of the right layout.
            status(unsafe { ffi::lc_profile_info_get(self.ptr.as_ptr(), &mut i) })?;
            Ok(ProfileInfo {
                name: c_string(&i.name),
                calibration_signature: c_string(&i.calibration_signature),
                unique_model: c_string(&i.unique_model),
                illuminants: [i.illuminant1, i.illuminant2, i.illuminant3],
                temperatures: [i.temperature1, i.temperature2],
                has_color_matrix2: i.has_color_matrix2 != 0,
                has_forward_matrix: i.has_forward_matrix1 != 0,
                illuminant_model: i.illuminant_model,
                baseline_exposure_offset: i.baseline_exposure_offset,
                default_black_render: i.default_black_render,
                hue_sat_dims: i.hsm_dims,
                hue_sat_encoding: i.hsm_encoding,
                has_hue_sat2: i.has_hsm2 != 0,
                look_dims: i.look_dims,
                look_encoding: i.look_encoding,
                tone_points: i.tone_points,
                embed_policy: i.embed_policy,
                has_rgb_tables: i.has_rgb_tables != 0,
                has_gain_table_map: i.has_gain_table_map != 0,
            })
        }
        #[cfg(not(lc_dng_sdk))]
        Err(Error::Unavailable)
    }

    /// The colour spec for `white` (`analog_balance`: the file's `AnalogBalance`, if any).
    pub fn color_spec(&self, white: White, analog_balance: Option<[f64; 3]>) -> Result<ColorSpec> {
        let (mode, input) = match white {
            White::Neutral(n) => (0, n),
            White::Xy(x, y) => (1, [x, y, 0.0]),
        };
        if !input.iter().all(|v| v.is_finite()) || analog_balance.is_some_and(|a| !a.iter().all(|v| v.is_finite() && *v > 0.0)) {
            return Err(Error::BadArgument);
        }
        #[cfg(lc_dng_sdk)]
        {
            let (mut xy, mut cw, mut m) = ([0f64; 2], [0f64; 3], [0f64; 9]);
            let ab = analog_balance.as_ref().map_or(std::ptr::null(), |a| a.as_ptr());
            // SAFETY: every pointer is to a live array of the length the shim reads/writes
            // (analog balance 3 or null, input 3, outputs 2 / 3 / 9).
            status(unsafe { ffi::lc_color_spec(self.ptr.as_ptr(), ab, mode, input.as_ptr(), xy.as_mut_ptr(), cw.as_mut_ptr(), m.as_mut_ptr()) })?;
            let spec = ColorSpec { white_xy: xy, camera_white: cw, camera_to_pcs: [[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]] };
            if !(xy.iter().chain(&cw).chain(&m).all(|v| v.is_finite())) {
                return Err(Error::Sdk);
            }
            Ok(spec)
        }
        #[cfg(not(lc_dng_sdk))]
        {
            let _ = (mode, input);
            Err(Error::Unavailable)
        }
    }

    /// The hue/sat map interpolated for white `xy` (`None`: the profile has none).
    pub fn hue_sat_map(&self, xy: [f64; 2]) -> Result<Option<HueSatMap>> {
        if !xy.iter().all(|v| v.is_finite() && *v > 0.0) {
            return Err(Error::BadArgument);
        }
        #[cfg(lc_dng_sdk)]
        {
            let encoding = self.info()?.hue_sat_encoding;
            // SAFETY: `dims` holds 3 u32, `buf` is a live mutable slice of `buf.len()` floats.
            let t = read_table(|dims, buf| unsafe {
                ffi::lc_profile_hue_sat_map(self.ptr.as_ptr(), xy[0], xy[1], dims.as_mut_ptr(), buf.as_mut_ptr(), buf.len())
            })?;
            Ok(t.map(|(dims, deltas)| HueSatMap { dims, deltas, encoding }))
        }
        #[cfg(not(lc_dng_sdk))]
        Err(Error::Unavailable)
    }

    /// The profile's look table (`None`: it has none).
    pub fn look_table(&self) -> Result<Option<HueSatMap>> {
        #[cfg(lc_dng_sdk)]
        {
            let encoding = self.info()?.look_encoding;
            // SAFETY: as in `hue_sat_map`.
            let t = read_table(|dims, buf| unsafe { ffi::lc_profile_look_table(self.ptr.as_ptr(), dims.as_mut_ptr(), buf.as_mut_ptr(), buf.len()) })?;
            Ok(t.map(|(dims, deltas)| HueSatMap { dims, deltas, encoding }))
        }
        #[cfg(not(lc_dng_sdk))]
        Err(Error::Unavailable)
    }

    /// The profile's tone curve at `xs` (0..1, linear in and out), or the ACR3 default when the
    /// profile has none; the flag says whether it was the profile's own.
    pub fn tone_curve(&self, xs: &[f64]) -> Result<(Vec<f64>, bool)> {
        #[cfg(lc_dng_sdk)]
        {
            let mut ys = vec![0f64; xs.len()];
            // SAFETY: `xs` and `ys` are live slices of the same length `n`.
            let own = status(unsafe { ffi::lc_profile_tone_curve(self.ptr.as_ptr(), xs.as_ptr(), ys.as_mut_ptr(), xs.len()) })?;
            Ok((ys, own == 1))
        }
        #[cfg(not(lc_dng_sdk))]
        {
            let _ = xs;
            Err(Error::Unavailable)
        }
    }

    /// `RefBaselineRGBTone` with this profile's tone curve, in place on planar linear ProPhoto.
    pub fn rgb_tone(&self, r: &mut [f32], g: &mut [f32], b: &mut [f32]) -> Result<()> {
        if r.len() != g.len() || r.len() != b.len() {
            return Err(Error::BadArgument);
        }
        #[cfg(lc_dng_sdk)]
        {
            // SAFETY: three live, distinct mutable slices of equal length `n`.
            status(unsafe { ffi::lc_rgb_tone(self.ptr.as_ptr(), r.as_mut_ptr(), g.as_mut_ptr(), b.as_mut_ptr(), r.len()) })?;
            Ok(())
        }
        #[cfg(not(lc_dng_sdk))]
        Err(Error::Unavailable)
    }
}

/// The ACR3 default tone curve (Camera Raw's base curve for profiles without their own) at `xs`.
#[allow(unsafe_code)]
pub fn acr3_tone_curve(xs: &[f64]) -> Result<Vec<f64>> {
    #[cfg(lc_dng_sdk)]
    {
        let mut ys = vec![0f64; xs.len()];
        // SAFETY: a null profile selects the default curve; `xs`/`ys` are live, equal-length.
        status(unsafe { ffi::lc_profile_tone_curve(std::ptr::null(), xs.as_ptr(), ys.as_mut_ptr(), xs.len()) })?;
        Ok(ys)
    }
    #[cfg(not(lc_dng_sdk))]
    {
        let _ = xs;
        Err(Error::Unavailable)
    }
}

/// Camera Raw's white balance: Temp (K) / Tint → xy chromaticity (`dng_temperature`).
#[allow(unsafe_code)]
pub fn temp_tint_to_xy(temp: f64, tint: f64) -> Result<[f64; 2]> {
    if !(temp.is_finite() && temp > 0.0 && tint.is_finite()) {
        return Err(Error::BadArgument);
    }
    #[cfg(lc_dng_sdk)]
    {
        let mut xy = [0f64; 2];
        // SAFETY: `xy` is a live array of 2 doubles.
        status(unsafe { ffi::lc_temp_tint_to_xy(temp, tint, xy.as_mut_ptr()) })?;
        Ok(xy)
    }
    #[cfg(not(lc_dng_sdk))]
    Err(Error::Unavailable)
}

/// xy chromaticity → Camera Raw's Temp (K) / Tint.
#[allow(unsafe_code)]
pub fn xy_to_temp_tint(x: f64, y: f64) -> Result<[f64; 2]> {
    if !(x.is_finite() && y.is_finite() && x > 0.0 && y > 0.0) {
        return Err(Error::BadArgument);
    }
    #[cfg(lc_dng_sdk)]
    {
        let mut tt = [0f64; 2];
        // SAFETY: `tt` is a live array of 2 doubles.
        status(unsafe { ffi::lc_xy_to_temp_tint(x, y, tt.as_mut_ptr()) })?;
        Ok(tt)
    }
    #[cfg(not(lc_dng_sdk))]
    Err(Error::Unavailable)
}

/// Decode a Camera Raw XMP look table (the text of its `crs:Table_<digest>` attribute).
#[allow(unsafe_code)]
pub fn decode_look_table(text: &str) -> Result<LookTable> {
    if text.is_empty() || text.len() > 64 << 20 {
        return Err(Error::BadArgument);
    }
    #[cfg(lc_dng_sdk)]
    {
        let (mut encoding, mut range) = (0u32, [0f64; 2]);
        // SAFETY: `text` is live for `len` bytes (no terminator needed: the shim copies it);
        // `dims`, `encoding`, `range` and `buf` are live outputs of the sizes the shim writes.
        let t = read_table(|dims, buf| unsafe {
            ffi::lc_look_table_decode(
                text.as_ptr().cast(),
                text.len(),
                dims.as_mut_ptr(),
                &mut encoding,
                range.as_mut_ptr(),
                buf.as_mut_ptr(),
                buf.len(),
            )
        })?
        .ok_or(Error::None)?;
        Ok(LookTable { map: HueSatMap { dims: t.0, deltas: t.1, encoding }, amount_range: range })
    }
    #[cfg(not(lc_dng_sdk))]
    Err(Error::Unavailable)
}

/// A hue/sat map prepared for the SDK's reference application (`RefBaselineHueSatMap`).
pub struct HsmApplier {
    #[cfg(lc_dng_sdk)]
    ptr: std::ptr::NonNull<ffi::LcHsm>,
}

// SAFETY (Send/Sync): read-only after construction (application reads the table only).
#[allow(unsafe_code)]
unsafe impl Send for HsmApplier {}
#[allow(unsafe_code)]
unsafe impl Sync for HsmApplier {}

#[cfg(lc_dng_sdk)]
#[allow(unsafe_code)]
impl Drop for HsmApplier {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `lc_hsm_new` and is freed exactly once, here.
        unsafe { ffi::lc_hsm_free(self.ptr.as_ptr()) }
    }
}

#[allow(unsafe_code)]
impl HsmApplier {
    pub fn new(map: &HueSatMap) -> Result<HsmApplier> {
        let n = (map.dims[0] as usize).saturating_mul(map.dims[1] as usize).saturating_mul(map.dims[2].max(1) as usize);
        if n == 0 || n != map.deltas.len() || n > MAX_DELTAS {
            return Err(Error::BadArgument);
        }
        #[cfg(lc_dng_sdk)]
        {
            let flat: Vec<f32> = map.deltas.iter().flatten().copied().collect();
            let mut out = std::ptr::null_mut();
            // SAFETY: `dims` holds 3 u32 and `flat` is a live slice of `flat.len()` floats.
            status(unsafe { ffi::lc_hsm_new(map.dims.as_ptr(), flat.as_ptr(), flat.len(), map.encoding, &mut out) })?;
            Ok(HsmApplier { ptr: std::ptr::NonNull::new(out).ok_or(Error::Sdk)? })
        }
        #[cfg(not(lc_dng_sdk))]
        Err(Error::Unavailable)
    }

    /// Apply in place to planar linear ProPhoto values (`overrange`: keep values above 1).
    pub fn apply(&self, r: &mut [f32], g: &mut [f32], b: &mut [f32], overrange: bool) -> Result<()> {
        if r.len() != g.len() || r.len() != b.len() {
            return Err(Error::BadArgument);
        }
        #[cfg(lc_dng_sdk)]
        {
            // SAFETY: three live, distinct mutable slices of equal length `n`.
            status(unsafe { ffi::lc_hsm_apply(self.ptr.as_ptr(), r.as_mut_ptr(), g.as_mut_ptr(), b.as_mut_ptr(), r.len(), overrange as i32) })?;
            Ok(())
        }
        #[cfg(not(lc_dng_sdk))]
        {
            let _ = overrange;
            Err(Error::Unavailable)
        }
    }
}

/// A Camera Raw XMP RGB table (`crs:Table_<digest>` of a `crs:RGBTable`), decoded and applied
/// by the SDK (`dng_rgb_table`, `dng_rgb_to_rgb_table_data`).
pub struct RgbTable {
    #[cfg(lc_dng_sdk)]
    ptr: std::ptr::NonNull<ffi::LcRgbt>,
    /// Dimensions, divisions, primaries (0 sRGB, 1 Adobe, 2 ProPhoto, 3 P3, 4 Rec.2020), gamma
    /// (0 linear, 1 sRGB, 2 1.8, 3 2.2, 4 Rec.709), gamut (0 clip, 1 extend).
    pub info: [u32; 5],
    /// Table amounts the profile's Amount 0 % and 200 % stand for.
    pub amount_range: [f64; 2],
}

// SAFETY (Send/Sync): read-only after decoding; `apply` works on a copy of the table.
#[allow(unsafe_code)]
unsafe impl Send for RgbTable {}
#[allow(unsafe_code)]
unsafe impl Sync for RgbTable {}

#[cfg(lc_dng_sdk)]
#[allow(unsafe_code)]
impl Drop for RgbTable {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `lc_rgb_table_new` and is freed exactly once, here.
        unsafe { ffi::lc_rgb_table_free(self.ptr.as_ptr()) }
    }
}

#[allow(unsafe_code)]
impl RgbTable {
    pub fn decode(text: &str) -> Result<RgbTable> {
        if text.is_empty() || text.len() > 64 << 20 {
            return Err(Error::BadArgument);
        }
        #[cfg(lc_dng_sdk)]
        {
            let (mut info, mut range, mut out) = ([0u32; 5], [0f64; 2], std::ptr::null_mut());
            // SAFETY: `text` is live for `len` bytes; `info` (5), `range` (2) and `out` are live
            // outputs of the sizes the shim writes.
            status(unsafe { ffi::lc_rgb_table_new(text.as_ptr().cast(), text.len(), info.as_mut_ptr(), range.as_mut_ptr(), &mut out) })?;
            Ok(RgbTable { ptr: std::ptr::NonNull::new(out).ok_or(Error::Sdk)?, info, amount_range: range })
        }
        #[cfg(not(lc_dng_sdk))]
        Err(Error::Unavailable)
    }

    /// Apply at table amount `amount` in place to planar linear ProPhoto values.
    pub fn apply(&self, amount: f64, r: &mut [f32], g: &mut [f32], b: &mut [f32], overrange: bool) -> Result<()> {
        if r.len() != g.len() || r.len() != b.len() || r.is_empty() || !amount.is_finite() {
            return Err(Error::BadArgument);
        }
        #[cfg(lc_dng_sdk)]
        {
            // SAFETY: three live, distinct mutable slices of equal length `n`.
            status(unsafe {
                ffi::lc_rgb_table_apply(self.ptr.as_ptr(), amount, r.as_mut_ptr(), g.as_mut_ptr(), b.as_mut_ptr(), r.len(), overrange as i32)
            })?;
            Ok(())
        }
        #[cfg(not(lc_dng_sdk))]
        {
            let _ = overrange;
            Err(Error::Unavailable)
        }
    }
}

/// A reference render (`dng_render`) of a DNG file, optionally with an external profile used
/// instead of the file's own: `(width, height, interleaved RGB 0..1)` in sRGB (`prophoto` false)
/// or ProPhoto (gamma 1.8) — what the SDK's defaults make of the file.
#[allow(unsafe_code)]
pub fn render_dng(dng: &[u8], dcp: Option<&[u8]>, max_size: u32, prophoto: bool) -> Result<(u32, u32, Vec<f32>)> {
    #[cfg(lc_dng_sdk)]
    {
        let (mut w, mut h, mut out) = (0u32, 0u32, std::ptr::null_mut());
        let (dp, dl) = dcp.map_or((std::ptr::null(), 0), |d| (d.as_ptr(), d.len()));
        // SAFETY: `dng` / `dcp` are live slices of the given lengths (or null, 0); on success the
        // shim mallocs `w*h*3` floats into `out`, which are copied and then freed with `lc_free`.
        status(unsafe { ffi::lc_render_dng(dng.as_ptr(), dng.len(), dp, dl, max_size, prophoto as i32, &mut w, &mut h, &mut out) })?;
        if out.is_null() {
            return Err(Error::Sdk);
        }
        let n = (w as usize).saturating_mul(h as usize).saturating_mul(3);
        // SAFETY: the shim allocated exactly `n` floats at `out` and filled them.
        let v = unsafe { std::slice::from_raw_parts(out, n) }.to_vec();
        // SAFETY: `out` came from the shim's malloc and is freed once.
        unsafe { ffi::lc_free(out.cast()) };
        Ok((w, h, v))
    }
    #[cfg(not(lc_dng_sdk))]
    {
        let _ = (dng, dcp, max_size, prophoto);
        Err(Error::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_an_error_not_a_crash() {
        if !available() {
            assert_eq!(Profile::parse(b"IIRC").err(), Some(Error::Unavailable));
            return;
        }
        assert!(Profile::parse(b"").is_err());
        assert!(Profile::parse(b"IIRC\0\0\0\0garbage garbage").is_err());
        assert!(Profile::parse(&[0xffu8; 4096]).is_err());
        assert!(decode_look_table("not a table").is_err());
        assert!(render_dng(b"II*\0 not a dng at all", None, 256, false).is_err());
        assert!(temp_tint_to_xy(f64::NAN, 0.0).is_err());
        let bad = HueSatMap { dims: [2, 2, 1], deltas: vec![[0.0, 1.0, 1.0]; 3], encoding: 0 };
        assert!(HsmApplier::new(&bad).is_err());
    }

    #[test]
    fn temperature_round_trips() {
        if !available() {
            return;
        }
        for (t, tint) in [(2850.0, 0.0), (5000.0, 10.0), (6500.0, -20.0), (12000.0, 40.0)] {
            let xy = temp_tint_to_xy(t, tint).unwrap();
            let back = xy_to_temp_tint(xy[0], xy[1]).unwrap();
            assert!((back[0] - t).abs() < 0.5 && (back[1] - tint).abs() < 0.05, "{t}/{tint} → {back:?}");
        }
        // on the Planckian locus at tint 0 (5003 K: x 0.3451, y 0.3516)
        let xy = temp_tint_to_xy(5003.0, 0.0).unwrap();
        assert!((xy[0] - 0.3451).abs() < 0.001 && (xy[1] - 0.3516).abs() < 0.001, "{xy:?}");
    }

    #[test]
    fn identity_table_leaves_colour_alone_and_acr3_curve_is_monotone() {
        if !available() {
            return;
        }
        let map = HueSatMap { dims: [6, 3, 1], deltas: vec![[0.0, 1.0, 1.0]; 18], encoding: 0 };
        let a = HsmApplier::new(&map).unwrap();
        let (mut r, mut g, mut b) = (vec![0.2f32, 0.9, 0.05], vec![0.5f32, 0.9, 0.3], vec![0.1f32, 0.9, 0.6]);
        a.apply(&mut r, &mut g, &mut b, false).unwrap();
        assert!((r[0] - 0.2).abs() < 1e-5 && (g[2] - 0.3).abs() < 1e-5 && (b[1] - 0.9).abs() < 1e-5);
        let xs: Vec<f64> = (0..=100).map(|i| i as f64 / 100.0).collect();
        let ys = acr3_tone_curve(&xs).unwrap();
        assert!(ys.windows(2).all(|w| w[1] >= w[0]) && ys[0].abs() < 1e-9 && (ys[100] - 1.0).abs() < 1e-9);
        assert!(ys[18] > 0.3 && ys[18] < 0.5, "grey 0.18 → {}", ys[18]);
    }
}
