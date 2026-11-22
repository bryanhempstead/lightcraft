//! Flat PSD export (E07), ported from storytold/craft-libs `craft-psd` (`flat.rs`, MIT OR
//! Apache-2.0, © 2026 ArtCraft Team and the craft-libs contributors — the same licence as
//! LightCraft): one merged image, gray or RGB, 8-bit PackBits rows or raw 16-bit big-endian
//! samples, with ICC (resource 1039), EXIF (1058), XMP (1060) and resolution (1005). Alpha is
//! dropped. PSB (version 2) is written when a side is over 30,000 pixels (up to 300,000).

use crate::encode::{EncodeImage, EncodeMeta, Samples};
use crate::{Error, Result};

const MAX_PSD: u32 = 30_000;
const MAX_PSB: u32 = 300_000;

/// PackBits (Apple / TIFF run-length) encode `row` onto `out`.
pub(crate) fn packbits(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < row.len() {
        // a run of 3 or more equal bytes
        let b = row.get(i).copied().unwrap_or(0);
        let mut run = 1;
        while run < 128 && row.get(i + run) == Some(&b) {
            run += 1;
        }
        if run >= 3 {
            out.push((257 - run) as u8);
            out.push(b);
            i += run;
            continue;
        }
        // literals until the next run of 3 (at most 128)
        let start = i;
        while i < row.len() && i - start < 128 {
            let c = row.get(i).copied().unwrap_or(0);
            if row.get(i + 1) == Some(&c) && row.get(i + 2) == Some(&c) {
                break;
            }
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(row.get(start..i).unwrap_or_default());
    }
}

fn resource(out: &mut Vec<u8>, id: u16, data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).map_err(|_| Error::Encode("PSD resource too large".into()))?;
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    Ok(())
}

/// Encode a flat PSD (PSB for a side over 30,000 pixels). 8- or 16-bit samples.
pub fn encode_psd(img: &EncodeImage, meta: &EncodeMeta) -> Result<Vec<u8>> {
    let (w, h, ch) = (img.width, img.height, usize::from(img.channels));
    if w == 0 || h == 0 || !(1..=4).contains(&ch) {
        return Err(Error::Encode("PSD: invalid size or channels".into()));
    }
    if w > MAX_PSB || h > MAX_PSB {
        return Err(Error::Encode(format!("PSD: at most {MAX_PSB} pixels a side")));
    }
    let psb = w > MAX_PSD || h > MAX_PSD;
    let n = (w as usize).checked_mul(h as usize).ok_or_else(|| Error::Encode("PSD: too large".into()))?;
    let needed = n.checked_mul(ch).ok_or_else(|| Error::Encode("PSD: too large".into()))?;
    let (depth, have) = match img.samples {
        Samples::U8(s) => (8u16, s.len()),
        Samples::U16(s) => (16u16, s.len()),
        Samples::F32(_) => return Err(Error::Encode("PSD export takes 8- or 16-bit samples".into())),
    };
    if have < needed {
        return Err(Error::Encode("PSD: sample buffer too short".into()));
    }
    let colour = if ch <= 2 { 1usize } else { 3 };
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&(if psb { 2u16 } else { 1 }).to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&(colour as u16).to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&depth.to_be_bytes());
    out.extend_from_slice(&(if colour == 1 { 1u16 } else { 3 }).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    let mut res = Vec::new();
    if let Some(ppi) = meta.ppi.filter(|p| *p > 0) {
        let fixed = (u32::from(ppi) << 16).to_be_bytes();
        let one = 1u16.to_be_bytes();
        resource(&mut res, 1005, &[&fixed[..], &one, &one, &fixed, &one, &one].concat())?;
    }
    if let Some(d) = meta.icc {
        resource(&mut res, 1039, d)?;
    }
    if let Some(d) = meta.exif {
        resource(&mut res, 1058, d)?;
    }
    if let Some(d) = meta.xmp {
        resource(&mut res, 1060, d.as_bytes())?;
    }
    out.extend_from_slice(&u32::try_from(res.len()).map_err(|_| Error::Encode("PSD resources too large".into()))?.to_be_bytes());
    out.extend_from_slice(&res);
    // no layers (PSB: an 8-byte length)
    if psb {
        out.extend_from_slice(&0u64.to_be_bytes());
    } else {
        out.extend_from_slice(&0u32.to_be_bytes());
    }
    let (wu, hu) = (w as usize, h as usize);
    match img.samples {
        Samples::U8(src) => {
            out.extend_from_slice(&1u16.to_be_bytes());
            // row byte counts: 2 bytes each (PSB: 4)
            let width = if psb { 4 } else { 2 };
            let rows = colour * hu;
            let table = out.len();
            out.try_reserve(rows * width).map_err(|_| Error::Encode("PSD: out of memory".into()))?;
            out.resize(table + rows * width, 0);
            let mut row = Vec::with_capacity(wu);
            for c in 0..colour {
                for y in 0..hu {
                    row.clear();
                    let start = y * wu * ch;
                    row.extend(src.get(start + c..start + wu * ch).unwrap_or_default().iter().step_by(ch).copied());
                    let before = out.len();
                    packbits(&row, &mut out);
                    let count = out.len() - before;
                    let at = table + (c * hu + y) * width;
                    let bytes: Vec<u8> = if psb {
                        u32::try_from(count).map_err(|_| Error::Encode("PSD row too long".into()))?.to_be_bytes().to_vec()
                    } else {
                        u16::try_from(count).map_err(|_| Error::Encode("PSD row too long".into()))?.to_be_bytes().to_vec()
                    };
                    if let Some(slot) = out.get_mut(at..at + width) {
                        slot.copy_from_slice(&bytes);
                    }
                }
            }
        }
        Samples::U16(src) => {
            out.extend_from_slice(&0u16.to_be_bytes());
            for c in 0..colour {
                for s in src.iter().take(needed).skip(c).step_by(ch) {
                    out.extend_from_slice(&s.to_be_bytes());
                }
            }
        }
        Samples::F32(_) => {}
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packbits_round_trips_through_the_reader() {
        for row in [vec![1u8, 1, 1, 1, 2, 3, 4, 4, 4], vec![7; 300], (0..=255).collect::<Vec<u8>>(), vec![5]] {
            let mut enc = Vec::new();
            packbits(&row, &mut enc);
            let mut dec = Vec::new();
            crate::psd::unpackbits_for_tests(&enc, row.len(), &mut dec);
            assert_eq!(dec, row);
        }
    }

    #[test]
    fn writes_a_psd_the_reader_opens() {
        let px: Vec<u8> = vec![10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255];
        let img = EncodeImage::new(2, 2, 4, Samples::U8(&px));
        let bytes = encode_psd(&img, &EncodeMeta { xmp: Some("<x/>"), ppi: Some(300), ..Default::default() }).unwrap();
        let d = crate::decode(&bytes, crate::DecodeOptions::default()).unwrap();
        assert_eq!((d.image.width, d.image.height), (2, 2));
        let px16: Vec<u16> = vec![1000, 2000, 3000, 4000, 5000, 6000];
        let img = EncodeImage::new(2, 1, 3, Samples::U16(&px16));
        assert!(encode_psd(&img, &EncodeMeta::default()).is_ok());
        assert!(encode_psd(&EncodeImage::new(2, 2, 3, Samples::U8(&[0; 5])), &EncodeMeta::default()).is_err());
    }
}
