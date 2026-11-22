//! Remove ▸ Detect (Lightroom Classic's Detect objects, ⌥⇧O): people, wires and cables and sensor dust are found
//! and *offered*: each is a candidate the Remove panel lists and the photo highlights; the user picks which go
//! (click, ⌘-click, ⇧-click, all. / none.) and `remove.apply` removes them, generatively when the inpainting model
//! is there (one Generative Remove spot per object), else with content-aware Fill spots; dust gets heal spots.

use lightcraft_catalog::PhotoId;
use lightcraft_develop::{SegMask, Spot, SpotMode};
use lightcraft_geom::Point;
use serde_json::{Value, json};

use crate::Session;

/// What a candidate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Person,
    Wires,
    Dust,
}

/// One detected thing to remove.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub kind: Kind,
    pub label: String,
    /// The spots that remove it without the model (content-aware rows over an object, heal spots on dust).
    pub spots: Vec<Spot>,
    /// Its bounding box, normalized `[x0, y0, x1, y1]`.
    pub rect: [f64; 4],
}

/// The candidates found in one photo.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub photo: Option<PhotoId>,
    pub items: Vec<Candidate>,
    /// Searches still running in the background.
    pub pending: usize,
}

/// Most points one merged (generative) object spot keeps.
const MAX_POINTS: usize = 4000;

fn bounds(spots: &[Spot], aspect: f64) -> Option<[f64; 4]> {
    let a = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    // spot sizes are fractions of the long edge
    let (lx, ly) = if a >= 1.0 { (1.0, a) } else { (1.0 / a, 1.0) };
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for s in spots {
        for p in s.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()) {
            let (rx, ry) = (s.size * lx, s.size * ly);
            b = [b[0].min(p.x - rx), b[1].min(p.y - ry), b[2].max(p.x + rx), b[3].max(p.y + ry)];
        }
    }
    (b[2] > b[0] && b[3] > b[1]).then(|| [b[0].max(0.0), b[1].max(0.0), b[2].min(1.0), b[3].min(1.0)])
}

/// One Generative Remove spot over everything `rows` cover (their points, the largest size).
pub fn merged(rows: &[Spot]) -> Option<Spot> {
    let size = rows.iter().map(|s| s.size).filter(|s| s.is_finite()).fold(0.0, f64::max);
    let points: Vec<Point> = rows.iter().flat_map(|s| s.points.iter().copied()).take(MAX_POINTS).collect();
    (!points.is_empty() && size > 0.0).then(|| Spot { mode: SpotMode::Fill, points, size, feather: 30.0, ..Default::default() })
}

impl Session {
    fn photo_aspect(&self, id: PhotoId) -> f64 {
        let (Some(p), Some(d)) = (self.catalog.photo(id), self.develop_of(id)) else { return 1.0 };
        let (pw, ph) = (f64::from(p.width.max(1)), f64::from(p.height.max(1)));
        if d.orientation.swaps_axes() { ph / pw } else { pw / ph }
    }

    /// Start a new list of candidates for photo `id`.
    pub(crate) fn remove_found_reset(&mut self, id: PhotoId) {
        self.remove_found = Found { photo: Some(id), items: Vec::new(), pending: 0 };
    }

    /// Add detected objects (people, or the wires and cables) as candidates.
    pub(crate) fn remove_found_add(&mut self, id: PhotoId, kind: Kind, segs: &[SegMask]) -> usize {
        if self.remove_found.photo != Some(id) {
            self.remove_found_reset(id);
        }
        let aspect = self.photo_aspect(id);
        let mut n = 0;
        for seg in segs {
            let spots = crate::segment::spots_covering(seg, aspect);
            let Some(rect) = bounds(&spots, aspect) else { continue };
            let people = self.remove_found.items.iter().filter(|c| c.kind == Kind::Person).count();
            let label = match kind {
                Kind::Person => format!("{}.", people + 1),
                Kind::Wires => "wires.".into(),
                Kind::Dust => "dust.".into(),
            };
            self.remove_found.items.push(Candidate { kind, label, spots, rect });
            n += 1;
        }
        n
    }

    /// Add the dust spots found as one candidate.
    pub(crate) fn remove_found_dust(&mut self, id: PhotoId, spots: Vec<Spot>) {
        if self.remove_found.photo != Some(id) {
            self.remove_found_reset(id);
        }
        let aspect = self.photo_aspect(id);
        if let Some(rect) = bounds(&spots, aspect) {
            let label = format!("dust.{}", spots.len());
            self.remove_found.items.push(Candidate { kind: Kind::Dust, label, spots, rect });
        }
    }

    /// The candidates of photo `id` as JSON.
    pub fn remove_candidates(&self, id: PhotoId) -> Value {
        let mine = self.remove_found.photo == Some(id);
        let items: Vec<Value> = if mine {
            self.remove_found
                .items
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let pts: Vec<Value> =
                        c.spots.iter().map(|s| json!({"points": s.points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(), "size": s.size})).collect();
                    json!({"index": i, "kind": c.kind, "label": c.label, "rect": c.rect, "spots": pts})
                })
                .collect()
        } else {
            Vec::new()
        };
        json!({"candidates": items, "pending": if mine { self.remove_found.pending } else { 0 }})
    }

    /// Remove candidates `indices` of the active photo (one undo step): generatively (one spot per object) or with
    /// content-aware rows; dust with heal spots. They leave the list. → spots added.
    pub fn remove_apply(&mut self, indices: &[usize], generative: bool) -> Result<usize, String> {
        let id = self.active().ok_or("no active photo")?;
        if self.remove_found.photo != Some(id) {
            return Err("nothing was detected in this photo: run remove.detect first".into());
        }
        let mut picked: Vec<usize> = indices.iter().copied().filter(|i| *i < self.remove_found.items.len()).collect();
        picked.sort_unstable();
        picked.dedup();
        if picked.is_empty() {
            return Err("pick at least one detected object".into());
        }
        let mut d = (*self.develop_of(id).ok_or("no such photo")?).clone();
        let mut added = 0;
        let mut gen_spots = Vec::new();
        for &i in &picked {
            let Some(c) = self.remove_found.items.get(i).cloned() else { continue };
            match c.kind {
                Kind::Dust => {
                    for mut s in c.spots {
                        s.source_offset = crate::cmd::spot_source(self, id, &d, &s);
                        d.spots.push(s);
                        added += 1;
                    }
                }
                _ if generative => {
                    if let Some(s) = merged(&c.spots) {
                        d.spots.push(s);
                        gen_spots.push(d.spots.len() - 1);
                        added += 1;
                    }
                }
                _ => {
                    added += c.spots.len();
                    d.spots.extend(c.spots);
                }
            }
        }
        for i in gen_spots {
            self.gen_make(id, &mut d, i);
        }
        self.set_develop(id, d, "Remove Detected").map_err(|e| e.to_string())?;
        for &i in picked.iter().rev() {
            self.remove_found.items.remove(i);
        }
        if generative {
            self.gen_schedule(id);
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_merged_spot_covers_every_row_and_stays_bounded() {
        let rows: Vec<Spot> = (0..5)
            .map(|k| Spot {
                mode: SpotMode::Fill,
                points: vec![Point::new(0.1, 0.1 + k as f64 * 0.01), Point::new(0.2, 0.1)],
                size: 0.01,
                ..Default::default()
            })
            .collect();
        let m = merged(&rows).unwrap();
        assert_eq!((m.points.len(), m.size, m.mode), (10, 0.01, SpotMode::Fill));
        let b = bounds(&rows, 1.5).unwrap();
        assert!(b[0] < 0.1 && b[2] > 0.2 && b[3] > 0.14, "{b:?}");
        let many = vec![Spot { points: vec![Point::new(0.5, 0.5); 10_000], size: 0.01, ..Default::default() }];
        assert_eq!(merged(&many).unwrap().points.len(), MAX_POINTS);
        assert!(merged(&[]).is_none());
        assert!(bounds(&[Spot { points: vec![Point::new(f64::NAN, 0.5)], ..Default::default() }], 1.0).is_none());
    }
}
