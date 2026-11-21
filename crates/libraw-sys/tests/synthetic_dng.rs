//! LibRaw through the safe API on a tiny DNG generated here (no fixture files), and on hostile
//! input: truncated and garbage bytes must give an error, never a crash. Skipped (passes) when
//! LibRaw isn't built in.

use lightcraft_libraw_sys as lr;

const W: usize = 50;
const H: usize = 34;
const BLACK: u32 = 64;
const WHITE: u32 = 4095;

enum V {
    Short(Vec<u16>),
    Long(Vec<u32>),
    Byte(Vec<u8>),
    Ascii(&'static str),
    Rational(Vec<(u32, u32)>),
    SRational(Vec<(i32, i32)>),
}

/// A little-endian TIFF/DNG: one IFD, one uncompressed 16-bit CFA strip.
fn dng(orientation: u16) -> (Vec<u8>, Vec<u16>) {
    let samples: Vec<u16> = (0..W * H).map(|i| (BLACK as usize + (i * 37) % 3000) as u16).collect();
    let pixels: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    let matrix =
        [(8292, 10000), (-1611, 10000), (-1132, 10000), (-4759, 10000), (12711, 10000), (2275, 10000), (-1013, 10000), (2415, 10000), (5509, 10000)];
    let mut entries: Vec<(u16, V)> = vec![
        (254, V::Long(vec![0])),
        (256, V::Long(vec![W as u32])),
        (257, V::Long(vec![H as u32])),
        (258, V::Short(vec![16])),
        (259, V::Short(vec![1])),
        (262, V::Short(vec![32803])),
        (271, V::Ascii("Zzyzx")),
        (272, V::Ascii("Synthetic")),
        (273, V::Long(vec![0])), // patched below
        (274, V::Short(vec![orientation])),
        (277, V::Short(vec![1])),
        (278, V::Long(vec![H as u32])),
        (279, V::Long(vec![pixels.len() as u32])),
        (284, V::Short(vec![1])),
        (33421, V::Short(vec![2, 2])),
        (33422, V::Byte(vec![0, 1, 1, 2])),
        (50706, V::Byte(vec![1, 4, 0, 0])),
        (50708, V::Ascii("LightCraft Synthetic")),
        (50714, V::Long(vec![BLACK])),
        (50717, V::Long(vec![WHITE])),
        (50721, V::SRational(matrix.to_vec())),
        (50728, V::Rational(vec![(1, 2), (1, 1), (2, 3)])),
        (50778, V::Short(vec![21])),
    ];
    entries.sort_by_key(|e| e.0);
    let encode = |v: &V| -> (u16, u32, Vec<u8>) {
        match v {
            V::Short(s) => (3, s.len() as u32, s.iter().flat_map(|x| x.to_le_bytes()).collect()),
            V::Long(s) => (4, s.len() as u32, s.iter().flat_map(|x| x.to_le_bytes()).collect()),
            V::Byte(s) => (1, s.len() as u32, s.clone()),
            V::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (2, b.len() as u32, b)
            }
            V::Rational(s) => (5, s.len() as u32, s.iter().flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat()).collect()),
            V::SRational(s) => (10, s.len() as u32, s.iter().flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat()).collect()),
        }
    };
    let ifd_len = 2 + entries.len() * 12 + 4;
    let mut extra: Vec<u8> = Vec::new();
    let extra_at = 8 + ifd_len;
    let mut encoded: Vec<(u16, u16, u32, Vec<u8>)> = entries.iter().map(|(t, v)| (*t, encode(v))).map(|(t, (ty, n, b))| (t, ty, n, b)).collect();
    // strip after the out-of-line values
    let out_of_line: usize = encoded.iter().filter(|e| e.3.len() > 4).map(|e| e.3.len().div_ceil(2) * 2).sum();
    let strip_at = (extra_at + out_of_line) as u32;
    for e in encoded.iter_mut().filter(|e| e.0 == 273) {
        e.3 = strip_at.to_le_bytes().to_vec();
    }
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&(encoded.len() as u16).to_le_bytes());
    for (tag, ty, n, b) in &encoded {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        if b.len() <= 4 {
            let mut v = b.clone();
            v.resize(4, 0);
            out.extend_from_slice(&v);
        } else {
            out.extend_from_slice(&((extra_at + extra.len()) as u32).to_le_bytes());
            extra.extend_from_slice(b);
            if extra.len() % 2 == 1 {
                extra.push(0);
            }
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&extra);
    assert_eq!(out.len() as u32, strip_at);
    out.extend_from_slice(&pixels);
    (out, samples)
}

#[test]
fn decodes_a_synthetic_dng() {
    if !lr::available() {
        eprintln!("skip: LibRaw not built in");
        return;
    }
    let (bytes, samples) = dng(6);
    let d = lr::decode(&bytes).unwrap();
    let i = &d.info;
    assert_eq!((i.raw_width, i.raw_height, i.channels), (W, H, 1));
    assert_eq!(i.visible, lr::Area { x: 0, y: 0, width: W, height: H });
    assert_eq!(d.samples, samples, "the samples as stored");
    assert_eq!(i.white, WHITE);
    assert!(i.black.values.iter().all(|&b| b == BLACK as f32), "{:?}", i.black);
    assert_eq!(i.cfa.as_ref().map(|c| (c.width, c.height, c.pattern.clone())), Some((2, 2, vec![0, 1, 1, 2])));
    assert_eq!(i.orientation, 6);
    assert_eq!(i.make, "Zzyzx", "(a make containing \"htc\", like LightCraft, reads as HTC)");
    assert_eq!(i.model, "Synthetic");
    // AsShotNeutral (1/2, 1, 2/3) → multipliers (2, 1, 1.5)
    let m = i.as_shot_multipliers.unwrap();
    assert!((m[0] - 2.0).abs() < 1e-3 && m[1] == 1.0 && (m[2] - 1.5).abs() < 1e-3, "{m:?}");
    let x = i.xyz_to_camera.unwrap();
    assert!((x[0][0] - 0.8292).abs() < 1e-3 && (x[2][2] - 0.5509).abs() < 1e-3, "{x:?}");
    // header only: the same description, no samples
    let h = lr::identify(&bytes).unwrap();
    assert_eq!((h.raw_width, h.raw_height, h.channels, h.cfa.clone()), (W, H, 0, i.cfa.clone()));
}

#[test]
fn hostile_input_is_an_error_never_a_crash() {
    if !lr::available() {
        eprintln!("skip: LibRaw not built in");
        return;
    }
    let (bytes, _) = dng(1);
    // every truncation of the header and some of the strip
    for len in (0..bytes.len()).step_by(7).chain([bytes.len() - 1]) {
        let _ = lr::decode(&bytes[..len]);
        let _ = lr::identify(&bytes[..len]);
    }
    // flipped bytes through the IFD
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for i in 0..400 {
        let mut b = bytes.clone();
        for _ in 0..1 + i % 8 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed as usize) % 400.min(b.len());
            b[at] = (seed >> 32) as u8;
        }
        if let Ok(d) = lr::decode(&b) {
            assert_eq!(d.samples.len(), d.info.raw_width * d.info.raw_height * d.info.channels);
        }
    }
    // garbage and other files
    for junk in [&b"II*\0"[..], b"MM\0*\0\0\0\x08", b"\xff\xd8\xff\xe0\0\x10JFIF\0", &[0u8; 64][..], &[0xffu8; 5000][..], b"FUJIFILMCCD-RAW 0201"] {
        assert!(lr::decode(junk).is_err());
        assert!(lr::identify(junk).is_err());
    }
}
