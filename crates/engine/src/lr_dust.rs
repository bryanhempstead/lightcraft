//! Photos Lightroom cleaned with Dust Removal (an AI filter baked into its settings as
//! `FilterList` patches we can't use): LightCraft's own Find Dust Spots runs on them instead,
//! keeping the spots inside the areas Lightroom patched (`ReferenceImageArea` of each patch) and
//! no bigger than sensor dust (a big "spot" is usually a dark detail, e.g. a head against the sky,
//! and healing it would damage the photo). `library.lightroomDust`.

use std::collections::HashMap;

use lightcraft_geom::Rect;
use serde_json::{Value, json};

use crate::Session;
use crate::lr_migrate::{LrImage, Records, library_ids, lr_quarter_turns};
use crate::preset_import::{Lua, parse_lua};

/// Largest found spot healed (radius, fraction of the long edge): sensor dust is small.
const MAX_DUST: f64 = 0.01;
/// Margin around a patch a found spot may sit in (fraction of the frame).
const MARGIN: f64 = 0.01;
/// Patches read per photo (hostile input stays bounded).
const MAX_PATCHES: usize = 500;

fn num(v: Option<&Lua>) -> Option<f64> {
    match v {
        Some(Lua::Num(n)) if n.is_finite() => Some(*n),
        Some(Lua::Str(s)) => s.trim().parse().ok().filter(|x: &f64| x.is_finite()),
        _ => None,
    }
}

fn items(v: Option<&Lua>) -> &[Lua] {
    match v {
        Some(Lua::Table(list, _)) => list,
        _ => &[],
    }
}

/// The areas Lightroom's Dust Removal patched, normalized to the file's stored frame (0..1).
pub fn dust_areas(develop: &str) -> Vec<Rect> {
    if !develop.contains("Dust") {
        return Vec::new();
    }
    let Ok(root) = parse_lua(develop) else { return Vec::new() };
    let mut out = Vec::new();
    for f in items(root.get("FilterList").and_then(|l| l.get("Filters"))) {
        let dust = f.get("Name").and_then(Lua::str).is_some_and(|n| n.eq_ignore_ascii_case("Dust Removal"))
            || f.get("Title").and_then(Lua::str).is_some_and(|t| t.contains("DustRemoval"));
        if !dust {
            continue;
        }
        for im in items(f.get("Images")).iter().take(MAX_PATCHES) {
            let stage = items(im.get("Alpha")).first().or_else(|| items(im.get("ColorVariations")).first());
            let (Some(w), Some(h)) = (num(stage.and_then(|a| a.get("Stage3X"))), num(stage.and_then(|a| a.get("Stage3Y")))) else { continue };
            let k = |key: &str| num(im.get(key));
            let (Some(l), Some(t), Some(r), Some(b)) =
                (k("ReferenceImageAreaLeft"), k("ReferenceImageAreaTop"), k("ReferenceImageAreaRight"), k("ReferenceImageAreaBottom"))
            else {
                continue;
            };
            if w < 1.0 || h < 1.0 || r <= l || b <= t {
                continue;
            }
            let c = |x: f64| x.clamp(0.0, 1.0);
            out.push(Rect { x0: c(l / w), y0: c(t / h), x1: c(r / w), y1: c(b / h) });
        }
    }
    out
}

/// Run Find Dust Spots on the library's photos Lightroom used Dust Removal on (see the module
/// docs). Photos edited here since the migration, and areas that already have a spot, are left
/// alone. One undo step.
pub fn find_dust(s: &mut Session, rec: &Records, dry_run: bool, sensitivity: f64) -> crate::Result<Value> {
    let undo0 = s.undo.len();
    let ids = library_ids(s, rec);
    let by_lr: HashMap<i64, &LrImage> = rec.images.iter().map(|i| (i.id, i)).collect();
    let (mut photos, mut not_in_library, mut kept, mut added, mut failed) = (0usize, 0usize, 0usize, 0usize, Vec::new());
    let (mut outside, mut too_big) = (0usize, 0usize);
    let mut list: Vec<(i64, Vec<Rect>)> = rec.images.iter().map(|i| (i.id, dust_areas(&i.develop))).filter(|(_, a)| !a.is_empty()).collect();
    list.sort_by_key(|(i, _)| *i);
    let saved = s.selection.clone();
    for (lr, areas) in list {
        photos += 1;
        let Some(id) = ids.get(&lr).copied() else {
            not_in_library += 1;
            continue;
        };
        let Some(p) = s.catalog.photo(id) else { continue };
        // (edited here: a History step of ours, not a sync's or a Lightroom step gone back to)
        if p.history.iter().any(|h| !h.label.starts_with("Lightroom")) {
            kept += 1;
            continue;
        }
        // Lightroom's areas are in the stored frame; spots are in the frame as shown
        let turns = by_lr.get(&lr).and_then(|im| lr_quarter_turns(&im.orientation)).unwrap_or(0);
        let o = lightcraft_geom::Orientation::from_parts(false, turns);
        let areas: Vec<Rect> = areas
            .iter()
            .map(|r| o.map_norm_rect(*r))
            .map(|m| Rect { x0: m.x0.min(m.x1), y0: m.y0.min(m.y1), x1: m.x0.max(m.x1), y1: m.y0.max(m.y1) })
            .collect();
        let existing: Vec<(f64, f64)> = p.develop.spots.iter().filter_map(|sp| sp.points.first().map(|q| (q.x, q.y))).collect();
        let inside = |r: &Rect, x: f64, y: f64| x >= r.x0 - MARGIN && x <= r.x1 + MARGIN && y >= r.y0 - MARGIN && y <= r.y1 + MARGIN;
        s.selection = crate::Selection::single(id);
        let found = match s.execute("spot.findDust", &json!({"add": false, "sensitivity": sensitivity})) {
            Ok(v) => v["spots"].as_array().cloned().unwrap_or_default(),
            Err(e) => {
                failed.push(json!({"photoId": id.0, "error": e.to_string()}));
                continue;
            }
        };
        let mut to_add: Vec<(f64, f64, f64)> = Vec::new();
        for (x, y, size) in found.iter().filter_map(|d| Some((d["x"].as_f64()?, d["y"].as_f64()?, d["size"].as_f64()?))) {
            if !areas.iter().any(|r| inside(r, x, y)) {
                outside += 1;
                continue;
            }
            // bigger than sensor dust: more likely a dark detail (a head against the sky)
            if size > MAX_DUST {
                too_big += 1;
                continue;
            }
            // a spot already there (an earlier run, or one made by hand)
            if existing.iter().any(|(ex, ey)| (ex - x).hypot(ey - y) < size.max(0.003)) {
                continue;
            }
            to_add.push((x, y, size));
        }
        added += to_add.len();
        if dry_run {
            continue;
        }
        for (x, y, size) in to_add {
            if let Err(e) = s.execute("spot.add", &json!({"mode": "heal", "points": [[x, y]], "size": size})) {
                failed.push(json!({"photoId": id.0, "error": e.to_string()}));
            }
        }
    }
    s.selection = saved;
    let steps = s.undo.len().saturating_sub(undo0);
    s.merge_undo(steps, "Lightroom Dust Removal");
    Ok(json!({
        "dryRun": dry_run,
        "photos": photos,
        "notInLibrary": not_in_library,
        "keptChangedHere": kept,
        "spots": added,
        "outsideLightroomAreas": outside,
        "tooBig": too_big,
        "failed": failed,
    }))
}

#[cfg(test)]
#[path = "tests_lr_dust.rs"]
mod tests;
