//! Bounded decoding of the ONNX protobuf fields a plain inference graph needs, from the public Apache-2.0 schema
//! (https://github.com/onnx/onnx/blob/main/onnx/onnx.proto3). No generated code, no protoc, no external tensor files.
//! Adapted from `lightcraft_enhance::onnx_proto` (LightCraft's own code) and extended to every attribute and tensor
//! type these graphs use (int64 shapes, float lists, `Constant` tensors, several outputs).

use crate::{Error, Result};

/// The largest model file read.
pub const MAX_BYTES: usize = 512 << 20;
/// Elements of one tensor at most.
pub const MAX_ELEMENTS: usize = 1 << 28;
const MAX_ITEMS: usize = 1 << 16;
const MAX_STRING: usize = 4096;
const MAX_LIST: usize = 1 << 16;

fn bad() -> Error {
    Error::Model("malformed or oversized ONNX file".into())
}

enum Wire<'a> {
    Int(u64),
    Bytes(&'a [u8]),
    F32(f32),
    F64,
}

fn varint(rest: &mut &[u8]) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let (b, tail) = rest.split_first().ok_or_else(bad)?;
        *rest = tail;
        if shift == 63 && *b > 1 {
            return Err(bad());
        }
        value |= u64::from(*b & 127) << shift;
        if b & 128 == 0 {
            return Ok(value);
        }
    }
    Err(bad())
}

struct Fields<'a> {
    rest: &'a [u8],
}

impl<'a> Fields<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let out = self.rest.get(..n).ok_or_else(bad)?;
        self.rest = self.rest.get(n..).ok_or_else(bad)?;
        Ok(out)
    }
    fn next(&mut self) -> Result<Option<(u32, Wire<'a>)>> {
        if self.rest.is_empty() {
            return Ok(None);
        }
        let tag = varint(&mut self.rest)?;
        let number = u32::try_from(tag >> 3).map_err(|_| bad())?;
        if number == 0 || number >= 1 << 29 {
            return Err(bad());
        }
        let wire = match tag & 7 {
            0 => Wire::Int(varint(&mut self.rest)?),
            1 => {
                let b: [u8; 8] = self.take(8)?.try_into().map_err(|_| bad())?;
                let _ = b;
                Wire::F64
            }
            2 => {
                let n = usize::try_from(varint(&mut self.rest)?).map_err(|_| bad())?;
                Wire::Bytes(self.take(n)?)
            }
            5 => {
                let b: [u8; 4] = self.take(4)?.try_into().map_err(|_| bad())?;
                Wire::F32(f32::from_le_bytes(b))
            }
            _ => return Err(bad()),
        };
        Ok(Some((number, wire)))
    }
}

fn fields(b: &[u8]) -> Fields<'_> {
    Fields { rest: b }
}

fn string(b: &[u8]) -> Result<String> {
    if b.len() > MAX_STRING {
        return Err(bad());
    }
    Ok(String::from_utf8_lossy(b).into_owned())
}

fn push<T>(v: &mut Vec<T>, value: T, cap: usize) -> Result<()> {
    if v.len() >= cap {
        return Err(bad());
    }
    v.try_reserve(1).map_err(|_| bad())?;
    v.push(value);
    Ok(())
}

fn ints(w: Wire<'_>, out: &mut Vec<i64>, cap: usize) -> Result<()> {
    match w {
        Wire::Int(v) => push(out, v as i64, cap),
        Wire::Bytes(mut b) => {
            while !b.is_empty() {
                let v = varint(&mut b)?;
                push(out, v as i64, cap)?;
            }
            Ok(())
        }
        _ => Err(bad()),
    }
}

fn floats(w: Wire<'_>, out: &mut Vec<f32>, cap: usize) -> Result<()> {
    match w {
        Wire::F32(v) => push(out, v, cap),
        Wire::Bytes(b) => {
            if !b.len().is_multiple_of(4) || out.len().saturating_add(b.len() / 4) > cap {
                return Err(bad());
            }
            out.try_reserve(b.len() / 4).map_err(|_| bad())?;
            out.extend(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)));
            Ok(())
        }
        _ => Err(bad()),
    }
}

/// A tensor stored in the file: float32 or int64 (other element types are refused when used).
#[derive(Clone, Debug, Default)]
pub struct TensorProto {
    pub name: String,
    pub dims: Vec<i64>,
    pub data_type: i32,
    pub floats: Vec<f32>,
    pub ints: Vec<i64>,
    pub raw: Vec<u8>,
}

impl TensorProto {
    pub const FLOAT: i32 = 1;
    pub const INT32: i32 = 6;
    pub const INT64: i32 = 7;

    /// The element count its dims give (1 for a scalar).
    pub fn elements(&self) -> Result<usize> {
        self.dims
            .iter()
            .try_fold(1usize, |n, &d| usize::try_from(d).ok().and_then(|d| n.checked_mul(d)).ok_or_else(bad))
            .and_then(|n| if n > MAX_ELEMENTS { Err(bad()) } else { Ok(n) })
    }

    /// Its values as floats (float32 data; int64 converted).
    pub fn to_f32(&self) -> Result<Vec<f32>> {
        let n = self.elements()?;
        let v: Vec<f32> = match self.data_type {
            Self::FLOAT if !self.raw.is_empty() => {
                if self.raw.len() != n * 4 {
                    return Err(bad());
                }
                self.raw.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
            }
            Self::FLOAT => self.floats.clone(),
            Self::INT64 | Self::INT32 => self.to_i64()?.into_iter().map(|v| v as f32).collect(),
            t => return Err(Error::Model(format!("tensor {} has element type {t}, which isn't supported", self.name))),
        };
        if v.len() != n {
            return Err(bad());
        }
        Ok(v)
    }

    /// Its values as integers (int64 / int32 data).
    pub fn to_i64(&self) -> Result<Vec<i64>> {
        let n = self.elements()?;
        let v: Vec<i64> = match self.data_type {
            Self::INT64 if !self.raw.is_empty() => {
                if self.raw.len() != n * 8 {
                    return Err(bad());
                }
                self.raw.as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c)).collect()
            }
            Self::INT32 if !self.raw.is_empty() => {
                if self.raw.len() != n * 4 {
                    return Err(bad());
                }
                self.raw.as_chunks::<4>().0.iter().map(|c| i64::from(i32::from_le_bytes(*c))).collect()
            }
            Self::INT64 | Self::INT32 => self.ints.clone(),
            Self::FLOAT => self.to_f32()?.into_iter().map(|v| v as i64).collect(),
            t => return Err(Error::Model(format!("tensor {} has element type {t}, which isn't supported", self.name))),
        };
        if v.len() != n {
            return Err(bad());
        }
        Ok(v)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Attribute {
    pub name: String,
    pub f: Option<f32>,
    pub i: Option<i64>,
    pub s: Vec<u8>,
    pub t: Option<TensorProto>,
    pub floats: Vec<f32>,
    pub ints: Vec<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub input: Vec<String>,
    pub output: Vec<String>,
    pub op_type: String,
    pub attribute: Vec<Attribute>,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&Attribute> {
        self.attribute.iter().find(|a| a.name == name)
    }
    pub fn int(&self, name: &str, default: i64) -> i64 {
        self.attr(name).and_then(|a| a.i).unwrap_or(default)
    }
    pub fn float(&self, name: &str, default: f32) -> f32 {
        self.attr(name).and_then(|a| a.f).unwrap_or(default)
    }
    pub fn ints(&self, name: &str) -> Option<&[i64]> {
        self.attr(name).map(|a| a.ints.as_slice())
    }
    pub fn string(&self, name: &str) -> Option<String> {
        self.attr(name).map(|a| String::from_utf8_lossy(&a.s).into_owned())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub node: Vec<Node>,
    pub initializer: Vec<TensorProto>,
    pub input: Vec<String>,
    pub output: Vec<String>,
}

fn tensor(b: &[u8]) -> Result<TensorProto> {
    let mut t = TensorProto::default();
    let mut f = fields(b);
    while let Some((n, w)) = f.next()? {
        match (n, w) {
            (1, w) => ints(w, &mut t.dims, 16)?,
            (2, Wire::Int(v)) => t.data_type = i32::try_from(v).map_err(|_| bad())?,
            (4, w) => floats(w, &mut t.floats, MAX_ELEMENTS)?,
            (5 | 7, w) => ints(w, &mut t.ints, MAX_ELEMENTS)?,
            (8, Wire::Bytes(b)) => t.name = string(b)?,
            (9, Wire::Bytes(b)) => {
                if b.len() > MAX_ELEMENTS * 8 {
                    return Err(bad());
                }
                t.raw = b.to_vec();
            }
            (13, _) | (14, Wire::Int(1..)) => return Err(Error::Model("the model keeps its weights in another file".into())),
            _ => {}
        }
    }
    Ok(t)
}

fn attribute(b: &[u8]) -> Result<Attribute> {
    let mut a = Attribute::default();
    let mut f = fields(b);
    while let Some((n, w)) = f.next()? {
        match (n, w) {
            (1, Wire::Bytes(b)) => a.name = string(b)?,
            (2, Wire::F32(v)) => a.f = Some(v),
            (3, Wire::Int(v)) => a.i = Some(v as i64),
            (4, Wire::Bytes(b)) => {
                if b.len() > MAX_STRING {
                    return Err(bad());
                }
                a.s = b.to_vec();
            }
            (5, Wire::Bytes(b)) => a.t = Some(tensor(b)?),
            (7, w) => floats(w, &mut a.floats, MAX_LIST)?,
            (8, w) => ints(w, &mut a.ints, MAX_LIST)?,
            _ => {}
        }
    }
    Ok(a)
}

fn node(b: &[u8]) -> Result<Node> {
    let mut a = Node::default();
    let mut f = fields(b);
    while let Some((n, w)) = f.next()? {
        match (n, w) {
            (1, Wire::Bytes(b)) => push(&mut a.input, string(b)?, 64)?,
            (2, Wire::Bytes(b)) => push(&mut a.output, string(b)?, 64)?,
            (4, Wire::Bytes(b)) => a.op_type = string(b)?,
            (5, Wire::Bytes(b)) => push(&mut a.attribute, attribute(b)?, 64)?,
            (7, Wire::Bytes(b)) if !b.is_empty() && b != b"ai.onnx" => {
                return Err(Error::Model(format!("the model uses operators of the domain {}", String::from_utf8_lossy(b))));
            }
            _ => {}
        }
    }
    Ok(a)
}

fn value_name(b: &[u8]) -> Result<String> {
    let mut f = fields(b);
    while let Some((n, w)) = f.next()? {
        if let (1, Wire::Bytes(b)) = (n, w) {
            return string(b);
        }
    }
    Ok(String::new())
}

fn graph(b: &[u8]) -> Result<Graph> {
    let mut g = Graph::default();
    let mut f = fields(b);
    while let Some((n, w)) = f.next()? {
        match (n, w) {
            (1, Wire::Bytes(b)) => push(&mut g.node, node(b)?, MAX_ITEMS)?,
            (5, Wire::Bytes(b)) => push(&mut g.initializer, tensor(b)?, MAX_ITEMS)?,
            (11, Wire::Bytes(b)) => push(&mut g.input, value_name(b)?, MAX_ITEMS)?,
            (12, Wire::Bytes(b)) => push(&mut g.output, value_name(b)?, 256)?,
            (15, _) => return Err(Error::Model("sparse initializers aren't supported".into())),
            _ => {}
        }
    }
    Ok(g)
}

/// The graph of an ONNX model file's bytes.
pub fn decode(b: &[u8]) -> Result<Graph> {
    if b.len() > MAX_BYTES {
        return Err(bad());
    }
    let mut f = fields(b);
    let mut out = None;
    while let Some((n, w)) = f.next()? {
        if let (7, Wire::Bytes(b)) = (n, w) {
            if out.is_some() {
                return Err(bad());
            }
            out = Some(graph(b)?);
        }
    }
    out.ok_or_else(bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for bytes in [&b""[..], b"\x00", b"\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff", b"\x3a\x10abc", b"\x3a\x05\x0a\x03\x0a\x01"] {
            assert!(decode(bytes).is_err());
        }
        // a varint length running past the end
        assert!(decode(&[0x3a, 0xff, 0xff, 0x03]).is_err());
    }

    #[test]
    fn tensor_sizes_are_checked() {
        let t = TensorProto { name: "x".into(), dims: vec![2, 2], data_type: TensorProto::FLOAT, floats: vec![1.0; 3], ..Default::default() };
        assert!(t.to_f32().is_err());
        let t = TensorProto { dims: vec![-1], data_type: TensorProto::FLOAT, ..Default::default() };
        assert!(t.to_f32().is_err());
        let t = TensorProto { dims: vec![1 << 20, 1 << 20], data_type: TensorProto::FLOAT, ..Default::default() };
        assert!(t.elements().is_err());
        let t = TensorProto {
            dims: vec![2],
            data_type: TensorProto::INT64,
            raw: [3i64.to_le_bytes(), (-1i64).to_le_bytes()].concat(),
            ..Default::default()
        };
        assert_eq!(t.to_i64().ok(), Some(vec![3, -1]));
    }
}
