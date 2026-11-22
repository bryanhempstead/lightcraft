//! The side files the background indexers keep their results in, next to the library: one record per photo
//! (keyed by the photo's content key), appended as each photo is done, so an index survives a quit or a crash
//! and the next run starts where the last one stopped.
//!
//! Format: a header line `LCVX1 <tag>\n` (the tag names the models and version: a different tag means the
//! results are stale and the file starts over), then records `[u32 key length][key][u32 value length][value]
//! [u32 CRC-32 of key + value]`, little-endian. A later record for the same key replaces an earlier one; an empty
//! value removes it. Loading stops at the first record that is cut short or fails its checksum (a crash
//! mid-append) and the file is cut back there. When more than half of the file is replaced records, it is
//! rewritten (temporary file + rename).

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::{Error, Result};

const MAGIC: &str = "LCVX1 ";
/// Keys and values larger than this are refused (a damaged length).
const MAX_KEY: usize = 64 << 10;
const MAX_VALUE: usize = 16 << 20;
/// Files are never read past this.
const MAX_FILE: u64 = 4 << 30;

/// How a value is written into and read from a record.
pub trait Codec: Sized {
    fn encode(&self, out: &mut Vec<u8>);
    fn decode(b: &[u8]) -> Option<Self>;
}

/// An index file and everything in it.
pub struct Store<V> {
    path: PathBuf,
    tag: String,
    entries: HashMap<String, V>,
    file: Option<std::fs::File>,
    /// Bytes of records in the file, and of each live one.
    bytes: u64,
    sizes: HashMap<String, u64>,
}

fn io(path: &Path, e: std::io::Error) -> Error {
    Error::Io(format!("{}: {e}", path.display()))
}

fn record(key: &str, value: &[u8]) -> Vec<u8> {
    let mut r = Vec::with_capacity(key.len() + value.len() + 12);
    r.extend_from_slice(&(key.len() as u32).to_le_bytes());
    r.extend_from_slice(key.as_bytes());
    r.extend_from_slice(&(value.len() as u32).to_le_bytes());
    r.extend_from_slice(value);
    let mut h = crc32fast::Hasher::new();
    h.update(key.as_bytes());
    h.update(value);
    r.extend_from_slice(&h.finalize().to_le_bytes());
    r
}

/// The next record of `b` from `at`: key, value, and where the following one starts.
fn next(b: &[u8], at: usize) -> Option<(String, &[u8], usize)> {
    let u32_at =
        |i: usize| -> Option<usize> { b.get(i..i.checked_add(4)?).and_then(|s| s.try_into().ok()).map(|s: [u8; 4]| u32::from_le_bytes(s) as usize) };
    let kl = u32_at(at)?;
    if kl > MAX_KEY {
        return None;
    }
    let k0 = at.checked_add(4)?;
    let key = b.get(k0..k0.checked_add(kl)?)?;
    let vl = u32_at(k0 + kl)?;
    if vl > MAX_VALUE {
        return None;
    }
    let v0 = k0 + kl + 4;
    let value = b.get(v0..v0.checked_add(vl)?)?;
    let crc = u32_at(v0 + vl)? as u32;
    let mut h = crc32fast::Hasher::new();
    h.update(key);
    h.update(value);
    if h.finalize() != crc {
        return None;
    }
    Some((String::from_utf8_lossy(key).into_owned(), value, v0 + vl + 4))
}

impl<V: Codec> Store<V> {
    /// Open (or start) the index at `path` for results tagged `tag`. A file with another tag, or one that isn't an
    /// index at all, is set aside as `<name>.old` and a new one started.
    pub fn open(path: &Path, tag: &str) -> Result<Store<V>> {
        let mut s =
            Store { path: path.to_path_buf(), tag: tag.replace('\n', " "), entries: HashMap::new(), file: None, bytes: 0, sizes: HashMap::new() };
        let header = format!("{MAGIC}{}\n", s.tag);
        let mut bytes = Vec::new();
        match std::fs::File::open(path) {
            Ok(f) => {
                f.take(MAX_FILE).read_to_end(&mut bytes).map_err(|e| io(path, e))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(path, e)),
        }
        let mut good = 0usize;
        if !bytes.is_empty() {
            if bytes.starts_with(header.as_bytes()) {
                good = header.len();
                while let Some((key, value, end)) = next(&bytes, good) {
                    let size = (end - good) as u64;
                    s.bytes += size;
                    if value.is_empty() {
                        s.entries.remove(&key);
                        s.sizes.remove(&key);
                    } else if let Some(v) = V::decode(value) {
                        s.entries.insert(key.clone(), v);
                        s.sizes.insert(key, size);
                    }
                    good = end;
                }
                if good < bytes.len() {
                    log::warn!("{}: {} damaged bytes at the end dropped", path.display(), bytes.len() - good);
                }
            } else {
                let old = path.with_extension("old");
                log::info!("{}: made by other models or another version; starting over (kept as {})", path.display(), old.display());
                std::fs::rename(path, &old).map_err(|e| io(path, e))?;
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path).map_err(|e| io(path, e))?;
        if good == 0 {
            f.set_len(0).map_err(|e| io(path, e))?;
            f.write_all(header.as_bytes()).map_err(|e| io(path, e))?;
        } else if (good as u64) < f.metadata().map_err(|e| io(path, e))?.len() {
            f.set_len(good as u64).map_err(|e| io(path, e))?;
        }
        f.seek(SeekFrom::End(0)).map_err(|e| io(path, e))?;
        s.file = Some(f);
        Ok(s)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.entries.iter()
    }

    fn append(&mut self, key: &str, value: &[u8]) -> Result<u64> {
        if key.len() > MAX_KEY || value.len() > MAX_VALUE {
            return Err(Error::Io("index record too large".into()));
        }
        let r = record(key, value);
        let path = self.path.clone();
        let f = self.file.as_mut().ok_or_else(|| Error::Io(format!("{} is not open", path.display())))?;
        f.write_all(&r).map_err(|e| io(&path, e))?;
        self.bytes += r.len() as u64;
        Ok(r.len() as u64)
    }

    /// Keep `value` for `key` (written to the file at once).
    pub fn put(&mut self, key: &str, value: V) -> Result<()> {
        let mut b = Vec::new();
        value.encode(&mut b);
        if b.is_empty() {
            return Err(Error::Io("an empty index value".into()));
        }
        let n = self.append(key, &b)?;
        self.sizes.insert(key.to_string(), n);
        self.entries.insert(key.to_string(), value);
        self.maybe_compact()
    }

    /// Forget `key`.
    pub fn remove(&mut self, key: &str) -> Result<()> {
        if self.entries.remove(key).is_some() {
            self.sizes.remove(key);
            self.append(key, &[])?;
        }
        Ok(())
    }

    /// Make what was written durable.
    pub fn sync(&mut self) -> Result<()> {
        let path = self.path.clone();
        match self.file.as_mut() {
            Some(f) => f.sync_data().map_err(|e| io(&path, e)),
            None => Ok(()),
        }
    }

    fn maybe_compact(&mut self) -> Result<()> {
        let live: u64 = self.sizes.values().sum();
        if self.bytes < 1 << 20 || self.bytes < live.saturating_mul(2) {
            return Ok(());
        }
        self.compact()
    }

    /// Rewrite the file with only the live records.
    pub fn compact(&mut self) -> Result<()> {
        let tmp = self.path.with_extension("tmp");
        let mut out = format!("{MAGIC}{}\n", self.tag).into_bytes();
        let mut keys: Vec<&String> = self.entries.keys().collect();
        keys.sort();
        let mut sizes = HashMap::new();
        for k in keys {
            if let Some(v) = self.entries.get(k) {
                let mut b = Vec::new();
                v.encode(&mut b);
                let r = record(k, &b);
                sizes.insert(k.clone(), r.len() as u64);
                out.extend_from_slice(&r);
            }
        }
        {
            let mut f = std::fs::File::create(&tmp).map_err(|e| io(&tmp, e))?;
            f.write_all(&out).map_err(|e| io(&tmp, e))?;
            f.sync_all().map_err(|e| io(&tmp, e))?;
        }
        self.file = None;
        std::fs::rename(&tmp, &self.path).map_err(|e| io(&self.path, e))?;
        let mut f = std::fs::OpenOptions::new().read(true).write(true).open(&self.path).map_err(|e| io(&self.path, e))?;
        f.seek(SeekFrom::End(0)).map_err(|e| io(&self.path, e))?;
        self.file = Some(f);
        self.bytes = out.len() as u64;
        self.sizes = sizes;
        Ok(())
    }
}

// ------------------------------------------------------------------------------------------------ values

struct Reader<'a> {
    b: &'a [u8],
}

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let (h, t) = (self.b.get(..n)?, self.b.get(n..)?);
        self.b = t;
        Some(h)
    }
    fn u8(&mut self) -> Option<u8> {
        self.bytes(1)?.first().copied()
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn f32s(&mut self, n: usize) -> Option<Vec<f32>> {
        let b = self.bytes(n.checked_mul(4)?)?;
        Some(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
    }
}

fn put_f32s(out: &mut Vec<u8>, v: &[f32]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

/// One face of a photo, as the face indexer found it. Coordinates are normalized (0..1) to the upright,
/// uncropped photo, top-left origin.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceEntry {
    /// `[x, y, w, h]`.
    pub rect: [f32; 4],
    pub landmarks: [[f32; 2]; 5],
    /// The detector's confidence; 0 for a face that came from a region the detector didn't find.
    pub score: f32,
    /// Unit-length face embedding (SFace, 128 numbers).
    pub embedding: Vec<f32>,
    /// Probability each eye is closed (`None`: too small to tell, or not analysed).
    pub eyes: Option<[f32; 2]>,
    /// Detail in the face, 0..100 (the culling focus measure, on the face alone).
    pub sharpness: f32,
}

/// Everything the face indexer found in one photo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceRecord {
    pub faces: Vec<FaceEntry>,
    /// The analysed picture's size (pixels): how much detail the coordinates carry.
    pub width: u32,
    pub height: u32,
}

impl Codec for FaceRecord {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(1); // version
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&(self.faces.len().min(u16::MAX as usize) as u16).to_le_bytes());
        for f in self.faces.iter().take(u16::MAX as usize) {
            put_f32s(out, &f.rect);
            for l in &f.landmarks {
                put_f32s(out, l);
            }
            put_f32s(out, &[f.score, f.sharpness]);
            match f.eyes {
                Some(e) => {
                    out.push(1);
                    put_f32s(out, &e);
                }
                None => out.push(0),
            }
            let n = f.embedding.len().min(u16::MAX as usize);
            out.extend_from_slice(&(n as u16).to_le_bytes());
            put_f32s(out, f.embedding.get(..n).unwrap_or(&[]));
        }
    }

    fn decode(b: &[u8]) -> Option<FaceRecord> {
        let mut r = Reader { b };
        if r.u8()? != 1 {
            return None;
        }
        let (width, height) = (r.u32()?, r.u32()?);
        let n = r.u16()? as usize;
        let mut faces = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let rect: [f32; 4] = r.f32s(4)?.try_into().ok()?;
            let mut landmarks = [[0f32; 2]; 5];
            for l in &mut landmarks {
                *l = [r.f32()?, r.f32()?];
            }
            let (score, sharpness) = (r.f32()?, r.f32()?);
            let eyes = match r.u8()? {
                1 => Some([r.f32()?, r.f32()?]),
                _ => None,
            };
            let dim = r.u16()? as usize;
            let embedding = r.f32s(dim)?;
            faces.push(FaceEntry { rect, landmarks, score, embedding, eyes, sharpness });
        }
        Some(FaceRecord { faces, width, height })
    }
}

/// A photo's image embedding (CLIP), unit length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Embedding(pub Vec<f32>);

impl Codec for Embedding {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(1);
        out.extend_from_slice(&(self.0.len().min(u16::MAX as usize) as u16).to_le_bytes());
        put_f32s(out, self.0.get(..self.0.len().min(u16::MAX as usize)).unwrap_or(&[]));
    }

    fn decode(b: &[u8]) -> Option<Embedding> {
        let mut r = Reader { b };
        if r.u8()? != 1 {
            return None;
        }
        let n = r.u16()? as usize;
        Some(Embedding(r.f32s(n)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-vision-index-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    fn face(x: f32) -> FaceEntry {
        FaceEntry { rect: [x, 0.1, 0.2, 0.3], landmarks: [[0.5; 2]; 5], score: 0.9, embedding: vec![x; 128], eyes: Some([0.1, 0.9]), sharpness: 42.0 }
    }

    #[test]
    fn records_survive_reopening_and_a_torn_tail() {
        let d = dir();
        let p = d.join("faces.lcidx");
        {
            let mut s: Store<FaceRecord> = Store::open(&p, "faces v1").unwrap();
            s.put("a", FaceRecord { faces: vec![face(0.1)], width: 10, height: 20 }).unwrap();
            s.put("b", FaceRecord { faces: vec![face(0.2), face(0.3)], width: 10, height: 20 }).unwrap();
            s.put("a", FaceRecord { faces: vec![], width: 1, height: 1 }).unwrap();
            s.remove("b").unwrap();
            s.put("c", FaceRecord::default()).unwrap();
        }
        // a crash mid-append
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(&[9, 0, 0, 0, b'x']).unwrap();
        drop(f);
        let s: Store<FaceRecord> = Store::open(&p, "faces v1").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s.get("a").map(|r| r.width), Some(1));
        assert!(s.get("b").is_none());
        drop(s);
        // another tag starts over
        let s: Store<FaceRecord> = Store::open(&p, "faces v2").unwrap();
        assert!(s.is_empty());
        assert!(d.join("faces.old").exists());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn compaction_keeps_the_live_records() {
        let d = dir();
        let p = d.join("search.lcidx");
        let mut s: Store<Embedding> = Store::open(&p, "clip").unwrap();
        for i in 0..2000 {
            s.put("same", Embedding(vec![i as f32; 512])).unwrap();
        }
        s.put("other", Embedding(vec![1.0; 4])).unwrap();
        assert!(std::fs::metadata(&p).unwrap().len() < 3 << 20);
        drop(s);
        let s: Store<Embedding> = Store::open(&p, "clip").unwrap();
        assert_eq!(s.get("same").map(|e| e.0[0]), Some(1999.0));
        assert_eq!(s.len(), 2);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn damaged_values_are_refused() {
        assert!(FaceRecord::decode(&[1, 0, 0]).is_none());
        assert!(FaceRecord::decode(&[2]).is_none());
        assert!(Embedding::decode(&[1, 255, 255, 0]).is_none());
        let mut b = Vec::new();
        FaceRecord { faces: vec![face(0.5)], width: 3, height: 4 }.encode(&mut b);
        assert_eq!(FaceRecord::decode(&b).map(|r| r.faces[0].clone()), Some(face(0.5)));
        assert!(FaceRecord::decode(&b[..b.len() - 1]).is_none());
    }
}
