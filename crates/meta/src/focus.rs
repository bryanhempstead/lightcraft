//! Where the camera focused: the autofocus point a camera records in its maker notes, as a box on the upright photo
//! ([`FocusPoint`]). Select Subject uses it to pick the subject the photographer focused on.
//!
//! Tag identities and layouts come from the public ExifTool tag-name documentation (Sony, Canon AFInfo2, Nikon
//! AFInfo2V0100 / V0300 / V0400 and FujiFilm tables); no ExifTool code was read. The Sony reader is upstream
//! LightCraft's (storytold/lightcraft `crates/meta/src/focus.rs`, LR-VIEW-FOCUSPOINT, same licence), whose layouts
//! were confirmed on real ILCE-1 (α1) and ILCE-7SM3 ARWs.
//!
//! **Sony** (the MakerNote IFD: header-less in ARWs, after a `SONY DSC` header in JPEGs):
//!
//! | Tag | Layout | Use |
//! |---|---|---|
//! | `0x2027` FocusLocation | 4 × int16u: the frame's width and height, then the focus centre x, y in its pixels (the sensor's landscape frame) | the point |
//! | `0x2037` FocusFrameSize | int16u width, int16u height of the frame shown, then 2 bytes that are zero when there was none | the box size |
//! | `0x201b` FocusMode | int8u: 0 Manual, 2 AF-S, 3 AF-C, 4 AF-A, 6 DMF, 7 AF-D | manual = no point |
//! | `0x201c` AFAreaModeSetting, `0x2021` AFTracking | int8u | words |
//! | `0x9402` (enciphered: stored `p³ mod 249`) bytes 0x16, 0x17 | focus mode and the area mode actually used (α1 / A7S III layout only) | Multi at the exact centre = no point |
//!
//! **Canon** (`0x0026` AFInfo2, `0x003c` AFInfo3 on newer bodies; the same layout, int16 values): AFInfoSize,
//! AFAreaMode (0 = manual focus), NumAFPoints *n*, ValidAFPoints *v*, CanonImageWidth / Height, AFImageWidth / Height,
//! then *n* widths, *n* heights, *n* x and *n* y positions — relative to the AF image's centre, **y up** on EOS bodies
//! (ExifTool's note) — then the AFPointsInFocus and AFPointsSelected bit masks (⌈*n* / 16⌉ words each, point *i* at
//! bit *i* mod 16 of word *i* / 16). The box covers the points in focus, else the selected ones, else the only valid
//! one. Checked on Bryan's EOS R6 CR3s (Spot AF at the centre; Face + Tracking with seven points in focus).
//!
//! **Nikon** (`0x00b7` AFInfo2, its version in the first four bytes, values in the maker note's byte order):
//! `0100` (D3 – D810 generation): byte 4 AFDetectionMethod (1 = contrast detect: only then are the coordinates
//! valid), AFImageWidth / Height, AFAreaXPosition / YPosition (the area's centre) and AFAreaWidth / Height as int16u at
//! bytes 16 – 26; `0300` (Expeed 6: Z5 – Z7 II, Z50, Zfc, D780, D6): byte 7 AFCoordinatesAvailable, the same six values
//! at bytes 42 – 52; `0400` (Expeed 7: Z8, Z9, Zf, Z6 III…): byte 7, the values at bytes 62 – 72. Other versions,
//! phase-detect points named on a grid, and `MANUAL` FocusMode (`0x0007`) give no point.
//!
//! **Fujifilm** (`FUJIFILM` maker note, little-endian): `0x1023` FocusPixel (int16u x, y in the full-size image's
//! pixels); `0x1021` FocusMode 1 = manual gives none. The box is a small square around it (no size is recorded).
//!
//! The Nikon and Fujifilm readers follow the documentation only: no file from those cameras was at hand to check them
//! (their tests are synthetic). Anything malformed, out of range or missing is no point — never a panic.

use crate::{Orientation, Rect};
use lightcraft_tiff::{ByteOrder, Ifd, Tiff, makernote, tags as t};
use serde::{Deserialize, Serialize};

/// Where the camera focused, on the upright photo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusPoint {
    /// The focus frame, normalized to the photo's full, EXIF-oriented frame (y down, 0..1 on both
    /// axes, clipped to it) — the frame face regions use, so it follows Rotate, flips and crops.
    pub rect: Rect,
    /// Focus mode as the camera names it (`AF-C`, `AF-S`, `DMF`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// The AF area chosen on the camera (`Wide`, `Zone`, `Flexible Spot`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// What the camera followed, when it says (`Animal Eye Tracking`, `Face tracking`, `Lock On AF`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<String>,
}

impl FocusPoint {
    /// The box's centre (normalized, upright).
    pub fn center(&self) -> (f64, f64) {
        ((self.rect.x0 + self.rect.x1) / 2.0, (self.rect.y0 + self.rect.y1) / 2.0)
    }
    /// The words that describe it, in order (mode, area, tracking), for "AF-C · Wide · Animal Eye Tracking".
    pub fn words(&self) -> Vec<&str> {
        [&self.mode, &self.area, &self.tracking].into_iter().filter_map(|s| s.as_deref()).filter(|s| !s.is_empty()).collect()
    }
    /// The box clipped to the frame and checked: `None` when it is not a usable box (non-finite,
    /// empty, or entirely outside the photo).
    pub fn sanitized(mut self) -> Option<FocusPoint> {
        let r = self.rect;
        if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (x0, x1) = (r.x0.min(r.x1).clamp(0.0, 1.0), r.x0.max(r.x1).clamp(0.0, 1.0));
        let (y0, y1) = (r.y0.min(r.y1).clamp(0.0, 1.0), r.y0.max(r.y1).clamp(0.0, 1.0));
        if x1 - x0 <= 0.0 || y1 - y0 <= 0.0 {
            return None;
        }
        self.rect = Rect { x0, y0, x1, y1 };
        Some(self)
    }
}

/// Sony bodies whose `0x9402` block layout has been checked against real files (its area-mode byte is read only on
/// these; the plain tags are read on every body that writes them).
pub const SONY_CONFIRMED: &[&str] = &["ILCE-1", "ILCE-7SM3"];

/// Sony model families that write FocusLocation (ExifTool: SLT, ILCA, ILCE and the RX cameras; not NEX).
fn sony_family(model: &str) -> bool {
    ["ILCE-", "ILCA-", "SLT-", "DSC-RX"].iter().any(|p| model.starts_with(p))
}

/// Side of the box drawn when the camera gave a point but no frame size, as a fraction of the frame's short side.
const DEFAULT_SIDE: f64 = 0.03;
/// Files are never read past this for a focus point.
pub const MAX_FILE: usize = 512 << 20;

/// The focus point a whole file records (a TIFF-based raw — ARW, CR2, NEF, DNG —, a JPEG, a Canon CR3 or a
/// Fujifilm RAF), or `None`.
pub fn read_file(bytes: &[u8]) -> Option<FocusPoint> {
    if bytes.len() > MAX_FILE {
        return None;
    }
    if let Some(c) = crate::cr3::parse_cr3(bytes) {
        return cr3(&c);
    }
    if bytes.starts_with(b"FUJIFILMCCD-RAW") {
        let exif = crate::container::embedded(bytes).exif?;
        return read_exif(&exif);
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let exif = crate::container::jpeg_segments(bytes).exif?;
        return read_exif(&exif);
    }
    read_exif(bytes)
}

/// The focus point of a TIFF-structured Exif stream (a whole TIFF-based raw, or a JPEG's Exif block).
pub fn read_exif(data: &[u8]) -> Option<FocusPoint> {
    let data = crate::exif::strip_exif_header(data);
    let tiff = Tiff::parse(data).ok()?;
    let ifd0 = tiff.ifds.first()?;
    let make = ifd0.string(t::MAKE)?;
    let model = ifd0.string(t::MODEL).unwrap_or_default();
    let orientation = ifd0.u16(t::ORIENTATION).filter(|v| (1..=8).contains(v)).map(Orientation::from_exif).unwrap_or_default();
    read(data, &tiff, &make, &model, orientation)
}

/// The focus point a file's maker note records, or `None`. `data` is the TIFF stream the maker note's offsets refer
/// to (a whole raw, or an Exif block).
pub fn read(data: &[u8], tiff: &Tiff, make: &str, model: &str, orientation: Orientation) -> Option<FocusPoint> {
    let e = tiff.exif()?.get(t::MAKER_NOTE)?;
    let mn = makernote::parse_makernote(data, e.offset, e.count() as u64, tiff.order, make)?;
    let upper = make.trim().to_ascii_uppercase();
    let model = model.trim();
    if upper.starts_with("SONY") {
        if !sony_family(model) || !matches!(mn.kind, makernote::MakerNoteKind::Sony | makernote::MakerNoteKind::Plain) {
            return None;
        }
        return sony(&mn.ifd, mn.order, orientation, SONY_CONFIRMED.contains(&model));
    }
    if upper.starts_with("CANON") {
        return canon(&mn.ifd, orientation);
    }
    if upper.starts_with("NIKON") {
        return nikon(&mn.ifd, mn.order, orientation);
    }
    if upper.starts_with("FUJIFILM") {
        let exif = tiff.exif()?;
        let size = (exif.u32(t::PIXEL_X_DIMENSION)?, exif.u32(t::PIXEL_Y_DIMENSION)?);
        return fuji(&mn.ifd, size, orientation);
    }
    None
}

/// A Canon CR3: CMT1 (IFD0: make, model, orientation) and CMT3 (the maker note as its own TIFF stream).
fn cr3(c: &crate::cr3::Cr3<'_>) -> Option<FocusPoint> {
    let ifd0 = Tiff::parse(c.cmt[0]?).ok()?;
    let ifd0 = ifd0.ifds.first()?;
    if !ifd0.string(t::MAKE)?.trim().to_ascii_uppercase().starts_with("CANON") {
        return None;
    }
    let orientation = ifd0.u16(t::ORIENTATION).filter(|v| (1..=8).contains(v)).map(Orientation::from_exif).unwrap_or_default();
    let note = Tiff::parse(c.cmt[2]?).ok()?;
    canon(note.ifds.first()?, orientation)
}

fn byte(ifd: &Ifd, tag: u16) -> Option<u8> {
    ifd.value(tag)?.get_u64(0).and_then(|v| u8::try_from(v).ok())
}

const FOCUS_MODE: u16 = 0x201b;
const AF_AREA_SETTING: u16 = 0x201c;
const AF_TRACKING: u16 = 0x2021;
const FOCUS_LOCATION: u16 = 0x2027;
const FOCUS_FRAME_SIZE: u16 = 0x2037;
const TAG_9402: u16 = 0x9402;

/// Decode the Sony tags of a parsed maker-note IFD (`confirmed`: the body's `0x9402` layout is known).
pub(crate) fn sony(ifd: &Ifd, order: ByteOrder, orientation: Orientation, confirmed: bool) -> Option<FocusPoint> {
    let loc = ifd.value(FOCUS_LOCATION)?;
    let [w, h, x, y] = [0, 1, 2, 3].map(|i| loc.get_u64(i));
    let (w, h, x, y) = (w? as f64, h? as f64, x? as f64, y? as f64);
    if w < 1.0 || h < 1.0 || x > w || y > h {
        return None;
    }
    let mode = byte(ifd, FOCUS_MODE);
    if mode == Some(0) {
        return None;
    }
    // the enciphered block: its focus mode must agree with the plain one, else its layout is not the one we know
    let block = ifd.bytes(TAG_9402).filter(|_| confirmed).and_then(|b| {
        let (m, area) = (decipher(*b.get(0x16)?) & 0x7f, decipher(*b.get(0x17)?));
        (Some(m) == mode).then_some(area)
    });
    if block == Some(255) {
        return None;
    }
    let setting = byte(ifd, AF_AREA_SETTING);
    let tracking = byte(ifd, AF_TRACKING);
    let wide_untracked = match block {
        Some(a) => a == 0,
        None => setting == Some(0) && tracking.is_none_or(|t| t == 0),
    };
    if wide_untracked && x * 2.0 == w && y * 2.0 == h {
        return None;
    }
    // frame size: two int16u in the note's byte order, then a non-zero marker when there is a frame
    let size = ifd.bytes(FOCUS_FRAME_SIZE).and_then(|b| {
        let (fw, fh) = (order.read_u16(b, 0)? as f64, order.read_u16(b, 2)? as f64);
        let shown = b.get(4..6).is_none_or(|m| m.iter().any(|&v| v != 0));
        (shown && fw >= 1.0 && fh >= 1.0).then_some((fw, fh))
    });
    let (fw, fh) = size.unwrap_or_else(|| {
        let side = w.min(h) * DEFAULT_SIDE;
        (side, side)
    });
    let sensor = Rect { x0: (x - fw / 2.0) / w, y0: (y - fh / 2.0) / h, x1: (x + fw / 2.0) / w, y1: (y + fh / 2.0) / h };
    let name = |v: Option<u8>, f: fn(u8) -> Option<&'static str>| v.and_then(f).map(str::to_string);
    let area = name(setting, area_setting_name);
    let used = name(block, area_used_name);
    let tracking = used.filter(|u| Some(u) != area.as_ref() && u != "Multi").or_else(|| name(tracking, tracking_name).filter(|t| t != "Off"));
    FocusPoint { rect: orientation.map_norm_rect(sensor), mode: name(mode, focus_mode_name), area, tracking }.sanitized()
}

/// Undo the `0x94xx` byte substitution: the stored byte is `p³ mod 249` for `p < 249`.
fn decipher(b: u8) -> u8 {
    if b >= 249 {
        return b;
    }
    (0..249u32).find(|p| (p * p * p) % 249 == u32::from(b)).and_then(|p| u8::try_from(p).ok()).unwrap_or(b)
}

fn focus_mode_name(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Manual",
        2 => "AF-S",
        3 => "AF-C",
        4 => "AF-A",
        6 => "DMF",
        7 => "AF-D",
        _ => return None,
    })
}

fn area_setting_name(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Wide",
        1 | 9 => "Center",
        3 | 4 => "Flexible Spot",
        11 => "Zone",
        12 => "Expanded Flexible Spot",
        13 => "Custom AF Area",
        _ => return None,
    })
}

fn tracking_name(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Off",
        1 => "Face tracking",
        2 => "Lock On AF",
        _ => return None,
    })
}

fn area_used_name(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Multi",
        1 => "Center",
        2 => "Spot",
        3 => "Flexible Spot",
        11 => "Zone",
        12 => "Expanded Flexible Spot",
        13 => "Custom AF Area",
        14 => "Tracking",
        15 => "Face Tracking",
        20 => "Animal Eye Tracking",
        21 => "Human Eye Tracking",
        _ => return None,
    })
}

const CANON_AF_INFO2: u16 = 0x0026;
const CANON_AF_INFO3: u16 = 0x003c;
/// More AF points than any camera has: a damaged count.
const MAX_AF_POINTS: usize = 8192;

fn canon_area_name(v: i64) -> Option<&'static str> {
    Some(match v {
        1 => "AF Point Expansion",
        2 => "Single-point AF",
        4 => "Auto",
        5 => "Face Detect AF",
        6 => "Face + Tracking",
        7 => "Zone AF",
        8 | 10 => "AF Point Expansion",
        9 => "Spot AF",
        11 | 12 => "Flexizone Multi",
        13 => "Flexizone Single",
        14 | 16 | 17 => "Large Zone AF",
        19..=21 => "Flexible Zone AF",
        22 => "Whole Area AF",
        _ => return None,
    })
}

/// Canon AFInfo2 / AFInfo3 (see the module notes).
pub(crate) fn canon(ifd: &Ifd, orientation: Orientation) -> Option<FocusPoint> {
    let v = ifd.value(CANON_AF_INFO3).or_else(|| ifd.value(CANON_AF_INFO2))?;
    // int16 values, whichever TIFF type the file gives them
    let at = |i: usize| -> Option<i64> { v.get_u64(i).map(|u| i64::from(u as u16 as i16)).or_else(|| v.get_i64(i)) };
    let mode = at(1)?;
    if mode == 0 {
        return None;
    }
    let n = usize::try_from(at(2)?).ok().filter(|n| (1..=MAX_AF_POINTS).contains(n))?;
    let valid = usize::try_from(at(3)?).ok()?.min(n);
    let (aw, ah) = (at(6)? as f64, at(7)? as f64);
    if valid == 0 || aw < 1.0 || ah < 1.0 {
        return None;
    }
    let words = n.div_ceil(16);
    let base = 8;
    let (widths, heights, xs, ys) = (base, base + n, base + 2 * n, base + 3 * n);
    let (in_focus, selected) = (base + 4 * n, base + 4 * n + words);
    let bit = |mask: usize, i: usize| at(mask + i / 16).is_some_and(|w| (w as u16) & (1 << (i % 16)) != 0);
    let mut points: Vec<usize> = (0..valid).filter(|&i| bit(in_focus, i)).collect();
    if points.is_empty() {
        points = (0..valid).filter(|&i| bit(selected, i)).collect();
    }
    if points.is_empty() && valid == 1 {
        points.push(0);
    }
    if points.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for i in points {
        let (w, h) = (at(widths + i)? as f64, at(heights + i)? as f64);
        let (cx, cy) = (aw / 2.0 + at(xs + i)? as f64, ah / 2.0 - at(ys + i)? as f64);
        let (w, h) = (w.max(1.0), h.max(1.0));
        x0 = x0.min(cx - w / 2.0);
        x1 = x1.max(cx + w / 2.0);
        y0 = y0.min(cy - h / 2.0);
        y1 = y1.max(cy + h / 2.0);
    }
    let sensor = Rect { x0: x0 / aw, y0: y0 / ah, x1: x1 / aw, y1: y1 / ah };
    FocusPoint { rect: orientation.map_norm_rect(sensor), mode: None, area: canon_area_name(mode).map(str::to_string), tracking: None }.sanitized()
}

const NIKON_FOCUS_MODE: u16 = 0x0007;
const NIKON_AF_INFO2: u16 = 0x00b7;

/// Nikon AFInfo2 (see the module notes).
pub(crate) fn nikon(ifd: &Ifd, order: ByteOrder, orientation: Orientation) -> Option<FocusPoint> {
    if ifd.string(NIKON_FOCUS_MODE).is_some_and(|m| m.trim().eq_ignore_ascii_case("MANUAL")) {
        return None;
    }
    let b = ifd.bytes(NIKON_AF_INFO2)?;
    let version = b.get(0..4)?;
    let at = match version {
        b"0100" => (b.get(4) == Some(&1)).then_some(16)?,
        b"0300" => (b.get(7) == Some(&1)).then_some(42)?,
        b"0400" | b"0401" => (b.get(7) == Some(&1)).then_some(62)?,
        _ => return None,
    };
    let u = |i: usize| order.read_u16(b, at + 2 * i).map(f64::from);
    let (aw, ah, x, y, w, h) = (u(0)?, u(1)?, u(2)?, u(3)?, u(4)?, u(5)?);
    if aw < 1.0 || ah < 1.0 || x > aw || y > ah || w < 1.0 || h < 1.0 {
        return None;
    }
    let sensor = Rect { x0: (x - w / 2.0) / aw, y0: (y - h / 2.0) / ah, x1: (x + w / 2.0) / aw, y1: (y + h / 2.0) / ah };
    FocusPoint { rect: orientation.map_norm_rect(sensor), mode: None, area: None, tracking: None }.sanitized()
}

const FUJI_FOCUS_MODE: u16 = 0x1021;
const FUJI_FOCUS_PIXEL: u16 = 0x1023;

/// Fujifilm FocusPixel on a full-size image of `size` pixels (see the module notes).
pub(crate) fn fuji(ifd: &Ifd, size: (u32, u32), orientation: Orientation) -> Option<FocusPoint> {
    if ifd.value(FUJI_FOCUS_MODE).and_then(|v| v.get_u64(0)) == Some(1) {
        return None;
    }
    let p = ifd.value(FUJI_FOCUS_PIXEL)?;
    let (x, y) = (p.get_u64(0)? as f64, p.get_u64(1)? as f64);
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    // the stored frame is the landscape one; a portrait-tagged Exif size says the same pixels the other way round
    let (w, h) = if w < h { (h, w) } else { (w, h) };
    if w < 1.0 || h < 1.0 || x > w || y > h || (x == 0.0 && y == 0.0) {
        return None;
    }
    let side = w.min(h) * DEFAULT_SIDE * 1.5;
    let sensor = Rect { x0: (x - side / 2.0) / w, y0: (y - side / 2.0) / h, x1: (x + side / 2.0) / w, y1: (y + side / 2.0) / h };
    FocusPoint { rect: orientation.map_norm_rect(sensor), mode: None, area: None, tracking: None }.sanitized()
}

#[cfg(test)]
mod tests;
