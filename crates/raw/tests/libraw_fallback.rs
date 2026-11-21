//! The LibRaw fallback (`crates/libraw-sys`): native decoder first, then LibRaw, then the
//! caller's embedded-preview path with the native decoder's error. Without LibRaw built in
//! (vendor/ not fetched, wasm) decoding behaves exactly as before.

use lightcraft_raw::{Decoder, RawError, RawFormat, decode, decode_reporting, libraw_available, probe, probe_info};
use lightcraft_tiff::tags as t;
use lightcraft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};

const W: u32 = 64;
const H: u32 = 40;

/// An uncompressed 16-bit RGGB CFA TIFF in IFD0. `dng`: with `DNGVersion` and the DNG level tags
/// (LightCraft's DNG decoder reads it); without, an unknown make's TIFF raw (no native decoder).
fn cfa_file(dng: bool) -> (Vec<u8>, Vec<u16>) {
    cfa_file_with(dng, W)
}

/// `width`: the width the header claims (the strip always holds `W × H` samples).
fn cfa_file_with(dng: bool, width: u32) -> (Vec<u8>, Vec<u16>) {
    let samples: Vec<u16> = (0..(W * H) as usize).map(|i| (128 + (i * 53) % 3500) as u16).collect();
    let strip: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
    ifd0.set(t::IMAGE_WIDTH, Value::Long(vec![width]));
    ifd0.set(t::IMAGE_LENGTH, Value::Long(vec![H]));
    ifd0.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
    ifd0.set(t::COMPRESSION, Value::Short(vec![1]));
    ifd0.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
    ifd0.set(t::MAKE, Value::Ascii("Zzyzx".into()));
    ifd0.set(t::MODEL, Value::Ascii("Fallback One".into()));
    ifd0.set(t::ORIENTATION, Value::Short(vec![1]));
    ifd0.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
    ifd0.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
    ifd0.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
    if dng {
        ifd0.set(t::DNG_VERSION, Value::Byte(vec![1, 4, 0, 0]));
        ifd0.set(t::BLACK_LEVEL, Value::Long(vec![128]));
        ifd0.set(t::WHITE_LEVEL, Value::Long(vec![4095]));
    }
    ifd0.set_image(ImageData::Strips { rows_per_strip: H, strips: vec![strip] });
    (TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap(), samples)
}

#[test]
fn native_decoder_comes_first() {
    let (bytes, samples) = cfa_file(true);
    assert_eq!(probe(&bytes), Some(RawFormat::Dng));
    let (img, by) = decode_reporting(&bytes).unwrap();
    assert_eq!(by, Decoder::Native, "LightCraft's own DNG decoder reads it");
    assert_eq!(img.data, lightcraft_raw::RawData::U16(samples));
}

#[test]
fn libraw_reads_what_no_native_decoder_does() {
    let (bytes, samples) = cfa_file(false);
    assert_eq!(probe(&bytes), Some(RawFormat::OtherTiff));
    if !libraw_available() {
        // as before LibRaw: the container has no decoder, callers show the embedded preview
        let e = decode(&bytes).unwrap_err();
        assert!(matches!(e, RawError::Unsupported(_)), "{e}");
        assert!(probe_info(&bytes).is_err());
        return;
    }
    let (img, by) = decode_reporting(&bytes).unwrap();
    assert_eq!(by, Decoder::LibRaw);
    assert_eq!(img.format, RawFormat::OtherTiff, "keeps the container's name");
    assert_eq!((img.width, img.height, img.cpp), (W as usize, H as usize, 1));
    assert_eq!(img.cfa.as_ref().map(|c| c.name()), Some("RGGB".to_string()));
    assert_eq!(img.data, lightcraft_raw::RawData::U16(samples));
    assert_eq!(img.metadata.model.as_deref(), Some("Fallback One"));
    // developed by LightCraft's own pipeline
    let rgb = img.develop(lightcraft_raw::Method::Bilinear).unwrap();
    assert_eq!((rgb.width, rgb.height), img.info().developed_size());
    // the header-only description agrees with the decode
    let info = probe_info(&bytes).unwrap();
    assert_eq!((info.width, info.height, info.crop, info.cfa), (img.width, img.height, img.crop, img.cfa.clone()));
}

#[test]
fn when_both_fail_the_native_error_is_kept() {
    // the header claims a 8000-pixel-wide sensor, the strip holds 64: neither decoder can read it
    let (bytes, _) = cfa_file_with(false, 8000);
    let e = decode(&bytes).unwrap_err();
    assert_eq!(e, RawError::Unsupported("OtherTiff files are not decoded yet".into()), "the native decoder's reason");
    // a container that isn't a raw at all is still "not raw"
    assert_eq!(decode(b"\xff\xd8\xff\xe0 a jpeg"), Err(RawError::NotRaw));
}

#[test]
fn truncated_and_garbage_input_never_panics() {
    let (bytes, _) = cfa_file(false);
    for len in (0..bytes.len()).step_by(11) {
        let _ = decode(&bytes[..len]);
        let _ = probe_info(&bytes[..len]);
    }
    let mut b = bytes.clone();
    for i in (8..b.len().min(600)).step_by(3) {
        b[i] ^= 0x5a;
        let _ = decode(&b);
    }
    // non-TIFF containers LibRaw reads are only recognised with it built in
    let crw = b"II\x1a\0\0\0HEAPCCDR\x01\0\0\0\0\0garbage";
    assert_eq!(probe(crw), libraw_available().then_some(RawFormat::Other));
    assert!(decode(crw).is_err());
}
