//! LibRaw as the fallback decoder (Bryan's fork; `crates/libraw-sys`).
//!
//! [`crate::decode`] tries LightCraft's own decoder for the container first; when that fails
//! (a variant it doesn't read yet, a container it has no decoder for, or an error), LibRaw unpacks
//! the file. Only LibRaw's *unpacking* is used: the samples, levels, CFA, as-shot white balance,
//! Adobe's colour matrix where LibRaw knows the camera, geometry, crop and orientation become a
//! normal [`RawImage`] that LightCraft develops like its native decoders' output (its own
//! demosaic, the camera's Adobe profile through the DNG SDK, its own colour and tone).
//!
//! Without LibRaw built in (not fetched, wasm) [`available`] is false and nothing changes.
//! `LIGHTCRAFT_RAW_DECODER` picks the order for comparisons: `native` (default: native, then
//! LibRaw), `libraw` (LibRaw first), `nolibraw` (native only, as before) or `none` (no decoder:
//! every raw shows its embedded preview).

use crate::{BlackLevel, Cfa, ColorData, Mode, OpcodeLists, RawData, RawError, RawFormat, RawImage, Rect, Result};
use lightcraft_color::Mat3;
use lightcraft_geom::Orientation;
use lightcraft_libraw_sys as sys;

/// Which decoder read a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Decoder {
    /// LightCraft's own decoder for the container.
    Native,
    /// LibRaw (the fallback).
    LibRaw,
}

/// The decoder order (`LIGHTCRAFT_RAW_DECODER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Order {
    NativeThenLibRaw,
    LibRawThenNative,
    NativeOnly,
    None,
}

pub(crate) fn order() -> Order {
    order_from(std::env::var("LIGHTCRAFT_RAW_DECODER").ok().as_deref(), available())
}

fn order_from(knob: Option<&str>, libraw: bool) -> Order {
    match knob.map(str::trim) {
        Some("libraw") if libraw => Order::LibRawThenNative,
        Some("nolibraw") => Order::NativeOnly,
        Some("none") => Order::None,
        _ if libraw => Order::NativeThenLibRaw,
        _ => Order::NativeOnly,
    }
}

/// LibRaw is built in (see `tools/fetch-libraw.sh`).
pub fn available() -> bool {
    sys::available()
}

/// Whether a native decoder's error is worth a LibRaw attempt (a limit hit by the native decoder
/// would be hit by LibRaw too).
pub(crate) fn worth_trying(e: &RawError) -> bool {
    !matches!(e, RawError::Limit(_) | RawError::NotRaw)
}

/// Non-TIFF raw containers only LibRaw reads (recognised by [`crate::probe`] when it's built in):
/// Canon CRW (CIFF), Minolta MRW, Phase One IIQ.
pub(crate) fn probe_other(bytes: &[u8]) -> bool {
    let ciff = bytes.len() >= 14 && (bytes.starts_with(b"II") || bytes.starts_with(b"MM")) && bytes.get(6..14) == Some(b"HEAPCCDR");
    let mrw = bytes.starts_with(b"\0MRM");
    // Phase One: "IIII" / "MMMM" in the first 32 bytes, "Raw" in the word 8 bytes later
    let head = bytes.get(..32.min(bytes.len())).unwrap_or_default();
    let iiq = (0..head.len().saturating_sub(3)).any(|p| {
        let tag = bytes.get(p + 8..p + 12);
        (head.get(p..p + 4) == Some(b"IIII") && tag.and_then(|t| t.get(1..4)) == Some(b"waR"))
            || (head.get(p..p + 4) == Some(b"MMMM") && tag.and_then(|t| t.get(..3)) == Some(b"Raw"))
    });
    ciff || mrw || iiq
}

fn error(e: sys::Error) -> RawError {
    match e {
        sys::Error::TooLarge => RawError::Limit("LibRaw raw data"),
        sys::Error::Unavailable => RawError::Unsupported("LibRaw is not built in".into()),
        // LIBRAW_DATA_ERROR / LIBRAW_IO_ERROR
        sys::Error::LibRaw { code: -100008 | -100009, message } => RawError::Corrupt(message),
        sys::Error::Corrupt(s) => RawError::Corrupt(format!("LibRaw: {s}")),
        e => RawError::Unsupported(e.to_string()),
    }
}

/// Decode with LibRaw into a [`RawImage`] of `format` (what [`crate::probe`] called it).
pub(crate) fn decode(bytes: &[u8], format: RawFormat, mode: Mode) -> Result<RawImage> {
    let (info, samples) = match mode {
        Mode::Full => {
            let d = sys::decode(bytes).map_err(error)?;
            (d.info, d.samples)
        }
        Mode::Header => (sys::identify(bytes).map_err(error)?, Vec::new()),
    };
    image(format, info, samples, bytes, mode)
}

fn image(format: RawFormat, info: sys::Info, samples: Vec<u16>, bytes: &[u8], mode: Mode) -> Result<RawImage> {
    let v = info.visible;
    let active = Rect::new(v.x, v.y, v.width, v.height);
    // samples per pixel: CFA / mono 1; linear (sRAW, linear DNG) 3 (a 4th channel is dropped)
    let cpp = match (&info.cfa, info.channels) {
        (Some(_), _) => 1,
        (None, 0) => 3,
        (None, 1) => 1,
        (None, _) => 3,
    };
    let data = match mode {
        Mode::Header => Vec::new(),
        Mode::Full => {
            let ch = info.channels;
            if info.cfa.is_some() && ch != 1 {
                return Err(RawError::Unsupported(format!("LibRaw: {ch} samples per CFA pixel")));
            }
            if ch == 4 { samples.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect() } else { samples }
        }
    };
    let cfa = info.cfa.as_ref().map(|c| Cfa { width: c.width, height: c.height, pattern: c.pattern.clone() });
    let b = &info.black;
    let per_pixel = cpp.max(1);
    let black = if info.cfa.is_some() {
        BlackLevel { repeat_rows: b.rows.max(1), repeat_cols: b.cols.max(1), values: b.values.clone(), delta_h: vec![], delta_v: vec![] }
    } else {
        let values: Vec<f32> = (0..per_pixel).map(|s| b.values.get(s).or_else(|| b.values.first()).copied().unwrap_or(0.0)).collect();
        BlackLevel { repeat_rows: 1, repeat_cols: 1, values, delta_h: vec![], delta_v: vec![] }
    };
    // the camera's default crop, relative to the active area (the whole area when none fits)
    let crop = info
        .default_crop
        .and_then(|c| {
            let (x, y) = (c.x.checked_sub(v.x)?, c.y.checked_sub(v.y)?);
            (x + c.width <= v.width && y + c.height <= v.height).then_some(Rect::new(x, y, c.width, c.height))
        })
        .unwrap_or(Rect::new(0, 0, v.width, v.height));
    let mut color = ColorData::default();
    if let Some(m) = info.xyz_to_camera {
        // Adobe's ColorMatrix (D65) as LibRaw tabulates it
        color.illuminant = [21, 0];
        color.color_matrix = [Some(Mat3(m)), None];
    }
    let mut metadata = lightcraft_meta::extract(bytes);
    fill_metadata(&mut metadata, &info);
    metadata.width = Some(crop.width as u32);
    metadata.height = Some(crop.height as u32);
    let orientation = metadata.orientation.unwrap_or_else(|| Orientation::from_exif(info.orientation));
    let image = RawImage {
        format,
        width: info.raw_width,
        height: info.raw_height,
        cpp,
        data: RawData::U16(data),
        cfa,
        bits: info.bits,
        black,
        white: vec![info.white.max(1) as f32],
        active_area: active,
        crop,
        orientation,
        color,
        wb_multipliers: info.as_shot_multipliers,
        linearized: false,
        opcodes: OpcodeLists::default(),
        metadata,
    };
    image.validate_for(mode)?;
    Ok(image)
}

/// Fill what the file's Exif didn't give from LibRaw (which also reads maker notes and the
/// non-TIFF containers).
fn fill_metadata(m: &mut lightcraft_meta::Metadata, i: &sys::Info) {
    let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
    if m.make.is_none() {
        m.make = text(&i.make);
    }
    if m.model.is_none() {
        m.model = text(&i.model);
    }
    if m.lens_model.is_none() {
        m.lens_model = text(&i.lens);
    }
    if m.lens_make.is_none() {
        m.lens_make = text(&i.lens_make);
    }
    if m.iso.is_none() && i.iso.is_finite() && i.iso >= 1.0 {
        m.iso = Some(i.iso.round() as u32);
    }
    if m.exposure_time.is_none() && i.shutter.is_finite() && i.shutter > 0.0 {
        m.exposure_time = Some(f64::from(i.shutter));
    }
    if m.f_number.is_none() && i.aperture.is_finite() && i.aperture > 0.0 {
        m.f_number = Some(f64::from(i.aperture));
    }
    if m.focal_length.is_none() && i.focal_length.is_finite() && i.focal_length > 0.0 {
        m.focal_length = Some(f64::from(i.focal_length));
    }
    if m.orientation.is_none() && i.orientation != 1 {
        m.orientation = Some(Orientation::from_exif(i.orientation));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> sys::Info {
        sys::Info {
            raw_width: 8,
            raw_height: 6,
            visible: sys::Area { x: 2, y: 1, width: 6, height: 4 },
            default_crop: Some(sys::Area { x: 3, y: 2, width: 4, height: 2 }),
            orientation: 6,
            cfa: Some(sys::Cfa { width: 2, height: 2, pattern: vec![1, 0, 2, 1] }),
            channels: 1,
            black: sys::Black { rows: 2, cols: 2, values: vec![10.0, 11.0, 12.0, 13.0] },
            white: 1000,
            bits: 12,
            as_shot_multipliers: Some([2.0, 1.0, 1.5]),
            xyz_to_camera: Some([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            make: "Canon".into(),
            model: "EOS Test".into(),
            normalized_make: "Canon".into(),
            normalized_model: "EOS Test".into(),
            software: String::new(),
            lens: "RF 50mm".into(),
            lens_make: String::new(),
            iso: 400.0,
            shutter: 0.01,
            aperture: 2.8,
            focal_length: 50.0,
            timestamp: 0,
            dng_version: 0,
            decoder: "test".into(),
        }
    }

    #[test]
    fn libraw_info_becomes_a_raw_image() {
        let img = image(RawFormat::Cr3, info(), (0..48).collect(), b"", Mode::Full).unwrap();
        assert_eq!((img.width, img.height, img.cpp), (8, 6, 1));
        assert_eq!(img.active_area, Rect::new(2, 1, 6, 4));
        assert_eq!(img.crop, Rect::new(1, 1, 4, 2), "relative to the active area");
        assert_eq!(img.orientation, Orientation::Rotate90);
        assert_eq!(img.cfa.as_ref().unwrap().name(), "GRBG");
        assert_eq!(img.black.at(1, 1, 0, 1), 13.0);
        assert_eq!(img.white, vec![1000.0]);
        assert_eq!(img.wb_multipliers, Some([2.0, 1.0, 1.5]));
        assert!(crate::color::has_matrix(&img.color));
        assert_eq!(img.metadata.model.as_deref(), Some("EOS Test"));
        assert_eq!(img.metadata.iso, Some(400));
        assert_eq!(img.metadata.lens_model.as_deref(), Some("RF 50mm"));
        // it develops like any other raw
        let rgb = img.develop(crate::Method::Bilinear).unwrap();
        assert_eq!((rgb.width, rgb.height), (4, 2));
        // header mode: no samples, same description
        let h = image(RawFormat::Cr3, sys::Info { channels: 0, ..info() }, Vec::new(), b"", Mode::Header).unwrap();
        assert_eq!(h.info().crop, img.info().crop);
    }

    #[test]
    fn linear_four_channel_data_keeps_three() {
        let i = sys::Info { cfa: None, channels: 4, black: sys::Black { rows: 1, cols: 1, values: vec![1.0, 2.0, 3.0, 4.0] }, ..info() };
        let img = image(RawFormat::Cr2, i, (0..8 * 6 * 4).map(|v| v as u16).collect(), b"", Mode::Full).unwrap();
        assert_eq!(img.cpp, 3);
        assert_eq!(img.data.len(), 8 * 6 * 3);
        assert_eq!(img.data.get(3), 4.0, "second pixel's red");
        assert_eq!(img.black.values, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn inconsistent_info_is_an_error_not_a_panic() {
        // too few samples for the frame
        assert!(image(RawFormat::Cr3, info(), vec![0; 7], b"", Mode::Full).is_err());
        // CFA data with three samples per pixel
        assert!(image(RawFormat::Cr3, sys::Info { channels: 3, ..info() }, vec![0; 144], b"", Mode::Full).is_err());
        // a crop outside the visible area falls back to the whole area
        let img = image(
            RawFormat::Cr3,
            sys::Info { default_crop: Some(sys::Area { x: 0, y: 0, width: 8, height: 6 }), ..info() },
            (0..48).collect(),
            b"",
            Mode::Full,
        )
        .unwrap();
        assert_eq!(img.crop, Rect::new(0, 0, 6, 4));
    }

    #[test]
    fn decoder_order() {
        assert_eq!(order_from(None, true), Order::NativeThenLibRaw, "native first by default");
        assert_eq!(order_from(Some("native"), true), Order::NativeThenLibRaw);
        assert_eq!(order_from(Some(" libraw "), true), Order::LibRawThenNative);
        assert_eq!(order_from(Some("nolibraw"), true), Order::NativeOnly);
        assert_eq!(order_from(Some("none"), true), Order::None);
        // without LibRaw built in: as before
        assert_eq!(order_from(None, false), Order::NativeOnly);
        assert_eq!(order_from(Some("libraw"), false), Order::NativeOnly);
        assert_eq!(order_from(Some("none"), false), Order::None);
        assert!(worth_trying(&RawError::Unsupported("x".into())) && worth_trying(&RawError::Corrupt("x".into())));
        assert!(!worth_trying(&RawError::Limit("x")) && !worth_trying(&RawError::NotRaw));
    }

    #[test]
    fn other_containers_are_recognised_by_magic() {
        assert!(probe_other(b"II\x1a\0\0\0HEAPCCDR\x01\0"));
        assert!(probe_other(b"\0MRM\0\0\0\0"));
        assert!(probe_other(b"IIII\x01\0\0\0\0waR\0\0"));
        assert!(probe_other(b"II*\0\x08\0\0\0MMMM\0\0\0\x01Raw\0"));
        assert!(!probe_other(b"II*\0\x08\0\0\0"));
        assert!(!probe_other(b""));
        assert!(!probe_other(b"\xff\xd8\xff\xe0"));
    }
}
