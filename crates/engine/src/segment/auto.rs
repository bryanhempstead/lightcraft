//! Lightroom's automatic AI masks on the worker thread: Select Subject / Sky, Landscape classes,
//! detected People and their parts. They use whichever models are installed: SAM 3 (subject,
//! people as separate instances) and the SegFormer parsers (`lightcraft_segment::parsers`: scene
//! classes, body and face parts), each optional.

use lightcraft_develop::SegMask;
use lightcraft_segment::parsers;
use lightcraft_segment::segformer::{Parsed, SegFormer, square_logits};

use super::Input;

/// Side of the grids the parsers' masks are stored at.
const SIDE: usize = 256;
/// People smaller than this share of the photo aren't listed (a crowd far away).
const MIN_PERSON: f64 = 0.002;

/// What to compute.
#[derive(Clone, Debug, PartialEq)]
pub enum Auto {
    Subject,
    Sky,
    /// A Landscape class (`lightcraft_develop::ai::LANDSCAPE_CLASSES` id).
    Landscape(String),
    /// The share of the photo each Landscape class covers.
    Coverage,
    /// The photo's people.
    People,
    /// The parts of the person at `rect` (normalized); the job's crop render covers `region`.
    Parts {
        rect: [f64; 4],
        region: [f64; 4],
        parts: Vec<String>,
    },
}

/// A detected person.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Person {
    /// Their box, normalized `[x0, y0, x1, y1]`.
    pub rect: [f64; 4],
    /// Their outline over the whole photo.
    #[serde(skip)]
    pub seg: SegMask,
    pub score: f32,
}

/// A result.
#[derive(Clone, Debug)]
pub enum AutoOut {
    Seg(SegMask),
    Coverage(Vec<(String, f32)>),
    People(Vec<Person>),
    Parts { seg: Option<SegMask>, detail: Vec<SegMask> },
}

/// The models one job may use (each `None` when not installed).
pub struct Models<'a> {
    pub sam: Option<(&'a mut lightcraft_segment::Sam3, &'a mut lightcraft_segment::Encoded)>,
    pub scene: Option<&'a Parsed>,
}

fn indices_of(model_labels: &[String], names: &[&str]) -> Vec<usize> {
    parsers::indices(model_labels, names)
}

/// The scene parser's probability of `names` (cells over the whole photo, `vw × vh`).
fn scene_prob(scene: &Parsed, labels: &[String], names: &[&str]) -> Vec<f32> {
    scene.prob(&indices_of(labels, names))
}

/// Logits `side²` → stored segmentation, `None` when nothing is selected.
fn seg_if_any(side: usize, logits: &[f32]) -> Option<SegMask> {
    logits.iter().any(|l| *l > 0.0).then(|| SegMask::from_logits(side, logits))
}

/// Select Sky / Landscape classes (`class`: one id, or several joined by commas: their union)
/// from the scene parse; `None` when the photo has none of it.
pub fn landscape(scene: &Parsed, labels: &[String], class: &str) -> Result<Option<SegMask>, String> {
    let mut names: Vec<&str> = Vec::new();
    for c in class.split(',').map(str::trim).filter(|c| !c.is_empty()) {
        let n = parsers::LANDSCAPE.iter().find(|l| l.0 == c).map(|l| l.2).ok_or_else(|| format!("unknown Landscape class `{c}`"))?;
        names.extend_from_slice(n);
    }
    if names.is_empty() {
        return Err("no Landscape class given".into());
    }
    let p = scene_prob(scene, labels, &names);
    Ok(seg_if_any(SIDE, &square_logits(&p, scene.vw, scene.vh, SIDE)))
}

/// The share of the photo each Landscape class covers (by most likely class per cell).
pub fn coverage(scene: &Parsed, labels: &[String]) -> Vec<(String, f32)> {
    let am = scene.argmax();
    let n = am.len().max(1) as f32;
    parsers::LANDSCAPE
        .iter()
        .map(|(id, _, names)| {
            let idx = indices_of(labels, names);
            (id.to_string(), am.iter().filter(|c| idx.contains(c)).count() as f32 / n)
        })
        .collect()
}

/// The subject: SAM 3's people and animals (the most prominent), else what SAM 3 selects at the
/// photo's centre; without SAM 3 the scene parser's people and animals.
pub fn subject(m: &mut Models) -> Result<Option<SegMask>, String> {
    if let Some((sam, enc)) = m.sam.as_mut() {
        if let Some(p) = sam.segment_subject(enc, None, 0.5).map_err(|e| e.to_string())? {
            return Ok(Some(SegMask::from_logits(lightcraft_segment::MASK_SIDE, &p.logits)));
        }
        let p = sam.segment_clicks(enc, &[lightcraft_segment::Click { x: 0.5, y: 0.5, positive: true }]).map_err(|e| e.to_string())?;
        return Ok(seg_if_any(lightcraft_segment::MASK_SIDE, &p.logits));
    }
    Ok(None)
}

/// The subject from the scene parse (people and animals covering at least 1 % of the photo).
pub fn scene_subject(scene: &Parsed, labels: &[String]) -> Option<SegMask> {
    let p = scene_prob(scene, labels, parsers::SCENE_SUBJECT);
    let big = parsers::components(&p, scene.vw, scene.vh, 0.5).first().is_some_and(|c| c.len() as f32 >= 0.01 * p.len() as f32);
    big.then(|| SegMask::from_logits(SIDE, &square_logits(&p, scene.vw, scene.vh, SIDE)))
}

/// The bounding box of the cells above 0 of a `side²` grid, normalized; `None` when empty.
fn logit_box(l: &[f32], side: usize) -> Option<([f64; 4], usize)> {
    let cells: Vec<usize> = (0..l.len()).filter(|i| l[*i] > 0.0).collect();
    let b = parsers::bbox(&cells, side)?;
    let s = side as f64;
    Some(([b[0] as f64 / s, b[1] as f64 / s, b[2] as f64 / s, b[3] as f64 / s], cells.len()))
}

/// The photo's people, left to right: SAM 3's person instances, else the scene parser's people
/// (separate where they don't touch).
pub fn people(m: &mut Models, labels: Option<&[String]>, faces: &[(f64, f64)]) -> Result<Vec<Person>, String> {
    let mut out = Vec::new();
    if let Some((sam, enc)) = m.sam.as_mut() {
        let side = lightcraft_segment::MASK_SIDE;
        for p in sam.segment_instances(enc, "person", 0.5).map_err(|e| e.to_string())? {
            if let Some((rect, n)) = logit_box(&p.logits, side)
                && n as f64 >= MIN_PERSON * (side * side) as f64
            {
                out.push(Person { rect, seg: SegMask::from_logits(side, &p.logits), score: p.score });
            }
        }
    } else if let (Some(scene), Some(labels)) = (m.scene, labels) {
        let p = scene_prob(scene, labels, parsers::SCENE_PEOPLE);
        let (w, h) = (scene.vw, scene.vh);
        // people standing together are one region: split it by their faces
        let mut comps: Vec<Vec<usize>> = Vec::new();
        for mut comp in parsers::components(&p, w, h, 0.5) {
            comp.sort_unstable();
            let inside: Vec<(f64, f64)> = faces
                .iter()
                .copied()
                .filter(|(fx, fy)| {
                    let (cx, cy) = (((fx * w as f64) as usize).min(w - 1), ((fy * h as f64) as usize).min(h - 1));
                    comp.binary_search(&(cy * w + cx)).is_ok() || comp.iter().any(|&i| (i % w).abs_diff(cx) <= 1 && (i / w).abs_diff(cy) <= 1)
                })
                .collect();
            if inside.len() < 2 {
                comps.push(comp);
                continue;
            }
            let mut parts: Vec<Vec<usize>> = vec![Vec::new(); inside.len()];
            for &i in &comp {
                let (x, y) = (((i % w) as f64 + 0.5) / w as f64, ((i / w) as f64 + 0.5) / h as f64);
                // nearest face, bodies hanging below their face (vertical distance counts less
                // below a face: a body hangs below its head)
                let k = inside
                    .iter()
                    .enumerate()
                    .map(|(k, (fx, fy))| {
                        let dy = y - fy;
                        let wy = if dy > 0.0 { 0.25 } else { 1.0 };
                        (k, (x - fx).powi(2) + (wy * dy).powi(2))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map_or(0, |(k, _)| k);
                parts[k].push(i);
            }
            comps.extend(parts.into_iter().filter(|c| !c.is_empty()));
        }
        for mut comp in comps {
            comp.sort_unstable();
            if (comp.len() as f64) < MIN_PERSON * (w * h) as f64 {
                continue;
            }
            // this person's cells and a fringe of 2 cells around them
            let mut keep = vec![false; w * h];
            for &i in &comp {
                let (x, y) = (i % w, i / w);
                for yy in y.saturating_sub(2)..(y + 3).min(h) {
                    for xx in x.saturating_sub(2)..(x + 3).min(w) {
                        if let Some(k) = keep.get_mut(yy * w + xx) {
                            *k = true;
                        }
                    }
                }
            }
            let mine: Vec<f32> = p.iter().zip(&keep).map(|(v, k)| if *k { *v } else { 0.0 }).collect();
            let Some(b) = parsers::bbox(&comp, w) else { continue };
            let rect = [b[0] as f64 / w as f64, b[1] as f64 / h as f64, b[2] as f64 / w as f64, b[3] as f64 / h as f64];
            let score = comp.iter().filter_map(|i| p.get(*i)).sum::<f32>() / comp.len().max(1) as f32;
            out.push(Person { rect, seg: SegMask::from_logits(SIDE, &square_logits(&mine, w, h, SIDE)), score });
        }
    }
    out.sort_by(|a, b| (a.rect[0] + a.rect[2]).total_cmp(&(b.rect[0] + b.rect[2])));
    Ok(out)
}

/// Intersection over union of two normalized boxes.
pub fn iou(a: [f64; 4], b: [f64; 4]) -> f64 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = ix * iy;
    let area = |r: [f64; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let u = area(a) + area(b) - inter;
    if u > 0.0 { inter / u } else { 0.0 }
}

/// The region of the photo to render for a person's parts: their box with a margin, inside the
/// photo.
pub fn person_region(rect: [f64; 4]) -> [f64; 4] {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let m = 0.08 * w.max(h);
    [(rect[0] - m).max(0.0), (rect[1] - m).max(0.0), (rect[2] + m).min(1.0), (rect[3] + m).min(1.0)]
}

/// The probability `seg` gives at normalized point `(x, y)` (bilinear on its logits).
fn seg_prob(grid: &[f32], side: usize, r: [f64; 4], x: f64, y: f64) -> f32 {
    if side == 0 || r[2] <= r[0] || r[3] <= r[1] {
        return 0.0;
    }
    let (u, v) = ((x - r[0]) / (r[2] - r[0]), (y - r[1]) / (r[3] - r[1]));
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return 0.0;
    }
    let at = |cx: usize, cy: usize| grid.get(cy.min(side - 1) * side + cx.min(side - 1)).copied().unwrap_or(-16.0);
    let (fx, fy) = ((u * side as f64 - 0.5).clamp(0.0, (side - 1) as f64), (v * side as f64 - 0.5).clamp(0.0, (side - 1) as f64));
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
    let l = (at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx) * (1.0 - ty) + (at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx) * ty;
    1.0 / (1.0 + (-l).exp())
}

/// Bilinear resample of a `w × h` plane to `ow × oh` (cell centres aligned).
fn resample(p: &[f32], w: usize, h: usize, ow: usize, oh: usize) -> Vec<f32> {
    let at = |x: usize, y: usize| p.get(y.min(h.saturating_sub(1)) * w + x.min(w.saturating_sub(1))).copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(ow * oh);
    for oy in 0..oh {
        let fy = ((oy as f32 + 0.5) * h as f32 / oh as f32 - 0.5).clamp(0.0, h.saturating_sub(1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy - fy.floor());
        for ox in 0..ow {
            let fx = ((ox as f32 + 0.5) * w as f32 / ow as f32 - 0.5).clamp(0.0, w.saturating_sub(1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx - fx.floor());
            out.push((at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx) * (1.0 - ty) + (at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx) * ty);
        }
    }
    out
}

/// A part of an 8-bit RGB image (pixel box `[x0, y0, x1, y1]`).
fn crop_rgb(img: &Input, b: [usize; 4]) -> Option<(Vec<u8>, usize, usize)> {
    let (x0, y0, x1, y1) = (b[0].min(img.w), b[1].min(img.h), b[2].min(img.w), b[3].min(img.h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
    for y in y0..y1 {
        out.extend_from_slice(img.rgb.get((y * img.w + x0) * 3..(y * img.w + x1) * 3)?);
    }
    Some((out, x1 - x0, y1 - y0))
}

/// A person's parts. `crop` is the photo rendered over `region` (normalized); `person` their
/// outline over the whole photo (a gate: the parsers label everyone in the crop). Returns the
/// coarse outline (when Entire Person is among the parts) and the patches over the person and
/// their face.
pub fn parts(
    body: &SegFormer,
    face: Option<&SegFormer>,
    crop: &Input,
    region: [f64; 4],
    person: &SegMask,
    parts: &[String],
) -> Result<(Option<SegMask>, Vec<SegMask>), String> {
    let pg = person.logits().ok_or("the person's outline is damaged")?;
    let (pside, pr) = (person.side as usize, person.bounds().unwrap_or([0.0, 0.0, 1.0, 1.0]));
    let pb = body.parse(&crop.rgb, crop.w, crop.h, parsers::BODY.input).map_err(|e| e.to_string())?;
    let (bw, bh) = (pb.vw, pb.vh);
    let (rw, rh) = (region[2] - region[0], region[3] - region[1]);
    // the person's own cells (lenient: their outline is coarser than the parse)
    let gate: Vec<f32> = (0..bw * bh)
        .map(|i| {
            let (x, y) = (region[0] + ((i % bw) as f64 + 0.5) / bw as f64 * rw, region[1] + ((i / bw) as f64 + 0.5) / bh as f64 * rh);
            let p = seg_prob(&pg, pside, pr, x, y);
            (p * 3.0).min(1.0)
        })
        .collect();
    let blabels = body.labels();
    let mut bunion = vec![0f32; bw * bh];
    for part in parts {
        let mut p = pb.prob(&parsers::indices(blabels, parsers::body_classes(part)));
        if part == "person" {
            // SAM 3's outline is the better one where it is sure
            for (i, v) in p.iter_mut().enumerate() {
                let (x, y) = (region[0] + ((i % bw) as f64 + 0.5) / bw as f64 * rw, region[1] + ((i / bw) as f64 + 0.5) / bh as f64 * rh);
                *v = v.max(seg_prob(&pg, pside, pr, x, y));
            }
        }
        for ((u, v), g) in bunion.iter_mut().zip(&p).zip(&gate) {
            *u = u.max((v * g).min(1.0));
        }
    }
    let mut detail = vec![SegMask::from_logits_in(SIDE, &square_logits(&bunion, bw, bh, SIDE), region)];
    // the face: the body parse's largest face (and hair above it), parsed again, zoomed in
    let face_wanted = parts.iter().any(|p| !parsers::face_classes(p).is_empty());
    if let (Some(fm), true) = (face, face_wanted) {
        let fp: Vec<f32> = pb.prob(&parsers::indices(blabels, &["Face"])).iter().zip(&gate).map(|(a, g)| a * g).collect();
        if let Some(b) = parsers::components(&fp, bw, bh, 0.5).first().and_then(|c| parsers::bbox(c, bw)) {
            // in crop pixels; a square 1.9× the face, a little above its centre (hair, forehead)
            let (sx, sy) = (crop.w as f64 / bw as f64, crop.h as f64 / bh as f64);
            let (fx0, fy0, fx1, fy1) = (b[0] as f64 * sx, b[1] as f64 * sy, b[2] as f64 * sx, b[3] as f64 * sy);
            let side = 1.9 * (fx1 - fx0).max(fy1 - fy0);
            let (cx, cy) = ((fx0 + fx1) / 2.0, (fy0 + fy1) / 2.0 - 0.08 * side);
            let bx =
                [(cx - side / 2.0).max(0.0), (cy - side / 2.0).max(0.0), (cx + side / 2.0).min(crop.w as f64), (cy + side / 2.0).min(crop.h as f64)];
            let pix = [bx[0] as usize, bx[1] as usize, bx[2].ceil() as usize, bx[3].ceil() as usize];
            if let Some((rgb, w, h)) = crop_rgb(crop, pix).filter(|(_, w, h)| *w >= 16 && *h >= 16) {
                let pf = fm.parse(&rgb, w, h, parsers::FACE.input).map_err(|e| e.to_string())?;
                let flabels = fm.labels();
                // at the stored grid: the parse upsampled, the pixels downsampled
                let up = |p: Vec<f32>| resample(&p, pf.vw, pf.vh, SIDE, SIDE);
                let small = lightcraft_segment::mask::resize_rgb(&rgb, w, h, SIDE, SIDE).map_err(|e| e.to_string())?;
                let mut funion = vec![0f32; SIDE * SIDE];
                for part in parts {
                    let p = up(pf.prob(&parsers::indices(flabels, parsers::face_classes(part))));
                    let p = match part.as_str() {
                        "eyeSclera" => parsers::split_eyes(&p, &small, SIDE, SIDE).0,
                        "iris" => parsers::split_eyes(&p, &small, SIDE, SIDE).1,
                        "teeth" => parsers::teeth(&p, &small, SIDE, SIDE),
                        _ => p,
                    };
                    for (u, v) in funion.iter_mut().zip(&p) {
                        *u = u.max(*v);
                    }
                }
                let logits: Vec<f32> = funion.iter().map(|p| lightcraft_segment::segformer::logit(*p)).collect();
                let to_n = |px: f64, py: f64| (region[0] + px / crop.w as f64 * rw, region[1] + py / crop.h as f64 * rh);
                let (a, z) = (to_n(pix[0] as f64, pix[1] as f64), to_n(pix[2] as f64, pix[3] as f64));
                detail.push(SegMask::from_logits_in(SIDE, &logits, [a.0, a.1, z.0, z.1]));
            }
        }
    }
    let seg = parts.iter().any(|p| p == "person").then(|| person.clone());
    Ok((seg, detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_regions_and_overlap() {
        assert!((iou([0.0, 0.0, 0.5, 0.5], [0.0, 0.0, 0.5, 0.5]) - 1.0).abs() < 1e-9);
        assert_eq!(iou([0.0, 0.0, 0.1, 0.1], [0.5, 0.5, 0.6, 0.6]), 0.0);
        assert_eq!(iou([0.0; 4], [0.0; 4]), 0.0);
        let r = person_region([0.4, 0.2, 0.6, 0.9]);
        assert!(r[0] < 0.4 && r[1] < 0.2 && r[2] > 0.6 && r[3] <= 1.0);
        let l: Vec<f32> = (0..16).map(|i| if i == 5 || i == 6 { 4.0 } else { -4.0 }).collect();
        let (b, n) = logit_box(&l, 4).unwrap();
        assert_eq!(n, 2);
        assert_eq!(b, [0.25, 0.25, 0.75, 0.5]);
        assert!(logit_box(&[-1.0; 16], 4).is_none());
    }

    #[test]
    fn scene_parses_become_landscape_masks_and_people() {
        // 4 × 2 cells, 3 classes: sky on top, a person bottom-left, grass bottom-right
        let labels: Vec<String> = ["sky", "person", "grass"].iter().map(|s| s.to_string()).collect();
        let cls = [0, 0, 0, 0, 1, 1, 2, 2];
        let mut probs = vec![0f32; 3 * 8];
        for (i, c) in cls.iter().enumerate() {
            probs[c * 8 + i] = 1.0;
        }
        let scene = Parsed { classes: 3, w: 4, h: 2, vw: 4, vh: 2, probs };
        let sky = landscape(&scene, &labels, "sky").unwrap().unwrap();
        let l = sky.logits().unwrap();
        assert!(l[0] > 8.0 && l[SIDE * SIDE - 1] < -8.0, "sky on top");
        assert!(landscape(&scene, &labels, "water").unwrap().is_none(), "no water");
        assert!(landscape(&scene, &labels, "nope").is_err());
        let cov = coverage(&scene, &labels);
        assert_eq!(cov[0], ("sky".to_string(), 0.5));
        assert_eq!(cov.iter().find(|c| c.0 == "vegetation").map(|c| c.1), Some(0.25));
        let mut m = Models { sam: None, scene: Some(&scene) };
        let ppl = people(&mut m, Some(&labels), &[]).unwrap();
        assert_eq!(ppl.len(), 1);
        assert_eq!(ppl[0].rect, [0.0, 0.5, 0.5, 1.0]);
        assert!(scene_subject(&scene, &labels).is_some());
        // without any model: no people, no subject
        assert!(people(&mut Models { sam: None, scene: None }, None, &[]).unwrap().is_empty());
        assert!(subject(&mut Models { sam: None, scene: None }).unwrap().is_none());
    }
}
