//! `library.repairLightroomImport`: re-derive two things earlier Lightroom migrations read wrong,
//! from the Lightroom source (a read-only copy of the catalog, or records read from one), and fix
//! them in the library as one undo step:
//!
//! - **straightened crops**: Lightroom's `CropLeft / Top / Right / Bottom` are two opposite corners
//!   of the rotated crop in the unrotated image ([`crate::crs::crop_from_lr`]); earlier versions
//!   read them as an axis-aligned rectangle in the straightened frame (the crop came out
//!   θ·(r + 1/r) the wrong shape), some also with the angle mirrored;
//! - **grouped radial gradients**: inside `MaskGroupBasedCorrections` Camera Raw ignores
//!   `crs:Flipped` (only `MaskInverted` inverts); earlier versions inverted those radials.
//!
//! Only values still exactly as a migration left them are changed: a crop or radial that differs
//! from both the old reading and the right one was changed here since, and is reported as
//! skipped. Nothing else of the photo is touched.

use std::sync::Arc;

use lightcraft_catalog::Op;
use lightcraft_develop::{DevelopSettings, Mask, MaskShape};
use lightcraft_geom::CropGeometry;
use serde_json::{Value, json};

use crate::Session;
use crate::lr_migrate::{LrImage, Records, library_ids, lr_quarter_turns, prepare_image, reorient_partial};

/// Values closer than this are the same (they went through the same arithmetic and JSON).
const EPS: f64 = 1e-6;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPS
}

fn same_crop(a: &CropGeometry, b: &CropGeometry) -> bool {
    close(a.angle, b.angle)
        && close(a.rect.x0, b.rect.x0)
        && close(a.rect.y0, b.rect.y0)
        && close(a.rect.x1, b.rect.x1)
        && close(a.rect.y1, b.rect.y1)
}

fn crop_value(g: &CropGeometry) -> Value {
    json!({"x0": g.rect.x0, "y0": g.rect.y0, "x1": g.rect.x1, "y1": g.rect.y1, "angle": g.angle})
}

/// A radial's geometry is the same (everything but its `invert`).
fn same_radial(a: &MaskShape, b: &MaskShape) -> Option<(bool, bool)> {
    match (a, b) {
        (
            MaskShape::Radial { center: c1, rx: x1, ry: y1, angle: a1, feather: f1, invert: i1 },
            MaskShape::Radial { center: c2, rx: x2, ry: y2, angle: a2, feather: f2, invert: i2 },
        ) => {
            (close(c1.x, c2.x) && close(c1.y, c2.y) && close(*x1, *x2) && close(*y1, *y2) && close(*a1, *a2) && close(*f1, *f2)).then_some((*i1, *i2))
        }
        _ => None,
    }
}

/// Masks partial JSON → our masks (as the migration applied them).
fn masks_of(partial: &[Value]) -> Vec<Mask> {
    lightcraft_develop::apply_partial(&DevelopSettings::default(), &json!({"masks": partial}), 1.0).masks
}

/// What one photo needs.
#[derive(Default)]
struct Fix {
    crop: Option<(CropGeometry, CropGeometry)>,
    /// (mask, component, from, to)
    radials: Vec<(usize, usize, bool, bool)>,
    crop_skipped: bool,
    masks_skipped: bool,
    crop_right: bool,
    radials_right: usize,
}

/// Work out the crop and radial fixes of photo `d` from Lightroom's `im`.
fn plan(im: &LrImage, d: &DevelopSettings, photo_aspect: Option<f64>) -> Fix {
    let mut fix = Fix::default();
    let Ok((props, values)) = crate::preset_import::develop_text_props(&im.develop) else { return fix };
    let prep = prepare_image(im, photo_aspect);
    let Some(partial) = prep.develop else { return fix };
    let turns = lr_quarter_turns(&im.orientation).unwrap_or(0);
    let turn = |mut v: Value| {
        reorient_partial(&mut v, lightcraft_geom::Orientation::from_parts(false, turns));
        v
    };
    // ---- the crop (only a straightened one was read wrong)
    let num =
        |k: &str| props.get(k).and_then(|v| v.first()).and_then(|s| s.trim().trim_start_matches('+').parse::<f64>().ok()).filter(|v| v.is_finite());
    let lr_angle = num("crs:CropAngle").unwrap_or(0.0).clamp(-45.0, 45.0);
    let want = partial.pointer("/crop/geometry").cloned().and_then(|g| serde_json::from_value::<CropGeometry>(g).ok());
    if let Some(want) = want.filter(|_| lr_angle != 0.0) {
        let e = |k: &str, dflt: f64| num(&format!("crs:Crop{k}")).unwrap_or(dflt).clamp(0.0, 1.0);
        let rect = json!({"x0": e("Left", 0.0), "y0": e("Top", 0.0), "x1": e("Right", 1.0), "y1": e("Bottom", 1.0)});
        // what earlier versions made of it: the edges as a rectangle, the angle negated (since
        // 2026-10) or kept (before)
        let old: Vec<CropGeometry> = [-lr_angle, lr_angle]
            .iter()
            .filter_map(|a| {
                let v = turn(json!({"crop": {"geometry": {"rect": rect, "angle": a}}}));
                serde_json::from_value(v["crop"]["geometry"].clone()).ok()
            })
            .collect();
        let have = d.crop.geometry;
        if same_crop(&have, &want) {
            fix.crop_right = true;
        } else if old.iter().any(|o| same_crop(&have, o)) {
            fix.crop = Some((have, want));
        } else {
            fix.crop_skipped = true;
        }
    }
    // ---- grouped radials whose `crs:Flipped` inverted them
    if values.contains_key("crs:MaskGroupBasedCorrections") {
        let aspect = if im.width > 0 && im.height > 0 {
            im.width as f64 / im.height as f64
        } else {
            photo_aspect.map(|a| if turns % 2 == 1 { 1.0 / a } else { a }).unwrap_or(crate::crs_masks::DEFAULT_ASPECT)
        };
        let old = turn(json!({"masks": crate::crs_masks::masks_legacy_flipped(&values, aspect)}));
        let old = masks_of(old["masks"].as_array().map(Vec::as_slice).unwrap_or_default());
        let new = masks_of(partial["masks"].as_array().map(Vec::as_slice).unwrap_or_default());
        let mut todo = Vec::new();
        for (i, (o, n)) in old.iter().zip(&new).enumerate() {
            for (j, (oc, nc)) in o.components.iter().zip(&n.components).enumerate() {
                let Some((from, to)) = same_radial(&oc.shape, &nc.shape).filter(|(f, t)| f != t) else { continue };
                // the photo's own component there: still the migrated radial (same mask, same
                // shape), or changed here since?
                let here = d.masks.get(i).filter(|m| m.name == o.name && m.components.len() == o.components.len()).and_then(|m| m.components.get(j));
                match here.and_then(|c| same_radial(&c.shape, &oc.shape).map(|(h, _)| h)) {
                    Some(h) if h == to => fix.radials_right += 1,
                    Some(_) => todo.push((i, j, from, to)),
                    None => fix.masks_skipped = true,
                }
            }
        }
        // one radial changed here: the photo's masks are left as they are
        if !fix.masks_skipped {
            fix.radials = todo;
        }
    }
    fix
}

/// The catalog the library's Lightroom migration read (its [`crate::lr_migrate::RESUME_FILE`]),
/// if it is still there.
pub fn recorded_catalog(s: &Session) -> Option<String> {
    let dir = s.library.as_ref().filter(|l| l.on_disk)?.dir.clone();
    let doc: Value = serde_json::from_slice(&std::fs::read(dir.join(crate::lr_migrate::RESUME_FILE)).ok()?).ok()?;
    doc["catalog"].as_str().filter(|c| std::path::Path::new(c).is_file()).map(str::to_string)
}

/// Repair photos migrated from Lightroom (`rec`: the catalog's records) — see the module docs.
/// `limit`: how many changed / skipped photos the report lists (the counts are complete).
pub fn repair_import(s: &mut Session, rec: &Records, dry_run: bool, limit: usize) -> crate::Result<Value> {
    let ids = library_ids(s, rec);
    let (mut crops, mut crops_right, mut crops_skipped) = (0usize, 0usize, 0usize);
    let (mut radials, mut radial_photos, mut radials_right, mut masks_skipped) = (0usize, 0usize, 0usize, 0usize);
    let (mut changes, mut skipped, mut ops) = (Vec::new(), Vec::new(), Vec::new());
    let mut photos = 0usize;
    for im in &rec.images {
        let Some(p) = ids.get(&im.id).and_then(|id| s.catalog.photo(*id)) else { continue };
        if im.develop.trim().is_empty() {
            continue;
        }
        let aspect = (p.width > 0 && p.height > 0).then(|| p.width as f64 / p.height as f64);
        let fix = plan(im, &p.develop, aspect);
        crops_right += usize::from(fix.crop_right);
        radials_right += fix.radials_right;
        if fix.crop_skipped || fix.masks_skipped {
            crops_skipped += usize::from(fix.crop_skipped);
            masks_skipped += usize::from(fix.masks_skipped);
            if skipped.len() < limit {
                let what: Vec<&str> = [(fix.crop_skipped, "crop"), (fix.masks_skipped, "masks")].iter().filter(|w| w.0).map(|w| w.1).collect();
                skipped.push(
                    json!({"photo": p.id.0, "file": p.file_name, "lightroomId": im.id, "what": what, "why": "changed here since the migration"}),
                );
            }
        }
        if fix.crop.is_none() && fix.radials.is_empty() {
            continue;
        }
        let mut d = (*p.develop).clone();
        let mut entry = json!({"photo": p.id.0, "file": p.file_name, "lightroomId": im.id});
        if let Some((before, after)) = fix.crop {
            crops += 1;
            d.crop.geometry = after;
            entry["crop"] = json!({"before": crop_value(&before), "after": crop_value(&after)});
        }
        if !fix.radials.is_empty() {
            radial_photos += 1;
            let mut list = Vec::new();
            for &(i, j, from, to) in &fix.radials {
                if let Some(MaskShape::Radial { invert, .. }) = d.masks.get_mut(i).and_then(|m| m.components.get_mut(j)).map(|c| &mut c.shape) {
                    *invert = to;
                    radials += 1;
                    list.push(
                        json!({"mask": d.masks.get(i).map(|m| m.name.clone()), "component": j, "before": {"invert": from}, "after": {"invert": to}}),
                    );
                }
            }
            entry["radials"] = Value::Array(list);
        }
        if d == *p.develop {
            continue;
        }
        photos += 1;
        if changes.len() < limit {
            changes.push(entry);
        }
        if !dry_run {
            ops.push(Op::SetDevelop { id: p.id, settings: Arc::new(d), label: "Repair Lightroom Import".into(), edited: p.edited.clone() });
        }
    }
    if !ops.is_empty() {
        s.commit("Repair Lightroom Import", Op::Batch { ops })?;
    }
    Ok(json!({
        "dryRun": dry_run,
        "photosChanged": photos,
        "crops": {"fixed": crops, "alreadyRight": crops_right, "skippedChangedHere": crops_skipped},
        "radials": {"fixed": radials, "photos": radial_photos, "alreadyRight": radials_right, "skippedChangedHere": masks_skipped},
        "changes": changes,
        "skipped": skipped,
    }))
}

#[cfg(test)]
#[path = "tests_lr_repair.rs"]
mod tests;
