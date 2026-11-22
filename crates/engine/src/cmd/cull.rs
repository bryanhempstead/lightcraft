//! Assisted culling: score photos for focus and clipping, group similar shots taken close
//! together (bursts) and mark the sharpest of each group; optionally reject blurry ones and pick
//! each group's best. With the face models (L09, Lightroom's AI culling criteria) also: the faces
//! in each photo, how many blink (eyes closed) and how sharp the main face is — a burst's best is
//! then the one with nobody blinking and the sharpest face, and photos can be rejected for closed
//! eyes or a soft face. With the expression model too: smiles and how pleasant the main faces look
//! (`expression`, 0..100) — a burst's best then also prefers smiles, and photos can be rejected below an
//! expression score. Scores are kept on the photos (rules / smart albums: `sharpness`, `bestOfGroup`,
//! `faceFocus`, `eyesClosed`, `faceCount`, `smiles`, `expression`).

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

/// What a culling run does besides scoring.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CullOptions {
    pub reject_below: Option<f32>,
    pub pick_best: bool,
    pub reject_eyes: bool,
    pub reject_face_below: Option<f32>,
    pub reject_expression_below: Option<f32>,
    /// Write the scores as LightCraft's bookkeeping (culling at import), not as an undo step. Flags are never set
    /// this way.
    pub bookkeeping: bool,
}

impl CullOptions {
    fn from_params(p: &Value) -> CullOptions {
        let num = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32);
        CullOptions {
            reject_below: num("rejectBelow"),
            pick_best: super::bool_or(p, "pickBest", false),
            reject_eyes: super::bool_or(p, "rejectEyesClosed", false),
            reject_face_below: num("rejectFaceBelow"),
            reject_expression_below: num("rejectExpressionBelow"),
            bookkeeping: super::bool_or(p, "bookkeeping", false),
        }
    }

    fn flags(&self) -> bool {
        !self.bookkeeping
            && (self.reject_below.is_some()
                || self.pick_best
                || self.reject_eyes
                || self.reject_face_below.is_some()
                || self.reject_expression_below.is_some())
    }
}

/// One photo measured for culling: capture time (seconds), focus, clipping, look.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: PhotoId,
    pub t: Option<i64>,
    pub sharp: f32,
    pub clip: f32,
    pub sig: [f32; 64],
}

fn measure(src: &lightcraft_raster::Rgb32f) -> (f32, f32, [f32; 64]) {
    use lightcraft_pipeline::cull;
    (cull::sharpness(src), cull::clipped(src), cull::signature(src))
}

/// A culling run on worker threads (the app): photos are measured from thumbnail-level sources off the UI thread
/// (as background work: the grid and loupe go first); [`Session::cull_poll`] applies the result when all are in.
pub struct CullJob {
    pub total: usize,
    done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    rx: std::sync::mpsc::Receiver<std::result::Result<Row, (PhotoId, String)>>,
    rows: Vec<Row>,
    failed: Vec<Value>,
    opts: CullOptions,
    faces_pending: usize,
    generation: u64,
}

impl CullJob {
    /// Photos measured so far.
    pub fn done(&self) -> usize {
        self.done.load(std::sync::atomic::Ordering::Relaxed).min(self.total)
    }
}

impl Drop for CullJob {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Workers measuring photos for a culling run at once.
const CULL_WORKERS: usize = 2;

fn analyze(s: &mut Session, p: &Value) -> Result<Value> {
    let ids: Vec<PhotoId> = if p.get("ids").is_some() || s.selection.ids.len() > 1 { s.targets(p) } else { s.visible_cloned() };
    let opts = CullOptions::from_params(p);
    let background = super::bool_or(p, "background", false) && s.vision.background;
    s.cull_start(&ids, opts, super::bool_or(p, "faces", true), background)
}

impl Session {
    /// Assisted Culling of `ids` (see [`analyze`]'s command): with `background`, on worker threads (→ `{queued}`;
    /// the result lands with [`Session::cull_poll`]), else to the end.
    pub fn cull_start(&mut self, ids: &[PhotoId], opts: CullOptions, faces: bool, background: bool) -> Result<Value> {
        // faces: from the faces index; photos not in it yet are analysed first (in the background in the
        // app: their face scores arrive with the results, `Session::vision_poll`)
        let mut faces_pending = 0;
        if faces && s_installed_faces(self) {
            let queued = self.vision_enqueue(crate::vision::Kind::Faces, ids)?;
            if self.vision.background {
                faces_pending = queued;
            }
        }
        if !background {
            // measure (thumbnail-level sources: fast, and enough for focus at 512 px)
            let mut rows: Vec<Row> = Vec::new();
            let mut failed = Vec::new();
            for &id in ids {
                match self.source_now(id, crate::media::SourceLevel::Thumb) {
                    Ok(src) => {
                        let t = self.catalog.photo(id).and_then(|p| p.captured.as_deref().and_then(secs));
                        let (sharp, clip, sig) = measure(&src);
                        rows.push(Row { id, t, sharp, clip, sig });
                    }
                    Err(e) => failed.push(json!([id.0, e])),
                }
            }
            return cull_finish(self, rows, failed, opts, faces_pending);
        }
        // a new run replaces one still measuring
        self.cull_job = None;
        let mut work: std::collections::VecDeque<(PhotoId, Option<i64>, crate::media::SourceRef)> = Default::default();
        let mut seen = std::collections::HashSet::new();
        for &id in ids {
            let Some(p) = self.catalog.photo(id).cloned() else { continue };
            if !seen.insert(id) {
                continue;
            }
            let t = p.captured.as_deref().and_then(secs);
            work.push_back((id, t, self.media.source_ref(&p, crate::media::SourceLevel::Thumb)));
        }
        let total = work.len();
        let queue = std::sync::Arc::new(std::sync::Mutex::new(work));
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..CULL_WORKERS.min(total.max(1)) {
            let (queue, done, cancel, tx) = (queue.clone(), done.clone(), cancel.clone(), tx.clone());
            std::thread::Builder::new()
                .name("cull".into())
                .spawn(move || {
                    loop {
                        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        let next = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop_front();
                        let Some((id, t, src)) = next else { break };
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            crate::memory::in_background(|| src.load()).map(|img| measure(&img))
                        }))
                        .unwrap_or_else(|_| Err("the analysis stopped unexpectedly".into()));
                        let msg = match r {
                            Ok((sharp, clip, sig)) => Ok(Row { id, t, sharp, clip, sig }),
                            Err(e) => Err((id, e)),
                        };
                        done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if tx.send(msg).is_err() {
                            break;
                        }
                    }
                })
                .map_err(|e| crate::EngineError::Other(format!("can't start culling: {e}")))?;
        }
        let generation = self.library_generation();
        self.cull_job = Some(CullJob { total, done, cancel, rx, rows: Vec::new(), failed: Vec::new(), opts, faces_pending, generation });
        Ok(json!({"queued": total, "background": true, "facesPending": faces_pending}))
    }

    /// Collect what the culling workers measured; when every photo is in, score the run (one undo step, or
    /// bookkeeping at import) and return its result. Cheap when nothing is running.
    pub fn cull_poll(&mut self) -> Option<Result<Value>> {
        let job = self.cull_job.as_mut()?;
        while let Ok(m) = job.rx.try_recv() {
            match m {
                Ok(r) => job.rows.push(r),
                Err((id, e)) => job.failed.push(json!([id.0, e])),
            }
        }
        if job.rows.len() + job.failed.len() < job.total {
            return None;
        }
        let job = self.cull_job.take()?;
        if job.generation != self.library_generation() {
            return None;
        }
        let (rows, failed, opts, pending) = (job.rows.clone(), job.failed.clone(), job.opts, job.faces_pending);
        Some(cull_finish(self, rows, failed, opts, pending))
    }

    /// Stop a culling run still measuring (nothing is written).
    pub fn cull_cancel(&mut self) -> bool {
        self.cull_job.take().is_some()
    }
}

fn s_installed_faces(s: &Session) -> bool {
    s.vision.installed(lightcraft_vision::models::Package::Faces)
}

/// Score measured photos: bursts, their best, face scores from the index, flags as `opts` says.
pub(crate) fn cull_finish(s: &mut Session, mut rows: Vec<Row>, failed: Vec<Value>, opts: CullOptions, faces_pending: usize) -> Result<Value> {
    // photos deleted (or another library opened) while they were measured
    rows.retain(|r| s.catalog.photo(r.id).is_some());
    // bursts: consecutive (by capture time) look-alike shots
    rows.sort_by_key(|r| (r.t.unwrap_or(i64::MAX), r.id));
    let mut group_of: Vec<Option<u32>> = vec![None; rows.len()];
    let mut next = 1u32;
    for i in 1..rows.len() {
        let (a, b) = (&rows[i - 1], &rows[i]);
        let close = matches!((a.t, b.t), (Some(x), Some(y)) if (y - x).abs() <= BURST_GAP);
        if close && lightcraft_pipeline::cull::similarity(&a.sig, &b.sig) >= SAME {
            let g = *group_of[i - 1].get_or_insert_with(|| {
                next += 1;
                next - 1
            });
            group_of[i] = Some(g);
        }
    }
    for r in &rows {
        if let Some(key) = s.catalog.photo(r.id).map(|p| crate::media::content_key(p)) {
            s.signatures.insert(key, r.sig);
        }
    }
    // face scores, from the index
    let faces: Vec<Option<FaceScores>> = rows.iter().map(|r| face_scores(s, r.id)).collect();
    // a burst's best: nobody blinking first, then smiles and the sharpest main face, then the sharpest photo
    let rank = |i: usize| -> f32 {
        match faces.get(i).copied().flatten().filter(|f| f.faces > 0) {
            Some(f) => -1000.0 * f32::from(f.eyes_closed) + f.focus.unwrap_or(0.0) * 2.0 + f.expression.unwrap_or(0.0) * 1.5 + rows[i].sharp,
            None => rows[i].sharp,
        }
    };
    let mut best: std::collections::HashMap<u32, (usize, f32)> = Default::default();
    for (i, g) in group_of.iter().enumerate() {
        if let Some(g) = g {
            let e = best.entry(*g).or_insert((i, f32::MIN));
            if rank(i) > e.1 {
                *e = (i, rank(i));
            }
        }
    }
    let mut ops = Vec::new();
    let (mut rejected, mut picked) = (0, 0);
    let mut out = Vec::new();
    let set_flags = opts.flags();
    for (i, r) in rows.iter().enumerate() {
        let g = group_of[i];
        let is_best = g.is_some_and(|g| best.get(&g).is_some_and(|b| b.0 == i));
        let prev = s.catalog.photo(r.id).and_then(|p| p.analysis).unwrap_or_default();
        let fs = faces.get(i).copied().flatten();
        let (n_faces, eyes_closed, face_focus, smiles, expression) = match fs {
            Some(f) => (f.faces, f.eyes_closed, f.focus, f.smiles, f.expression),
            None => (prev.faces, prev.eyes_closed, prev.face_focus, prev.smiles, prev.expression),
        };
        let analysis =
            Analysis { sharpness: r.sharp, clipped: r.clip, group: g, best: is_best, faces: n_faces, eyes_closed, face_focus, smiles, expression };
        ops.push(Op::SetAnalysis { id: r.id, analysis: Some(analysis) });
        let flag = s.catalog.photo(r.id).map(|p| p.flag).unwrap_or_default();
        let blink = opts.reject_eyes && eyes_closed > 0;
        let soft_face = opts.reject_face_below.is_some_and(|t| face_focus.is_some_and(|f| f < t));
        let frown = opts.reject_expression_below.is_some_and(|t| expression.is_some_and(|e| e < t));
        if set_flags && (opts.reject_below.is_some_and(|t| r.sharp < t) || blink || soft_face || frown) && flag != Flag::Reject {
            ops.push(Op::SetFlag { id: r.id, flag: Flag::Reject });
            rejected += 1;
        } else if set_flags && opts.pick_best && is_best && flag == Flag::None {
            ops.push(Op::SetFlag { id: r.id, flag: Flag::Pick });
            picked += 1;
        }
        out.push(json!({
            "id": r.id.0, "sharpness": (r.sharp * 10.0).round() / 10.0, "clipped": r.clip, "group": g, "best": is_best,
            "faces": n_faces, "eyesClosed": eyes_closed, "faceFocus": face_focus.map(|f| (f * 10.0).round() / 10.0),
            "smiles": smiles, "expression": expression.map(|e| (e * 10.0).round() / 10.0),
        }));
    }
    if !ops.is_empty() {
        if opts.bookkeeping {
            s.apply_bookkeeping(Op::Batch { ops })?;
        } else {
            s.commit("Assisted Culling", Op::Batch { ops })?;
        }
    }
    Ok(json!({"photos": out, "groups": best.len(), "rejected": rejected, "picked": picked, "failed": failed, "facesPending": faces_pending}))
}

/// What the faces index says about a photo, for culling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FaceScores {
    pub faces: u16,
    pub eyes_closed: u16,
    pub focus: Option<f32>,
    /// Main faces smiling (`None`-like 0 when expressions weren't read: see `expression`).
    pub smiles: u16,
    /// 0..100: how pleasant the main faces look (`None`: expressions not read, or no face big enough).
    pub expression: Option<f32>,
}

/// A face smiles: the expression model's `happy` at least even odds.
pub(crate) const SMILE: f32 = 0.5;

/// Both eyes of a face shut: neither clearly open and together more likely shut than not (a squint or a wink
/// isn't a blink).
pub(crate) fn blinking(e: [f32; 2]) -> bool {
    e[0].min(e[1]) >= 0.7 && (e[0] + e[1]) / 2.0 >= 0.85
}

/// Faces, blinks among the main faces (at least a quarter the size of the largest) and the largest face's focus.
pub(crate) fn scores_of(record: &lightcraft_vision::index::FaceRecord) -> FaceScores {
    let area = |f: &lightcraft_vision::index::FaceEntry| f.rect[2].max(0.0) * f.rect[3].max(0.0);
    let found: Vec<&lightcraft_vision::index::FaceEntry> = record.faces.iter().filter(|f| f.score > 0.0).collect();
    let largest = found.iter().map(|f| area(f)).fold(0f32, f32::max);
    let main = found.iter().filter(|f| area(f) >= largest * 0.25);
    let main: Vec<&&lightcraft_vision::index::FaceEntry> = main.collect();
    let eyes_closed = main.iter().filter(|f| f.eyes.is_some_and(blinking)).count();
    let focus = found.iter().max_by(|a, b| area(a).total_cmp(&area(b))).map(|f| f.sharpness).filter(|v| v.is_finite());
    let read: Vec<[f32; 7]> = main.iter().filter_map(|f| f.expression).collect();
    let smiles = read.iter().filter(|e| e[lightcraft_vision::expression::HAPPY] >= SMILE).count();
    let expression = (record.expressions && !read.is_empty()).then(|| {
        let pleasant: f32 =
            read.iter().map(|e| e[lightcraft_vision::expression::HAPPY] + 0.5 * e[lightcraft_vision::expression::NEUTRAL]).sum::<f32>();
        (100.0 * pleasant / read.len() as f32).clamp(0.0, 100.0)
    });
    FaceScores {
        faces: found.len().min(u16::MAX as usize) as u16,
        eyes_closed: eyes_closed.min(u16::MAX as usize) as u16,
        focus,
        smiles: smiles.min(u16::MAX as usize) as u16,
        expression: expression.filter(|v| v.is_finite()),
    }
}

fn face_scores(s: &mut Session, id: PhotoId) -> Option<FaceScores> {
    let key = crate::media::content_key(s.catalog.photo(id)?);
    let index = s.faces_index().ok()?;
    let index = index.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    index.get(&key).map(scores_of)
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
            "{ids?, rejectBelow?: sharpness 0..100, rejectEyesClosed?: bool, rejectFaceBelow?: face focus 0..100, rejectExpressionBelow?: expression 0..100, pickBest?: bool, faces?: bool (true), background?: bool, bookkeeping?: bool} — background: true measures on worker threads in the app (→ {queued}; the result lands when all are in), bookkeeping: true writes the scores without an undo step and never flags (culling at import); score the selected photos (else all in view) for focus (0..100) and clipping, group look-alike shots taken within 10 s and mark the best of each (nobody blinking, smiles and the sharpest face, sharpest photo); with the face models also the faces, blinks and main-face focus, and with the expression model smiles and an expression score (0..100: smiling high, neutral about 50, frowning low); photos not analysed yet are first (in the background in the app → facesPending); optionally reject blurry photos, blinks, soft faces or poor expressions and pick each group's best; one undo step → {photos: [{id, sharpness, clipped, group, best, faces, eyesClosed, faceFocus, smiles, expression}], groups, rejected, picked, facesPending}",
            always,
            analyze
        ),
    ]
}
