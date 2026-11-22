//! Edit Capture Time's plan (LR-LIB-CAPTURETIME): which photos move, by how much, from what to
//! what. `photo.setCaptureTime` commits it; the dialog shows it before anything changes.
//!
//! - **Anchor**: the photo whose corrected time is typed (the one right-clicked, else the active
//!   photo). The other photos keep their distance to it: they all move by the same amount.
//! - **Followers**: a virtual copy is the same picture, so a master and its copies always move
//!   together; the photos hidden in a collapsed stack follow its top.
//! - **No capture time**: photos without one are left alone when the others shift (moving an
//!   import time would invent a capture time); only the anchor itself gets a typed time, or every
//!   photo with `each`.
//! - **Match camera clocks**: a reference photo from another camera taken at the same moment as
//!   the anchor — the anchor's camera's photos in the selection move by the difference.

use std::collections::{HashMap, HashSet};

use lightcraft_catalog::PhotoId;
use lightcraft_catalog::dates::{iso_seconds, normalize_iso, parse_time_input, shift_iso};
use serde_json::{Value, json};

use crate::cmd::bad;
use crate::{Result, Session};

const CMD: &str = "photo.setCaptureTime";

/// One photo's change.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: PhotoId,
    /// Its capture time now (`None`: it has none).
    pub from: Option<String>,
    pub to: String,
    /// Not selected itself: a virtual copy / original of a selected photo, or in a collapsed stack.
    pub follows: bool,
}

/// What a `photo.setCaptureTime` request would do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub anchor: Option<PhotoId>,
    /// The anchor's time before (its capture time, else its import time).
    pub anchor_from: String,
    /// The amount every photo moves (`None` with `each`: they all get one time).
    pub delta: Option<i64>,
    /// In capture-time order (before the change).
    pub rows: Vec<Row>,
    pub skipped: Vec<(PhotoId, &'static str)>,
    /// Rows that follow: virtual copies / originals, photos in collapsed stacks.
    pub copies: usize,
    pub stacked: usize,
    /// Match camera clocks: the reference photo and the camera that moves.
    pub reference: Option<PhotoId>,
    pub camera: Option<String>,
}

impl Plan {
    /// Rows whose time changes.
    pub fn changed(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| r.from.as_deref() != Some(r.to.as_str()))
    }

    pub fn to_json(&self) -> Value {
        json!({
            "anchor": self.anchor.map(|a| a.0),
            "anchorFrom": self.anchor_from,
            "delta": self.delta,
            "photos": self.rows.iter().map(|r| json!({"id": r.id.0, "from": r.from, "to": r.to, "follows": r.follows})).collect::<Vec<_>>(),
            "skipped": self.skipped.iter().map(|(id, why)| json!({"id": id.0, "reason": why})).collect::<Vec<_>>(),
            "copies": self.copies,
            "stacked": self.stacked,
            "reference": self.reference.map(|r| r.0),
            "camera": self.camera,
        })
    }
}

/// Why a photo that isn't selected moves along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    /// A virtual copy of a selected photo, or the original of a selected copy.
    Copy,
    /// Hidden in a collapsed stack whose top is selected.
    Stack,
}

/// The anchor for request `p` among `targets`: `anchor` if it is one of them, else the active
/// photo if it is, else the first.
pub fn anchor_of(s: &Session, p: &Value, targets: &[PhotoId]) -> Result<PhotoId> {
    if let Some(a) = p.get("anchor").and_then(Value::as_u64).map(PhotoId) {
        return if targets.contains(&a) { Ok(a) } else { Err(bad(CMD, "`anchor` must be one of the photos being changed")) };
    }
    let first = targets.first().copied().ok_or_else(|| bad(CMD, "no photos"))?;
    Ok(s.active().filter(|a| targets.contains(a)).unwrap_or(first))
}

/// The photos a request changes: `ids` / the selection, plus the virtual copies and originals
/// of those photos and the hidden members of collapsed stacks whose top is among them. Each id
/// once; followers say why they follow.
pub fn with_followers(s: &Session, targets: &[PhotoId]) -> Vec<(PhotoId, Option<Follow>)> {
    let cat = &s.catalog;
    let mut seen: HashSet<PhotoId> = HashSet::new();
    let mut out: Vec<(PhotoId, Option<Follow>)> = Vec::new();
    for id in targets {
        if cat.photo(*id).is_some() && seen.insert(*id) {
            out.push((*id, None));
        }
    }
    // master → master + its copies
    let mut family: HashMap<PhotoId, Vec<PhotoId>> = HashMap::new();
    for p in cat.photos().filter(|p| !p.deleted) {
        if let Some(m) = p.copy_of {
            family.entry(m).or_insert_with(|| vec![m]).push(p.id);
        }
    }
    let stacks = cat.stack_index();
    let mut i = 0;
    // (a follower can bring its own: a copy at the top of a collapsed stack)
    while let Some(&(id, _)) = out.get(i) {
        i += 1;
        let Some(p) = cat.photo(id) else { continue };
        let master = p.copy_of.unwrap_or(id);
        let mut more: Vec<(PhotoId, Follow)> = family.get(&master).into_iter().flatten().map(|m| (*m, Follow::Copy)).collect();
        if let Some((sid, 0)) = stacks.get(&id)
            && let Some(st) = cat.stack(*sid).filter(|st| st.collapsed)
        {
            more.extend(st.photos.iter().map(|m| (*m, Follow::Stack)));
        }
        for (m, why) in more {
            if cat.photo(m).is_some_and(|p| !p.deleted) && seen.insert(m) {
                out.push((m, Some(why)));
            }
        }
    }
    out
}

/// Plan request `p` (`ids?`, `anchor?`, `time?`, `each?`, `shift?`, `hours?`, `match?`; see
/// `photo.setCaptureTime`).
pub fn plan(s: &Session, p: &Value) -> Result<Plan> {
    let cat = &s.catalog;
    let mut targets: Vec<PhotoId> = s.targets(p).into_iter().filter(|id| cat.photo(*id).is_some()).collect();
    if targets.is_empty() {
        return Err(bad(CMD, "no photos"));
    }
    let anchor = anchor_of(s, p, &targets)?;
    let ap = cat.photo(anchor).ok_or_else(|| bad(CMD, "no photos"))?;
    let anchor_from = ap.date().to_string();
    let from = iso_seconds(&anchor_from).ok_or_else(|| bad(CMD, "the photo's date doesn't parse"))?;
    let mut plan = Plan { anchor: Some(anchor), anchor_from: anchor_from.clone(), ..Default::default() };
    let mut delta = (f64_param(p, "shift") + f64_param(p, "hours") * 3600.0).round() as i64;
    let each = p.get("each").and_then(Value::as_bool).unwrap_or(false);
    let mut each_time = None;
    // match camera clocks: the reference's time for the anchor, the anchor's camera moves
    if let Some(r) = p.get("match").and_then(Value::as_u64).map(PhotoId) {
        let rp = cat.photo(r).ok_or_else(|| bad(CMD, format!("no photo {}", r.0)))?;
        let camera = ap.meta.camera.trim().to_string();
        if camera.is_empty() {
            return Err(bad(CMD, "the photo records no camera, so there is no camera clock to match"));
        }
        if rp.meta.camera.trim() == camera {
            return Err(bad(CMD, format!("the reference photo is from the same camera ({camera}): pick one from the other camera")));
        }
        let to = iso_seconds(rp.date()).ok_or_else(|| bad(CMD, "the reference photo's date doesn't parse"))?;
        delta += to - from;
        targets.retain(|id| cat.photo(*id).is_some_and(|p| p.meta.camera.trim() == camera));
        plan.reference = Some(r);
        plan.camera = Some(camera);
    } else if let Some(t) = p.get("time").and_then(Value::as_str) {
        let t = parse_time_input(t, &anchor_from)
            .or_else(|| normalize_iso(t))
            .ok_or_else(|| bad(CMD, format!("`{t}` is not a time (2026-09-30 14:05:00, 3:34 pm, 15:34, +1h, -30m, +1d)")))?;
        if each {
            each_time = Some(t);
        } else {
            delta += iso_seconds(&t).unwrap_or(from) - from;
        }
    }
    plan.delta = each_time.is_none().then_some(delta);
    let anchor_master = ap.copy_of.unwrap_or(anchor);
    let set_anchor = p.get("time").is_some() || plan.reference.is_some();
    let mut rows = Vec::new();
    for (id, follows) in with_followers(s, &targets) {
        let Some(ph) = cat.photo(id) else { continue };
        if plan.camera.as_deref().is_some_and(|c| ph.meta.camera.trim() != c) {
            continue;
        }
        let anchor_like = ph.copy_of.unwrap_or(id) == anchor_master;
        let to = match &each_time {
            Some(t) => Some(t.clone()),
            // without a capture time there is nothing to shift, except the anchor being set
            None if ph.captured.is_none() && !(anchor_like && set_anchor) => {
                plan.skipped.push((id, "no capture time"));
                continue;
            }
            None => shift_iso(ph.date(), delta),
        };
        let Some(to) = to else {
            plan.skipped.push((id, "its date doesn't parse"));
            continue;
        };
        match follows {
            Some(Follow::Copy) => plan.copies += 1,
            Some(Follow::Stack) => plan.stacked += 1,
            None => {}
        }
        rows.push((iso_seconds(ph.date()).unwrap_or(i64::MIN), Row { id, from: ph.captured.clone(), to, follows: follows.is_some() }));
    }
    rows.sort_by_key(|(t, _)| *t);
    plan.rows = rows.into_iter().map(|(_, r)| r).collect();
    Ok(plan)
}

fn f64_param(p: &Value, k: &str) -> f64 {
    p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0)
}
