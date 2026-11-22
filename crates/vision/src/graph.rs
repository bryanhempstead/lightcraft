//! A small ONNX graph interpreter for the face models: the graph is read from the file ([`crate::proto`]) and run
//! node by node on the CPU. Matrix products (convolutions as im2col, `Gemm`, `MatMul`) go through candle's CPU
//! matmul; everything else is plain Rust. Depthwise convolutions are computed directly.
//!
//! Only the operators these small detection / recognition / classification networks are made of are understood
//! (see [`Op`]); any other is a clear [`Error::Model`] naming it. Every shape read from the file is checked before it
//! sizes an allocation or indexes a buffer, so a malformed model is an error, never a panic.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use candle_core::{Device, Tensor};
use rayon::prelude::*;

use crate::proto::{self, Node, TensorProto};
use crate::{Error, Result};

/// A dense float tensor (row-major).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

fn bad(why: impl Into<String>) -> Error {
    Error::Model(why.into())
}

fn numel(shape: &[usize]) -> Result<usize> {
    shape
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .filter(|&n| n <= proto::MAX_ELEMENTS)
        .ok_or_else(|| bad("a tensor in the model is too large"))
}

impl T {
    pub fn new(shape: Vec<usize>, data: Vec<f32>) -> Result<T> {
        if numel(&shape)? != data.len() {
            return Err(bad(format!("tensor of shape {shape:?} holds {} values", data.len())));
        }
        Ok(T { shape, data })
    }

    fn zeros(shape: Vec<usize>) -> Result<T> {
        let n = numel(&shape)?;
        let mut data = Vec::new();
        data.try_reserve_exact(n).map_err(|_| bad("not enough memory to run the model"))?;
        data.resize(n, 0.0);
        Ok(T { shape, data })
    }

    fn dims4(&self) -> Result<(usize, usize, usize, usize)> {
        match self.shape.as_slice() {
            [n, c, h, w] => Ok((*n, *c, *h, *w)),
            s => Err(bad(format!("expected a 4-d tensor, got shape {s:?}"))),
        }
    }

    fn from_proto(t: &TensorProto) -> Result<T> {
        let shape: Vec<usize> = t.dims.iter().map(|&d| usize::try_from(d).map_err(|_| bad("negative tensor dimension"))).collect::<Result<_>>()?;
        T::new(shape, t.to_f32()?)
    }
}

fn mm_err(e: candle_core::Error) -> Error {
    Error::Model(format!("matrix product: {e}"))
}

/// `a` (`m × k`, prepared) times `b` (`k × n`, row-major).
fn matmul(a: &Tensor, b: Vec<f32>, k: usize, n: usize) -> Result<Vec<f32>> {
    let b = Tensor::from_vec(b, (k, n), &Device::Cpu).map_err(mm_err)?;
    a.matmul(&b).and_then(|t| t.flatten_all()).and_then(|t| t.to_vec1::<f32>()).map_err(mm_err)
}

/// One graph node, resolved to value slots.
struct Step {
    node: Node,
    inputs: Vec<Option<usize>>,
    outputs: Vec<usize>,
    /// Weight matrices prepared for candle (convolutions: one per group; Gemm: `B` as `n × k`).
    mats: Vec<Tensor>,
}

/// A loaded model.
pub struct Model {
    steps: Vec<Step>,
    consts: Vec<Option<Arc<T>>>,
    inputs: Vec<(String, usize)>,
    outputs: Vec<(String, usize)>,
    /// The last step that reads each slot (intermediate values are dropped after it).
    last_use: Vec<usize>,
}

const SUPPORTED: &[&str] = &[
    "Conv",
    "Relu",
    "LeakyRelu",
    "PRelu",
    "Sigmoid",
    "Tanh",
    "Exp",
    "ReduceSum",
    "ReduceMean",
    "Clip",
    "Add",
    "Sub",
    "Mul",
    "Div",
    "BatchNormalization",
    "MaxPool",
    "AveragePool",
    "GlobalAveragePool",
    "Resize",
    "Upsample",
    "Transpose",
    "Reshape",
    "Flatten",
    "Squeeze",
    "Unsqueeze",
    "Concat",
    "Gemm",
    "MatMul",
    "Softmax",
    "Dropout",
    "Identity",
    "Constant",
];

impl Model {
    /// Read the model at `path`.
    pub fn load(path: &Path) -> Result<Model> {
        let len = std::fs::metadata(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?.len();
        if len > proto::MAX_BYTES as u64 {
            return Err(bad("model file too large"));
        }
        let bytes = std::fs::read(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        Model::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Model> {
        let g = proto::decode(bytes)?;
        let mut slot: HashMap<String, usize> = HashMap::new();
        let mut consts: Vec<Option<Arc<T>>> = Vec::new();
        for t in &g.initializer {
            let id = consts.len();
            consts.push(Some(Arc::new(T::from_proto(t)?)));
            slot.insert(t.name.clone(), id);
        }
        let mut inputs = Vec::new();
        for name in &g.input {
            if !slot.contains_key(name) {
                let id = consts.len();
                consts.push(None);
                slot.insert(name.clone(), id);
                inputs.push((name.clone(), id));
            }
        }
        let mut steps = Vec::new();
        for node in g.node {
            if !SUPPORTED.contains(&node.op_type.as_str()) {
                return Err(bad(format!("the model uses the operator {}, which LightCraft can't run", node.op_type)));
            }
            let ins = node
                .input
                .iter()
                .map(|n| {
                    if n.is_empty() {
                        Ok(None)
                    } else {
                        slot.get(n).copied().map(Some).ok_or_else(|| bad(format!("value {n} is used before it is made")))
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            let outs = node
                .output
                .iter()
                .map(|n| {
                    let id = consts.len();
                    consts.push(None);
                    slot.insert(n.clone(), id);
                    id
                })
                .collect();
            let mut step = Step { node, inputs: ins, outputs: outs, mats: Vec::new() };
            if step.node.op_type == "Constant" {
                let t = step.node.attr("value").and_then(|a| a.t.as_ref()).ok_or_else(|| bad("a Constant without a tensor"))?;
                let v = Arc::new(T::from_proto(t)?);
                if let Some(&o) = step.outputs.first()
                    && let Some(c) = consts.get_mut(o)
                {
                    *c = Some(v);
                }
                continue;
            }
            prepare(&mut step, &consts)?;
            steps.push(step);
        }
        let outputs = g
            .output
            .iter()
            .map(|n| slot.get(n).map(|&s| (n.clone(), s)).ok_or_else(|| bad(format!("output {n} is never made"))))
            .collect::<Result<Vec<_>>>()?;
        let mut last_use = vec![usize::MAX; consts.len()];
        for (i, s) in steps.iter().enumerate() {
            for id in s.inputs.iter().flatten() {
                if let Some(l) = last_use.get_mut(*id) {
                    *l = i;
                }
            }
        }
        for (_, id) in &outputs {
            if let Some(l) = last_use.get_mut(*id) {
                *l = usize::MAX;
            }
        }
        Ok(Model { steps, consts, inputs, outputs, last_use })
    }

    /// Run the model on its inputs (by position) and give every graph output by name.
    pub fn run(&self, inputs: Vec<T>) -> Result<HashMap<String, T>> {
        if inputs.len() != self.inputs.len() {
            return Err(bad(format!("the model takes {} inputs, {} given", self.inputs.len(), inputs.len())));
        }
        let mut v: Vec<Option<Arc<T>>> = self.consts.clone();
        for ((_, id), t) in self.inputs.iter().zip(inputs) {
            if let Some(s) = v.get_mut(*id) {
                *s = Some(Arc::new(t));
            }
        }
        let profile = std::env::var_os("LIGHTCRAFT_PROFILE").is_some();
        let mut times: HashMap<String, (std::time::Duration, usize)> = HashMap::new();
        for (i, step) in self.steps.iter().enumerate() {
            let args: Vec<Option<Arc<T>>> = step.inputs.iter().map(|id| id.and_then(|id| v.get(id).cloned().flatten())).collect();
            let t0 = std::time::Instant::now();
            let outs = eval(step, &args).map_err(|e| match e {
                Error::Model(m) => {
                    Error::Model(format!("{} ({}): {m}", step.node.op_type, step.node.output.first().map(String::as_str).unwrap_or("")))
                }
                e => e,
            })?;
            drop(args);
            if profile {
                let e = times.entry(step.node.op_type.clone()).or_default();
                e.0 += t0.elapsed();
                e.1 += 1;
            }
            for (id, t) in step.outputs.iter().zip(outs) {
                if let Some(s) = v.get_mut(*id) {
                    *s = Some(Arc::new(t));
                }
            }
            // drop what no later step reads
            for id in step.inputs.iter().flatten() {
                if self.last_use.get(*id) == Some(&i)
                    && self.consts.get(*id).is_some_and(Option::is_none)
                    && let Some(s) = v.get_mut(*id)
                {
                    *s = None;
                }
            }
        }
        if profile {
            let mut t: Vec<_> = times.into_iter().collect();
            t.sort_by_key(|e| std::cmp::Reverse(e.1.0));
            eprintln!(
                "vision graph: {}",
                t.iter().map(|(k, (d, n))| format!("{k}×{n} {:.1} ms", d.as_secs_f64() * 1e3)).collect::<Vec<_>>().join(", ")
            );
        }
        let mut out = HashMap::new();
        for (name, id) in &self.outputs {
            let t = v.get_mut(*id).and_then(Option::take).ok_or_else(|| bad(format!("output {name} was not computed")))?;
            out.insert(name.clone(), Arc::try_unwrap(t).unwrap_or_else(|a| (*a).clone()));
        }
        Ok(out)
    }

    /// Names of the graph's outputs, in order.
    pub fn output_names(&self) -> Vec<String> {
        self.outputs.iter().map(|(n, _)| n.clone()).collect()
    }
}

fn konst<'a>(consts: &'a [Option<Arc<T>>], id: Option<&Option<usize>>) -> Option<&'a T> {
    id.copied().flatten().and_then(|i| consts.get(i)).and_then(|c| c.as_deref())
}

/// Weight matrices for candle, made once at load.
fn prepare(step: &mut Step, consts: &[Option<Arc<T>>]) -> Result<()> {
    match step.node.op_type.as_str() {
        "Conv" => {
            let w = konst(consts, step.inputs.get(1)).ok_or_else(|| bad("a convolution whose weights aren't constant"))?;
            let (m, cg, kh, kw) = w.dims4()?;
            let g = usize::try_from(step.node.int("group", 1))
                .ok()
                .filter(|&g| g >= 1 && m.is_multiple_of(g))
                .ok_or_else(|| bad("bad convolution groups"))?;
            // depthwise convolutions are computed directly
            if cg == 1 && g == m {
                return Ok(());
            }
            let mg = m / g;
            let k = cg * kh * kw;
            for gi in 0..g {
                let part = w.data.get(gi * mg * k..(gi + 1) * mg * k).ok_or_else(|| bad("convolution weights too short"))?;
                step.mats.push(Tensor::from_slice(part, (mg, k), &Device::Cpu).map_err(mm_err)?);
            }
        }
        "Gemm" => {
            if let Some(b) = konst(consts, step.inputs.get(1)) {
                let [r, c] = b.shape.as_slice() else { return Err(bad("Gemm weights aren't a matrix")) };
                let trans_b = step.node.int("transB", 0) != 0;
                // kept as n × k: out = A · Bᵀ computed as (B · Aᵀ)ᵀ
                let t = Tensor::from_slice(&b.data, (*r, *c), &Device::Cpu).map_err(mm_err)?;
                step.mats.push(if trans_b { t } else { t.t().and_then(|t| t.contiguous()).map_err(mm_err)? });
            }
        }
        _ => {}
    }
    Ok(())
}

fn arg(args: &[Option<Arc<T>>], i: usize) -> Result<&T> {
    args.get(i).and_then(|a| a.as_deref()).ok_or_else(|| bad(format!("input {i} is missing")))
}

fn opt(args: &[Option<Arc<T>>], i: usize) -> Option<&T> {
    args.get(i).and_then(|a| a.as_deref())
}

fn map(x: &T, f: impl Fn(f32) -> f32 + Sync) -> T {
    T { shape: x.shape.clone(), data: x.data.par_iter().map(|&v| f(v)).collect() }
}

fn eval(step: &Step, args: &[Option<Arc<T>>]) -> Result<Vec<T>> {
    let n = &step.node;
    let one = |t: T| Ok(vec![t]);
    match n.op_type.as_str() {
        "Conv" => one(conv(step, args)?),
        "Relu" => one(map(arg(args, 0)?, |v| v.max(0.0))),
        "LeakyRelu" => {
            let a = n.float("alpha", 0.01);
            one(map(arg(args, 0)?, move |v| if v >= 0.0 { v } else { v * a }))
        }
        "Sigmoid" => one(map(arg(args, 0)?, |v| 1.0 / (1.0 + (-v).exp()))),
        "Tanh" => one(map(arg(args, 0)?, f32::tanh)),
        "Exp" => one(map(arg(args, 0)?, |v| v.min(80.0).exp())),
        "ReduceSum" | "ReduceMean" => {
            let x = arg(args, 0)?;
            let axes: Vec<i64> = match (n.ints("axes"), opt(args, 1)) {
                (Some(a), _) => a.to_vec(),
                (None, Some(t)) => t.data.iter().map(|&v| v as i64).collect(),
                (None, None) => (0..x.shape.len() as i64).collect(),
            };
            one(reduce(x, &axes, n.int("keepdims", 1) != 0, n.op_type == "ReduceMean")?)
        }
        "Clip" => {
            let lo = opt(args, 1).and_then(|t| t.data.first().copied()).unwrap_or_else(|| n.float("min", f32::MIN));
            let hi = opt(args, 2).and_then(|t| t.data.first().copied()).unwrap_or_else(|| n.float("max", f32::MAX));
            one(map(arg(args, 0)?, move |v| v.max(lo).min(hi)))
        }
        "PRelu" => one(broadcast(arg(args, 0)?, arg(args, 1)?, |x, s| if x >= 0.0 { x } else { x * s })?),
        "Add" => one(broadcast(arg(args, 0)?, arg(args, 1)?, |a, b| a + b)?),
        "Sub" => one(broadcast(arg(args, 0)?, arg(args, 1)?, |a, b| a - b)?),
        "Mul" => one(broadcast(arg(args, 0)?, arg(args, 1)?, |a, b| a * b)?),
        "Div" => one(broadcast(arg(args, 0)?, arg(args, 1)?, |a, b| a / b)?),
        "BatchNormalization" => one(batch_norm(n, args)?),
        "MaxPool" => one(pool(n, arg(args, 0)?, true)?),
        "AveragePool" => one(pool(n, arg(args, 0)?, false)?),
        "GlobalAveragePool" => {
            let x = arg(args, 0)?;
            let (b, c, h, w) = x.dims4()?;
            let hw = (h * w).max(1);
            let data = x.data.chunks(hw).map(|p| p.iter().sum::<f32>() / hw as f32).collect();
            one(T::new(vec![b, c, 1, 1], data)?)
        }
        "Resize" | "Upsample" => one(resize(n, args)?),
        "Transpose" => one(transpose(arg(args, 0)?, n.ints("perm"))?),
        "Reshape" => {
            let x = arg(args, 0)?;
            let s = arg(args, 1)?;
            one(reshape(x, &s.data)?)
        }
        "Flatten" => {
            let x = arg(args, 0)?;
            let axis = norm_axis(n.int("axis", 1), x.shape.len() + 1)?;
            let a: usize = x.shape.iter().take(axis).product();
            let b: usize = x.shape.iter().skip(axis).product();
            one(T::new(vec![a, b], x.data.clone())?)
        }
        "Squeeze" | "Unsqueeze" => {
            let x = arg(args, 0)?;
            let axes: Vec<i64> = match (n.ints("axes"), opt(args, 1)) {
                (Some(a), _) => a.to_vec(),
                (None, Some(t)) => t.data.iter().map(|&v| v as i64).collect(),
                (None, None) => Vec::new(),
            };
            let mut shape = x.shape.clone();
            if n.op_type == "Squeeze" {
                let rank = shape.len();
                let mut drop: Vec<usize> = axes.iter().map(|&a| norm_axis(a, rank)).collect::<Result<_>>()?;
                if drop.is_empty() {
                    drop = (0..rank).filter(|&i| shape.get(i) == Some(&1)).collect();
                }
                drop.sort_unstable();
                for &a in drop.iter().rev() {
                    if a < shape.len() {
                        shape.remove(a);
                    }
                }
            } else {
                let rank = shape.len() + axes.len();
                let mut add: Vec<usize> = axes.iter().map(|&a| norm_axis(a, rank)).collect::<Result<_>>()?;
                add.sort_unstable();
                for a in add {
                    if a > shape.len() {
                        return Err(bad("bad Unsqueeze axis"));
                    }
                    shape.insert(a, 1);
                }
            }
            one(T::new(shape, x.data.clone())?)
        }
        "Concat" => one(concat(n, args)?),
        "Gemm" => one(gemm(step, args)?),
        "MatMul" => one(mat_mul(arg(args, 0)?, arg(args, 1)?)?),
        "Softmax" => {
            let x = arg(args, 0)?;
            let axis = norm_axis(n.int("axis", -1), x.shape.len())?;
            let inner: usize = x.shape.iter().skip(axis + 1).product();
            let d = *x.shape.get(axis).ok_or_else(|| bad("bad Softmax axis"))?;
            let mut out = x.clone();
            let outer = x.data.len() / (d * inner).max(1);
            for o in 0..outer {
                for i in 0..inner {
                    let at = |k: usize| o * d * inner + k * inner + i;
                    let m = (0..d).filter_map(|k| x.data.get(at(k))).fold(f32::MIN, |a, &b| a.max(b));
                    let mut sum = 0.0;
                    for k in 0..d {
                        if let Some(v) = out.data.get_mut(at(k)) {
                            *v = (*v - m).exp();
                            sum += *v;
                        }
                    }
                    for k in 0..d {
                        if let Some(v) = out.data.get_mut(at(k)) {
                            *v /= sum.max(f32::MIN_POSITIVE);
                        }
                    }
                }
            }
            one(out)
        }
        "Dropout" | "Identity" => one(arg(args, 0)?.clone()),
        op => Err(bad(format!("the operator {op} isn't supported"))),
    }
}

/// Sum (or mean) over `axes`.
fn reduce(x: &T, axes: &[i64], keep: bool, mean: bool) -> Result<T> {
    let rank = x.shape.len();
    let axes: Vec<usize> = axes.iter().map(|&a| norm_axis(a, rank)).collect::<Result<_>>()?;
    let out_shape: Vec<usize> = x.shape.iter().enumerate().map(|(i, &d)| if axes.contains(&i) { 1 } else { d }).collect();
    let mut out = T::zeros(out_shape.clone())?;
    let mut st = vec![1usize; rank];
    for i in (0..rank.saturating_sub(1)).rev() {
        st[i] = st[i + 1] * out_shape[i + 1];
    }
    let mut idx = vec![0usize; rank];
    for &v in &x.data {
        let o: usize = (0..rank).map(|d| if axes.contains(&d) { 0 } else { idx[d] * st[d] }).sum();
        if let Some(t) = out.data.get_mut(o) {
            *t += v;
        }
        for d in (0..rank).rev() {
            idx[d] += 1;
            if idx[d] < x.shape[d] {
                break;
            }
            idx[d] = 0;
        }
    }
    if mean {
        let n: usize = axes.iter().map(|&a| x.shape[a]).product::<usize>().max(1);
        for v in &mut out.data {
            *v /= n as f32;
        }
    }
    if !keep {
        out.shape = out_shape.iter().enumerate().filter(|(i, _)| !axes.contains(i)).map(|(_, &d)| d).collect();
    }
    Ok(out)
}

fn norm_axis(a: i64, rank: usize) -> Result<usize> {
    let r = rank as i64;
    let a = if a < 0 { a + r } else { a };
    if (0..r.max(1)).contains(&a) { Ok(a as usize) } else { Err(bad("axis out of range")) }
}

/// numpy-style broadcasting of a binary operation.
fn broadcast(a: &T, b: &T, f: impl Fn(f32, f32) -> f32 + Sync) -> Result<T> {
    if a.shape == b.shape {
        return Ok(T { shape: a.shape.clone(), data: a.data.par_iter().zip(&b.data).map(|(&x, &y)| f(x, y)).collect() });
    }
    if b.data.len() == 1 {
        let y = b.data.first().copied().unwrap_or(0.0);
        if a.shape.len() >= b.shape.len() {
            return Ok(map(a, |x| f(x, y)));
        }
    }
    let rank = a.shape.len().max(b.shape.len());
    let pad = |s: &[usize]| -> Vec<usize> { std::iter::repeat_n(1, rank - s.len()).chain(s.iter().copied()).collect() };
    let (sa, sb) = (pad(&a.shape), pad(&b.shape));
    let mut shape = Vec::with_capacity(rank);
    for (&x, &y) in sa.iter().zip(&sb) {
        shape.push(match (x, y) {
            (x, y) if x == y => x,
            (1, y) => y,
            (x, 1) => x,
            _ => return Err(bad(format!("shapes {:?} and {:?} don't broadcast", a.shape, b.shape))),
        });
    }
    let strides = |s: &[usize]| -> Vec<usize> {
        let mut st = vec![0; rank];
        let mut acc = 1;
        for i in (0..rank).rev() {
            st[i] = if s[i] == 1 { 0 } else { acc };
            acc *= s[i];
        }
        st
    };
    let (ta, tb) = (strides(&sa), strides(&sb));
    let total = numel(&shape)?;
    let mut out = T::zeros(shape.clone())?;
    // the last axis is walked as a run
    let last = *shape.last().unwrap_or(&1);
    out.data.par_chunks_mut(last.max(1)).enumerate().for_each(|(row, dst)| {
        let mut rem = row;
        let (mut ia, mut ib) = (0usize, 0usize);
        for d in (0..rank.saturating_sub(1)).rev() {
            let k = rem % shape[d];
            rem /= shape[d];
            ia += k * ta[d];
            ib += k * tb[d];
        }
        let (la, lb) = (ta.last().copied().unwrap_or(0), tb.last().copied().unwrap_or(0));
        for (j, o) in dst.iter_mut().enumerate() {
            let x = a.data.get(ia + j * la).copied().unwrap_or(0.0);
            let y = b.data.get(ib + j * lb).copied().unwrap_or(0.0);
            *o = f(x, y);
        }
    });
    if out.data.len() != total {
        return Err(bad("broadcast size mismatch"));
    }
    Ok(out)
}

fn batch_norm(n: &Node, args: &[Option<Arc<T>>]) -> Result<T> {
    let x = arg(args, 0)?;
    let (scale, bias, mean, var) = (arg(args, 1)?, arg(args, 2)?, arg(args, 3)?, arg(args, 4)?);
    let eps = n.float("epsilon", 1e-5);
    let c = *x.shape.get(1).ok_or_else(|| bad("BatchNormalization of a 1-d tensor"))?;
    if [scale, bias, mean, var].iter().any(|t| t.data.len() != c) {
        return Err(bad("BatchNormalization parameters don't match the channels"));
    }
    let inner: usize = x.shape.iter().skip(2).product::<usize>().max(1);
    let k: Vec<(f32, f32)> = (0..c)
        .map(|i| {
            let s = scale.data[i] / (var.data[i] + eps).sqrt();
            (s, bias.data[i] - mean.data[i] * s)
        })
        .collect();
    let mut out = x.clone();
    out.data.par_chunks_mut(inner).enumerate().for_each(|(j, p)| {
        let (s, b) = k.get(j % c).copied().unwrap_or((1.0, 0.0));
        for v in p {
            *v = *v * s + b;
        }
    });
    Ok(out)
}

struct Geometry {
    kh: usize,
    kw: usize,
    sh: usize,
    sw: usize,
    pt: usize,
    pl: usize,
    dh: usize,
    dw: usize,
    oh: usize,
    ow: usize,
}

fn geometry(n: &Node, h: usize, w: usize, kernel: Option<(usize, usize)>, ceil: bool) -> Result<Geometry> {
    let pair = |name: &str, d: usize| -> Result<(usize, usize)> {
        match n.ints(name) {
            Some([a, b]) => {
                Ok((usize::try_from(*a).map_err(|_| bad("negative attribute"))?, usize::try_from(*b).map_err(|_| bad("negative attribute"))?))
            }
            Some(_) => Err(bad(format!("{name} isn't two numbers"))),
            None => Ok((d, d)),
        }
    };
    if n.string("auto_pad").is_some_and(|p| p != "NOTSET" && !p.is_empty()) {
        return Err(bad("auto_pad isn't supported"));
    }
    let (kh, kw) = match kernel {
        Some(k) => k,
        None => pair("kernel_shape", 0)?,
    };
    let (sh, sw) = pair("strides", 1)?;
    let (dh, dw) = pair("dilations", 1)?;
    let (pt, pl, pb, pr) = match n.ints("pads") {
        Some([a, b, c, d]) => {
            let u = |v: &i64| usize::try_from(*v).map_err(|_| bad("negative padding"));
            (u(a)?, u(b)?, u(c)?, u(d)?)
        }
        Some(_) => return Err(bad("pads isn't four numbers")),
        None => (0, 0, 0, 0),
    };
    if kh == 0 || kw == 0 || sh == 0 || sw == 0 || dh == 0 || dw == 0 || kh > 64 || kw > 64 || pt.max(pl).max(pb).max(pr) > 64 || sh.max(sw) > 64 {
        return Err(bad("unsupported kernel geometry"));
    }
    let span_h = dh * (kh - 1) + 1;
    let span_w = dw * (kw - 1) + 1;
    let (ih, iw) = (h + pt + pb, w + pl + pr);
    if ih < span_h || iw < span_w {
        return Err(bad("the picture is smaller than the kernel"));
    }
    let out = |i: usize, s: usize, st: usize| if ceil { (i - s).div_ceil(st) + 1 } else { (i - s) / st + 1 };
    Ok(Geometry { kh, kw, sh, sw, pt, pl, dh, dw, oh: out(ih, span_h, sh), ow: out(iw, span_w, sw) })
}

fn conv(step: &Step, args: &[Option<Arc<T>>]) -> Result<T> {
    let n = &step.node;
    let x = arg(args, 0)?;
    let w = arg(args, 1)?;
    let bias = opt(args, 2);
    let (b, c, h, wd) = x.dims4()?;
    let (m, cg, kh, kw) = w.dims4()?;
    let g = usize::try_from(n.int("group", 1)).ok().filter(|&g| g >= 1).ok_or_else(|| bad("bad groups"))?;
    if c != cg * g || !m.is_multiple_of(g) {
        return Err(bad(format!("convolution of {c} channels with weights {:?} in {g} groups", w.shape)));
    }
    if bias.is_some_and(|t| t.data.len() != m) {
        return Err(bad("convolution bias doesn't match"));
    }
    let geo = geometry(n, h, wd, Some((kh, kw)), false)?;
    let (oh, ow) = (geo.oh, geo.ow);
    let mut out = T::zeros(vec![b, m, oh, ow])?;
    let plane = oh * ow;
    for bi in 0..b {
        let img = x.data.get(bi * c * h * wd..(bi + 1) * c * h * wd).ok_or_else(|| bad("input too short"))?;
        let dst = out.data.get_mut(bi * m * plane..(bi + 1) * m * plane).ok_or_else(|| bad("output too short"))?;
        if cg == 1 && g == m {
            // depthwise: for each tap, a scaled run of an input row added to the output row
            dst.par_chunks_mut(plane).enumerate().for_each(|(ch, o)| {
                let wk = w.data.get(ch * kh * kw..(ch + 1) * kh * kw).unwrap_or(&[]);
                let b0 = bias.and_then(|t| t.data.get(ch)).copied().unwrap_or(0.0);
                let src = img.get(ch * h * wd..(ch + 1) * h * wd).unwrap_or(&[]);
                o.fill(b0);
                for oy in 0..oh {
                    let Some(orow) = o.get_mut(oy * ow..(oy + 1) * ow) else { continue };
                    for ky in 0..kh {
                        let Some(iy) = (oy * geo.sh + ky * geo.dh).checked_sub(geo.pt).filter(|&y| y < h) else { continue };
                        let Some(irow) = src.get(iy * wd..(iy + 1) * wd) else { continue };
                        for kx in 0..kw {
                            let wv = wk.get(ky * kw + kx).copied().unwrap_or(0.0);
                            let (lo, hi, off) = span(ow, wd, geo.sw, kx * geo.dw, geo.pl);
                            add_run(orow, irow, lo, hi, off, geo.sw, wv);
                        }
                    }
                }
            });
            continue;
        }
        let mg = m / g;
        let k = cg * kh * kw;
        for gi in 0..g {
            let mat = step.mats.get(gi).ok_or_else(|| bad("convolution weights weren't prepared"))?;
            let plain = kh == 1 && kw == 1 && geo.sh == 1 && geo.sw == 1 && geo.pt == 0 && geo.pl == 0 && oh == h && ow == wd;
            let cols: Vec<f32> = if plain {
                img.get(gi * cg * h * wd..(gi + 1) * cg * h * wd).ok_or_else(|| bad("input too short"))?.to_vec()
            } else {
                let n_cols = numel(&[k, plane])?;
                let mut cols = Vec::new();
                cols.try_reserve_exact(n_cols).map_err(|_| bad("not enough memory to run the model"))?;
                cols.resize(n_cols, 0.0);
                cols.par_chunks_mut(plane).enumerate().for_each(|(row, dst)| {
                    let ch = gi * cg + row / (kh * kw);
                    let (ky, kx) = ((row / kw) % kh, row % kw);
                    let src = img.get(ch * h * wd..(ch + 1) * h * wd).unwrap_or(&[]);
                    let (lo, hi, off) = span(ow, wd, geo.sw, kx * geo.dw, geo.pl);
                    for oy in 0..oh {
                        let Some(orow) = dst.get_mut(oy * ow..(oy + 1) * ow) else { continue };
                        let Some(iy) = (oy * geo.sh + ky * geo.dh).checked_sub(geo.pt).filter(|&y| y < h) else { continue };
                        let Some(irow) = src.get(iy * wd..(iy + 1) * wd) else { continue };
                        add_run(orow, irow, lo, hi, off, geo.sw, 1.0);
                    }
                });
                cols
            };
            let y = matmul(mat, cols, k, plane)?;
            let part = dst.get_mut(gi * mg * plane..(gi + 1) * mg * plane).ok_or_else(|| bad("output too short"))?;
            if y.len() != part.len() {
                return Err(bad("convolution result has the wrong size"));
            }
            part.par_chunks_mut(plane).zip(y.par_chunks(plane)).enumerate().for_each(|(j, (o, r))| {
                let b0 = bias.and_then(|t| t.data.get(gi * mg + j)).copied().unwrap_or(0.0);
                for (o, r) in o.iter_mut().zip(r) {
                    *o = r + b0;
                }
            });
        }
    }
    Ok(out)
}

/// The outputs `lo..hi` of a row whose input `ox · stride + tap − pad` lies inside a row of `w` pixels, and the
/// input index of `lo`.
fn span(ow: usize, w: usize, stride: usize, tap: usize, pad: usize) -> (usize, usize, usize) {
    // first ox with ox·stride + tap ≥ pad
    let lo = pad.saturating_sub(tap).div_ceil(stride.max(1));
    // last ox with ox·stride + tap − pad ≤ w − 1
    let hi = match (w + pad).checked_sub(tap + 1) {
        Some(v) => (v / stride.max(1) + 1).min(ow),
        None => 0,
    };
    let off = (lo * stride + tap).saturating_sub(pad);
    (lo, hi.max(lo), off)
}

/// `out[lo..hi] += k · input[off, off + stride, …]`.
fn add_run(out: &mut [f32], input: &[f32], lo: usize, hi: usize, off: usize, stride: usize, k: f32) {
    let Some(o) = out.get_mut(lo..hi) else { return };
    if stride == 1 {
        let Some(i) = input.get(off..off + o.len()) else { return };
        for (o, i) in o.iter_mut().zip(i) {
            *o += k * i;
        }
    } else {
        for (j, o) in o.iter_mut().enumerate() {
            *o += k * input.get(off + j * stride).copied().unwrap_or(0.0);
        }
    }
}

fn pool(n: &Node, x: &T, max: bool) -> Result<T> {
    let (b, c, h, w) = x.dims4()?;
    let geo = geometry(n, h, w, None, n.int("ceil_mode", 0) != 0)?;
    let count_pad = n.int("count_include_pad", 0) != 0;
    let (oh, ow) = (geo.oh, geo.ow);
    let mut out = T::zeros(vec![b, c, oh, ow])?;
    out.data.par_chunks_mut((oh * ow).max(1)).enumerate().for_each(|(p, o)| {
        let src = x.data.get(p * h * w..(p + 1) * h * w).unwrap_or(&[]);
        for oy in 0..oh {
            for ox in 0..ow {
                let (mut acc, mut cnt) = (if max { f32::MIN } else { 0.0 }, 0usize);
                for ky in 0..geo.kh {
                    let y = (oy * geo.sh + ky * geo.dh) as isize - geo.pt as isize;
                    for kx in 0..geo.kw {
                        let xx = (ox * geo.sw + kx * geo.dw) as isize - geo.pl as isize;
                        if y < 0 || xx < 0 || y as usize >= h || xx as usize >= w {
                            if count_pad {
                                cnt += 1;
                            }
                            continue;
                        }
                        let v = src.get(y as usize * w + xx as usize).copied().unwrap_or(0.0);
                        if max {
                            acc = acc.max(v);
                        } else {
                            acc += v;
                        }
                        cnt += 1;
                    }
                }
                if let Some(d) = o.get_mut(oy * ow + ox) {
                    *d = if max { acc } else { acc / cnt.max(1) as f32 };
                }
            }
        }
    });
    Ok(out)
}

fn resize(n: &Node, args: &[Option<Arc<T>>]) -> Result<T> {
    let x = arg(args, 0)?;
    let (b, c, h, w) = x.dims4()?;
    if h == 0 || w == 0 {
        return Err(bad("Resize of an empty picture"));
    }
    let mode = n.string("mode").unwrap_or_else(|| "nearest".into());
    if mode != "nearest" {
        return Err(bad(format!("Resize mode {mode} isn't supported")));
    }
    let coords = n.string("coordinate_transformation_mode").unwrap_or_else(|| "half_pixel".into());
    // Upsample: scales are input 1; Resize: roi, scales, sizes
    let (scales, sizes) = if n.op_type == "Upsample" { (opt(args, 1), None) } else { (opt(args, 2).filter(|t| !t.data.is_empty()), opt(args, 3)) };
    let (oh, ow) = match (sizes, scales) {
        (Some(s), _) if s.data.len() == 4 => (s.data[2] as usize, s.data[3] as usize),
        (_, Some(s)) if s.data.len() == 4 => ((h as f32 * s.data[2]).floor() as usize, (w as f32 * s.data[3]).floor() as usize),
        _ => return Err(bad("Resize without scales or sizes")),
    };
    if oh == 0 || ow == 0 || oh > h * 64 || ow > w * 64 {
        return Err(bad("bad Resize size"));
    }
    let src = |o: usize, inn: usize, out: usize| -> usize {
        let s = inn as f64 / out as f64;
        let v = match coords.as_str() {
            "asymmetric" => o as f64 * s,
            "align_corners" if out > 1 => o as f64 * (inn - 1) as f64 / (out - 1) as f64,
            _ => (o as f64 + 0.5) * s - 0.5,
        };
        let v = match n.string("nearest_mode").as_deref() {
            Some("floor") => v.floor(),
            Some("ceil") => v.ceil(),
            Some("round_prefer_ceil") => (v + 0.5).floor(),
            _ => (v - 0.5).ceil(), // round_prefer_floor (the default)
        };
        (v.max(0.0) as usize).min(inn - 1)
    };
    let ys: Vec<usize> = (0..oh).map(|y| src(y, h, oh)).collect();
    let xs: Vec<usize> = (0..ow).map(|x| src(x, w, ow)).collect();
    let mut out = T::zeros(vec![b, c, oh, ow])?;
    out.data.par_chunks_mut(oh * ow).enumerate().for_each(|(p, o)| {
        let s = x.data.get(p * h * w..(p + 1) * h * w).unwrap_or(&[]);
        for (oy, &sy) in ys.iter().enumerate() {
            for (ox, &sx) in xs.iter().enumerate() {
                if let Some(d) = o.get_mut(oy * ow + ox) {
                    *d = s.get(sy * w + sx).copied().unwrap_or(0.0);
                }
            }
        }
    });
    Ok(out)
}

fn transpose(x: &T, perm: Option<&[i64]>) -> Result<T> {
    let rank = x.shape.len();
    let perm: Vec<usize> = match perm {
        Some(p) => p.iter().map(|&a| norm_axis(a, rank)).collect::<Result<_>>()?,
        None => (0..rank).rev().collect(),
    };
    let mut seen = vec![false; rank];
    for &p in &perm {
        match seen.get_mut(p) {
            Some(s) if !*s => *s = true,
            _ => return Err(bad("bad permutation")),
        }
    }
    if perm.len() != rank {
        return Err(bad("bad permutation"));
    }
    let shape: Vec<usize> = perm.iter().map(|&p| x.shape[p]).collect();
    let mut st = vec![1usize; rank];
    for i in (0..rank.saturating_sub(1)).rev() {
        st[i] = st[i + 1] * x.shape[i + 1];
    }
    let src_st: Vec<usize> = perm.iter().map(|&p| st[p]).collect();
    let mut out = T::zeros(shape.clone())?;
    let last = *shape.last().unwrap_or(&1);
    let ls = src_st.last().copied().unwrap_or(1);
    out.data.par_chunks_mut(last.max(1)).enumerate().for_each(|(row, dst)| {
        let mut rem = row;
        let mut base = 0;
        for d in (0..rank.saturating_sub(1)).rev() {
            base += (rem % shape[d]) * src_st[d];
            rem /= shape[d];
        }
        for (j, o) in dst.iter_mut().enumerate() {
            *o = x.data.get(base + j * ls).copied().unwrap_or(0.0);
        }
    });
    Ok(out)
}

fn reshape(x: &T, spec: &[f32]) -> Result<T> {
    let mut shape = Vec::with_capacity(spec.len());
    let mut infer = None;
    for (i, &d) in spec.iter().enumerate() {
        let d = d as i64;
        match d {
            -1 if infer.is_none() => {
                infer = Some(i);
                shape.push(1);
            }
            0 => shape.push(*x.shape.get(i).ok_or_else(|| bad("bad Reshape"))?),
            d if d > 0 => shape.push(d as usize),
            _ => return Err(bad("bad Reshape")),
        }
    }
    if let Some(i) = infer {
        let known: usize = shape.iter().product::<usize>().max(1);
        if !x.data.len().is_multiple_of(known) {
            return Err(bad("bad Reshape"));
        }
        shape[i] = x.data.len() / known;
    }
    T::new(shape, x.data.clone())
}

fn concat(n: &Node, args: &[Option<Arc<T>>]) -> Result<T> {
    let parts: Vec<&T> = args.iter().filter_map(|a| a.as_deref()).collect();
    let first = parts.first().ok_or_else(|| bad("Concat of nothing"))?;
    let rank = first.shape.len();
    let axis = norm_axis(n.int("axis", 0), rank)?;
    let outer: usize = first.shape.iter().take(axis).product();
    let mut shape = first.shape.clone();
    shape[axis] = 0;
    for p in &parts {
        if p.shape.len() != rank || p.shape.iter().enumerate().any(|(i, &d)| i != axis && d != first.shape[i]) {
            return Err(bad("Concat of tensors with different shapes"));
        }
        shape[axis] += p.shape[axis];
    }
    let mut data = Vec::with_capacity(numel(&shape)?);
    for o in 0..outer {
        for p in &parts {
            let run = p.data.len() / outer.max(1);
            data.extend_from_slice(p.data.get(o * run..(o + 1) * run).ok_or_else(|| bad("Concat input too short"))?);
        }
    }
    T::new(shape, data)
}

fn gemm(step: &Step, args: &[Option<Arc<T>>]) -> Result<T> {
    let n = &step.node;
    let a = arg(args, 0)?;
    let [ar, ac] = a.shape.as_slice() else { return Err(bad("Gemm input isn't a matrix")) };
    let trans_a = n.int("transA", 0) != 0;
    let (m, k) = if trans_a { (*ac, *ar) } else { (*ar, *ac) };
    let at: Vec<f32> = if trans_a { a.data.clone() } else { transpose(a, None)?.data }; // k × m
    // B as n × k
    let bmat = match step.mats.first() {
        Some(t) => t.clone(),
        None => {
            let b = arg(args, 1)?;
            let [r, c] = b.shape.as_slice() else { return Err(bad("Gemm weights aren't a matrix")) };
            let t = Tensor::from_slice(&b.data, (*r, *c), &Device::Cpu).map_err(mm_err)?;
            if n.int("transB", 0) != 0 { t } else { t.t().and_then(|t| t.contiguous()).map_err(mm_err)? }
        }
    };
    let (nn, kb) = bmat.dims2().map_err(mm_err)?;
    if kb != k {
        return Err(bad(format!("Gemm of {m}×{k} by {kb}-wide weights")));
    }
    let y = matmul(&bmat, at, k, m)?; // n × m
    let alpha = n.float("alpha", 1.0);
    let beta = n.float("beta", 1.0);
    let mut out = transpose(&T::new(vec![nn, m], y)?, None)?; // m × n
    for v in &mut out.data {
        *v *= alpha;
    }
    if let Some(c) = opt(args, 2) {
        let scaled = T { shape: c.shape.clone(), data: c.data.iter().map(|v| v * beta).collect() };
        out = broadcast(&out, &scaled, |x, y| x + y)?;
    }
    Ok(out)
}

fn mat_mul(a: &T, b: &T) -> Result<T> {
    let (Some(&k), Some(&n)) = (a.shape.last(), b.shape.last()) else { return Err(bad("MatMul of a scalar")) };
    let [kb, _] = b.shape.as_slice() else { return Err(bad("MatMul by a batched matrix isn't supported")) };
    if *kb != k {
        return Err(bad("MatMul shapes don't match"));
    }
    let m = a.data.len() / k.max(1);
    let at = Tensor::from_slice(&a.data, (m, k), &Device::Cpu).map_err(mm_err)?;
    let y = matmul(&at, b.data.clone(), k, n)?;
    let mut shape = a.shape.clone();
    if let Some(l) = shape.last_mut() {
        *l = n;
    }
    T::new(shape, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(op: &str, attrs: Vec<proto::Attribute>) -> Node {
        Node { op_type: op.into(), attribute: attrs, ..Default::default() }
    }

    fn ints(name: &str, v: &[i64]) -> proto::Attribute {
        proto::Attribute { name: name.into(), ints: v.to_vec(), ..Default::default() }
    }

    /// A direct (slow, obviously right) convolution.
    fn reference_conv(x: &T, w: &T, stride: usize, pad: usize, groups: usize) -> T {
        let (b, c, h, wd) = x.dims4().unwrap();
        let (m, cg, kh, kw) = w.dims4().unwrap();
        let oh = (h + 2 * pad - kh) / stride + 1;
        let ow = (wd + 2 * pad - kw) / stride + 1;
        let mut out = T::zeros(vec![b, m, oh, ow]).unwrap();
        let mg = m / groups;
        for bi in 0..b {
            for o in 0..m {
                let g = o / mg;
                for oy in 0..oh {
                    for ox in 0..ow {
                        let mut acc = 0.0;
                        for ci in 0..cg {
                            let ch = g * cg + ci;
                            for ky in 0..kh {
                                for kx in 0..kw {
                                    let y = (oy * stride + ky) as isize - pad as isize;
                                    let xx = (ox * stride + kx) as isize - pad as isize;
                                    if y >= 0 && xx >= 0 && (y as usize) < h && (xx as usize) < wd {
                                        acc +=
                                            w.data[((o * cg + ci) * kh + ky) * kw + kx] * x.data[((bi * c + ch) * h + y as usize) * wd + xx as usize];
                                    }
                                }
                            }
                        }
                        out.data[((bi * m + o) * oh + oy) * ow + ox] = acc;
                    }
                }
            }
        }
        out
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1 << 24) as f32 - 0.5
            })
            .collect()
    }

    #[test]
    fn convolutions_match_a_direct_reference() {
        // (channels, width, out, kernel, stride, pad, groups): includes channels == width (candle 0.9.2's CPU bug)
        for (c, wdt, m, k, s, p, g) in
            [(3, 9, 4, 3, 2, 1, 1), (8, 8, 8, 3, 1, 1, 8), (6, 7, 4, 1, 1, 0, 2), (4, 4, 6, 3, 1, 1, 1), (5, 11, 5, 3, 2, 1, 5)]
        {
            let x = T::new(vec![1, c, 7, wdt], noise(c * 7 * wdt, 1)).unwrap();
            let w = T::new(vec![m, c / g, k, k], noise(m * (c / g) * k * k, 2)).unwrap();
            let n = node(
                "Conv",
                vec![
                    ints("strides", &[s as i64, s as i64]),
                    ints("pads", &[p as i64; 4]),
                    proto::Attribute { name: "group".into(), i: Some(g as i64), ..Default::default() },
                ],
            );
            let mut step = Step { node: n, inputs: vec![Some(0), Some(1)], outputs: vec![2], mats: Vec::new() };
            let consts = vec![None, Some(Arc::new(w.clone()))];
            prepare(&mut step, &consts).unwrap();
            let got = conv(&step, &[Some(Arc::new(x.clone())), Some(Arc::new(w.clone()))]).unwrap();
            let want = reference_conv(&x, &w, s, p, g);
            assert_eq!(got.shape, want.shape);
            for (a, b) in got.data.iter().zip(&want.data) {
                assert!((a - b).abs() < 1e-4, "{c} {wdt} {m} {k} {s} {p} {g}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn broadcasting_and_transposes() {
        let a = T::new(vec![2, 3, 1], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let b = T::new(vec![3, 2], vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0]).unwrap();
        let s = broadcast(&a, &b, |x, y| x + y).unwrap();
        assert_eq!(s.shape, vec![2, 3, 2]);
        assert_eq!(&s.data[..4], &[11.0, 21.0, 32.0, 42.0]);
        let t = transpose(&T::new(vec![2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap(), Some(&[1, 0])).unwrap();
        assert_eq!(t.data, vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
        assert!(broadcast(&T::new(vec![2], vec![0.0; 2]).unwrap(), &T::new(vec![3], vec![0.0; 3]).unwrap(), |x, _| x).is_err());
        assert!(transpose(&a, Some(&[0, 0, 1])).is_err());
        assert!(reshape(&a, &[4.0, -1.0]).is_err());
        assert_eq!(reshape(&a, &[0.0, -1.0]).unwrap().shape, vec![2, 3]);
    }

    #[test]
    fn a_bad_model_is_an_error() {
        assert!(Model::from_bytes(b"not a model").is_err());
        assert!(Model::from_bytes(&[]).is_err());
    }
}
