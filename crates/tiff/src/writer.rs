//! TIFF writer: IFD trees with strips / tiles, `SubIFDs` and pointer IFDs (Exif, GPS, Interop, …).
//!
//! Layout: header, then for each IFD of the main chain its image chunks, child IFDs, out-of-line values
//! and finally the IFD itself (all word-aligned). Children are written before their parent so every
//! pointer is known when the parent's entries are emitted; only the chain's next-IFD pointers are patched.

use crate::{ByteOrder, Result, TiffError, Value, tags};
use std::collections::BTreeMap;

/// Pixel data attached to an IFD. The writer fills in the offset / byte-count tags.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageData {
    /// Strips (each already compressed as the IFD's `Compression` says). Sets `RowsPerStrip`.
    Strips { rows_per_strip: u32, strips: Vec<Vec<u8>> },
    /// Tiles in row-major tile order. Sets `TileWidth` / `TileLength`.
    Tiles { tile_width: u32, tile_height: u32, tiles: Vec<Vec<u8>> },
}

/// An IFD to be written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IfdBuilder {
    pub entries: BTreeMap<u16, Value>,
    /// Written and referenced through the `SubIFDs` (330) tag.
    pub sub_ifds: Vec<IfdBuilder>,
    /// Pointer IFDs keyed by the pointer tag (e.g. [`tags::EXIF_IFD`], [`tags::GPS_IFD`], [`tags::INTEROP_IFD`]).
    pub children: BTreeMap<u16, IfdBuilder>,
    pub image: Option<ImageData>,
}

impl IfdBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, tag: u16, value: Value) -> &mut Self {
        self.entries.insert(tag, value);
        self
    }
    /// Builder-style [`IfdBuilder::set`].
    pub fn with(mut self, tag: u16, value: Value) -> Self {
        self.entries.insert(tag, value);
        self
    }
    pub fn set_child(&mut self, tag: u16, child: IfdBuilder) -> &mut Self {
        self.children.insert(tag, child);
        self
    }
    pub fn add_sub_ifd(&mut self, sub: IfdBuilder) -> &mut Self {
        self.sub_ifds.push(sub);
        self
    }
    pub fn set_image(&mut self, image: ImageData) -> &mut Self {
        self.image = Some(image);
        self
    }
}

/// Serialises IFD chains.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TiffWriter {
    pub order: ByteOrder,
    pub bigtiff: bool,
}

impl TiffWriter {
    pub fn new(order: ByteOrder, bigtiff: bool) -> Self {
        Self { order, bigtiff }
    }

    /// Write `chain` (IFD0, IFD1, …) as a complete TIFF stream.
    pub fn write(&self, chain: &[IfdBuilder]) -> Result<Vec<u8>> {
        if chain.is_empty() {
            return Err(TiffError::Invalid("no IFDs to write".into()));
        }
        let o = self.order;
        let mut buf = Vec::new();
        buf.extend_from_slice(match o {
            ByteOrder::Little => b"II",
            ByteOrder::Big => b"MM",
        });
        let mut patch = if self.bigtiff {
            o.put_u16(&mut buf, 43);
            o.put_u16(&mut buf, 8);
            o.put_u16(&mut buf, 0);
            let p = buf.len();
            o.put_u64(&mut buf, 0);
            p
        } else {
            o.put_u16(&mut buf, 42);
            let p = buf.len();
            o.put_u32(&mut buf, 0);
            p
        };
        for ifd in chain {
            let (off, next_pos) = self.write_ifd(&mut buf, ifd, 0)?;
            self.patch_offset(&mut buf, patch, off)?;
            patch = next_pos;
        }
        Ok(buf)
    }

    fn patch_offset(&self, buf: &mut [u8], pos: usize, v: u64) -> Result<()> {
        let mut tmp = Vec::with_capacity(8);
        if self.bigtiff {
            self.order.put_u64(&mut tmp, v);
        } else {
            self.order.put_u32(&mut tmp, u32::try_from(v).map_err(|_| TiffError::Limit("classic TIFF larger than 4 GiB"))?);
        }
        buf[pos..pos + tmp.len()].copy_from_slice(&tmp);
        Ok(())
    }

    fn offset_value(&self, v: Vec<u64>) -> Result<Value> {
        if self.bigtiff {
            Ok(Value::Long8(v))
        } else {
            Ok(Value::Long(
                v.into_iter().map(|x| u32::try_from(x).map_err(|_| TiffError::Limit("classic TIFF larger than 4 GiB"))).collect::<Result<_>>()?,
            ))
        }
    }

    /// Writes the IFD (and everything it references); returns (IFD offset, position of its next-IFD pointer).
    fn write_ifd(&self, buf: &mut Vec<u8>, ifd: &IfdBuilder, depth: usize) -> Result<(u64, usize)> {
        if depth > 16 {
            return Err(TiffError::Limit("IFD nesting depth"));
        }
        let o = self.order;
        let mut entries = ifd.entries.clone();
        if let Some(img) = &ifd.image {
            let chunks = match img {
                ImageData::Strips { rows_per_strip, strips } => {
                    entries.insert(tags::ROWS_PER_STRIP, Value::Long(vec![*rows_per_strip]));
                    strips
                }
                ImageData::Tiles { tile_width, tile_height, tiles } => {
                    entries.insert(tags::TILE_WIDTH, Value::Long(vec![*tile_width]));
                    entries.insert(tags::TILE_LENGTH, Value::Long(vec![*tile_height]));
                    tiles
                }
            };
            let mut offs = Vec::with_capacity(chunks.len());
            let mut counts = Vec::with_capacity(chunks.len());
            for c in chunks {
                align(buf);
                offs.push(buf.len() as u64);
                counts.push(c.len() as u64);
                buf.extend_from_slice(c);
            }
            let (ot, ct) = match img {
                ImageData::Strips { .. } => (tags::STRIP_OFFSETS, tags::STRIP_BYTE_COUNTS),
                ImageData::Tiles { .. } => (tags::TILE_OFFSETS, tags::TILE_BYTE_COUNTS),
            };
            entries.insert(ot, self.offset_value(offs)?);
            entries.insert(ct, self.offset_value(counts)?);
        }
        if !ifd.sub_ifds.is_empty() {
            let mut offs = Vec::new();
            for s in &ifd.sub_ifds {
                offs.push(self.write_ifd(buf, s, depth + 1)?.0);
            }
            entries.insert(tags::SUB_IFDS, self.offset_value(offs)?);
        }
        for (&tag, child) in &ifd.children {
            let off = self.write_ifd(buf, child, depth + 1)?.0;
            entries.insert(tag, self.offset_value(vec![off])?);
        }
        let inline = if self.bigtiff { 8 } else { 4 };
        // out-of-line values
        let mut encoded: Vec<(u16, &Value, Vec<u8>, Option<u64>)> = Vec::with_capacity(entries.len());
        for (&tag, v) in &entries {
            let bytes = encode(o, v);
            let pos = if bytes.len() > inline {
                align(buf);
                let p = buf.len() as u64;
                buf.extend_from_slice(&bytes);
                Some(p)
            } else {
                None
            };
            encoded.push((tag, v, bytes, pos));
        }
        align(buf);
        let ifd_off = buf.len() as u64;
        if !self.bigtiff && ifd_off > u32::MAX as u64 {
            return Err(TiffError::Limit("classic TIFF larger than 4 GiB"));
        }
        if self.bigtiff {
            o.put_u64(buf, encoded.len() as u64);
        } else {
            o.put_u16(buf, u16::try_from(encoded.len()).map_err(|_| TiffError::Limit("more than 65535 entries"))?);
        }
        for (tag, v, bytes, pos) in &encoded {
            o.put_u16(buf, *tag);
            o.put_u16(buf, v.field_type() as u16);
            if self.bigtiff {
                o.put_u64(buf, v.count() as u64);
            } else {
                o.put_u32(buf, u32::try_from(v.count()).map_err(|_| TiffError::Limit("value count"))?);
            }
            match pos {
                Some(p) => {
                    if self.bigtiff {
                        o.put_u64(buf, *p);
                    } else {
                        o.put_u32(buf, u32::try_from(*p).map_err(|_| TiffError::Limit("classic TIFF larger than 4 GiB"))?);
                    }
                }
                None => {
                    buf.extend_from_slice(bytes);
                    buf.extend(std::iter::repeat_n(0u8, inline - bytes.len()));
                }
            }
        }
        let next_pos = buf.len();
        if self.bigtiff {
            o.put_u64(buf, 0);
        } else {
            o.put_u32(buf, 0);
        }
        Ok((ifd_off, next_pos))
    }
}

fn align(buf: &mut Vec<u8>) {
    if buf.len() % 2 == 1 {
        buf.push(0);
    }
}

/// Serialise a value's bytes in `o` order.
pub fn encode(o: ByteOrder, v: &Value) -> Vec<u8> {
    let mut b = Vec::new();
    match v {
        Value::Byte(x) | Value::Undefined(x) => b.extend_from_slice(x),
        Value::Ascii(s) => {
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        }
        Value::SByte(x) => b.extend(x.iter().map(|&c| c as u8)),
        Value::Short(x) => x.iter().for_each(|&c| o.put_u16(&mut b, c)),
        Value::SShort(x) => x.iter().for_each(|&c| o.put_u16(&mut b, c as u16)),
        Value::Long(x) | Value::Ifd(x) => x.iter().for_each(|&c| o.put_u32(&mut b, c)),
        Value::SLong(x) => x.iter().for_each(|&c| o.put_u32(&mut b, c as u32)),
        Value::Float(x) => x.iter().for_each(|&c| o.put_u32(&mut b, c.to_bits())),
        Value::Rational(x) => x.iter().for_each(|&(n, d)| {
            o.put_u32(&mut b, n);
            o.put_u32(&mut b, d);
        }),
        Value::SRational(x) => x.iter().for_each(|&(n, d)| {
            o.put_u32(&mut b, n as u32);
            o.put_u32(&mut b, d as u32);
        }),
        Value::Double(x) => x.iter().for_each(|&c| o.put_u64(&mut b, c.to_bits())),
        Value::Long8(x) | Value::Ifd8(x) => x.iter().for_each(|&c| o.put_u64(&mut b, c)),
        Value::SLong8(x) => x.iter().for_each(|&c| o.put_u64(&mut b, c as u64)),
    }
    b
}

/// Convenience: approximate `v` by an unsigned rational with denominator up to 1e6 (for writing Exif rationals).
pub fn rational(v: f64) -> (u32, u32) {
    if !v.is_finite() || v <= 0.0 {
        return (0, 1);
    }
    for d in [1u32, 10, 100, 1000, 10_000, 100_000, 1_000_000] {
        let n = v * d as f64;
        if n > u32::MAX as f64 {
            return ((v * (d / 10).max(1) as f64).round().min(u32::MAX as f64) as u32, (d / 10).max(1));
        }
        if (n - n.round()).abs() < 1e-9 * d as f64 || d == 1_000_000 {
            return (n.round() as u32, d);
        }
    }
    (0, 1)
}

/// Signed variant of [`rational`].
pub fn srational(v: f64) -> (i32, i32) {
    if !v.is_finite() {
        return (0, 1);
    }
    for d in [1i32, 10, 100, 1000, 10_000, 100_000, 1_000_000] {
        let n = v * d as f64;
        if n.abs() > i32::MAX as f64 {
            let d = (d / 10).max(1);
            return ((v * d as f64).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32, d);
        }
        if (n - n.round()).abs() < 1e-9 * d as f64 || d == 1_000_000 {
            return (n.round() as i32, d);
        }
    }
    (0, 1)
}

/// Put one tag into a classic TIFF's IFD0 without touching the rest of the file: the value is
/// appended, IFD0 is written again after it (sorted, `tag` replaced or added) and the header
/// points at the new copy, so every other entry's out-of-line value keeps its offset. `data` is
/// the value's bytes in the file's byte order (`field_type` 1 BYTE / 7 UNDEFINED for an XMP
/// packet). Used to give a DNG re-saved by Adobe's SDK (built without XMP) its XMP back.
pub fn set_ifd0_tag(file: &[u8], tag: u16, field_type: u16, data: &[u8]) -> Result<Vec<u8>> {
    let order = match file.get(0..2) {
        Some(b"II") => ByteOrder::Little,
        Some(b"MM") => ByteOrder::Big,
        _ => return Err(TiffError::NotTiff),
    };
    let u16_at = |o: usize| -> Result<u16> {
        let b = file.get(o..o.saturating_add(2)).ok_or(TiffError::Truncated(o as u64))?;
        let a = [b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)];
        Ok(if order == ByteOrder::Little { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) })
    };
    let u32_at = |o: usize| -> Result<u32> {
        let b = file.get(o..o.saturating_add(4)).ok_or(TiffError::Truncated(o as u64))?;
        let mut a = [0u8; 4];
        a.copy_from_slice(b);
        Ok(if order == ByteOrder::Little { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    if u16_at(2)? != 42 {
        return Err(TiffError::Invalid("only a classic TIFF's IFD0 can be edited (not BigTIFF)".into()));
    }
    let unit = match field_type {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => return Err(TiffError::Invalid(format!("unknown field type {field_type}"))),
    };
    if !data.len().is_multiple_of(unit) {
        return Err(TiffError::Invalid("the value isn't a whole number of its type".into()));
    }
    let ifd0 = u32_at(4)? as usize;
    let n = u16_at(ifd0)? as usize;
    let mut entries: Vec<[u8; 12]> = Vec::with_capacity(n + 1);
    for i in 0..n {
        let at = ifd0 + 2 + 12 * i;
        let b = file.get(at..at + 12).ok_or(TiffError::Truncated(at as u64))?;
        let mut e = [0u8; 12];
        e.copy_from_slice(b);
        entries.push(e);
    }
    let next = u32_at(ifd0 + 2 + 12 * n)?;
    let put16 = |v: u16| if order == ByteOrder::Little { v.to_le_bytes() } else { v.to_be_bytes() };
    let put32 = |v: u32| if order == ByteOrder::Little { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut out = file.to_vec();
    let count = u32::try_from(data.len() / unit).map_err(|_| TiffError::Limit("tag value larger than 4 GB"))?;
    let mut e = [0u8; 12];
    e[0..2].copy_from_slice(&put16(tag));
    e[2..4].copy_from_slice(&put16(field_type));
    e[4..8].copy_from_slice(&put32(count));
    if data.len() <= 4 {
        e[8..8 + data.len()].copy_from_slice(data);
    } else {
        if !out.len().is_multiple_of(2) {
            out.push(0);
        }
        let at = u32::try_from(out.len()).map_err(|_| TiffError::Limit("file larger than 4 GB"))?;
        e[8..12].copy_from_slice(&put32(at));
        out.extend_from_slice(data);
    }
    let tag_of = |e: &[u8; 12]| if order == ByteOrder::Little { u16::from_le_bytes([e[0], e[1]]) } else { u16::from_be_bytes([e[0], e[1]]) };
    entries.retain(|x| tag_of(x) != tag);
    entries.push(e);
    entries.sort_by_key(tag_of);
    let len = u16::try_from(entries.len()).map_err(|_| TiffError::Limit("too many IFD0 entries"))?;
    if !out.len().is_multiple_of(2) {
        out.push(0);
    }
    let new_ifd = u32::try_from(out.len()).map_err(|_| TiffError::Limit("file larger than 4 GB"))?;
    out.extend_from_slice(&put16(len));
    for x in &entries {
        out.extend_from_slice(x);
    }
    out.extend_from_slice(&put32(next));
    if u32::try_from(out.len()).is_err() {
        return Err(TiffError::Limit("file larger than 4 GB"));
    }
    out[4..8].copy_from_slice(&put32(new_ifd));
    Ok(out)
}

#[cfg(test)]
mod set_tag_tests {
    use super::*;

    #[test]
    fn a_tag_is_added_or_replaced_and_the_rest_survives() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let mut ifd = IfdBuilder::new();
            ifd.set(tags::IMAGE_WIDTH, Value::Long(vec![2]));
            ifd.set(tags::IMAGE_LENGTH, Value::Long(vec![1]));
            ifd.set(tags::MAKE, Value::Ascii("A long camera make that is stored out of line".into()));
            let file = TiffWriter::new(order, false).write(&[ifd]).unwrap();
            let xmp = b"<x:xmpmeta>hello</x:xmpmeta>".to_vec();
            let edited = set_ifd0_tag(&file, 700, 1, &xmp).unwrap();
            let t = crate::Tiff::parse(&edited).unwrap();
            let ifd0 = t.ifds.first().unwrap();
            assert_eq!(ifd0.get(700).and_then(|e| e.value.as_bytes()).map(|b| b.to_vec()), Some(xmp.clone()));
            assert_eq!(ifd0.get(tags::MAKE).and_then(|e| e.value.as_str()).as_deref(), Some("A long camera make that is stored out of line"));
            // replaced, not duplicated
            let again = set_ifd0_tag(&edited, 700, 1, b"<x/>").unwrap();
            let t = crate::Tiff::parse(&again).unwrap();
            assert_eq!(t.ifds.first().unwrap().entries.iter().filter(|e| e.tag == 700).count(), 1);
        }
        // nonsense is an error, never a panic
        assert!(set_ifd0_tag(b"", 700, 1, b"x").is_err());
        assert!(set_ifd0_tag(b"II*\0\xff\xff\xff\xff", 700, 1, b"x").is_err());
        assert!(set_ifd0_tag(b"II*\0\x08\0\0\0\x05\0", 700, 1, b"x").is_err());
        assert!(set_ifd0_tag(b"II+\0\x08\0\0\0", 700, 1, b"x").is_err());
        assert!(set_ifd0_tag(b"II*\0\x08\0\0\0\0\0\0\0\0\0", 700, 99, b"x").is_err());
    }
}
