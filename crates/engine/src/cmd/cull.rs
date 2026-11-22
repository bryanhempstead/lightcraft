//! Assisted culling: score photos for focus and clipping, group similar shots taken close
//! together (bursts) and mark the sharpest of each group; optionally reject blurry ones and pick
//! each group's best. Scores are kept on the photos (rules / smart albums: `sharpness`,
//! `bestOfGroup`).

use lightcraft_catalog::{Analysis, Flag, Op, PhotoId};
use serde_json::{Value, json};

use super::{CommandSpec, always, cmd};
use crate::{Result, Session};

/// Seconds between shots of the same burst at most.
const BURST_GAP: i64 = 10;
/// Signature similarity for "the same scene".
const SAME: f32 = 0.93;

fn secs(iso: &str) -> Option<i64> {
    let d = lightcraft_meta::DateTime::parse_iso(iso)?;
    Some(((d.year as i64 * 372 + d.month as i64 * 31 + d.day as i64) * 24 + d.hour as i64) * 3600 + d.minute as i64 * 60 + d.second as i64)
}

fn analyze(s: &mut Session, p: &Value) -> Result<Value> {
    let ids: Vec<PhotoId> = if p.get("ids").is_some() || s.selection.ids.len() > 1 { s.targets(p) } else { s.visible_cloned() };
    let reject_below = p.get("rejectBelow").and_then(Value::as_f64).map(|v| v as f32);
    let pick_best = p.get("pickBest").and_then(Value::as_bool).unwrap_or(false);
    // measure (thumbnail-level sources: fast, and enough for focus at 512 px)
    let mut rows: Vec<(PhotoId, Option<i64>, f32, f32, [f32; 64])> = Vec::new();
    let mut failed = Vec::new();
    for id in ids {
        match s.source_now(id, crate::media::SourceLevel::Thumb) {
            Ok(src) => {
                let t = s.catalog.photo(id).and_then(|p| p.captured.as_deref().and_then(secs));
                use lightcraft_pipeline::cull;
                rows.push((id, t, cull::sharpness(&src), cull::clipped(&src), cull::signature(&src)));
            }
            Err(e) => failed.push(json!([id.0, e])),
        }
    }
    // bursts: consecutive (by capture time) look-alike shots
    rows.sort_by_key(|r| (r.1.unwrap_or(i64::MAX), r.0));
    let mut group_of: Vec<Option<u32>> = vec![None; rows.len()];
    let mut next = 1u32;
    for i in 1..rows.len() {
        let (a, b) = (&rows[i - 1], &rows[i]);
        let close = matches!((a.1, b.1), (Some(x), Some(y)) if (y - x).abs() <= BURST_GAP);
        if close && lightcraft_pipeline::cull::similarity(&a.4, &b.4) >= SAME {
            let g = *group_of[i - 1].get_or_insert_with(|| {
                next += 1;
                next - 1
            });
            group_of[i] = Some(g);
        }
    }
    let mut best: std::collections::HashMap<u32, (usize, f32)> = Default::default();
    for (i, g) in group_of.iter().enumerate() {
        if let Some(g) = g {
            let e = best.entry(*g).or_insert((i, -1.0));
            if rows[i].2 > e.1 {
                *e = (i, rows[i].2);
            }
        }
    }
    let mut ops = Vec::new();
    let (mut rejected, mut picked) = (0, 0);
    let mut out = Vec::new();
    for (i, (id, _, sharp, clip, _)) in rows.iter().enumerate() {
        let g = group_of[i];
        let is_best = g.is_some_and(|g| best.get(&g).is_some_and(|b| b.0 == i));
        let prev = s.catalog.photo(*id).and_then(|p| p.analysis).unwrap_or_default();
        ops.push(Op::SetAnalysis { id: *id, analysis: Some(Analysis { sharpness: *sharp, clipped: *clip, group: g, best: is_best, ..prev }) });
        let flag = s.catalog.photo(*id).map(|p| p.flag).unwrap_or_default();
        if reject_below.is_some_and(|t| *sharp < t) && flag != Flag::Reject {
            ops.push(Op::SetFlag { id: *id, flag: Flag::Reject });
            rejected += 1;
        } else if pick_best && is_best && flag == Flag::None {
            ops.push(Op::SetFlag { id: *id, flag: Flag::Pick });
            picked += 1;
        }
        out.push(json!({"id": id.0, "sharpness": (sharp * 10.0).round() / 10.0, "clipped": clip, "group": g, "best": is_best}));
    }
    if !ops.is_empty() {
        s.commit("Assisted Culling", Op::Batch { ops })?;
    }
    Ok(json!({"photos": out, "groups": best.len(), "rejected": rejected, "picked": picked, "failed": failed}))
}

/// Find Similar: photos that look like `id` (signatures from thumbnail-level sources, cached by
/// content), most similar first; the view is filtered to them.
fn find_similar(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.findSimilar";
    let id = p.get("id").and_then(Value::as_u64).map(PhotoId).or(s.active()).ok_or_else(|| super::bad(C, "no photo"))?;
    let min = p.get("similarity").and_then(Value::as_f64).unwrap_or(0.8) as f32;
    let all: Vec<PhotoId> = s.catalog.photos().filter(|q| q.in_library()).map(|q| q.id).collect();
    let sig = |s: &mut Session, id: PhotoId| -> Option<[f32; 64]> {
        let key = s.catalog.photo(id).map(|p| crate::media::content_key(p))?;
        if let Some(v) = s.signatures.get(&key) {
            return Some(*v);
        }
        let src = s.source_now(id, crate::media::SourceLevel::Thumb).ok()?;
        let v = lightcraft_pipeline::cull::signature(&src);
        s.signatures.insert(key, v);
        Some(v)
    };
    let me = sig(s, id).ok_or_else(|| super::bad(C, "can't read the photo"))?;
    let mut hits: Vec<(PhotoId, f32)> = Vec::new();
    for q in all {
        if let Some(v) = sig(s, q) {
            let sim = lightcraft_pipeline::cull::similarity(&me, &v);
            if sim >= min {
                hits.push((q, sim));
            }
        }
    }
    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
    if p.get("filter").and_then(Value::as_bool).unwrap_or(true) {
        s.filter.only = hits.iter().map(|h| h.0).collect();
    }
    Ok(json!({"photos": hits.iter().map(|(i, v)| json!({"id": i.0, "similarity": (v * 1000.0).round() / 1000.0})).collect::<Vec<_>>()}))
}

/// The visual signature of a photo (cached by content), from its thumbnail.
fn signature_of(s: &mut Session, id: PhotoId) -> Option<[f32; 64]> {
    let key = s.catalog.photo(id).map(|p| crate::media::content_key(p))?;
    if let Some(v) = s.signatures.get(&key) {
        return Some(*v);
    }
    let src = s.source_now(id, crate::media::SourceLevel::Thumb).ok()?;
    let v = lightcraft_pipeline::cull::signature(&src);
    s.signatures.insert(key, v);
    Some(v)
}

/// Runs of look-alike photos among `ids` (in capture order): each photo joins the run before it
/// when it looks like that run's last photo (similarity ≥ `min`) and was taken within `gap`
/// seconds (0 = any time). Runs of one are left out.
fn similar_runs(s: &mut Session, ids: &[PhotoId], min: f32, gap: f64) -> Vec<Vec<PhotoId>> {
    let mut order: Vec<PhotoId> = ids.to_vec();
    order.sort_by(|a, b| {
        let d = |id: &PhotoId| s.catalog.photo(*id).map(|p| p.date().to_string()).unwrap_or_default();
        d(a).cmp(&d(b)).then(a.cmp(b))
    });
    let secs = |s: &Session, id: PhotoId| s.catalog.photo(id).and_then(|p| lightcraft_catalog::dates::iso_seconds(p.date()));
    let mut runs: Vec<Vec<PhotoId>> = Vec::new();
    let mut last: Option<(PhotoId, [f32; 64])> = None;
    for id in order {
        let Some(sig) = signature_of(s, id) else {
            last = None;
            continue;
        };
        let joins = last.is_some_and(|(prev, psig)| {
            let close = gap <= 0.0 || matches!((secs(s, prev), secs(s, id)), (Some(a), Some(b)) if ((b - a).abs() as f64) <= gap);
            close && lightcraft_pipeline::cull::similarity(&psig, &sig) >= min
        });
        match runs.last_mut() {
            Some(run) if joins => run.push(id),
            _ => runs.push(vec![id]),
        }
        last = Some((id, sig));
    }
    runs.retain(|r| r.len() > 1);
    runs
}

/// L27: stack look-alike photos (Lightroom 15's Auto-Stack by visual similarity).
fn auto_stack_similar(s: &mut Session, p: &Value) -> Result<Value> {
    use lightcraft_catalog::Stack;
    let min = p.get("similarity").and_then(Value::as_f64).unwrap_or(0.9).clamp(0.0, 1.0) as f32;
    let gap = p.get("gap").and_then(Value::as_f64).unwrap_or(60.0).max(0.0);
    let targets = if s.selection.ids.len() > 1 { s.selection.ids.clone() } else { s.visible_cloned() };
    // photos already stacked stay where they are
    let free: Vec<PhotoId> =
        targets.into_iter().filter(|id| s.catalog.stack_of(*id).is_none() && s.catalog.photo(*id).is_some_and(|p| p.copy_of.is_none())).collect();
    let runs = similar_runs(s, &free, min, gap);
    if super::bool_or(p, "preview", false) || runs.is_empty() {
        return Ok(json!({"stacks": runs.len(), "photos": runs.iter().map(Vec::len).sum::<usize>(), "preview": true}));
    }
    for run in &runs {
        let id = s.catalog.alloc_stack_id();
        s.commit("Auto-Stack by Similarity", Op::AddStack { stack: Stack { id, photos: run.clone(), collapsed: true } })?;
    }
    s.merge_undo(runs.len(), "Auto-Stack by Similarity");
    Ok(json!({"stacks": runs.len()}))
}

/// L28: duplicates — the same file content twice, or photos that look the same (similarity ≥
/// `similarity`, default 0.98) taken within `gap` seconds. Filters the view to them.
fn duplicates(s: &mut Session, p: &Value) -> Result<Value> {
    let min = p.get("similarity").and_then(Value::as_f64).unwrap_or(0.98).clamp(0.0, 1.0) as f32;
    let gap = p.get("gap").and_then(Value::as_f64).unwrap_or(5.0).max(0.0);
    let ids: Vec<PhotoId> = s.catalog.photos().filter(|q| q.in_library() && q.copy_of.is_none()).map(|q| q.id).collect();
    // same content
    let mut by_hash: std::collections::BTreeMap<String, Vec<PhotoId>> = Default::default();
    for id in &ids {
        if let Some(h) = s.catalog.photo(*id).and_then(|q| q.content_hash.clone()) {
            by_hash.entry(h).or_default().push(*id);
        }
    }
    let mut groups: Vec<Vec<PhotoId>> = by_hash.into_values().filter(|g| g.len() > 1).collect();
    let seen: std::collections::HashSet<PhotoId> = groups.iter().flatten().copied().collect();
    let rest: Vec<PhotoId> = ids.into_iter().filter(|id| !seen.contains(id)).collect();
    groups.extend(similar_runs(s, &rest, min, gap));
    if super::bool_or(p, "filter", true) {
        s.filter.only = groups.iter().flatten().copied().collect();
    }
    Ok(
        json!({"groups": groups.iter().map(|g| g.iter().map(|i| i.0).collect::<Vec<_>>()).collect::<Vec<_>>(), "photos": groups.iter().map(Vec::len).sum::<usize>()}),
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "stack.autoSimilar",
            "Auto-Stack by Similarity",
            ["Photo"],
            None,
            "{similarity?: 0..1 (0.9), gap?: seconds between neighbours (60; 0 = any), preview?: bool} — stacks runs of look-alike photos (the selection, else the view; photos already stacked and virtual copies stay out), collapsed; one undo step → {stacks}",
            always,
            auto_stack_similar
        ),
        cmd!(
            "library.duplicates",
            "Find Duplicates",
            ["Library"],
            None,
            "{similarity?: 0..1 (0.98), gap?: seconds (5), filter?: bool (true)} — photos whose files have the same content, and look-alikes taken within gap seconds; filters the view to them (Clear Filters to go back) → {groups: [[id]], photos}",
            always,
            duplicates
        ),
        cmd!(
            "library.findSimilar",
            "Find Similar Photos",
            ["Photo"],
            None,
            "{id?, similarity?: 0..1 (0.8), filter?: bool (true)} — photos that look like the active one (composition and tones), most similar first; filters the view to them (Clear Filters to go back) → {photos: [{id, similarity}]}",
            always,
            find_similar
        ),
        cmd!(
            "photo.analyze",
            "Assisted Culling",
            [],
            None,
            "{ids?, rejectBelow?: sharpness 0..100, pickBest?: bool} — score the selected photos (else all in view) for focus (0..100) and clipping, group look-alike shots taken within 10 s and mark the sharpest of each; optionally reject blurry photos and pick each group's best; one undo step → {photos: [{id, sharpness, clipped, group, best}], groups, rejected, picked}",
            always,
            analyze
        ),
    ]
}
