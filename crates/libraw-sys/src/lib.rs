//! LibRaw for Bryan's fork of LightCraft: the fallback raw decoder for every camera LightCraft's
//! own decoders don't read yet (LibRaw knows ~1,200 models). Only LibRaw's *unpacking* is used —
//! the sensor samples and what is needed to develop them (black / white levels, CFA layout, as-shot
//! white balance, Adobe's XYZ → camera matrix where LibRaw has it, geometry, default crop,
//! orientation, camera and lens names). LibRaw's demosaic and colour processing are not called:
//! LightCraft's own pipeline develops the data exactly like its native decoders' output.
//!
//! LibRaw is C++ (LGPL-2.1 / CDDL-1.0, `vendor/LibRaw-0.22.3`, fetched from libraw.org by
//! `tools/fetch-libraw.sh` and never committed). It is reached through a small C ABI shim
//! (`shim/lc_libraw.cpp`) that catches every exception. Without it (not fetched, wasm, other OS,
//! `LIGHTCRAFT_NO_LIBRAW=1`) every call returns [`Error::Unavailable`] and callers keep their own
//! path.
//!
//! Everything LibRaw reports is treated as untrusted: sizes, offsets and pattern dimensions are
//! checked before use, allocations are capped, and the conversion to LightCraft's conventions
//! happens here in safe Rust. This crate, `lightcraft-dng-sdk-sys` and `lightcraft-sysmem` are the
//! only ones allowed `unsafe` (CLAUDE.md → *Never crash*): each `unsafe` block is an FFI call with
//! checked lengths and owned buffers.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

/// Most samples a decode may produce (matches `lightcraft_raw::MAX_SAMPLES`).
pub const MAX_SAMPLES: usize = 1 << 30;

/// Why LibRaw gave no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// LibRaw isn't built into this binary.
    Unavailable,
    BadArgument,
    /// The data needs more memory than allowed.
    TooLarge,
    /// LibRaw read it, but in a form LightCraft can't develop (e.g. a CMYG sensor, float data).
    Unsupported(String),
    /// LibRaw's own error (`code` is its `LIBRAW_*` code).
    LibRaw {
        code: i32,
        message: String,
    },
    /// LibRaw's decoder hit corrupt sample data.
    Corrupt(String),
    /// LibRaw threw or ran out of memory.
    Failed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unavailable => f.write_str("LibRaw not built in"),
            Error::BadArgument => f.write_str("bad argument"),
            Error::TooLarge => f.write_str("raw data too large"),
            Error::Unsupported(s) => write!(f, "LibRaw: {s}"),
            Error::LibRaw { message, .. } => write!(f, "LibRaw: {message}"),
            Error::Corrupt(s) => write!(f, "LibRaw: corrupt data ({s})"),
            Error::Failed(s) => write!(f, "LibRaw: {s}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Whether LibRaw is built in.
pub fn available() -> bool {
    cfg!(lc_libraw)
}

/// LibRaw's version string (e.g. `0.22.3-Release`), when built in.
pub fn version() -> Option<String> {
    #[cfg(lc_libraw)]
    {
        ffi::version()
    }
    #[cfg(not(lc_libraw))]
    None
}

/// How many camera models LibRaw lists as supported (0 without it).
pub fn camera_count() -> usize {
    #[cfg(lc_libraw)]
    {
        usize::try_from(ffi::camera_count()).unwrap_or(0)
    }
    #[cfg(not(lc_libraw))]
    0
}

/// A colour filter array anchored at pixel (0, 0) of the full sensor frame. Colour indices:
/// 0 = red, 1 = green, 2 = blue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cfa {
    pub width: usize,
    pub height: usize,
    pub pattern: Vec<u8>,
}

/// Black level per pixel position, repeating every `rows × cols` pixels from the top-left of the
/// visible area; `values` = `rows × cols × channels` (black + per-colour + pattern parts summed).
#[derive(Clone, Debug, PartialEq)]
pub struct Black {
    pub rows: usize,
    pub cols: usize,
    pub values: Vec<f32>,
}

/// A rectangle in the full sensor frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// What LibRaw read about a file, in LightCraft's conventions.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// Full sensor frame (including masked borders).
    pub raw_width: usize,
    pub raw_height: usize,
    /// Area of valid image data within the full frame.
    pub visible: Area,
    /// The camera's default crop (inside `visible`), when it records one.
    pub default_crop: Option<Area>,
    /// Exif orientation code (1 = normal).
    pub orientation: u16,
    /// `None` for linear (demosaiced) or monochrome data.
    pub cfa: Option<Cfa>,
    /// Samples per pixel of the unpacked data (1 = CFA / mono); 0 when only identified.
    pub channels: usize,
    /// Known only after unpacking (some formats measure it from masked pixels).
    pub black: Black,
    pub white: u32,
    pub bits: u32,
    /// As-shot white balance as multipliers, green = 1.
    pub as_shot_multipliers: Option<[f32; 3]>,
    /// XYZ (D65) → camera (rows = camera channels): Adobe's `ColorMatrix` for the cameras LibRaw knows.
    pub xyz_to_camera: Option<[[f64; 3]; 3]>,
    pub make: String,
    pub model: String,
    pub normalized_make: String,
    pub normalized_model: String,
    pub software: String,
    pub lens: String,
    pub lens_make: String,
    pub iso: f32,
    /// Seconds.
    pub shutter: f32,
    pub aperture: f32,
    /// Millimetres.
    pub focal_length: f32,
    /// Unix time of capture (0 = unknown).
    pub timestamp: i64,
    pub dng_version: u32,
    /// LibRaw's decoder function for the file (e.g. `crxLoadRaw()`).
    pub decoder: String,
}

/// A decoded file: [`Info`] plus `raw_width × raw_height × channels` samples, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub info: Info,
    pub samples: Vec<u16>,
}

/// Read a file's headers (no sample data is decompressed). Black levels may be incomplete.
pub fn identify(bytes: &[u8]) -> Result<Info> {
    open(bytes, false).map(|d| d.info)
}

/// Decode a file's sensor data.
pub fn decode(bytes: &[u8]) -> Result<Decoded> {
    open(bytes, true)
}

/// Largest memory LibRaw may allocate for one file's sensor data (MB).
const MAX_RAW_MB: u32 = 2048;

fn open(bytes: &[u8], unpack: bool) -> Result<Decoded> {
    if bytes.is_empty() {
        return Err(Error::BadArgument);
    }
    #[cfg(lc_libraw)]
    {
        let (raw, samples) = ffi::open(bytes, unpack, MAX_RAW_MB)?;
        let info = convert(&raw, unpack)?;
        Ok(Decoded { info, samples })
    }
    #[cfg(not(lc_libraw))]
    {
        let _ = (unpack, MAX_RAW_MB);
        Err(Error::Unavailable)
    }
}

/// The shim's report, copied out (plain data).
#[derive(Clone, Debug, Default)]
#[cfg_attr(not(lc_libraw), allow(dead_code))]
struct RawReport {
    raw_width: u32,
    raw_height: u32,
    width: u32,
    height: u32,
    top_margin: u32,
    left_margin: u32,
    crop: [u32; 4],
    flip: i32,
    colors: i32,
    filters: u32,
    xtrans: [[i8; 6]; 6],
    cdesc: String,
    is_foveon: bool,
    channels: u32,
    is_float: bool,
    black: u32,
    maximum: u32,
    raw_bps: u32,
    linear_max: [u32; 4],
    cblack: Vec<u32>,
    cam_mul: [f32; 4],
    cam_xyz: [[f32; 3]; 4],
    dng_colormatrix: [[[f32; 3]; 4]; 2],
    dng_illuminant: [u32; 2],
    iso: f32,
    shutter: f32,
    aperture: f32,
    focal: f32,
    timestamp: i64,
    dng_version: u32,
    make: String,
    model: String,
    normalized_make: String,
    normalized_model: String,
    software: String,
    lens: String,
    lens_make: String,
    decoder: String,
    data_errors: u32,
}

/// LibRaw `flip` → Exif orientation (dcraw's `"50132467"` table inverted).
fn exif_orientation(flip: i32) -> u16 {
    match flip {
        1 => 2,
        2 => 4,
        3 => 3,
        4 => 5,
        5 => 8,
        6 => 6,
        7 => 7,
        _ => 1,
    }
}

/// LibRaw colour index at visible-area (row, col) → LightCraft's 0 R / 1 G / 2 B.
struct Colors<'a> {
    r: &'a RawReport,
    /// LibRaw colour index → R/G/B.
    map: [u8; 4],
}

impl Colors<'_> {
    fn libraw_index(&self, row: usize, col: usize) -> usize {
        if self.r.filters == 9 {
            let v = self.r.xtrans[row % 6][col % 6];
            return usize::try_from(v).unwrap_or(0).min(3);
        }
        // dcraw's FC(): 8 rows × 2 columns of 2-bit indices
        let shift = (((row << 1) & 14) | (col & 1)) << 1;
        ((self.r.filters >> shift) & 3) as usize
    }
    fn rgb(&self, row: usize, col: usize) -> u8 {
        self.map[self.libraw_index(row, col)]
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn lcm(a: usize, b: usize) -> usize {
    if a == 0 || b == 0 { a.max(b) } else { a / gcd(a, b) * b }
}

/// The largest black-level repeat kept as a pattern (LCM of the CFA and LibRaw's pattern).
const MAX_BLACK_REPEAT: usize = 48;

#[cfg_attr(not(lc_libraw), allow(dead_code))]
fn convert(r: &RawReport, unpacked: bool) -> Result<Info> {
    let (rw, rh) = (r.raw_width as usize, r.raw_height as usize);
    let (w, h) = (r.width as usize, r.height as usize);
    let (left, top) = (r.left_margin as usize, r.top_margin as usize);
    if rw == 0 || rh == 0 || w == 0 || h == 0 {
        return Err(Error::Unsupported("no image size".into()));
    }
    if r.is_foveon {
        return Err(Error::Unsupported("Foveon sensors aren't developed".into()));
    }
    if r.is_float {
        return Err(Error::Unsupported("floating-point data".into()));
    }
    // LibRaw keeps going on corrupt data (zeros or noise where it couldn't decode): not a photo
    if unpacked && r.data_errors > 0 {
        return Err(Error::Corrupt(format!("{} data error(s) while unpacking", r.data_errors)));
    }
    let channels = if unpacked { r.channels as usize } else { 0 };
    if unpacked && !(1..=4).contains(&channels) {
        return Err(Error::Unsupported(format!("{channels} samples per pixel")));
    }
    if rw.checked_mul(rh).and_then(|n| n.checked_mul(channels.max(1))).is_none_or(|n| n > MAX_SAMPLES) {
        return Err(Error::TooLarge);
    }
    // the visible area, clipped to the frame
    let vx = left.min(rw - 1);
    let vy = top.min(rh - 1);
    let visible = Area { x: vx, y: vy, width: w.min(rw - vx), height: h.min(rh - vy) };

    // colour layout
    let mut map = [0u8; 4];
    let desc: Vec<char> = r.cdesc.chars().collect();
    let cfa_colors = match r.filters {
        0 => None,
        1 => return Err(Error::Unsupported("Leaf 16×16 color filter".into())),
        _ => Some(()),
    };
    if cfa_colors.is_some() {
        for (i, m) in map.iter_mut().enumerate() {
            *m = match desc.get(i) {
                Some('R') => 0,
                Some('G') => 1,
                Some('B') => 2,
                // unused slots (3-colour sensors without a 4th index)
                None if i == 3 => 1,
                other => return Err(Error::Unsupported(format!("color filter {:?} ({other:?})", r.cdesc))),
            };
        }
        if r.colors != 3 {
            return Err(Error::Unsupported(format!("{}-color sensor", r.colors)));
        }
    }
    let colors = Colors { r, map };
    // CFA period: X-Trans 6×6, Bayer 2×2 unless the 8×2 pattern doesn't repeat every two rows
    let (cw, ch) = match r.filters {
        0 => (1, 1),
        9 => (6, 6),
        f if f == (f & 0xff).wrapping_mul(0x0101_0101) => (2, 2),
        _ => (2, 8),
    };
    let cfa = cfa_colors.map(|()| {
        let pattern = (0..ch)
            .flat_map(|y| (0..cw).map(move |x| (x, y)))
            .map(|(x, y)| {
                // full-frame (x, y) → visible-area coordinates, within the period
                let vxp = (x as i64 - visible.x as i64).rem_euclid(cw as i64) as usize;
                let vyp = (y as i64 - visible.y as i64).rem_euclid(ch as i64) as usize;
                colors.rgb(vyp, vxp)
            })
            .collect();
        Cfa { width: cw, height: ch, pattern }
    });

    // black: black + cblack[colour] + cblack[6 + (row % pr) * pc + col % pc]
    let cb = |i: usize| r.cblack.get(i).copied().unwrap_or(0) as f32;
    let (pr, pc) = (r.cblack.get(4).copied().unwrap_or(0) as usize, r.cblack.get(5).copied().unwrap_or(0) as usize);
    let pattern_ok = pr > 0 && pc > 0 && pr.checked_mul(pc).is_some_and(|n| 6 + n <= r.cblack.len());
    let pat = |row: usize, col: usize| if pattern_ok { cb(6 + (row % pr) * pc + col % pc) } else { 0.0 };
    let black = if cfa.is_some() {
        let (mut rows, mut cols) = (ch, cw);
        let mut extra = 0.0;
        if pattern_ok {
            let (lr, lc) = (lcm(ch, pr), lcm(cw, pc));
            if lr <= MAX_BLACK_REPEAT && lc <= MAX_BLACK_REPEAT {
                (rows, cols) = (lr, lc);
            } else {
                // too large to repeat: its mean
                extra = (0..pr * pc).map(|i| cb(6 + i)).sum::<f32>() / (pr * pc) as f32;
            }
        }
        let values = (0..rows)
            .flat_map(|y| (0..cols).map(move |x| (x, y)))
            .map(|(x, y)| r.black as f32 + cb(colors.libraw_index(y, x)) + if extra != 0.0 { extra } else { pat(y, x) })
            .collect();
        Black { rows, cols, values }
    } else {
        let n = channels.max(1);
        let common = if pattern_ok { (0..pr * pc).map(|i| cb(6 + i)).sum::<f32>() / (pr * pc) as f32 } else { 0.0 };
        Black { rows: 1, cols: 1, values: (0..n).map(|s| r.black as f32 + cb(s.min(3)) + common).collect() }
    };

    // the camera's crop, when it lies inside the frame
    let [cl, ct, cwid, chei] = r.crop.map(|v| v as usize);
    let default_crop = (cl < 0xffff && ct < 0xffff && cwid > 1 && chei > 1 && cl.saturating_add(cwid) <= rw && ct.saturating_add(chei) <= rh)
        .then_some(Area { x: cl, y: ct, width: cwid, height: chei });

    let m = r.cam_mul;
    let as_shot_multipliers = (m[1] > 0.0 && m[..3].iter().all(|v| v.is_finite() && *v > 0.0))
        .then(|| [m[0] / m[1], 1.0, m[2] / m[1]])
        .filter(|v| v.iter().all(|x| x.is_finite()));
    let usable = |m: &[[f32; 3]; 4]| m[..3].iter().flatten().all(|v| v.is_finite()) && m[..3].iter().flatten().any(|v| *v != 0.0);
    // LibRaw's table (Adobe's D65 matrices), else a DNG's own ColorMatrix for D65 (21), else its first
    let dng = (0..2).filter(|&k| r.dng_illuminant[k] != 0 && usable(&r.dng_colormatrix[k])).min_by_key(|&k| u32::from(r.dng_illuminant[k] != 21));
    let xyz = if usable(&r.cam_xyz) { Some(r.cam_xyz) } else { dng.map(|k| r.dng_colormatrix[k]) };
    let xyz_to_camera = xyz.map(|m| [0, 1, 2].map(|i| m[i].map(f64::from)));
    let bits = if (1..=16).contains(&r.raw_bps) { r.raw_bps } else { 32 - r.maximum.max(1).leading_zeros() };
    // Canon's maker notes record where the sensor stops being linear (SpecularWhiteLevel), below
    // the format's maximum: the white Camera Raw / Lightroom and LightCraft's own CR2/CR3 decoders use
    let min_black = black.values.iter().copied().fold(f32::INFINITY, f32::min);
    let linear = r.linear_max[..3].iter().copied().min().unwrap_or(0);
    let white = if r.make.to_ascii_lowercase().starts_with("canon") && linear > 0 && linear <= r.maximum && linear as f32 > min_black {
        linear
    } else {
        r.maximum
    };
    Ok(Info {
        raw_width: rw,
        raw_height: rh,
        visible,
        default_crop,
        orientation: exif_orientation(r.flip),
        cfa,
        channels,
        black,
        white,
        bits: bits.min(16),
        as_shot_multipliers,
        xyz_to_camera,
        make: r.make.clone(),
        model: r.model.clone(),
        normalized_make: r.normalized_make.clone(),
        normalized_model: r.normalized_model.clone(),
        software: r.software.clone(),
        lens: r.lens.clone(),
        lens_make: r.lens_make.clone(),
        iso: r.iso,
        shutter: r.shutter,
        aperture: r.aperture,
        focal_length: r.focal,
        timestamp: r.timestamp,
        dng_version: r.dng_version,
        decoder: r.decoder.clone(),
    })
}

#[cfg(lc_libraw)]
#[allow(unsafe_code)]
mod ffi {
    use super::{Error, RawReport, Result};
    use std::ffi::{CStr, c_char, c_int, c_void};

    pub const CBLACK: usize = 4104;

    #[repr(C)]
    pub struct LcInfo {
        pub raw_width: u32,
        pub raw_height: u32,
        pub width: u32,
        pub height: u32,
        pub top_margin: u32,
        pub left_margin: u32,
        pub crop_left: u32,
        pub crop_top: u32,
        pub crop_width: u32,
        pub crop_height: u32,
        pub flip: i32,
        pub colors: i32,
        pub filters: u32,
        pub xtrans: [[c_char; 6]; 6],
        pub cdesc: [c_char; 8],
        pub dng_version: u32,
        pub is_foveon: u32,
        pub channels: u32,
        pub is_float: u32,
        pub black: u32,
        pub maximum: u32,
        pub data_maximum: u32,
        pub raw_bps: u32,
        pub linear_max: [u32; 4],
        pub cblack: [u32; CBLACK],
        pub cam_mul: [f32; 4],
        pub pre_mul: [f32; 4],
        pub cam_xyz: [[f32; 3]; 4],
        pub rgb_cam: [[f32; 4]; 3],
        pub dng_colormatrix: [[[f32; 3]; 4]; 2],
        pub dng_illuminant: [u32; 2],
        pub iso_speed: f32,
        pub shutter: f32,
        pub aperture: f32,
        pub focal_len: f32,
        pub timestamp: i64,
        pub make: [c_char; 64],
        pub model: [c_char; 64],
        pub normalized_make: [c_char; 64],
        pub normalized_model: [c_char; 64],
        pub software: [c_char; 64],
        pub lens: [c_char; 128],
        pub lens_make: [c_char; 128],
        pub data_errors: u32,
        pub decoder: [c_char; 64],
    }

    unsafe extern "C" {
        pub fn lc_libraw_open(data: *const u8, len: usize, unpack: c_int, max_mb: u32, out: *mut LcInfo, err: *mut c_int) -> *mut c_void;
        pub fn lc_libraw_copy_samples(handle: *mut c_void, dst: *mut u16, n: usize) -> c_int;
        pub fn lc_libraw_close(handle: *mut c_void);
        pub fn lc_libraw_version() -> *const c_char;
        pub fn lc_libraw_camera_count() -> c_int;
        pub fn lc_libraw_strerror(code: c_int) -> *const c_char;
    }

    pub fn version() -> Option<String> {
        // SAFETY: returns a pointer to a static NUL-terminated string (or NULL).
        c_str(unsafe { lc_libraw_version() })
    }

    pub fn camera_count() -> c_int {
        // SAFETY: no arguments; reads a static table.
        unsafe { lc_libraw_camera_count() }
    }

    fn c_str(p: *const c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: LibRaw's version / error strings are static and NUL-terminated.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }

    /// A fixed-size C string field (the shim always NUL-terminates; a missing NUL ends at the array).
    #[allow(clippy::unnecessary_cast)] // `c_char` is i8 or u8 by platform
    fn field(chars: &[c_char]) -> String {
        let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }

    fn error(code: c_int) -> Error {
        match code {
            1 => Error::BadArgument,
            2 => Error::Failed("an exception"),
            3 => Error::Failed("out of memory"),
            4 => Error::Failed("no samples unpacked"),
            // LIBRAW_UNSUFFICIENT_MEMORY / LIBRAW_TOO_BIG / LIBRAW_MEMPOOL_OVERFLOW
            -100007 | -100012 | -100013 => Error::TooLarge,
            c => {
                // SAFETY: returns a static string for any code.
                let message = c_str(unsafe { lc_libraw_strerror(c) }).unwrap_or_else(|| format!("error {c}"));
                Error::LibRaw { code: c, message }
            }
        }
    }

    /// The shim's handle, closed on drop.
    struct Handle(*mut c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from `lc_libraw_open` and is closed exactly once.
            unsafe { lc_libraw_close(self.0) }
        }
    }

    pub fn open(bytes: &[u8], unpack: bool, max_mb: u32) -> Result<(RawReport, Vec<u16>)> {
        // `LcInfo` is ~17 KB: on the heap, zeroed (all fields are plain integers / floats / chars)
        let mut info: Box<LcInfo> = Box::new(LcInfo {
            raw_width: 0,
            raw_height: 0,
            width: 0,
            height: 0,
            top_margin: 0,
            left_margin: 0,
            crop_left: 0,
            crop_top: 0,
            crop_width: 0,
            crop_height: 0,
            flip: 0,
            colors: 0,
            filters: 0,
            xtrans: [[0; 6]; 6],
            cdesc: [0; 8],
            dng_version: 0,
            is_foveon: 0,
            channels: 0,
            is_float: 0,
            black: 0,
            maximum: 0,
            data_maximum: 0,
            raw_bps: 0,
            linear_max: [0; 4],
            cblack: [0; CBLACK],
            cam_mul: [0.0; 4],
            pre_mul: [0.0; 4],
            cam_xyz: [[0.0; 3]; 4],
            rgb_cam: [[0.0; 4]; 3],
            dng_colormatrix: [[[0.0; 3]; 4]; 2],
            dng_illuminant: [0; 2],
            iso_speed: 0.0,
            shutter: 0.0,
            aperture: 0.0,
            focal_len: 0.0,
            timestamp: 0,
            make: [0; 64],
            model: [0; 64],
            normalized_make: [0; 64],
            normalized_model: [0; 64],
            software: [0; 64],
            lens: [0; 128],
            lens_make: [0; 128],
            data_errors: 0,
            decoder: [0; 64],
        });
        let mut err: c_int = 0;
        // SAFETY: `bytes` outlives the handle (closed before this function returns); `info` and
        // `err` are valid, exclusively borrowed out-pointers.
        let h = unsafe { lc_libraw_open(bytes.as_ptr(), bytes.len(), c_int::from(unpack), max_mb, &mut *info, &mut err) };
        if h.is_null() {
            return Err(error(if err == 0 { 2 } else { err }));
        }
        let handle = Handle(h);
        let i = &*info;
        let report = RawReport {
            raw_width: i.raw_width,
            raw_height: i.raw_height,
            width: i.width,
            height: i.height,
            top_margin: i.top_margin,
            left_margin: i.left_margin,
            crop: [i.crop_left, i.crop_top, i.crop_width, i.crop_height],
            flip: i.flip,
            colors: i.colors,
            filters: i.filters,
            // `c_char` is i8 or u8 by platform
            #[allow(clippy::unnecessary_cast)]
            xtrans: i.xtrans.map(|r| r.map(|c| c as i8)),
            cdesc: field(&i.cdesc),
            is_foveon: i.is_foveon != 0,
            channels: i.channels,
            is_float: i.is_float != 0,
            black: i.black,
            maximum: i.maximum,
            raw_bps: i.raw_bps,
            linear_max: i.linear_max,
            cblack: i.cblack.to_vec(),
            cam_mul: i.cam_mul,
            cam_xyz: i.cam_xyz,
            dng_colormatrix: i.dng_colormatrix,
            dng_illuminant: i.dng_illuminant,
            iso: i.iso_speed,
            shutter: i.shutter,
            aperture: i.aperture,
            focal: i.focal_len,
            timestamp: i.timestamp,
            dng_version: i.dng_version,
            make: field(&i.make),
            model: field(&i.model),
            normalized_make: field(&i.normalized_make),
            normalized_model: field(&i.normalized_model),
            software: field(&i.software),
            lens: field(&i.lens),
            lens_make: field(&i.lens_make),
            decoder: field(&i.decoder),
            data_errors: i.data_errors,
        };
        if !unpack || report.is_float || report.channels == 0 {
            return Ok((report, Vec::new()));
        }
        let n = (report.raw_width as usize)
            .checked_mul(report.raw_height as usize)
            .and_then(|n| n.checked_mul(report.channels as usize))
            .filter(|&n| n > 0 && n <= super::MAX_SAMPLES)
            .ok_or(Error::TooLarge)?;
        let mut samples: Vec<u16> = Vec::new();
        samples.try_reserve_exact(n).map_err(|_| Error::TooLarge)?;
        samples.resize(n, 0);
        // SAFETY: `samples` holds exactly `n` u16s; the shim writes at most `n` (it checks `n`
        // against the frame size and returns an error otherwise).
        let rc = unsafe { lc_libraw_copy_samples(handle.0, samples.as_mut_ptr(), n) };
        drop(handle);
        if rc != 0 {
            return Err(error(rc));
        }
        Ok((report, samples))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> RawReport {
        let mut cblack = vec![0u32; 4104];
        cblack[0] = 2; // R
        cblack[2] = 4; // B
        RawReport {
            raw_width: 20,
            raw_height: 12,
            width: 16,
            height: 10,
            top_margin: 1,
            left_margin: 3,
            crop: [0xffff, 0xffff, 0, 0],
            colors: 3,
            // RGGB at the visible origin (dcraw 0x94949494), with G2 marked 3: 0x94949494 | …
            filters: 0x9494_9494,
            cdesc: "RGBG".into(),
            channels: 1,
            black: 100,
            maximum: 16383,
            raw_bps: 14,
            cblack,
            cam_mul: [2.0, 1.0, 1.5, 1.0],
            cam_xyz: [[0.5, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.8], [0.0; 3]],
            flip: 6,
            ..Default::default()
        }
    }

    #[test]
    fn converts_levels_cfa_and_orientation() {
        let i = convert(&report(), true).unwrap();
        assert_eq!(i.visible, Area { x: 3, y: 1, width: 16, height: 10 });
        assert_eq!(i.orientation, 6);
        assert_eq!(i.as_shot_multipliers, Some([2.0, 1.0, 1.5]));
        assert_eq!(i.white, 16383);
        assert_eq!(i.bits, 14);
        let cfa = i.cfa.unwrap();
        // RGGB at visible (0, 0) = full frame (3, 1): the full-frame origin is one column and one row before
        let at = |x: usize, y: usize| cfa.pattern[(y % cfa.height) * cfa.width + x % cfa.width];
        assert_eq!(at(3, 1), 0);
        assert_eq!(at(4, 1), 1);
        assert_eq!(at(4, 2), 2);
        assert_eq!(i.black.rows * i.black.cols, 4);
        assert_eq!(i.black.values, vec![102.0, 100.0, 100.0, 104.0]);
        assert!(i.default_crop.is_none());
        assert_eq!(i.xyz_to_camera.unwrap()[2][2], 0.800000011920929);
    }

    #[test]
    fn black_pattern_repeats_with_the_cfa() {
        let mut r = report();
        r.cblack[4] = 1;
        r.cblack[5] = 3;
        r.cblack[6..9].copy_from_slice(&[1, 2, 3]);
        let i = convert(&r, true).unwrap();
        assert_eq!((i.black.rows, i.black.cols), (2, 6));
        assert_eq!(i.black.values[0], 103.0); // R + pattern[0]
        assert_eq!(i.black.values[1], 102.0); // G + pattern[1]
        assert_eq!(i.black.values[2], 105.0); // R + pattern[2]
        // a pattern too large to repeat falls back to its mean
        r.cblack[4] = 7;
        r.cblack[5] = 50;
        assert_eq!(convert(&r, true).unwrap().black.rows, 2);
    }

    #[test]
    fn xtrans_and_hostile_reports() {
        let mut r = report();
        r.filters = 9;
        r.xtrans = [[1, 1, 0, 1, 1, 2], [1, 1, 2, 1, 1, 0], [2, 0, 1, 0, 2, 1], [1, 1, 2, 1, 1, 0], [1, 1, 0, 1, 1, 2], [0, 2, 1, 2, 0, 1]];
        let i = convert(&r, true).unwrap();
        let c = i.cfa.unwrap();
        assert_eq!((c.width, c.height), (6, 6));
        assert_eq!(c.pattern.iter().filter(|&&p| p == 1).count(), 20);
        assert_eq!((i.black.rows, i.black.cols), (6, 6));
        // garbage from a broken file never panics
        r.xtrans = [[-7; 6]; 6];
        assert!(convert(&r, true).is_ok());
        for bad in [
            RawReport { raw_width: 0, ..report() },
            RawReport { channels: 9, ..report() },
            RawReport { is_float: true, ..report() },
            RawReport { filters: 1, ..report() },
            RawReport { cdesc: "GMCY".into(), ..report() },
            RawReport { colors: 4, ..report() },
            RawReport { data_errors: 1, ..report() },
            RawReport { raw_width: u32::MAX, raw_height: u32::MAX, ..report() },
        ] {
            assert!(convert(&bad, true).is_err(), "{bad:?}");
        }
        // margins past the frame are clipped, not trusted
        let i = convert(&RawReport { left_margin: 1000, top_margin: 1000, width: 5000, ..report() }, true).unwrap();
        assert!(i.visible.x + i.visible.width <= 20 && i.visible.y + i.visible.height <= 12);
        // a crop outside the frame is dropped; a valid one kept
        assert!(convert(&RawReport { crop: [10, 1, 30, 5], ..report() }, true).unwrap().default_crop.is_none());
        assert_eq!(
            convert(&RawReport { crop: [4, 2, 12, 8], ..report() }, true).unwrap().default_crop,
            Some(Area { x: 4, y: 2, width: 12, height: 8 })
        );
        // zero / NaN multipliers and an all-zero matrix mean "unknown"
        let i = convert(&RawReport { cam_mul: [0.0, 0.0, 0.0, 0.0], cam_xyz: [[0.0; 3]; 4], ..report() }, true).unwrap();
        assert!(i.as_shot_multipliers.is_none() && i.xyz_to_camera.is_none());
        assert!(convert(&RawReport { cam_mul: [f32::NAN, 1.0, 1.0, 0.0], ..report() }, true).unwrap().as_shot_multipliers.is_none());
    }

    #[test]
    fn orientation_table() {
        assert_eq!([0, 1, 2, 3, 4, 5, 6, 7, 99, -1].map(exif_orientation), [1, 2, 4, 3, 5, 8, 6, 7, 1, 1]);
    }

    #[test]
    fn empty_input_is_an_error() {
        assert_eq!(decode(&[]), Err(Error::BadArgument));
        assert_eq!(identify(&[]), Err(Error::BadArgument));
    }

    #[test]
    fn without_libraw_everything_is_unavailable() {
        if available() {
            assert!(camera_count() > 500, "LibRaw lists {} cameras", camera_count());
            assert!(version().is_some_and(|v| v.starts_with("0.")));
            return;
        }
        assert_eq!(decode(b"II*\0 anything"), Err(Error::Unavailable));
        assert_eq!(identify(b"II*\0 anything"), Err(Error::Unavailable));
        assert_eq!(camera_count(), 0);
        assert!(version().is_none());
    }
}
