//! The Sony tests are upstream LightCraft's (storytold/lightcraft `crates/meta/src/focus/tests.rs`), adapted; the
//! Canon one mirrors what Bryan's EOS R6 CR3s hold (ExifTool's output read as a black box); Nikon and Fujifilm are
//! built from the documented layouts only.

use super::*;
use lightcraft_tiff::{Entry, IfdBuilder, TiffWriter, Value};

fn encipher(p: u8) -> u8 {
    if p >= 249 { p } else { ((u32::from(p).pow(3)) % 249) as u8 }
}

fn entry(tag: u16, value: Value) -> Entry {
    Entry { tag, value, offset: 0 }
}

fn ifd_of(mut entries: Vec<Entry>) -> Ifd {
    entries.sort_by_key(|e| e.tag);
    Ifd { entries, ..Default::default() }
}

/// The α1 sample `_DSC6954.ARW`: AF-C, Wide, Face tracking, Animal Eye Tracking, 8640×5760 at (3226, 1980), 175×180.
fn alpha1() -> Ifd {
    let mut block = vec![0u8; 0x40];
    block[0x16] = encipher(0x83);
    block[0x17] = encipher(20);
    ifd_of(vec![
        entry(FOCUS_MODE, Value::Byte(vec![3])),
        entry(AF_AREA_SETTING, Value::Byte(vec![0])),
        entry(AF_TRACKING, Value::Byte(vec![1])),
        entry(FOCUS_LOCATION, Value::Short(vec![8640, 5760, 3226, 1980])),
        entry(FOCUS_FRAME_SIZE, Value::Undefined(vec![0xaf, 0, 0xb4, 0, 1, 1])),
        entry(TAG_9402, Value::Undefined(block)),
    ])
}

fn set(ifd: &mut Ifd, tag: u16, value: Value) {
    ifd.entries.retain(|e| e.tag != tag);
    ifd.entries.push(entry(tag, value));
    ifd.entries.sort_by_key(|e| e.tag);
}

#[test]
fn the_cipher_matches_the_bytes_seen_in_files() {
    assert_eq!(decipher(0x20), 20);
    assert_eq!(decipher(0x77), 0x83);
    assert_eq!(decipher(0xff), 0xff);
    for p in 0..=255u8 {
        assert_eq!(decipher(encipher(p)), p);
    }
}

#[test]
fn reads_the_alpha1_sample() {
    let f = sony(&alpha1(), ByteOrder::Little, Orientation::Normal, true).unwrap();
    assert!((f.rect.x0 - (3226.0 - 87.5) / 8640.0).abs() < 1e-12);
    assert!((f.rect.y1 - (1980.0 + 90.0) / 5760.0).abs() < 1e-12);
    assert_eq!(f.words(), vec!["AF-C", "Wide", "Animal Eye Tracking"]);
    // a body whose block layout isn't confirmed: the point, from the plain tags only
    let f = sony(&alpha1(), ByteOrder::Little, Orientation::Normal, false).unwrap();
    assert_eq!(f.words(), vec!["AF-C", "Wide", "Face tracking"]);
}

#[test]
fn follows_the_exif_orientation() {
    let mut ifd = alpha1();
    set(&mut ifd, FOCUS_LOCATION, Value::Short(vec![8640, 5760, 864, 576]));
    let f = sony(&ifd, ByteOrder::Little, Orientation::from_exif(8), true).unwrap();
    let (cx, cy) = f.center();
    assert!((cx - 0.1).abs() < 1e-9 && (cy - 0.9).abs() < 1e-9, "{cx} {cy}");
    let f = sony(&ifd, ByteOrder::Little, Orientation::from_exif(6), true).unwrap();
    let (cx, cy) = f.center();
    assert!((cx - 0.9).abs() < 1e-9 && (cy - 0.1).abs() < 1e-9, "{cx} {cy}");
}

#[test]
fn no_real_point() {
    let mut ifd = alpha1();
    set(&mut ifd, FOCUS_MODE, Value::Byte(vec![0]));
    assert!(sony(&ifd, ByteOrder::Little, Orientation::Normal, true).is_none(), "manual focus");
    let mut ifd = alpha1();
    let mut block = vec![0u8; 0x18];
    block[0x16] = encipher(3);
    block[0x17] = encipher(255);
    set(&mut ifd, TAG_9402, Value::Undefined(block));
    assert!(sony(&ifd, ByteOrder::Little, Orientation::Normal, true).is_none(), "the block says manual");
    let mut ifd = alpha1();
    let mut block = vec![0u8; 0x18];
    block[0x16] = encipher(3);
    block[0x17] = encipher(0);
    set(&mut ifd, TAG_9402, Value::Undefined(block));
    set(&mut ifd, FOCUS_LOCATION, Value::Short(vec![8640, 5760, 4320, 2880]));
    assert!(sony(&ifd, ByteOrder::Little, Orientation::Normal, true).is_none(), "Multi at the exact centre");
    ifd.entries.retain(|e| e.tag != TAG_9402);
    set(&mut ifd, AF_TRACKING, Value::Byte(vec![0]));
    assert!(sony(&ifd, ByteOrder::Little, Orientation::Normal, true).is_none(), "Wide, untracked, at the centre");
    for loc in [vec![0, 5760, 0, 0], vec![8640, 0, 1, 0], vec![8640, 5760, 9000, 10], vec![8640, 5760, 10, 6000], vec![8640, 5760, 10]] {
        let mut ifd = alpha1();
        set(&mut ifd, FOCUS_LOCATION, Value::Short(loc.clone()));
        assert!(sony(&ifd, ByteOrder::Little, Orientation::Normal, true).is_none(), "{loc:?}");
    }
}

#[test]
fn wrong_types_and_extreme_values_never_panic() {
    let values = [
        Value::Ascii("x".into()),
        Value::Byte(vec![]),
        Value::SShort(vec![-1, -1, -1, -1]),
        Value::Long(vec![u32::MAX; 4]),
        Value::Double(vec![f64::NAN; 4]),
        Value::Undefined(vec![0xff; 300]),
    ];
    for v in &values {
        for tag in [FOCUS_MODE, AF_AREA_SETTING, AF_TRACKING, FOCUS_LOCATION, FOCUS_FRAME_SIZE, TAG_9402] {
            let mut ifd = alpha1();
            set(&mut ifd, tag, v.clone());
            if let Some(f) = sony(&ifd, ByteOrder::Little, Orientation::Normal, true) {
                assert!(f.rect.x0 >= 0.0 && f.rect.x1 <= 1.0 && f.rect.x0 < f.rect.x1);
            }
        }
        for tag in [CANON_AF_INFO2, CANON_AF_INFO3] {
            let _ = canon(&ifd_of(vec![entry(tag, v.clone())]), Orientation::Normal);
        }
        let _ = nikon(&ifd_of(vec![entry(NIKON_AF_INFO2, v.clone())]), ByteOrder::Big, Orientation::Normal);
        let _ = fuji(&ifd_of(vec![entry(FUJI_FOCUS_PIXEL, v.clone())]), (6240, 4160), Orientation::Normal);
    }
    assert!(FocusPoint { rect: Rect { x0: f64::NAN, y0: 0.0, x1: 1.0, y1: 1.0 }, mode: None, area: None, tracking: None }.sanitized().is_none());
}

/// Canon AFInfo2 with `n` points of which `pts` (x, y, w, h) are valid, `in_focus` the indexes in focus.
fn canon_info(mode: i16, n: usize, pts: &[(i16, i16, i16, i16)], in_focus: &[usize]) -> Vec<u16> {
    let words = n.div_ceil(16);
    let mut v: Vec<i16> = vec![0; 8 + 4 * n + 2 * words];
    v[1] = mode;
    v[2] = n as i16;
    v[3] = pts.len() as i16;
    v[4] = 5472;
    v[5] = 3648;
    v[6] = 5472;
    v[7] = 3648;
    for (i, (x, y, w, h)) in pts.iter().enumerate() {
        v[8 + i] = *w;
        v[8 + n + i] = *h;
        v[8 + 2 * n + i] = *x;
        v[8 + 3 * n + i] = *y;
    }
    for i in in_focus {
        v[8 + 4 * n + i / 16] |= (1u16 << (i % 16)) as i16;
    }
    v[0] = (v.len() * 2) as i16;
    v.into_iter().map(|x| x as u16).collect()
}

#[test]
fn canon_af_points() {
    // EOS R6 BHM_4857.CR3: Spot AF, one point at (2, 6) of a 5472 × 3648 AF image, 264 × 216, in focus
    let ifd = ifd_of(vec![entry(CANON_AF_INFO2, Value::Short(canon_info(9, 1053, &[(2, 6, 264, 216)], &[0])))]);
    let f = canon(&ifd, Orientation::Normal).unwrap();
    let (cx, cy) = f.center();
    assert!((cx - (2736.0 + 2.0) / 5472.0).abs() < 1e-9, "{cx}");
    assert!((cy - (1824.0 - 6.0) / 3648.0).abs() < 1e-9, "y is up: {cy}");
    assert_eq!(f.area.as_deref(), Some("Spot AF"));
    // Face + Tracking: the box covers the points in focus (not the other valid ones)
    let pts: Vec<(i16, i16, i16, i16)> = (0..40).map(|i| (-2508 + 132 * (i % 20) as i16, 1755 - 130 * (i / 20) as i16, 119, 122)).collect();
    let ifd = ifd_of(vec![entry(CANON_AF_INFO3, Value::Short(canon_info(6, 1053, &pts, &[3, 4, 23])))]);
    let f = canon(&ifd, Orientation::Normal).unwrap();
    let x0 = (2736.0 + (-2508.0 + 132.0 * 3.0) - 59.5) / 5472.0;
    assert!((f.rect.x0 - x0).abs() < 1e-9, "{:?}", f.rect);
    assert!((f.rect.y0 - (1824.0 - 1755.0 - 61.0) / 3648.0).abs() < 1e-9 && (f.rect.y1 - (1824.0 - 1625.0 + 61.0) / 3648.0).abs() < 1e-9);
    // a portrait (EXIF 8) moves it like every other box
    let g = canon(&ifd, Orientation::from_exif(8)).unwrap();
    assert_eq!(g.rect, Orientation::from_exif(8).map_norm_rect(f.rect));
    // manual focus, nothing valid, nothing in focus among many: no point
    assert!(canon(&ifd_of(vec![entry(CANON_AF_INFO2, Value::Short(canon_info(0, 1053, &[(0, 0, 10, 10)], &[0])))]), Orientation::Normal).is_none());
    assert!(canon(&ifd_of(vec![entry(CANON_AF_INFO2, Value::Short(canon_info(9, 1053, &[], &[])))]), Orientation::Normal).is_none());
    assert!(canon(&ifd_of(vec![entry(CANON_AF_INFO2, Value::Short(canon_info(6, 1053, &pts, &[])))]), Orientation::Normal).is_none());
    // a damaged count or a short array: none, never a panic
    let mut v = canon_info(9, 1053, &[(2, 6, 264, 216)], &[0]);
    v[2] = 30000;
    assert!(canon(&ifd_of(vec![entry(CANON_AF_INFO2, Value::Short(v))]), Orientation::Normal).is_none());
    let v = canon_info(9, 1053, &[(2, 6, 264, 216)], &[0]);
    for cut in [0, 3, 8, 100, 4220, 4229] {
        let _ = canon(&ifd_of(vec![entry(CANON_AF_INFO2, Value::Short(v[..cut].to_vec()))]), Orientation::Normal);
    }
}

fn nikon_info(version: &[u8; 4], method_or_flag: u8, at: usize, vals: [u16; 6], order: ByteOrder) -> Vec<u8> {
    let mut b = vec![0u8; at + 12 + 8];
    b[..4].copy_from_slice(version);
    if version == b"0100" {
        b[4] = method_or_flag;
    } else {
        b[7] = method_or_flag;
    }
    for (i, v) in vals.iter().enumerate() {
        let bytes = if order == ByteOrder::Big { v.to_be_bytes() } else { v.to_le_bytes() };
        b[at + 2 * i..at + 2 * i + 2].copy_from_slice(&bytes);
    }
    b
}

#[test]
fn nikon_af_area() {
    for (version, at) in [(b"0100", 16), (b"0300", 42), (b"0400", 62)] {
        for order in [ByteOrder::Big, ByteOrder::Little] {
            let b = nikon_info(version, 1, at, [6048, 4032, 1512, 1008, 600, 400], order);
            let f = nikon(&ifd_of(vec![entry(NIKON_AF_INFO2, Value::Undefined(b))]), order, Orientation::Normal).unwrap();
            let (cx, cy) = f.center();
            assert!((cx - 0.25).abs() < 1e-9 && (cy - 0.25).abs() < 1e-9, "{version:?}: {cx} {cy}");
            assert!((f.rect.x1 - f.rect.x0 - 600.0 / 6048.0).abs() < 1e-9);
        }
        // no coordinates (phase detect on the older layout, coordinates not available on the newer)
        let b = nikon_info(version, 0, at, [6048, 4032, 1512, 1008, 600, 400], ByteOrder::Big);
        assert!(nikon(&ifd_of(vec![entry(NIKON_AF_INFO2, Value::Undefined(b))]), ByteOrder::Big, Orientation::Normal).is_none());
    }
    // manual focus, an unknown version, a point outside the frame
    let b = nikon_info(b"0300", 1, 42, [6048, 4032, 1512, 1008, 600, 400], ByteOrder::Big);
    let manual = ifd_of(vec![entry(NIKON_FOCUS_MODE, Value::Ascii("MANUAL".into())), entry(NIKON_AF_INFO2, Value::Undefined(b))]);
    assert!(nikon(&manual, ByteOrder::Big, Orientation::Normal).is_none());
    let b = nikon_info(b"0200", 1, 42, [6048, 4032, 1512, 1008, 600, 400], ByteOrder::Big);
    assert!(nikon(&ifd_of(vec![entry(NIKON_AF_INFO2, Value::Undefined(b))]), ByteOrder::Big, Orientation::Normal).is_none());
    let b = nikon_info(b"0300", 1, 42, [6048, 4032, 7000, 1008, 600, 400], ByteOrder::Big);
    assert!(nikon(&ifd_of(vec![entry(NIKON_AF_INFO2, Value::Undefined(b))]), ByteOrder::Big, Orientation::Normal).is_none());
    // cut short anywhere
    let b = nikon_info(b"0400", 1, 62, [6048, 4032, 1512, 1008, 600, 400], ByteOrder::Big);
    for cut in 0..b.len() {
        let _ = nikon(&ifd_of(vec![entry(NIKON_AF_INFO2, Value::Undefined(b[..cut].to_vec()))]), ByteOrder::Big, Orientation::Normal);
    }
}

#[test]
fn fuji_focus_pixel() {
    let ifd = ifd_of(vec![entry(FUJI_FOCUS_MODE, Value::Short(vec![0])), entry(FUJI_FOCUS_PIXEL, Value::Short(vec![3120, 1040]))]);
    let f = fuji(&ifd, (6240, 4160), Orientation::Normal).unwrap();
    let (cx, cy) = f.center();
    assert!((cx - 0.5).abs() < 1e-9 && (cy - 0.25).abs() < 1e-9);
    // a portrait-tagged size names the same landscape frame
    assert_eq!(fuji(&ifd, (4160, 6240), Orientation::Normal), Some(f));
    let manual = ifd_of(vec![entry(FUJI_FOCUS_MODE, Value::Short(vec![1])), entry(FUJI_FOCUS_PIXEL, Value::Short(vec![3120, 1040]))]);
    assert!(fuji(&manual, (6240, 4160), Orientation::Normal).is_none());
    assert!(fuji(&ifd, (1920, 1280), Orientation::Normal).is_none(), "a point outside the frame named");
}

/// A whole TIFF whose Exif IFD carries `note` as the maker note.
fn tiff_with_note(make: &str, model: &str, orientation: u16, note: Vec<u8>) -> Vec<u8> {
    let mut exif = IfdBuilder::new();
    exif.set(t::MAKER_NOTE, Value::Undefined(note));
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::MAKE, Value::Ascii(make.into()));
    ifd0.set(t::MODEL, Value::Ascii(model.into()));
    ifd0.set(t::ORIENTATION, Value::Short(vec![orientation]));
    ifd0.set_child(t::EXIF_IFD, exif);
    TiffWriter { order: ByteOrder::Little, bigtiff: false }.write(&[ifd0]).unwrap()
}

/// A Fujifilm maker note (`FUJIFILM`, IFD offset 12, offsets from the note) holding FocusPixel.
fn fuji_note(x: u16, y: u16) -> Vec<u8> {
    let o = ByteOrder::Little;
    let mut n = b"FUJIFILM".to_vec();
    o.put_u32(&mut n, 12);
    o.put_u16(&mut n, 1);
    o.put_u16(&mut n, FUJI_FOCUS_PIXEL);
    o.put_u16(&mut n, 3);
    o.put_u32(&mut n, 2);
    o.put_u16(&mut n, x);
    o.put_u16(&mut n, y);
    o.put_u32(&mut n, 0);
    n
}

#[test]
fn whole_files_find_their_maker_note() {
    // a JPEG's Exif segment (Fujifilm), with the frame from its Exif size
    let mut exif = IfdBuilder::new();
    exif.set(t::MAKER_NOTE, Value::Undefined(fuji_note(3120, 2080)));
    exif.set(t::PIXEL_X_DIMENSION, Value::Long(vec![6240]));
    exif.set(t::PIXEL_Y_DIMENSION, Value::Long(vec![4160]));
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::MAKE, Value::Ascii("FUJIFILM".into()));
    ifd0.set(t::MODEL, Value::Ascii("X-T4".into()));
    ifd0.set_child(t::EXIF_IFD, exif);
    let tiff = TiffWriter { order: ByteOrder::Big, bigtiff: false }.write(&[ifd0]).unwrap();
    let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
    jpeg.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
    jpeg.extend_from_slice(b"Exif\0\0");
    jpeg.extend_from_slice(&tiff);
    jpeg.extend_from_slice(&[0xff, 0xda, 0, 2, 0x55, 0x55, 0xff, 0xd9]);
    let f = read_file(&jpeg).expect("a point");
    let (cx, cy) = f.center();
    assert!((cx - 0.5).abs() < 1e-9 && (cy - 0.5).abs() < 1e-9);
    // another maker's note, or a Sony body that doesn't write the tag: nothing
    assert!(read_file(&tiff_with_note("SONY", "NEX-7", 1, vec![0; 40])).is_none());
    assert!(read_file(&tiff_with_note("Leica", "M11", 1, vec![0; 40])).is_none());
    // truncated anywhere: never a panic
    for cut in (0..jpeg.len()).step_by(3) {
        let _ = read_file(&jpeg[..cut]);
    }
    assert!(read_file(&[]).is_none());
}

/// The focus points of real files (`LC_FOCUS_FILES`: paths separated by `:`), printed; with `LC_FOCUS_EXPECT` a
/// count of files that must have one.
#[test]
#[ignore]
fn real_files() {
    let Some(list) = std::env::var_os("LC_FOCUS_FILES") else { return };
    let mut found = 0;
    for p in std::env::split_paths(&list) {
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let f = read_file(&bytes);
        println!("{}: {:?}", p.display(), f.as_ref().map(|f| (f.center(), f.rect, f.words())));
        found += usize::from(f.is_some());
    }
    if let Some(n) = std::env::var("LC_FOCUS_EXPECT").ok().and_then(|v| v.parse::<usize>().ok()) {
        assert_eq!(found, n);
    }
}
