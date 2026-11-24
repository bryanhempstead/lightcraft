//! merges. (Bryan, 2026-11-23): photo merges found in the library — brackets for HDR, panoramas,
//! focus stacks, HDR panoramas — kept as groups he reviews, then merges or skips. Nothing is
//! merged without him.
//!
//! * **Finding** ([`plan`] + [`decide`]): in each folder, photos in capture order; frames from one
//!   camera, lens and focal length taken close together form a run. Frames of a run whose exposure
//!   steps (shutter or exposure compensation, at one aperture and ISO) are brackets (`hdr.`).
//!   Frames at one exposure are looked at as small grey thumbnails ([`Thumb`]): overlapping but
//!   shifted framing → `pano.`; one framing whose sharp area moves (or a recorded focus distance
//!   that steps) → `stack.`; brackets repeated at shifted framings → `hdr.pano.`. A burst (same
//!   framing, same focus) is nothing.
//! * **Groups** are library records (`merge.group.<id>`, journaled with the catalog): kind,
//!   frames, state (open / merged / dismissed) and the merged photo. A group that shares a frame
//!   with one found before — a skipped one included — is never suggested again.
//! * **After a merge** ([`Session::merge_group_done`]) the frames are hidden from every view
//!   (`Photo.merge.into`; never deleted, files untouched; `merge.unmerge` brings them back), the
//!   merged photo goes into every collection they were in, and it shows in merges. and in its
//!   folder (or in merges. only: `merge.showIn`). Its thumbnail carries the kind (`pano.` …).
//!   It is one catalog record, so an edit shows wherever it is.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lightcraft_catalog::{AlbumId, Catalog, MediaKind, MergeMark, MergeType, Op, Photo, PhotoId};
use lightcraft_raster::Rgb32f;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// Library records holding the groups: `merge.group.<id>`.
pub const GROUP_PREFIX: &str = "merge.group.";
/// The library record of what the automatic finder has looked at: folder → fingerprint.
pub const SCANNED: &str = "merge.scanned";

/// Where a group is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupState {
    /// Found, waiting for him: merge. or skip.
    #[default]
    Open,
    /// Merged: its frames are hidden, [`MergeGroup::result`] is the merged photo.
    Merged,
    /// Skipped: never suggested again.
    Dismissed,
}

impl GroupState {
    pub fn id(self) -> &'static str {
        match self {
            GroupState::Open => "open",
            GroupState::Merged => "merged",
            GroupState::Dismissed => "dismissed",
        }
    }
}

/// How sure the finder is of a group: `sure.`, `likely.`, `maybe.` (a borderline panorama is
/// `maybe.` rather than missed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    Maybe,
    #[default]
    Likely,
    Sure,
}

impl Confidence {
    /// `sure`, `likely`, `maybe` (commands, records).
    pub fn id(self) -> &'static str {
        match self {
            Confidence::Sure => "sure",
            Confidence::Likely => "likely",
            Confidence::Maybe => "maybe",
        }
    }
    /// The chip: `sure.`, `likely.`, `maybe.`.
    pub fn tag(self) -> &'static str {
        match self {
            Confidence::Sure => "sure.",
            Confidence::Likely => "likely.",
            Confidence::Maybe => "maybe.",
        }
    }
}

/// One found merge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeGroup {
    /// [`group_id`] of the frames it was found with (stable: the same frames, the same id).
    pub id: String,
    pub kind: MergeType,
    /// Frames in the group (what merge. merges), in capture order.
    pub photos: Vec<PhotoId>,
    #[serde(default)]
    pub state: GroupState,
    /// The merged photo (state merged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<PhotoId>,
    /// The folder of its frames ([`lightcraft_catalog::local::folder_of`]).
    #[serde(default)]
    pub folder: String,
    /// When it was found (ISO 8601).
    #[serde(default)]
    pub found: String,
    /// Every frame it was found with, in capture order, those taken out of it (`merge.setFrames`)
    /// too; empty: [`MergeGroup::photos`] (groups stored before 2026-11-24).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub found_frames: Vec<PhotoId>,
    /// How sure the finder was (`None`: found before it said, read as likely).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Why, in plain words ("5 frames in 2.1 s", "each frame overlaps the last by ~40 %").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

impl MergeGroup {
    pub fn key(&self) -> String {
        format!("{GROUP_PREFIX}{}", self.id)
    }
    /// The op that stores it.
    pub fn op(&self) -> Op {
        Op::SetRecord { key: self.key(), value: serde_json::to_value(self).ok() }
    }
    /// Every frame it was found with (in the group or taken out of it), in capture order.
    pub fn all_frames(&self) -> &[PhotoId] {
        if self.found_frames.is_empty() { &self.photos } else { &self.found_frames }
    }
    /// How sure the finder was (likely for groups found before it said).
    pub fn confidence(&self) -> Confidence {
        self.confidence.unwrap_or_default()
    }
    /// Its frames that are shown: still in the library, not folded into a merge.
    pub fn live_frames(&self, cat: &Catalog) -> Vec<PhotoId> {
        self.photos.iter().copied().filter(|id| cat.photo(*id).is_some_and(|p| frame_shown(p))).collect()
    }
    /// [`MergeGroup::live_frames`] with the frames taken out of it.
    pub fn live_all(&self, cat: &Catalog) -> Vec<PhotoId> {
        self.all_frames().iter().copied().filter(|id| cat.photo(*id).is_some_and(|p| frame_shown(p))).collect()
    }
    /// The merged photo, while it is in the library (not deleted).
    pub fn live_result(&self, cat: &Catalog) -> Option<PhotoId> {
        self.result.filter(|r| cat.photo(*r).is_some_and(|p| !p.deleted && !p.local))
    }
    /// Shown in merges.: open with two frames or more left (taken out ones count: they can be
    /// put back), or merged with its photo there.
    pub fn is_live(&self, cat: &Catalog) -> bool {
        match self.state {
            GroupState::Open => self.live_all(cat).len() >= 2,
            GroupState::Merged => self.live_result(cat).is_some(),
            GroupState::Dismissed => false,
        }
    }
    /// What `merge.find` says of it (no catalog needed); `merge.candidates` adds the frames
    /// ([`MergeGroup::to_json_full`]).
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind.id(),
            "tag": self.kind.tag(),
            "photos": self.photos.iter().map(|p| p.0).collect::<Vec<_>>(),
            "state": self.state.id(),
            "result": self.result.map(|r| r.0),
            "folder": self.folder,
            "found": self.found,
            "confidence": self.confidence().id(),
            "reasons": self.reasons,
        })
    }
    /// [`MergeGroup::to_json`] with its frames (`frames`: every frame found with it, `inGroup`
    /// false for those taken out), the cameras and subfolders they came from (relative to
    /// `root`, the folder of the merges. pool; `None`: the folder's own name), and reasons worked
    /// out from the frames' metadata for groups stored before the finder gave any.
    pub fn to_json_full(&self, cat: &Catalog, root: Option<&str>) -> Value {
        let mut v = self.to_json();
        let frames: Vec<Value> = self.all_frames().iter().map(|id| frame_json(cat, *id, self.photos.contains(id), root)).collect();
        let mut cameras: Vec<String> = Vec::new();
        let mut subfolders: Vec<String> = Vec::new();
        for f in &frames {
            if let Some(c) = f.get("camera").and_then(Value::as_str).filter(|c| !c.is_empty())
                && !cameras.iter().any(|x| x == c)
            {
                cameras.push(c.to_string());
            }
            if let Some(s) = f.get("subfolder").and_then(Value::as_str)
                && !subfolders.iter().any(|x| x == s)
            {
                subfolders.push(s.to_string());
            }
        }
        if self.reasons.is_empty() {
            v["reasons"] = json!(describe(cat, self));
        }
        v["frames"] = json!(frames);
        v["cameras"] = json!(cameras);
        v["subfolders"] = json!(subfolders);
        v
    }
}

/// A frame as `merge.candidates` lists it: `{id, file, folder, subfolder, camera, captured,
/// exposure, inGroup}`.
pub fn frame_json(cat: &Catalog, id: PhotoId, in_group: bool, root: Option<&str>) -> Value {
    let Some(p) = cat.photo(id) else {
        return json!({"id": id.0, "file": "", "folder": "", "subfolder": "", "camera": "", "captured": "", "exposure": "", "inGroup": in_group});
    };
    let file = match &p.source {
        lightcraft_catalog::Source::File { path } => path.clone(),
        _ => String::new(),
    };
    let folder = lightcraft_catalog::local::folder_of(p).unwrap_or_default();
    json!({
        "id": id.0,
        "file": file,
        "folder": folder,
        "subfolder": subfolder(&folder, root),
        "camera": p.meta.camera.trim(),
        "captured": p.captured.clone().unwrap_or_default(),
        "exposure": exposure_text(&p.meta),
        "inGroup": in_group,
    })
}

/// `folder` relative to the pool's folder `root` (`""`: the folder itself); without a root, the
/// folder's own name.
pub fn subfolder(folder: &str, root: Option<&str>) -> String {
    match root {
        Some(root) => {
            let (f, r) = (lightcraft_catalog::query::folder_key(folder), lightcraft_catalog::query::folder_key(root));
            let (f, r) = (f.trim_end_matches('/'), r.trim_end_matches('/'));
            if f == r {
                String::new()
            } else {
                f.strip_prefix(r).and_then(|rest| rest.strip_prefix('/')).map(str::to_string).unwrap_or_else(|| folder_name(f))
            }
        }
        None => folder_name(folder),
    }
}

fn folder_name(folder: &str) -> String {
    folder.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
}

/// A frame's exposure the way a camera writes it: `1/250 f/8 ISO 100`.
pub fn exposure_text(m: &lightcraft_catalog::Meta) -> String {
    let mut parts: Vec<String> = Vec::new();
    let sh = m.shutter.trim();
    if !sh.is_empty() {
        parts.push(if sh.contains('/') || sh.ends_with('s') || sh.ends_with('"') { sh.to_string() } else { format!("{sh} s") });
    }
    if let Some(a) = m.aperture.filter(|a| a.is_finite() && *a > 0.0) {
        parts.push(format!("f/{}", trim_num(f64::from(a))));
    }
    if let Some(i) = m.iso {
        parts.push(format!("ISO {i}"));
    }
    parts.join(" ")
}

/// `8`, `5.6`, `0.7`: one decimal at most, without a trailing `.0`.
fn trim_num(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// Exposure steps in stops as a person says them: `−2`, `0`, `+0.7`.
fn ev_text(v: f64) -> String {
    if v.abs() < 0.05 {
        "0".into()
    } else if v < 0.0 {
        format!("\u{2212}{}", trim_num(-v))
    } else {
        format!("+{}", trim_num(v))
    }
}

/// Reasons for a group from its frames' metadata alone (groups stored before the finder gave
/// any): how many frames in how long, and a bracket's exposures.
pub fn describe(cat: &Catalog, g: &MergeGroup) -> Vec<String> {
    let shots: Vec<Shot> = g.photos.iter().filter_map(|id| cat.photo(*id)).filter_map(|p| shot_any(p)).collect();
    let mut out = Vec::new();
    if shots.len() >= 2 {
        out.push(span_text(&shots));
    } else {
        out.push(format!("{} frame{}", g.photos.len(), if g.photos.len() == 1 { "" } else { "s" }));
    }
    if matches!(g.kind, MergeType::Hdr) && shots.len() >= 2 {
        out.push(exposures_text(&shots));
    }
    out
}

fn frame_shown(p: &Photo) -> bool {
    !p.deleted && !p.local && p.merge.and_then(|m| m.into).is_none()
}

/// A group's id from its frames: FNV-1a of the sorted ids, 16 hex digits.
pub fn group_id(photos: &[PhotoId]) -> String {
    let mut v: Vec<u64> = photos.iter().map(|p| p.0).collect();
    v.sort_unstable();
    v.dedup();
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    format!("{:016x}", lightcraft_catalog::fnv64(&bytes))
}

/// Every group the library holds (records that don't read are left out).
pub fn groups(cat: &Catalog) -> Vec<MergeGroup> {
    cat.records_with_prefix(GROUP_PREFIX).filter_map(|(_, v)| serde_json::from_value::<MergeGroup>(v.clone()).ok()).collect()
}

/// One group by id.
pub fn group(cat: &Catalog, id: &str) -> Option<MergeGroup> {
    cat.record(&format!("{GROUP_PREFIX}{}", id.trim())).and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// The group a merged photo came from.
pub fn group_of_result(cat: &Catalog, result: PhotoId) -> Option<MergeGroup> {
    groups(cat).into_iter().find(|g| g.state == GroupState::Merged && g.result == Some(result))
}

// ------------------------------------------------------------------------------------- scope

/// What a merges. view shows: the groups of a folder (its own, or with the folders inside it),
/// of a collection, or all of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Scope {
    #[default]
    All,
    Folder {
        path: String,
        #[serde(default)]
        deep: bool,
    },
    Album {
        id: AlbumId,
    },
}

impl Scope {
    /// From command parameters: `{scope: {kind, …}}`, or `{path, deep?}`, `{album}`, else all.
    pub fn from_params(p: &Value) -> std::result::Result<Scope, String> {
        if let Some(s) = p.get("scope").filter(|s| !s.is_null()) {
            if let Some(path) = s.as_str() {
                return Ok(if path == "all" { Scope::All } else { Scope::Folder { path: path.to_string(), deep: false } });
            }
            return serde_json::from_value(s.clone()).map_err(|e| format!("scope: {e}"));
        }
        if let Some(path) = p.get("path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            return Ok(Scope::Folder { path: path.to_string(), deep: p.get("deep").and_then(Value::as_bool).unwrap_or(false) });
        }
        if let Some(a) = p.get("album").and_then(Value::as_u64) {
            return Ok(Scope::Album { id: AlbumId(a) });
        }
        Ok(Scope::All)
    }

    /// Whether the group belongs to this scope.
    pub fn holds(&self, cat: &Catalog, g: &MergeGroup) -> bool {
        match self {
            Scope::All => true,
            Scope::Folder { path, deep } => {
                let root = lightcraft_catalog::query::folder_key(path);
                if root.is_empty() || g.folder.is_empty() {
                    return false;
                }
                if *deep {
                    lightcraft_catalog::query::folder_within(&g.folder, &root)
                } else {
                    g.folder.trim_end_matches('/') == root.trim_end_matches('/')
                }
            }
            Scope::Album { id } => {
                let members = || g.photos.iter().copied().chain(g.result);
                match cat.album(*id) {
                    Some(a) if a.folder => false,
                    Some(a) if a.is_smart() => members().any(|m| cat.photo(m).is_some_and(|p| cat.album_contains(*id, p))),
                    Some(a) => members().any(|m| a.photos.contains(&m)),
                    None => false,
                }
            }
        }
    }

    /// The photos the finder looks at for this scope (library photos).
    fn photos(&self, cat: &Catalog) -> Vec<PhotoId> {
        match self {
            Scope::All => cat.photos().filter(|p| p.in_library()).map(|p| p.id).collect(),
            Scope::Folder { path, deep } => {
                let f = lightcraft_catalog::Filter { library_folder: Some(path.clone()), library_folder_only: !deep, ..Default::default() };
                cat.query(&f, &lightcraft_catalog::Sort::default())
            }
            Scope::Album { id } => cat.album_photos(*id).into_iter().filter(|i| cat.photo(*i).is_some_and(|p| p.in_library())).collect(),
        }
    }

    /// The pool's folder (frames' subfolders are named relative to it); `None` for a collection
    /// or the whole library.
    pub fn root(&self) -> Option<&str> {
        match self {
            Scope::Folder { path, .. } => Some(path.as_str()),
            _ => None,
        }
    }

    /// What the view is called after its folder or collection: `Erika › merges.`.
    pub fn title(&self, cat: &Catalog) -> String {
        match self {
            Scope::All => "merges.".into(),
            Scope::Folder { path, .. } => format!("{} › merges.", lightcraft_catalog::folders::folder_label(path)),
            Scope::Album { id } => format!("{} › merges.", cat.album(*id).map(|a| a.name.as_str()).unwrap_or("Collection")),
        }
    }
}

/// One block of a merges. view: an open group (its frames), or the merged photos.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewRun {
    /// `merges:group:<id>` or `merges:merged`.
    pub key: String,
    /// What its header says: the kind's tag (`pano.`) or `merged.`.
    pub label: String,
    /// The open group it shows.
    pub group: Option<String>,
    pub kind: Option<MergeType>,
    pub ids: Vec<PhotoId>,
}

/// The merges. view of `scope`: each open group's frames (oldest group first; the frames taken
/// out of it too), then the merged photos.
pub fn view(cat: &Catalog, scope: &Scope) -> Vec<ViewRun> {
    let date = |id: PhotoId| cat.photo(id).map(|p| p.date().to_string()).unwrap_or_default();
    let mut open: Vec<(String, ViewRun)> = Vec::new();
    let mut merged: Vec<(String, PhotoId)> = Vec::new();
    let mut seen: HashSet<PhotoId> = HashSet::new();
    for g in groups(cat) {
        if g.state == GroupState::Dismissed || !scope.holds(cat, &g) {
            continue;
        }
        match g.state {
            GroupState::Open => {
                // frames taken out of it too: they show (dimmed) so they can be put back
                let ids: Vec<PhotoId> = g.live_all(cat).into_iter().filter(|i| !seen.contains(i)).collect();
                if ids.len() < 2 {
                    continue;
                }
                seen.extend(ids.iter().copied());
                let first = ids.first().copied().map(date).unwrap_or_default();
                open.push((
                    first,
                    ViewRun {
                        key: format!("merges:group:{}", g.id),
                        label: g.kind.tag().to_string(),
                        group: Some(g.id.clone()),
                        kind: Some(g.kind),
                        ids,
                    },
                ));
            }
            GroupState::Merged => {
                if let Some(r) = g.live_result(cat).filter(|r| seen.insert(*r)) {
                    merged.push((date(r), r));
                }
            }
            GroupState::Dismissed => {}
        }
    }
    open.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.key.cmp(&b.1.key)));
    merged.sort();
    let mut out: Vec<ViewRun> = open.into_iter().map(|(_, r)| r).collect();
    if !merged.is_empty() {
        out.push(ViewRun {
            key: "merges:merged".into(),
            label: "merged.".into(),
            group: None,
            kind: None,
            ids: merged.into_iter().map(|(_, r)| r).collect(),
        });
    }
    out
}

/// The photos of [`view`], in order.
pub fn view_order(cat: &Catalog, scope: &Scope) -> Vec<PhotoId> {
    view(cat, scope).into_iter().flat_map(|r| r.ids).collect()
}

/// The groups of `scope` sent back (`merge.dismiss`) whose frames are still in the library, the
/// oldest first: the merges. view's `sent back.` section.
pub fn sent_back(cat: &Catalog, scope: &Scope) -> Vec<MergeGroup> {
    let date = |g: &MergeGroup| g.all_frames().first().and_then(|id| cat.photo(*id)).map(|p| p.date().to_string()).unwrap_or_default();
    let mut v: Vec<(String, MergeGroup)> = groups(cat)
        .into_iter()
        .filter(|g| g.state == GroupState::Dismissed && scope.holds(cat, g) && g.live_all(cat).len() >= 2)
        .map(|g| (date(&g), g))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
    v.into_iter().map(|(_, g)| g).collect()
}

/// How many groups each folder and collection shows (the sidebar's `merges.` rows).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    /// Folder (as [`lightcraft_catalog::local::folder_of`] names it) → live groups whose frames are in it.
    pub by_folder: BTreeMap<String, usize>,
    pub by_album: HashMap<AlbumId, usize>,
}

impl Counts {
    /// Groups of the folder at `path` (`deep`: with the folders inside it).
    pub fn folder(&self, path: &str, deep: bool) -> usize {
        let root = lightcraft_catalog::query::folder_key(path);
        if root.is_empty() {
            return 0;
        }
        if deep {
            self.by_folder.iter().filter(|(k, _)| lightcraft_catalog::query::folder_within(k, &root)).map(|(_, n)| *n).sum()
        } else {
            self.by_folder.iter().filter(|(k, _)| k.trim_end_matches('/') == root.trim_end_matches('/')).map(|(_, n)| *n).sum()
        }
    }
    pub fn album(&self, id: AlbumId) -> usize {
        self.by_album.get(&id).copied().unwrap_or(0)
    }
}

/// [`Counts`] of the library: every live group (open with two frames or more, or merged with its photo).
pub fn counts(cat: &Catalog) -> Counts {
    let live: Vec<MergeGroup> = groups(cat).into_iter().filter(|g| g.is_live(cat)).collect();
    let mut out = Counts::default();
    let mut of_photo: HashMap<PhotoId, Vec<usize>> = HashMap::new();
    for (i, g) in live.iter().enumerate() {
        *out.by_folder.entry(g.folder.clone()).or_default() += 1;
        for m in g.photos.iter().copied().chain(g.result) {
            of_photo.entry(m).or_default().push(i);
        }
    }
    if live.is_empty() {
        return out;
    }
    for a in cat.albums().filter(|a| !a.folder) {
        let n = if a.is_smart() {
            live.iter().filter(|g| Scope::Album { id: a.id }.holds(cat, g)).count()
        } else {
            let hit: HashSet<usize> = a.photos.iter().filter_map(|p| of_photo.get(p)).flatten().copied().collect();
            hit.len()
        };
        if n > 0 {
            out.by_album.insert(a.id, n);
        }
    }
    out
}

// ------------------------------------------------------------------------------------- finding
//
// Three steps (Bryan, 2026-11-24: "a multi-step review process"):
//
// 1. **Time** ([`plan`]): per folder, frames from one body, lens and focal length fired close
//    together form a run. Exposure never splits a run: a handheld panorama in aperture priority
//    drifts a little from frame to frame.
// 2. **Similarity** ([`decide`]): every run's frames are compared as small grey thumbnails
//    ([`link`]): the same framing, or overlapping after a shift (panorama neighbours), or
//    unrelated. Frames of one framing make a *position*: a bracket when their exposures step, a
//    focus stack when the sharp area moves, else a burst. Positions linked by shifts are a
//    panorama — an HDR panorama when they are brackets. So a "bracket" whose frames are shifted
//    is a panorama, and a burst is nothing.
// 3. **Classify + score**: each group gets a [`Confidence`] and its reasons in plain words.
//
// Metadata alone is enough for two things: a lone bracket the camera stepped (exposure
// compensation) and one framing focused at three distances or more the camera recorded.

/// What the finder reads of a photo.
#[derive(Clone, Debug, PartialEq)]
struct Shot {
    id: PhotoId,
    /// Capture time, seconds (with the fraction when the camera recorded one).
    t: f64,
    folder: String,
    camera: String,
    lens: String,
    focal: Option<f32>,
    aperture: Option<f32>,
    iso: Option<u32>,
    /// Shutter speed, seconds.
    shutter: Option<f64>,
    bias: Option<f32>,
    dist: Option<f32>,
}

impl Shot {
    /// How bright the frame was exposed, in stops (longer shutter, more compensation: higher).
    fn level(&self) -> f64 {
        self.shutter.map(f64::log2).unwrap_or(0.0) + f64::from(self.bias.unwrap_or(0.0))
    }
    /// One body, lens and focal length.
    fn same_rig(&self, o: &Shot) -> bool {
        self.folder == o.folder
            && self.camera.eq_ignore_ascii_case(&o.camera)
            && self.lens.eq_ignore_ascii_case(&o.lens)
            && close_opt(self.focal, o.focal, 0.5)
    }
    /// One aperture and ISO (what a bracket keeps).
    fn same_base(&self, o: &Shot) -> bool {
        close_opt(self.aperture, o.aperture, 0.05) && self.iso == o.iso
    }
    /// The same exposure as `o`.
    fn same_exposure(&self, o: &Shot) -> bool {
        self.same_base(o) && (self.level() - o.level()).abs() < 0.25
    }
}

fn close_opt(a: Option<f32>, b: Option<f32>, tol: f32) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= tol.max(a.abs().max(b.abs()) * 0.01),
        (None, None) => true,
        _ => false,
    }
}

/// Seconds since 1970 of an ISO 8601 capture time, with its fraction (`2026-04-01T10:00:00.25`).
pub fn capture_secs(iso: &str) -> Option<f64> {
    let base = lightcraft_catalog::dates::iso_seconds(iso)? as f64;
    let frac = iso.get(19..).filter(|r| r.starts_with('.')).map(|r| {
        let digits: String = r.chars().skip(1).take_while(char::is_ascii_digit).take(6).collect();
        if digits.is_empty() { 0.0 } else { digits.parse::<f64>().unwrap_or(0.0) / 10f64.powi(digits.len() as i32) }
    });
    Some(base + frac.unwrap_or(0.0))
}

/// A shutter speed as the catalog keeps it (`1/250`, `2`, `0.5`, `30"`) in seconds.
pub fn shutter_secs(s: &str) -> Option<f64> {
    let s = s.trim().trim_end_matches(['s', '"']).trim();
    let v = match s.split_once('/') {
        Some((a, b)) => {
            let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
            if b == 0.0 {
                return None;
            }
            a / b
        }
        None => s.parse::<f64>().ok()?,
    };
    (v.is_finite() && v > 0.0 && v < 100_000.0).then_some(v)
}

/// A photo the finder may look at (in the library, a still, not a copy or a merge itself).
fn shot(p: &Photo) -> Option<Shot> {
    if !p.in_library() || p.kind == MediaKind::Video || p.copy_of.is_some() || p.merge_type().is_some() {
        return None;
    }
    shot_any(p)
}

/// What the metadata says of any photo with a capture time (a frame hidden by a merge too).
fn shot_any(p: &Photo) -> Option<Shot> {
    let t = capture_secs(p.captured.as_deref()?)?;
    let m = &p.meta;
    Some(Shot {
        id: p.id,
        t,
        folder: lightcraft_catalog::local::folder_of(p)?,
        camera: m.camera.trim().to_string(),
        lens: m.lens.trim().to_string(),
        focal: m.focal_mm.filter(|f| f.is_finite() && *f > 0.0),
        aperture: m.aperture.filter(|f| f.is_finite() && *f > 0.0),
        iso: m.iso,
        shutter: shutter_secs(&m.shutter),
        bias: m.exposure_bias.filter(|b| b.is_finite()),
        dist: m.focus_distance.filter(|d| d.is_finite() && *d > 0.0),
    })
}

/// "5 frames in 2.1 s".
fn span_text(shots: &[Shot]) -> String {
    let lo = shots.iter().map(|s| s.t).fold(f64::MAX, f64::min);
    let hi = shots.iter().map(|s| s.t).fold(f64::MIN, f64::max);
    let span = if hi >= lo { hi - lo } else { 0.0 };
    format!("{} frames in {} s", shots.len(), trim_num(span))
}

/// "exposures −2 / 0 / +2" in capture order: the compensation the camera stepped, else the
/// exposure (shutter) relative to the middle frame.
fn exposures_text(shots: &[Shot]) -> String {
    let biases: Vec<f64> = shots.iter().filter_map(|s| s.bias).map(f64::from).collect();
    let stepped = biases.len() == shots.len() && spread(&biases) >= 0.3;
    let evs: Vec<f64> = if stepped {
        biases
    } else {
        let mut l: Vec<f64> = shots.iter().map(Shot::level).collect();
        l.sort_by(f64::total_cmp);
        let mid = l.get(l.len() / 2).copied().unwrap_or(0.0);
        shots.iter().map(|s| s.level() - mid).collect()
    };
    format!("exposures {}", evs.iter().map(|v| ev_text((v * 3.0).round() / 3.0)).collect::<Vec<_>>().join(" / "))
}

fn spread(v: &[f64]) -> f64 {
    let hi = v.iter().copied().fold(f64::MIN, f64::max);
    let lo = v.iter().copied().fold(f64::MAX, f64::min);
    if hi >= lo { hi - lo } else { 0.0 }
}

/// A found merge, before it is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub kind: MergeType,
    pub photos: Vec<PhotoId>,
    pub folder: String,
    pub confidence: Confidence,
    pub reasons: Vec<String>,
}

/// A run of frames whose kind only their pixels tell (step 2).
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    folder: String,
    shots: Vec<Shot>,
}

impl Look {
    /// The photos whose thumbnails [`decide`] needs: every frame.
    pub fn photos(&self) -> Vec<PhotoId> {
        self.shots.iter().map(|s| s.id).collect()
    }
}

/// What metadata alone says about some photos: merges it is sure of, and runs to look at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub sure: Vec<Candidate>,
    pub looks: Vec<Look>,
}

impl Plan {
    /// Every photo whose thumbnail the looks need.
    pub fn to_load(&self) -> Vec<PhotoId> {
        let mut v: Vec<PhotoId> = self.looks.iter().flat_map(Look::photos).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Most frames a bracket holds.
const MAX_BRACKET: usize = 9;
/// Seconds allowed between two frames of a run, after the first one's exposure.
const RUN_GAP: f64 = 5.0;
/// Fewest stops a bracket spans.
const MIN_SPREAD: f64 = 0.9;
/// Most frames a run holds (a long burst is cut into runs this long).
const MAX_RUN: usize = 60;

/// Step 1: read the photos `ids` (and the groups already found: their frames are never suggested
/// again) into runs fired close together, as a [`Plan`].
pub fn plan(cat: &Catalog, ids: &[PhotoId]) -> Plan {
    let taken: HashSet<PhotoId> = groups(cat).iter().flat_map(|g| g.all_frames().iter().copied().chain(g.result)).collect();
    let mut shots: Vec<Shot> = ids.iter().filter(|id| !taken.contains(id)).filter_map(|id| cat.photo(*id)).filter_map(|p| shot(p)).collect();
    // one folder, one body at a time: two cameras firing at once never share a run
    shots.sort_by(|a, b| {
        a.folder
            .cmp(&b.folder)
            .then(a.camera.to_ascii_lowercase().cmp(&b.camera.to_ascii_lowercase()))
            .then(a.t.total_cmp(&b.t))
            .then(a.id.cmp(&b.id))
    });
    shots.dedup_by_key(|s| s.id);
    let mut out = Plan::default();
    let mut start = 0;
    for i in 1..=shots.len() {
        let breaks = match (shots.get(i - 1), shots.get(i)) {
            (Some(a), Some(b)) => {
                let gap = RUN_GAP + a.shutter.unwrap_or(0.0).min(60.0);
                !a.same_rig(b) || b.t - a.t > gap || b.t < a.t || i - start >= MAX_RUN
            }
            _ => true,
        };
        if breaks {
            if let Some(run) = shots.get(start..i)
                && run.len() >= 2
            {
                plan_run(run, &mut out);
            }
            start = i;
        }
    }
    out
}

fn plan_run(run: &[Shot], out: &mut Plan) {
    let folder = run.first().map(|s| s.folder.clone()).unwrap_or_default();
    let ids: Vec<PhotoId> = run.iter().map(|s| s.id).collect();
    if let Some(first) = run.first() {
        // a lone bracket the camera stepped (exposure compensation): nothing else to look at
        let biases: Vec<f64> = run.iter().filter_map(|s| s.bias).map(f64::from).collect();
        let distinct = |v: &[f64], step: f64| v.iter().enumerate().all(|(i, a)| v.iter().skip(i + 1).all(|b| (a - b).abs() >= step));
        let levels: Vec<f64> = run.iter().map(Shot::level).collect();
        if run.len() <= MAX_BRACKET
            && run.iter().all(|s| s.same_base(first))
            && biases.len() == run.len()
            && distinct(&biases, 0.25)
            && distinct(&levels, 0.25)
            && spread(&biases) >= MIN_SPREAD
        {
            out.sure.push(Candidate {
                kind: MergeType::Hdr,
                photos: ids,
                folder,
                confidence: Confidence::Sure,
                reasons: vec![span_text(run), exposures_text(run), "the camera stepped the exposure (bracketing)".into()],
            });
            return;
        }
        // one exposure, focused at three distances or more the camera recorded
        let dists: Vec<f32> = run.iter().filter_map(|s| s.dist).collect();
        if run.len() >= 3 && dists.len() == run.len() && run.iter().all(|s| s.same_exposure(first)) && distinct_dists(&dists) >= 3 {
            out.sure.push(Candidate {
                kind: MergeType::Stack,
                photos: ids,
                folder,
                confidence: Confidence::Sure,
                reasons: vec![span_text(run), focus_text(&dists), "one exposure".into()],
            });
            return;
        }
    }
    out.looks.push(Look { folder, shots: run.to_vec() });
}

fn distinct_dists(d: &[f32]) -> usize {
    let mut v: Vec<i64> = d.iter().map(|d| (d * 100.0).round() as i64).collect();
    v.sort_unstable();
    v.dedup();
    v.len()
}

/// "focus moves near → far" from recorded distances.
fn focus_text(d: &[f32]) -> String {
    let up = d.windows(2).all(|w| w[1] >= w[0]);
    let down = d.windows(2).all(|w| w[1] <= w[0]);
    if up {
        "focus moves near → far".into()
    } else if down {
        "focus moves far → near".into()
    } else {
        "focus moves between frames".into()
    }
}

/// A photo as the finder looks at it: a small grey thumbnail (64 px wide) and where it is sharp
/// (4 × 4 tiles of detail, summing to 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Thumb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
    pub sharp: [f32; 16],
}

const THUMB_W: usize = 64;
const SHARP_W: usize = 256;

/// Box-downscale `img`'s luminance (perceptual) to `tw` wide.
fn grey(img: &Rgb32f, tw: usize) -> Option<(usize, usize, Vec<f32>)> {
    let (w, h) = (img.width, img.height);
    if w < 4 || h < 4 || img.data.len() < w.saturating_mul(h) {
        return None;
    }
    let tw = tw.min(w).max(4);
    let th = ((h as f64 * tw as f64 / w as f64).round() as usize).max(4);
    let mut sum = vec![0f32; tw * th];
    let mut cnt = vec![0u32; tw * th];
    for y in 0..h {
        let ty = (y * th / h).min(th - 1);
        for x in 0..w {
            let tx = (x * tw / w).min(tw - 1);
            let [r, g, b] = img.data.get(y * w + x).copied().unwrap_or_default();
            let l = (0.2627 * r + 0.678 * g + 0.0593 * b).max(0.0);
            let l = if l.is_finite() { l.powf(0.45) } else { 0.0 };
            sum[ty * tw + tx] += l;
            cnt[ty * tw + tx] += 1;
        }
    }
    Some((tw, th, sum.iter().zip(&cnt).map(|(s, c)| if *c > 0 { s / *c as f32 } else { 0.0 }).collect()))
}

impl Thumb {
    /// The finder's look at an image (`None`: too small to tell).
    pub fn of(img: &Rgb32f) -> Option<Thumb> {
        let (w, h, px) = grey(img, THUMB_W)?;
        let (sw, sh, big) = grey(img, SHARP_W)?;
        let mut tiles = [0f32; 16];
        for y in 1..sh.saturating_sub(1) {
            for x in 1..sw.saturating_sub(1) {
                let c = big[y * sw + x];
                let lap = 4.0 * c - big[y * sw + x - 1] - big[y * sw + x + 1] - big[(y - 1) * sw + x] - big[(y + 1) * sw + x];
                let t = (y * 4 / sh).min(3) * 4 + (x * 4 / sw).min(3);
                tiles[t] += lap.abs();
            }
        }
        let total: f32 = tiles.iter().sum();
        let sharp = if total > 1e-6 && total.is_finite() { tiles.map(|v| v / total) } else { [1.0 / 16.0; 16] };
        Some(Thumb { w, h, px, sharp })
    }
}

/// How two frames' framing relates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    /// Overlapping, but shifted: panorama neighbours.
    Shifted,
    /// The same framing (a handheld wobble of a few percent included).
    Same,
    Different,
}

/// How frame `b` lies against frame `a` ([`link`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub rel: Relation,
    /// `b`'s content is `a`'s moved by (`dx`, `dy`), as fractions of the frame: positive `dx`
    /// means the camera turned right between them.
    pub dx: f64,
    pub dy: f64,
    /// Normalized cross-correlation where they overlap (−1…1).
    pub score: f64,
    /// Share of the frame they have in common.
    pub overlap: f64,
}

impl Link {
    const NONE: Link = Link { rel: Relation::Different, dx: 0.0, dy: 0.0, score: 0.0, overlap: 0.0 };
}

/// A shift below this (fraction of the frame) is the same framing.
const SAME_SHIFT: f64 = 0.06;
/// Furthest shift looked at (fraction of the frame): panorama frames overlapping by a quarter.
const MAX_SHIFT: f64 = 0.75;
/// Fewest share of the frame two shifted frames must have in common.
const MIN_OVERLAP: f64 = 0.2;

/// Normalized cross-correlation of `a` and `b` moved by (`dx`, `dy`) where they overlap at
/// least `min_share` of the frame (`None`: too little overlap or a flat area).
fn ncc(a: &Thumb, b: &Thumb, dx: i64, dy: i64, min_share: f64) -> Option<f64> {
    let (w, h) = (a.w as i64, a.h as i64);
    let (x0, x1) = (dx.max(0), (w + dx).min(w));
    let (y0, y1) = (dy.max(0), (h + dy).min(h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let n = ((x1 - x0) * (y1 - y0)) as f64;
    if n < min_share * (w * h) as f64 {
        return None;
    }
    let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0f64, 0f64, 0f64, 0f64, 0f64);
    for y in y0..y1 {
        for x in x0..x1 {
            let va = f64::from(*a.px.get((y * w + x) as usize)?);
            let vb = f64::from(*b.px.get(((y - dy) * w + (x - dx)) as usize)?);
            sa += va;
            sb += vb;
            saa += va * va;
            sbb += vb * vb;
            sab += va * vb;
        }
    }
    let (ma, mb) = (sa / n, sb / n);
    let (va, vb) = (saa / n - ma * ma, sbb / n - mb * mb);
    if va < 1e-5 || vb < 1e-5 {
        return None;
    }
    let r = (sab / n - ma * mb) / (va * vb).sqrt();
    r.is_finite().then_some(r)
}

/// How `b`'s framing lies against `a`'s (thumbnails of one shape; another shape is different):
/// the best shift (coarse, then fine), how well they match there, and against no shift. Exposure
/// changes don't matter (the correlation is normalized).
pub fn link(a: &Thumb, b: &Thumb) -> Link {
    if a.w != b.w || a.h != b.h || a.w < 4 || a.h < 4 || a.px.len() != a.w * a.h || b.px.len() != b.w * b.h {
        return Link::NONE;
    }
    let (w, h) = (a.w as f64, a.h as f64);
    let share = |dx: i64, dy: i64| ((w - dx.abs() as f64).max(0.0) * (h - dy.abs() as f64).max(0.0)) / (w * h);
    // a small overlap is easily matched by chance: prefer the bigger one when they score alike
    let weigh = |dx: i64, dy: i64, r: f64| r - 0.1 * (1.0 - share(dx, dy));
    let zero = ncc(a, b, 0, 0, 0.9);
    let (rx, ry) = ((w * MAX_SHIFT) as i64, (h * MAX_SHIFT) as i64);
    let mut best: Option<(i64, i64, f64)> = None;
    let consider = |best: &mut Option<(i64, i64, f64)>, dx: i64, dy: i64| {
        if let Some(r) = ncc(a, b, dx, dy, MIN_OVERLAP)
            && best.is_none_or(|(bx, by, br)| weigh(dx, dy, r) > weigh(bx, by, br))
        {
            *best = Some((dx, dy, r));
        }
    };
    for dy in (-ry..=ry).step_by(2) {
        for dx in (-rx..=rx).step_by(2) {
            consider(&mut best, dx, dy);
        }
    }
    if let Some((cx, cy, _)) = best {
        for dy in cy - 2..=cy + 2 {
            for dx in cx - 2..=cx + 2 {
                consider(&mut best, dx, dy);
            }
        }
    }
    let zero_r = zero.unwrap_or(0.0);
    let Some((bx, by, r)) = best else {
        return if zero_r >= 0.85 { Link { rel: Relation::Same, score: zero_r, overlap: 1.0, ..Link::NONE } } else { Link::NONE };
    };
    let (fx, fy) = (bx as f64 / w, by as f64 / h);
    let shift = fx.abs().max(fy.abs());
    let overlap = share(bx, by);
    // a real match is a peak: moving off it by a few pixels loses the match (a smooth sky or a
    // gradient matches anywhere and says nothing)
    let around: Vec<f64> = [(bx + 2, by), (bx - 2, by), (bx, by + 2), (bx, by - 2)].iter().filter_map(|(x, y)| ncc(a, b, *x, *y, 0.15)).collect();
    let peak = if around.is_empty() { 0.0 } else { r - around.iter().sum::<f64>() / around.len() as f64 };
    let rel = if shift >= SAME_SHIFT && r >= 0.5 && r >= zero_r + 0.1 && peak >= 0.01 {
        Relation::Shifted
    } else if zero_r >= 0.85 || (shift < SAME_SHIFT && r >= 0.75) {
        Relation::Same
    } else {
        Relation::Different
    };
    match rel {
        Relation::Same => Link { rel, dx: fx, dy: fy, score: r.max(zero_r), overlap },
        _ => Link { rel, dx: fx, dy: fy, score: r, overlap },
    }
}

/// How `b`'s framing relates to `a`'s ([`link`]).
pub fn relation(a: &Thumb, b: &Thumb) -> Relation {
    link(a, b).rel
}

/// Whether the sharp area moves across frames of one framing (a focus stack, not a burst).
pub fn focus_moves(thumbs: &[&Thumb]) -> bool {
    let mut most = 0f32;
    for (i, a) in thumbs.iter().enumerate() {
        for b in thumbs.iter().skip(i + 1) {
            let d: f32 = a.sharp.iter().zip(b.sharp.iter()).map(|(x, y)| (x - y).abs()).sum();
            most = most.max(d);
        }
    }
    most >= 0.3
}

/// Frames of one framing (indexes into a run).
#[derive(Clone, Debug)]
struct Position {
    frames: Vec<usize>,
    /// Their thumbnails said they are one framing (else only the metadata joined them).
    checked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PosKind {
    Single,
    Bracket,
    Stack,
    Burst,
}

/// Step 2 + 3: what a run is, from its thumbnails (a missing one leaves the metadata to judge).
pub fn decide(look: &Look, thumbs: &HashMap<PhotoId, Thumb>) -> Vec<Candidate> {
    let s = &look.shots;
    let n = s.len();
    if n < 2 {
        return Vec::new();
    }
    let link_of = |a: usize, b: usize| -> Option<Link> {
        match (s.get(a).and_then(|x| thumbs.get(&x.id)), s.get(b).and_then(|x| thumbs.get(&x.id))) {
            (Some(x), Some(y)) => Some(link(x, y)),
            _ => None,
        }
    };
    let links: Vec<Option<Link>> = (0..n - 1).map(|k| link_of(k, k + 1)).collect();
    // positions: frames of one framing in a row
    let mut pos: Vec<Position> = vec![Position { frames: vec![0], checked: true }];
    for (k, l) in links.iter().enumerate() {
        let f = k + 1;
        let rel = l.map(|l| l.rel);
        let Some(cur) = pos.last_mut() else { break };
        let join = match rel {
            Some(Relation::Shifted) => false,
            Some(Relation::Same) => joins(s, &cur.frames, f, false),
            // the pixels can't tell (or there are none): a bracket by its metadata only
            _ => joins(s, &cur.frames, f, true),
        };
        if join {
            cur.frames.push(f);
            if rel != Some(Relation::Same) {
                cur.checked = false;
            }
        } else {
            pos.push(Position { frames: vec![f], checked: true });
        }
    }
    let kinds: Vec<PosKind> = pos.iter().map(|p| classify(s, p, thumbs)).collect();
    // panoramas: positions linked by shifts
    let mut out = Vec::new();
    let mut i = 0;
    while i < pos.len() {
        let mut j = i;
        let mut chain: Vec<Link> = Vec::new();
        while j + 1 < pos.len() {
            let (Some(a), Some(b)) = (pos.get(j), pos.get(j + 1)) else { break };
            let (Some(&last), Some(&next)) = (a.frames.last(), b.frames.first()) else { break };
            let mut l = links.get(last).copied().flatten();
            // brackets: the boundary frames differ in exposure; their first frames match better
            if l.is_none_or(|l| l.rel != Relation::Shifted)
                && let Some(&first) = a.frames.first()
                && first != last
            {
                l = link_of(first, next).filter(|l| l.rel == Relation::Shifted).or(l);
            }
            match l {
                Some(l) if l.rel == Relation::Shifted => {
                    chain.push(l);
                    j += 1;
                }
                _ => break,
            }
        }
        if j > i {
            out.push(panorama(look, pos.get(i..=j).unwrap_or(&[]), kinds.get(i..=j).unwrap_or(&[]), &chain));
        } else if let (Some(p), Some(k)) = (pos.get(i), kinds.get(i)) {
            let shots: Vec<Shot> = p.frames.iter().filter_map(|f| s.get(*f).cloned()).collect();
            let photos: Vec<PhotoId> = shots.iter().map(|x| x.id).collect();
            match k {
                PosKind::Bracket => {
                    let stepped = shots.iter().all(|x| x.bias.is_some())
                        && spread(&shots.iter().filter_map(|x| x.bias).map(f64::from).collect::<Vec<_>>()) >= MIN_SPREAD;
                    let wide = spread(&shots.iter().map(Shot::level).collect::<Vec<_>>()) >= 1.5;
                    let confidence = match (p.checked, stepped || wide) {
                        (true, true) => Confidence::Sure,
                        (true, false) | (false, true) => Confidence::Likely,
                        (false, false) => Confidence::Maybe,
                    };
                    let framing = if p.checked { "same framing" } else { "framing could not be compared" };
                    out.push(Candidate {
                        kind: MergeType::Hdr,
                        photos,
                        folder: look.folder.clone(),
                        confidence,
                        reasons: vec![span_text(&shots), exposures_text(&shots), framing.into()],
                    });
                }
                PosKind::Stack => {
                    let dists: Vec<f32> = shots.iter().filter_map(|x| x.dist).collect();
                    let recorded = dists.len() == shots.len() && distinct_dists(&dists) >= 3;
                    let (confidence, why) = if recorded {
                        (if p.checked { Confidence::Sure } else { Confidence::Likely }, focus_text(&dists))
                    } else {
                        (Confidence::Likely, "the sharp area moves across the frame".to_string())
                    };
                    out.push(Candidate {
                        kind: MergeType::Stack,
                        photos,
                        folder: look.folder.clone(),
                        confidence,
                        reasons: vec![span_text(&shots), why, "same framing, one exposure".into()],
                    });
                }
                // a burst (same framing, focus and exposure) or a lone frame: nothing
                PosKind::Single | PosKind::Burst => {}
            }
        }
        i = j + 1;
    }
    out
}

/// Whether frame `f` joins the position `frames` (its last frame and `f` are one framing, or —
/// `meta_only` — the pixels couldn't tell): a burst or focus stack grows at one exposure, a
/// bracket with each new exposure.
fn joins(s: &[Shot], frames: &[usize], f: usize, meta_only: bool) -> bool {
    let (Some(first), Some(last), Some(new)) = (frames.first().and_then(|i| s.get(*i)), frames.last().and_then(|i| s.get(*i)), s.get(f)) else {
        return false;
    };
    let flat = frames.iter().filter_map(|i| s.get(*i)).all(|x| x.same_exposure(first));
    if !meta_only && flat && new.same_exposure(first) {
        return true;
    }
    if frames.len() >= MAX_BRACKET || !new.same_base(first) || (frames.len() >= 2 && flat) {
        return false;
    }
    let step = if meta_only { 0.6 } else { 0.3 };
    let distinct = frames.iter().filter_map(|i| s.get(*i)).all(|x| (new.level() - x.level()).abs() >= step);
    // a bracket by metadata alone is fired fast
    distinct && (!meta_only || new.t - last.t <= 2.0 + last.shutter.unwrap_or(0.0).min(30.0))
}

fn classify(s: &[Shot], p: &Position, thumbs: &HashMap<PhotoId, Thumb>) -> PosKind {
    let shots: Vec<&Shot> = p.frames.iter().filter_map(|f| s.get(*f)).collect();
    let Some(first) = shots.first() else { return PosKind::Single };
    if shots.len() < 2 {
        return PosKind::Single;
    }
    let flat = shots.iter().all(|x| x.same_exposure(first));
    let levels: Vec<f64> = shots.iter().map(|x| x.level()).collect();
    if !flat && spread(&levels) >= MIN_SPREAD {
        return PosKind::Bracket;
    }
    if flat && shots.len() >= 3 && p.checked {
        let dists: Vec<f32> = shots.iter().filter_map(|x| x.dist).collect();
        if dists.len() == shots.len() && distinct_dists(&dists) >= 3 {
            return PosKind::Stack;
        }
        let ts: Vec<&Thumb> = shots.iter().filter_map(|x| thumbs.get(&x.id)).collect();
        if ts.len() == shots.len() && focus_moves(&ts) {
            return PosKind::Stack;
        }
    }
    PosKind::Burst
}

/// A chain of positions linked by shifts: a panorama, or an HDR panorama when most of its
/// positions are brackets.
fn panorama(look: &Look, pos: &[Position], kinds: &[PosKind], chain: &[Link]) -> Candidate {
    let shots: Vec<Shot> = pos.iter().flat_map(|p| p.frames.iter()).filter_map(|f| look.shots.get(*f).cloned()).collect();
    let photos: Vec<PhotoId> = shots.iter().map(|x| x.id).collect();
    let brackets = kinds.iter().filter(|k| **k == PosKind::Bracket).count();
    let hdr = brackets >= 2 && brackets * 2 >= pos.len();
    // how sure: every overlap a clear match, moving one way
    let strong = chain.iter().all(|l| l.score >= 0.8 && l.overlap >= 0.3);
    let medium = chain.iter().all(|l| l.score >= 0.65);
    let one_way = chain.windows(2).all(|w| w[0].dx * w[1].dx + w[0].dy * w[1].dy > 0.0);
    let mut confidence = if strong && ((chain.len() >= 2 && one_way) || chain.iter().all(|l| l.score >= 0.9)) {
        Confidence::Sure
    } else if medium {
        Confidence::Likely
    } else {
        Confidence::Maybe
    };
    let mut reasons = vec![span_text(&shots)];
    let mean_overlap = chain.iter().map(|l| l.overlap).sum::<f64>() / chain.len().max(1) as f64;
    let pct = ((mean_overlap * 20.0).round() * 5.0) as i64;
    reasons.push(format!("each frame overlaps the last by ~{pct} %"));
    let (sx, sy): (f64, f64) = chain.iter().fold((0.0, 0.0), |(x, y), l| (x + l.dx, y + l.dy));
    if one_way {
        reasons.push(
            if sx.abs() >= sy.abs() {
                if sx >= 0.0 { "frames move left → right" } else { "frames move right → left" }
            } else if sy >= 0.0 {
                "frames move top → bottom"
            } else {
                "frames move bottom → top"
            }
            .into(),
        );
    }
    if hdr {
        let first: Vec<Shot> = pos.first().map(|p| p.frames.iter().filter_map(|f| look.shots.get(*f).cloned()).collect()).unwrap_or_default();
        reasons.push(format!("a bracket at each of {} positions", pos.len()));
        if first.len() >= 2 {
            reasons.push(exposures_text(&first));
        }
        // a bracket only the metadata joined lowers it
        if pos.iter().zip(kinds).any(|(p, k)| *k == PosKind::Bracket && !p.checked) {
            confidence = confidence.min(Confidence::Likely);
        }
    } else {
        let levels: Vec<f64> = shots.iter().map(Shot::level).collect();
        let drift = spread(&levels);
        if drift >= 0.25 {
            reasons.push(format!("exposure drifts by {} stop (auto exposure): still one panorama", trim_num((drift * 10.0).round() / 10.0)));
        }
    }
    Candidate { kind: if hdr { MergeType::HdrPano } else { MergeType::Pano }, photos, folder: look.folder.clone(), confidence, reasons }
}

/// A folder's fingerprint for the automatic finder: changes when its photos do.
fn fingerprints(cat: &Catalog) -> BTreeMap<String, u64> {
    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    for p in cat.photos().filter(|p| p.in_library()) {
        let Some(f) = lightcraft_catalog::local::folder_of(p) else { continue };
        let e = out.entry(f).or_insert(0xcbf2_9ce4_8422_2325);
        *e = lightcraft_catalog::mix64(*e ^ p.id.0.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::from(p.captured.is_some()));
    }
    out
}

fn scanned(cat: &Catalog) -> BTreeMap<String, u64> {
    cat.record(SCANNED).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

/// `img` box-averaged down to fit `size` px on its long edge (never up).
fn fit_rgba(img: &lightcraft_raster::Rgba8, size: usize) -> lightcraft_raster::Rgba8 {
    let (w, h) = (img.width, img.height);
    let long = w.max(h);
    if long <= size || w == 0 || h == 0 || img.data.len() < w * h {
        return img.clone();
    }
    let (tw, th) = (((w * size) / long).max(1), ((h * size) / long).max(1));
    let mut out = lightcraft_raster::Rgba8::new(tw, th);
    for ty in 0..th {
        let (y0, y1) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1).min(h));
        for tx in 0..tw {
            let (x0, x1) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1).min(w));
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    if let Some(px) = img.data.get(y * w + x) {
                        for (a, v) in acc.iter_mut().zip(px) {
                            *a += u32::from(*v);
                        }
                        n += 1;
                    }
                }
            }
            if let Some(o) = out.data.get_mut(ty * tw + tx) {
                *o = acc.map(|a| (a / n.max(1)).min(255) as u8);
            }
        }
    }
    out
}

// ------------------------------------------------------------------------------------- session

/// A finder run on a worker thread (the app): thumbnails are loaded and compared off the UI
/// thread as background work; [`Session::merge_find_poll`] stores the groups when it is done.
pub struct FindJob {
    /// What it found, and whether it read any thumbnail (worth not reading again).
    rx: std::sync::mpsc::Receiver<(Vec<Candidate>, bool)>,
    cancel: Arc<AtomicBool>,
    generation: u64,
    /// Folders looked at → their fingerprints, written when the run lands.
    scanned: BTreeMap<String, u64>,
    pub total: usize,
}

impl Drop for FindJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Session {
    /// Find merges among the photos of `scope` (`merge.find`): to the end, or with `background`
    /// on a worker thread (→ `{queued}`; the groups land with [`Session::merge_find_poll`]).
    pub fn merge_find(&mut self, scope: &Scope, background: bool) -> Result<Value> {
        let ids = scope.photos(&self.catalog);
        self.merge_find_ids(&ids, background, BTreeMap::new())
    }

    fn merge_find_ids(&mut self, ids: &[PhotoId], background: bool, scanned: BTreeMap<String, u64>) -> Result<Value> {
        let plan = plan(&self.catalog, ids);
        let load = plan.to_load();
        if !background || load.is_empty() {
            let mut thumbs = HashMap::new();
            for id in load {
                if let Ok(img) = self.source_now(id, crate::media::SourceLevel::Thumb)
                    && let Some(t) = Thumb::of(&img)
                {
                    thumbs.insert(id, t);
                }
            }
            let mut found = plan.sure.clone();
            for l in &plan.looks {
                found.extend(decide(l, &thumbs));
            }
            return self.merge_found(found, scanned, !thumbs.is_empty());
        }
        // a new run replaces one still looking
        self.merge_find_job = None;
        let mut work: Vec<(PhotoId, crate::media::SourceRef)> = Vec::new();
        for id in &load {
            if let Some(p) = self.catalog.photo(*id).cloned() {
                work.push((*id, self.media.source_ref(&p, crate::media::SourceLevel::Thumb)));
            }
        }
        let total = work.len();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let (c, sure, looks) = (cancel.clone(), plan.sure.clone(), plan.looks.clone());
        std::thread::Builder::new()
            .name("merge-find".into())
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut thumbs = HashMap::new();
                    for (id, src) in work {
                        if c.load(Ordering::Relaxed) {
                            return None;
                        }
                        if let Ok(img) = crate::memory::in_background(|| src.load())
                            && let Some(t) = Thumb::of(&img)
                        {
                            thumbs.insert(id, t);
                        }
                    }
                    let mut found = sure;
                    for l in &looks {
                        found.extend(decide(l, &thumbs));
                    }
                    Some((found, !thumbs.is_empty()))
                }));
                if let Ok(Some(found)) = r {
                    let _ = tx.send(found);
                }
            })
            .map_err(|e| EngineError::Other(format!("can't start finding merges: {e}")))?;
        let generation = self.library_generation();
        self.merge_find_job = Some(FindJob { rx, cancel, generation, scanned, total });
        Ok(json!({"queued": total, "background": true}))
    }

    /// A finder run is looking.
    pub fn merge_find_running(&self) -> bool {
        self.merge_find_job.is_some()
    }

    /// Store what a background finder run found, once it is done (`None` while it runs or when
    /// nothing runs). A run started for another library is dropped.
    pub fn merge_find_poll(&mut self) -> Option<Result<Value>> {
        let job = self.merge_find_job.as_ref()?;
        let (found, persist) = match job.rx.try_recv() {
            Ok(f) => f,
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.merge_find_job = None;
                return Some(Err(EngineError::Other("finding merges stopped".into())));
            }
        };
        let job = self.merge_find_job.take()?;
        if job.generation != self.library_generation() {
            return None;
        }
        Some(self.merge_found(found, job.scanned.clone(), persist))
    }

    /// The automatic finder (Settings ▸ find merges: auto.): start a background run over the
    /// folders whose photos changed since it last looked (all of them the first time). `None`:
    /// nothing new, or a run is going.
    pub fn merge_find_auto(&mut self) -> Option<Result<Value>> {
        if self.merge_find_job.is_some() {
            return None;
        }
        let now = fingerprints(&self.catalog);
        let mut seen = scanned(&self.catalog);
        seen.extend(self.merge_scanned.iter().map(|(k, v)| (k.clone(), *v)));
        let changed: BTreeMap<String, u64> = now.into_iter().filter(|(f, fp)| seen.get(f) != Some(fp)).collect();
        if changed.is_empty() {
            return None;
        }
        let ids: Vec<PhotoId> = self
            .catalog
            .photos()
            .filter(|p| p.in_library() && lightcraft_catalog::local::folder_of(p).is_some_and(|f| changed.contains_key(&f)))
            .map(|p| p.id)
            .collect();
        Some(self.merge_find_ids(&ids, true, changed))
    }

    /// Store found merges as open groups (bookkeeping: not an undo step), leaving out any that
    /// shares a frame with a group already there; note the folders looked at — in the library
    /// when something was found or thumbnails were read (`persist`: worth not redoing), else
    /// for this session only (metadata alone is cheap to read again, and a library that changes
    /// nothing gets no write).
    pub fn merge_found(&mut self, found: Vec<Candidate>, looked: BTreeMap<String, u64>, persist: bool) -> Result<Value> {
        let mut taken: HashSet<PhotoId> = groups(&self.catalog).iter().flat_map(|g| g.all_frames().iter().copied().chain(g.result)).collect();
        let now = (self.clock)();
        let mut ops = Vec::new();
        let mut out = Vec::new();
        for c in found {
            if c.photos.len() < 2 || c.photos.iter().any(|p| taken.contains(p) || self.catalog.photo(*p).is_none_or(|p| !p.in_library())) {
                continue;
            }
            taken.extend(c.photos.iter().copied());
            let g = MergeGroup {
                id: group_id(&c.photos),
                kind: c.kind,
                photos: c.photos,
                state: GroupState::Open,
                result: None,
                folder: c.folder,
                found: now.clone(),
                found_frames: Vec::new(),
                confidence: Some(c.confidence),
                reasons: c.reasons,
            };
            ops.push(g.op());
            out.push(g.to_json());
        }
        self.merge_scanned.extend(looked.iter().map(|(k, v)| (k.clone(), *v)));
        if !looked.is_empty() && (persist || !ops.is_empty()) {
            let mut seen = scanned(&self.catalog);
            seen.extend(looked);
            ops.push(Op::SetRecord { key: SCANNED.into(), value: serde_json::to_value(seen).ok() });
        }
        if !ops.is_empty() {
            self.apply_bookkeeping(Op::Batch { ops })?;
        }
        Ok(json!({"found": out.len(), "groups": out}))
    }

    /// After a group's frames `sources` were merged into `result`: hide them, mark the merged
    /// photo, put it into every collection they were in (where the first of them was), and note
    /// the group merged. One undo step.
    pub fn merge_group_done(&mut self, group_id: &str, result: PhotoId, sources: &[PhotoId]) -> Result<Value> {
        let mut g = group(&self.catalog, group_id).ok_or_else(|| EngineError::Other(format!("no merge group {group_id}")))?;
        let res = self.catalog.photo(result).cloned().ok_or(lightcraft_catalog::CatalogError::NoPhoto(result))?;
        let mut ops = Vec::new();
        for s in sources.iter().filter(|s| **s != result) {
            let Some(p) = self.catalog.photo(*s) else { continue };
            let mark = MergeMark { into: Some(result), ..p.merge.unwrap_or_default() };
            ops.push(Op::SetMerge { id: *s, merge: Some(mark) });
        }
        ops.push(Op::SetMerge { id: result, merge: Some(MergeMark { into: None, kind: Some(g.kind), only_merges: false }) });
        // the originals' place in capture order
        if res.captured.is_none()
            && let Some(c) = sources.iter().find_map(|s| self.catalog.photo(*s).and_then(|p| p.captured.clone()))
        {
            ops.push(Op::SetCaptured { id: result, captured: Some(c) });
        }
        for a in self.catalog.albums().filter(|a| !a.folder && !a.is_smart() && !a.photos.contains(&result)) {
            if let Some(at) = a.photos.iter().position(|p| sources.contains(p)) {
                let mut photos = a.photos.clone();
                photos.insert(at, result);
                ops.push(Op::SetAlbumPhotos { id: a.id, photos });
            }
        }
        if sources.len() >= 2 {
            g.photos = sources.to_vec();
        }
        g.state = GroupState::Merged;
        g.result = Some(result);
        ops.push(g.op());
        self.commit("Merge", Op::Batch { ops })?;
        Ok(json!({"group": g.id, "id": result.0, "hidden": sources.len()}))
    }

    /// The frames of group `g` to merge: `only` (those of its live frames, two or more), else all
    /// its live frames.
    pub fn merge_group_frames(&self, g: &MergeGroup, only: &[PhotoId]) -> Vec<PhotoId> {
        let live = g.live_frames(&self.catalog);
        let picked: Vec<PhotoId> = live.iter().copied().filter(|p| only.contains(p)).collect();
        if picked.len() >= 2 { picked } else { live }
    }

    /// Merge groups to the end (`merge.mergeGroups`): each with its kind (or `kind`), then
    /// [`Session::merge_group_done`].
    pub fn merge_groups_now(&mut self, ids: &[String], kind: Option<MergeType>, only: &[PhotoId], auto_settings: bool) -> Result<Value> {
        let mut merged = Vec::new();
        let mut failed = Vec::new();
        for id in ids {
            let Some(g) = group(&self.catalog, id).filter(|g| g.state == GroupState::Open) else {
                failed.push(json!({"group": id, "error": "no open merge group with this id"}));
                continue;
            };
            let frames = self.merge_group_frames(&g, only);
            let k = kind.unwrap_or(g.kind);
            let params = json!({"autoSettings": auto_settings, "stack": false, "autoCrop": true});
            let r = crate::merge::parse(k.command(), &params).and_then(|(mk, finish)| self.merge_now(mk, finish, &frames)).and_then(|info| {
                let rid = info.get("id").and_then(Value::as_u64).map(PhotoId).ok_or_else(|| EngineError::Other("the merge made no photo".into()))?;
                if k != g.kind {
                    self.set_group_kind(&g.id, k)?;
                }
                self.merge_group_done(&g.id, rid, &frames)
            });
            match r {
                Ok(v) => merged.push(v),
                Err(e) => failed.push(json!({"group": id, "error": e.to_string()})),
            }
        }
        Ok(json!({"merged": merged, "failed": failed}))
    }

    /// Change what an open group will be merged as (bookkeeping on the record; one undo step).
    pub fn set_group_kind(&mut self, id: &str, kind: MergeType) -> Result<()> {
        let mut g = group(&self.catalog, id).ok_or_else(|| EngineError::Other(format!("no merge group {id}")))?;
        if g.kind == kind {
            return Ok(());
        }
        g.kind = kind;
        self.commit("Merge Kind", g.op())
    }

    /// send.back.: the groups are not merges; their frames stay in their folder and are never
    /// suggested again (one undo step; `merge.restore` brings a group back).
    pub fn merge_dismiss(&mut self, ids: &[String]) -> Result<Value> {
        let mut ops = Vec::new();
        for id in ids {
            if let Some(mut g) = group(&self.catalog, id).filter(|g| g.state == GroupState::Open) {
                g.state = GroupState::Dismissed;
                ops.push(g.op());
            }
        }
        let n = ops.len();
        if n > 0 {
            self.commit("Send Back", Op::Batch { ops })?;
        }
        Ok(json!({"dismissed": n}))
    }

    /// restore.: sent-back groups are open again (one undo step).
    pub fn merge_restore(&mut self, ids: &[String]) -> Result<Value> {
        let mut ops = Vec::new();
        for id in ids {
            if let Some(mut g) = group(&self.catalog, id).filter(|g| g.state == GroupState::Dismissed) {
                g.state = GroupState::Open;
                ops.push(g.op());
            }
        }
        let n = ops.len();
        if n > 0 {
            self.commit("Restore Merge", Op::Batch { ops })?;
        }
        Ok(json!({"restored": n}))
    }

    /// Which of its found frames an open group merges (`photos`, any order; the others stay in
    /// it, taken out, so they can be put back). One undo step.
    pub fn merge_set_frames(&mut self, id: &str, photos: &[PhotoId]) -> Result<Value> {
        let mut g = group(&self.catalog, id).ok_or_else(|| EngineError::Other(format!("no merge group {id}")))?;
        if g.state != GroupState::Open {
            return Err(EngineError::Other(format!("merge group {id} is {}, not open", g.state.id())));
        }
        let all = g.all_frames().to_vec();
        if let Some(stray) = photos.iter().find(|p| !all.contains(p)) {
            return Err(EngineError::Other(format!("photo {} was not found with merge group {id}", stray.0)));
        }
        let wanted: Vec<PhotoId> = all.iter().copied().filter(|p| photos.contains(p)).collect();
        if wanted != g.photos {
            g.found_frames = all.clone();
            g.photos = wanted;
            self.commit("Merge Frames", g.op())?;
        }
        Ok(json!({
            "id": g.id,
            "photos": g.photos.iter().map(|p| p.0).collect::<Vec<_>>(),
            "out": all.iter().filter(|p| !g.photos.contains(p)).map(|p| p.0).collect::<Vec<_>>(),
        }))
    }

    /// Small JPEGs of photos from the thumbnail cache (`merge.thumbs`), `size` px on the long
    /// edge, written to `merges-thumbs/` in the library (a temporary folder without one); photos
    /// without a cached thumbnail yet are left out, unless `render` (then it is rendered now).
    pub fn merge_thumbs(&mut self, ids: &[PhotoId], size: usize, render: bool) -> Result<Value> {
        let size = size.clamp(64, 1024);
        let dir = match self.library.as_ref().filter(|l| l.on_disk) {
            Some(l) => l.dir.join("merges-thumbs"),
            None => std::env::temp_dir().join(format!("lightcraft-merges-thumbs-{}", std::process::id())),
        };
        std::fs::create_dir_all(&dir).map_err(|e| EngineError::Other(format!("can't make {}: {e}", dir.display())))?;
        let mut out = serde_json::Map::new();
        for id in ids {
            let Some(job) = self.thumb_job(*id, size) else { continue };
            let cached = job.cache.as_ref().and_then(|(c, k)| c.get(*k));
            let img = match cached {
                Some(img) => img,
                None if render => match job.run().rendered {
                    Ok(r) => Arc::new(r.image),
                    Err(_) => continue,
                },
                None => continue,
            };
            let img = fit_rgba(&img, size);
            let Some(bytes) = lightcraft_preview::encode_jpeg(&img) else { continue };
            let path = dir.join(format!("{}-{size}.jpg", id.0));
            if std::fs::write(&path, bytes).is_ok() {
                out.insert(id.0.to_string(), json!(path.to_string_lossy()));
            }
        }
        Ok(json!({"thumbs": out}))
    }

    /// The merged groups named by `ids` (group ids or merged photo ids).
    pub fn merged_groups_of(&self, ids: &[Value]) -> Vec<MergeGroup> {
        let all = groups(&self.catalog);
        let mut out: Vec<MergeGroup> = Vec::new();
        for v in ids {
            let hit = match v {
                Value::String(s) => all.iter().find(|g| g.id == s.trim()),
                Value::Number(n) => n.as_u64().and_then(|r| all.iter().find(|g| g.state == GroupState::Merged && g.result == Some(PhotoId(r)))),
                _ => None,
            };
            if let Some(g) = hit.filter(|g| g.state == GroupState::Merged)
                && !out.iter().any(|o| o.id == g.id)
            {
                out.push(g.clone());
            }
        }
        out
    }

    /// Unmerge: the frames show again; the group is open again; the merged photo stays (an
    /// ordinary photo with its tag). One undo step.
    pub fn merge_unmerge(&mut self, ids: &[Value]) -> Result<Value> {
        let mut ops = Vec::new();
        let mut shown = 0;
        let gs = self.merged_groups_of(ids);
        for mut g in gs.clone() {
            let result = g.result;
            for p in self.catalog.photos().filter(|p| p.merge.and_then(|m| m.into).is_some() && p.merge.and_then(|m| m.into) == result) {
                let mark = MergeMark { into: None, ..p.merge.unwrap_or_default() };
                ops.push(Op::SetMerge { id: p.id, merge: Some(mark) });
                shown += 1;
            }
            if let Some(r) = result
                && let Some(p) = self.catalog.photo(r)
                && p.merge.is_some_and(|m| m.only_merges)
            {
                ops.push(Op::SetMerge { id: r, merge: Some(MergeMark { only_merges: false, ..p.merge.unwrap_or_default() }) });
            }
            g.state = GroupState::Open;
            g.result = None;
            ops.push(g.op());
        }
        if !ops.is_empty() {
            self.commit("Unmerge", Op::Batch { ops })?;
        }
        Ok(json!({"unmerged": gs.len(), "shown": shown}))
    }

    /// Where merged photos show: `both` (merges. and their folder / collections) or `merges`
    /// (merges. only). One undo step.
    pub fn merge_show_in(&mut self, ids: &[Value], only_merges: bool) -> Result<Value> {
        let mut ops = Vec::new();
        for g in self.merged_groups_of(ids) {
            let Some(r) = g.result else { continue };
            let Some(p) = self.catalog.photo(r) else { continue };
            let mark = MergeMark { only_merges, kind: p.merge.and_then(|m| m.kind).or(Some(g.kind)), ..p.merge.unwrap_or_default() };
            if p.merge != Some(mark) {
                ops.push(Op::SetMerge { id: r, merge: Some(mark) });
            }
        }
        let n = ops.len();
        if n > 0 {
            self.commit(if only_merges { "Show in merges. Only" } else { "Show in Both" }, Op::Batch { ops })?;
        }
        Ok(json!({"changed": n}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_catalog::{Photo, Source};

    fn photo(cat: &mut Catalog, name: &str, t: &str, shutter: &str, bias: Option<f32>) -> PhotoId {
        let id = cat.alloc_photo_id();
        let mut p = Photo::new(id, Source::File { path: format!("/shoot/{name}") }, name, "DNG", 60, 40, "2026-11-01T00:00:00");
        p.captured = Some(t.into());
        p.meta.camera = "LEICA M".into();
        p.meta.lens = "35mm".into();
        p.meta.focal_mm = Some(35.0);
        p.meta.aperture = Some(8.0);
        p.meta.iso = Some(100);
        p.meta.shutter = shutter.into();
        p.meta.exposure_bias = bias;
        cat.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
        id
    }

    #[test]
    fn shutter_and_time_parse() {
        assert_eq!(shutter_secs("1/250"), Some(0.004));
        assert_eq!(shutter_secs("2"), Some(2.0));
        assert_eq!(shutter_secs("30\""), Some(30.0));
        assert_eq!(shutter_secs("1/0"), None);
        assert_eq!(shutter_secs(""), None);
        assert_eq!(shutter_secs("abc"), None);
        let a = capture_secs("2026-11-01T10:00:00").unwrap();
        assert!((capture_secs("2026-11-01T10:00:00.25").unwrap() - a - 0.25).abs() < 1e-9);
        assert_eq!(capture_secs("nonsense"), None);
    }

    #[test]
    fn brackets_are_found_from_exposure_steps() {
        let mut cat = Catalog::new();
        let a = photo(&mut cat, "a.dng", "2026-11-01T10:00:00", "1/125", Some(0.0));
        let b = photo(&mut cat, "b.dng", "2026-11-01T10:00:01", "1/500", Some(-2.0));
        let c = photo(&mut cat, "c.dng", "2026-11-01T10:00:01", "1/30", Some(2.0));
        // a minute later: another photo at one exposure, alone
        photo(&mut cat, "d.dng", "2026-11-01T10:01:00", "1/125", None);
        let ids: Vec<PhotoId> = cat.photos().map(|p| p.id).collect();
        let plan = plan(&cat, &ids);
        assert_eq!(plan.sure.len(), 1, "{:?}", plan.sure);
        let c0 = &plan.sure[0];
        assert_eq!((c0.kind, c0.photos.clone(), c0.folder.as_str(), c0.confidence), (MergeType::Hdr, vec![a, b, c], "/shoot", Confidence::Sure));
        assert_eq!(c0.reasons[0], "3 frames in 1 s");
        assert_eq!(c0.reasons[1], "exposures 0 / \u{2212}2 / +2");
        assert!(plan.looks.is_empty(), "{:?}", plan.looks);
    }

    #[test]
    fn repeated_brackets_and_one_exposure_runs_need_a_look() {
        let mut cat = Catalog::new();
        for (k, t) in ["10:00:00", "10:00:03"].iter().enumerate() {
            for (j, (s, b)) in [("1/125", 0.0), ("1/500", -2.0), ("1/30", 2.0)].iter().enumerate() {
                photo(&mut cat, &format!("b{k}{j}.dng"), &format!("2026-11-01T{t}"), s, Some(*b));
            }
        }
        // a pano: four frames at one exposure, then one far later
        let mut pano = Vec::new();
        for i in 0..4 {
            pano.push(photo(&mut cat, &format!("p{i}.dng"), &format!("2026-11-01T11:00:0{}", i * 2), "1/250", None));
        }
        let ids: Vec<PhotoId> = cat.photos().map(|p| p.id).collect();
        let plan = plan(&cat, &ids);
        assert!(plan.sure.is_empty(), "{:?}", plan.sure);
        assert_eq!(plan.looks.len(), 2, "{:?}", plan.looks);
        assert_eq!(plan.looks[0].photos().len(), 6, "both brackets: one run");
        assert_eq!(plan.looks[1].photos(), pano);
        assert_eq!(plan.to_load().len(), 10, "every frame of every run is looked at");
        // without thumbnails: the brackets are one HDR each (by their metadata, not sure), the
        // pano is nothing
        let none = HashMap::new();
        let found: Vec<(MergeType, Confidence)> = plan.looks.iter().flat_map(|l| decide(l, &none)).map(|c| (c.kind, c.confidence)).collect();
        assert_eq!(found, vec![(MergeType::Hdr, Confidence::Likely), (MergeType::Hdr, Confidence::Likely)]);
    }

    #[test]
    fn recorded_focus_distances_make_a_stack() {
        let mut cat = Catalog::new();
        let mut ids = Vec::new();
        for (i, d) in [0.5f32, 0.8, 1.5, 3.0].iter().enumerate() {
            let id = photo(&mut cat, &format!("f{i}.dng"), &format!("2026-11-01T12:00:0{i}"), "1/60", None);
            let mut p = (**cat.photo(id).unwrap()).clone();
            p.meta.focus_distance = Some(*d);
            cat.apply(Op::SetMeta { id, meta: Box::new(p.meta) }).unwrap();
            ids.push(id);
        }
        let plan = plan(&cat, &ids);
        assert_eq!(plan.sure.len(), 1);
        assert_eq!(plan.sure[0].kind, MergeType::Stack);
        assert_eq!(plan.sure[0].photos, ids);
    }

    #[test]
    fn missing_metadata_and_other_cameras_break_runs() {
        let mut cat = Catalog::new();
        let a = photo(&mut cat, "a.dng", "2026-11-01T10:00:00", "1/125", None);
        let b = photo(&mut cat, "b.dng", "2026-11-01T10:00:01", "1/500", None);
        let mut p = (**cat.photo(b).unwrap()).clone();
        p.meta.camera = "Canon EOS R6".into();
        cat.apply(Op::SetMeta { id: b, meta: Box::new(p.meta) }).unwrap();
        // no capture time: never part of a run
        let c = cat.alloc_photo_id();
        cat.apply(Op::AddPhoto { photo: Box::new(Photo::new(c, Source::File { path: "/shoot/c.dng".into() }, "c.dng", "DNG", 6, 4, "x")) }).unwrap();
        // a demo photo: no folder
        let d = cat.alloc_photo_id();
        cat.apply(Op::AddPhoto { photo: Box::new(Photo::new(d, Source::Demo { scene: 0 }, "d", "JPEG", 6, 4, "x")) }).unwrap();
        let plan = plan(&cat, &[a, b, c, d, PhotoId(999)]);
        assert_eq!(plan, Plan::default());
    }

    fn scene(w: usize, h: usize, off: f32, blur_left: bool, blur_right: bool) -> Rgb32f {
        // a textured scene, sampled from `off` (a pan): sharp detail, blurred where asked
        let mut img = Rgb32f::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let u = x as f32 + off;
                let blur = if (x < w / 2 && blur_left) || (x >= w / 2 && blur_right) { 0.0 } else { 1.0 };
                let v = 0.3
                    + 0.15 * ((u * 0.031).sin() + (y as f32 * 0.047).cos())
                    + 0.1 * (u * 0.011 + y as f32 * 0.007).sin()
                    + blur * 0.12 * (((u as i64 / 3 + y as i64 / 3) % 2) as f32 - 0.5);
                img.data[y * w + x] = [v, v, v];
            }
        }
        img
    }

    /// A run as the finder reads it: (photo id, seconds, shutter, compensation).
    fn run_of(frames: &[(u64, f64, &str, Option<f32>)]) -> Look {
        let shots = frames
            .iter()
            .map(|(id, t, sh, b)| Shot {
                id: PhotoId(*id),
                t: *t,
                folder: "/s".into(),
                camera: "LEICA M".into(),
                lens: "35mm".into(),
                focal: Some(35.0),
                aperture: Some(8.0),
                iso: Some(100),
                shutter: shutter_secs(sh),
                bias: *b,
                dist: None,
            })
            .collect();
        Look { folder: "/s".into(), shots }
    }

    /// The scene `ev` stops brighter.
    fn brighter(img: &Rgb32f, ev: f32) -> Rgb32f {
        let k = 2f32.powf(ev);
        Rgb32f::from_fn(img.width, img.height, |x, y| img.data[y * img.width + x].map(|v| v * k))
    }

    fn kinds(c: &[Candidate]) -> Vec<(MergeType, Vec<PhotoId>)> {
        c.iter().map(|c| (c.kind, c.photos.clone())).collect()
    }

    #[test]
    fn thumbnails_tell_a_pano_from_a_burst_and_a_focus_stack() {
        let t = |img: &Rgb32f| Thumb::of(img).unwrap();
        let a = t(&scene(480, 320, 0.0, false, false));
        let shifted = t(&scene(480, 320, 160.0, false, false));
        let same = t(&scene(480, 320, 0.0, false, false));
        assert_eq!(relation(&a, &shifted), Relation::Shifted);
        let l = link(&a, &shifted);
        assert!(l.dx > 0.25 && l.dx < 0.4 && l.dy.abs() < 0.05, "a pan to the right by a third: {l:?}");
        assert_eq!(relation(&a, &same), Relation::Same);
        // a flat frame says nothing
        let flat = t(&Rgb32f::from_fn(480, 320, |_, _| [0.2, 0.2, 0.2]));
        assert_eq!(relation(&a, &flat), Relation::Different);
        // another shape
        let tall = t(&scene(320, 480, 0.0, false, false));
        assert_eq!(relation(&a, &tall), Relation::Different);
        // the sharp side moves: a focus stack; a burst keeps it
        let near = t(&scene(480, 320, 0.0, false, true));
        let far = t(&scene(480, 320, 0.0, true, false));
        assert!(focus_moves(&[&near, &same, &far]));
        assert!(!focus_moves(&[&a, &same, &same]));
        let mut thumbs = HashMap::new();
        for (id, th) in [1, 2, 3].iter().zip([near, same.clone(), far]) {
            thumbs.insert(PhotoId(*id), th);
        }
        let stack = run_of(&[(1, 0.0, "1/60", None), (2, 1.0, "1/60", None), (3, 2.0, "1/60", None)]);
        let got = decide(&stack, &thumbs);
        assert_eq!(kinds(&got), vec![(MergeType::Stack, vec![PhotoId(1), PhotoId(2), PhotoId(3)])]);
        assert_eq!(got[0].confidence, Confidence::Likely);
        // a panorama: shifted frames at one exposure
        for (k, id) in [11, 12, 13].iter().enumerate() {
            thumbs.insert(PhotoId(*id), t(&scene(480, 320, 150.0 * k as f32, false, false)));
        }
        let pano = run_of(&[(11, 0.0, "1/250", None), (12, 2.0, "1/250", None), (13, 4.0, "1/250", None)]);
        let got = decide(&pano, &thumbs);
        assert_eq!(kinds(&got), vec![(MergeType::Pano, vec![PhotoId(11), PhotoId(12), PhotoId(13)])]);
        assert_eq!(got[0].confidence, Confidence::Sure, "{:?}", got[0]);
        assert!(got[0].reasons.iter().any(|r| r.starts_with("each frame overlaps the last by ~")), "{:?}", got[0].reasons);
        assert!(got[0].reasons.iter().any(|r| r == "frames move left → right"), "{:?}", got[0].reasons);
        // a burst: nothing
        for id in [21, 22, 23] {
            thumbs.insert(PhotoId(id), same.clone());
        }
        assert!(decide(&run_of(&[(21, 0.0, "1/250", None), (22, 0.3, "1/250", None), (23, 0.6, "1/250", None)]), &thumbs).is_empty());
    }

    /// Bryan's panoramas (2026-11-24): handheld, in aperture priority — the shutter drifts from
    /// frame to frame, sometimes by a stop. Shifted framing makes them a panorama, never an HDR;
    /// a bracket is one framing; brackets at shifted framings are an HDR panorama.
    #[test]
    fn drifting_exposure_panoramas_brackets_and_hdr_panoramas() {
        let t = |img: &Rgb32f| Thumb::of(img).unwrap();
        let mut thumbs = HashMap::new();
        // a pano whose shutter drifts 1/250 → 1/320 → 1/200 → 1/250
        for (k, ev) in [0.0f32, -0.33, 0.33, 0.0].iter().enumerate() {
            thumbs.insert(PhotoId(1 + k as u64), t(&brighter(&scene(480, 320, 140.0 * k as f32, false, false), *ev)));
        }
        let drift = run_of(&[(1, 0.0, "1/250", None), (2, 1.5, "1/320", None), (3, 3.0, "1/200", None), (4, 4.4, "1/250", None)]);
        let got = decide(&drift, &thumbs);
        assert_eq!(kinds(&got), vec![(MergeType::Pano, (1..=4).map(PhotoId).collect())], "{got:?}");
        assert!(got[0].reasons.iter().any(|r| r.starts_with("exposure drifts by")), "{:?}", got[0].reasons);
        // a "bracket" in the metadata (1/500, 1/250, 1/125: panning into the shade) whose frames
        // are shifted: a panorama
        for (k, ev) in [-1.0f32, 0.0, 1.0].iter().enumerate() {
            thumbs.insert(PhotoId(11 + k as u64), t(&brighter(&scene(480, 320, 160.0 * k as f32, false, false), *ev)));
        }
        let shifted = run_of(&[(11, 0.0, "1/500", None), (12, 0.5, "1/250", None), (13, 1.0, "1/125", None)]);
        assert_eq!(kinds(&decide(&shifted, &thumbs)), vec![(MergeType::Pano, vec![PhotoId(11), PhotoId(12), PhotoId(13)])]);
        // a true bracket, shutter only (manual mode): one framing → hdr., sure
        for (k, ev) in [-2.0f32, 0.0, 2.0].iter().enumerate() {
            thumbs.insert(PhotoId(21 + k as u64), t(&brighter(&scene(480, 320, 0.0, false, false), *ev * 0.5)));
        }
        let bracket = run_of(&[(21, 0.0, "1/500", None), (22, 0.3, "1/125", None), (23, 0.6, "1/30", None)]);
        let got = decide(&bracket, &thumbs);
        assert_eq!(kinds(&got), vec![(MergeType::Hdr, vec![PhotoId(21), PhotoId(22), PhotoId(23)])]);
        assert_eq!(got[0].confidence, Confidence::Sure);
        assert!(got[0].reasons.contains(&"same framing".to_string()), "{:?}", got[0].reasons);
        assert_eq!(got[0].reasons[1], "exposures \u{2212}2 / 0 / +2");
        // brackets repeated at shifted framings: one HDR panorama
        let mut frames = Vec::new();
        for pos in 0..3u64 {
            for (j, (sh, ev)) in [("1/125", 0.0f32), ("1/500", -2.0), ("1/30", 2.0)].iter().enumerate() {
                let id = 31 + pos * 3 + j as u64;
                thumbs.insert(PhotoId(id), t(&brighter(&scene(480, 320, 150.0 * pos as f32, false, false), *ev * 0.4)));
                frames.push((id, pos as f64 * 3.0 + j as f64 * 0.3, *sh, Some(*ev)));
            }
        }
        let got = decide(&run_of(&frames), &thumbs);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].kind, MergeType::HdrPano);
        assert_eq!(got[0].photos.len(), 9);
        assert!(got[0].reasons.contains(&"a bracket at each of 3 positions".to_string()), "{:?}", got[0].reasons);
    }

    /// A borderline panorama (little overlap, a poor match) is found as maybe. rather than missed;
    /// two unrelated frames fired together are nothing.
    #[test]
    fn borderline_panoramas_are_maybe_and_unrelated_frames_nothing() {
        let t = |img: &Rgb32f| Thumb::of(img).unwrap();
        let mut thumbs = HashMap::new();
        thumbs.insert(PhotoId(1), t(&scene(480, 320, 0.0, false, false)));
        // a third of the frame in common, and noisy
        let base = scene(480, 320, 300.0, false, false);
        // half of it hidden by something else (clouds moving, a person walking by)
        let noisy = Rgb32f::from_fn(480, 320, |x, y| {
            let n = 0.2 * ((x as f32 * 0.09).sin() * (y as f32 * 0.07).cos()) + 0.1 * ((x as f32 * 0.023 + y as f32 * 0.051).sin());
            base.data[y * 480 + x].map(|v| (v * 0.6 + n).max(0.0))
        });
        thumbs.insert(PhotoId(2), t(&noisy));
        let got = decide(&run_of(&[(1, 0.0, "1/250", None), (2, 2.0, "1/250", None)]), &thumbs);
        assert_eq!(kinds(&got), vec![(MergeType::Pano, vec![PhotoId(1), PhotoId(2)])], "{got:?}");
        assert!(got[0].confidence < Confidence::Sure, "{:?}", got[0]);
        // something else entirely: a different scene
        let other = Rgb32f::from_fn(480, 320, |x, y| {
            let v = 0.3 + 0.2 * ((x as f32 * 0.21).sin() * (y as f32 * 0.17).cos());
            [v, v, v]
        });
        thumbs.insert(PhotoId(3), t(&other));
        assert!(decide(&run_of(&[(1, 0.0, "1/250", None), (3, 2.0, "1/250", None)]), &thumbs).is_empty());
    }

    #[test]
    fn tiny_or_odd_images_never_panic() {
        assert!(Thumb::of(&Rgb32f::new(2, 2)).is_none());
        let nan = Rgb32f::from_fn(40, 30, |_, _| [f32::NAN, f32::INFINITY, -1.0]);
        let t = Thumb::of(&nan).unwrap();
        assert_eq!(relation(&t, &t), Relation::Different);
        let bad = Thumb { w: 10, h: 10, px: vec![0.0; 3], sharp: [0.0; 16] };
        assert_eq!(relation(&bad, &bad), Relation::Different);
    }

    #[test]
    fn group_ids_are_stable_and_scope_reads_params() {
        assert_eq!(group_id(&[PhotoId(3), PhotoId(1)]), group_id(&[PhotoId(1), PhotoId(3), PhotoId(3)]));
        assert_ne!(group_id(&[PhotoId(1)]), group_id(&[PhotoId(2)]));
        assert_eq!(Scope::from_params(&json!({})).unwrap(), Scope::All);
        assert_eq!(Scope::from_params(&json!({"path": "/a", "deep": true})).unwrap(), Scope::Folder { path: "/a".into(), deep: true });
        assert_eq!(Scope::from_params(&json!({"album": 4})).unwrap(), Scope::Album { id: AlbumId(4) });
        assert_eq!(Scope::from_params(&json!({"scope": {"kind": "album", "id": 4}})).unwrap(), Scope::Album { id: AlbumId(4) });
        assert!(Scope::from_params(&json!({"scope": {"kind": "nope"}})).is_err());
    }
}
