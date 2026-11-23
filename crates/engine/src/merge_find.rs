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

/// One found merge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeGroup {
    /// [`group_id`] of the frames it was found with (stable: the same frames, the same id).
    pub id: String,
    pub kind: MergeType,
    /// Frames in capture order.
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
}

impl MergeGroup {
    pub fn key(&self) -> String {
        format!("{GROUP_PREFIX}{}", self.id)
    }
    /// The op that stores it.
    pub fn op(&self) -> Op {
        Op::SetRecord { key: self.key(), value: serde_json::to_value(self).ok() }
    }
    /// Its frames that are shown: still in the library, not folded into a merge.
    pub fn live_frames(&self, cat: &Catalog) -> Vec<PhotoId> {
        self.photos.iter().copied().filter(|id| cat.photo(*id).is_some_and(|p| frame_shown(p))).collect()
    }
    /// The merged photo, while it is in the library (not deleted).
    pub fn live_result(&self, cat: &Catalog) -> Option<PhotoId> {
        self.result.filter(|r| cat.photo(*r).is_some_and(|p| !p.deleted && !p.local))
    }
    /// Shown in merges.: open with two frames or more left, or merged with its photo there.
    pub fn is_live(&self, cat: &Catalog) -> bool {
        match self.state {
            GroupState::Open => self.live_frames(cat).len() >= 2,
            GroupState::Merged => self.live_result(cat).is_some(),
            GroupState::Dismissed => false,
        }
    }
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
        })
    }
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
                if *deep { lightcraft_catalog::query::folder_within(&g.folder, &root) } else { g.folder.trim_end_matches('/') == root.trim_end_matches('/') }
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

/// The merges. view of `scope`: each open group's frames (oldest group first), then the merged
/// photos.
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
                let ids: Vec<PhotoId> = g.live_frames(cat).into_iter().filter(|i| !seen.contains(i)).collect();
                if ids.len() < 2 {
                    continue;
                }
                seen.extend(ids.iter().copied());
                let first = ids.first().copied().map(date).unwrap_or_default();
                open.push((
                    first,
                    ViewRun { key: format!("merges:group:{}", g.id), label: g.kind.tag().to_string(), group: Some(g.id.clone()), kind: Some(g.kind), ids },
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

fn shot(p: &Photo) -> Option<Shot> {
    if !p.in_library() || p.kind == MediaKind::Video || p.copy_of.is_some() || p.merge_type().is_some() {
        return None;
    }
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

/// A found merge, before it is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub kind: MergeType,
    pub photos: Vec<PhotoId>,
    pub folder: String,
}

/// Frames whose kind only their pixels tell.
#[derive(Clone, Debug, PartialEq)]
pub enum Look {
    /// Frames at one exposure: a panorama, a focus stack, or a burst (nothing).
    Same { folder: String, photos: Vec<PhotoId> },
    /// Brackets one after another: one HDR panorama when the framing shifts between them, else
    /// one HDR each.
    Brackets { folder: String, brackets: Vec<Vec<PhotoId>> },
}

impl Look {
    /// The photos whose thumbnails [`decide`] needs.
    pub fn photos(&self) -> Vec<PhotoId> {
        match self {
            Look::Same { photos, .. } => photos.clone(),
            Look::Brackets { brackets, .. } => brackets.iter().filter_map(|b| b.first().copied()).collect(),
        }
    }
}

/// What metadata alone says about some photos: merges it is sure of, and frames to look at.
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

/// Read the photos `ids` (and the groups already found: their frames are never suggested again)
/// into a [`Plan`].
pub fn plan(cat: &Catalog, ids: &[PhotoId]) -> Plan {
    let taken: HashSet<PhotoId> = groups(cat).iter().flat_map(|g| g.photos.iter().copied().chain(g.result)).collect();
    let mut shots: Vec<Shot> = ids.iter().filter(|id| !taken.contains(id)).filter_map(|id| cat.photo(*id)).filter_map(|p| shot(p)).collect();
    shots.sort_by(|a, b| a.folder.cmp(&b.folder).then(a.t.total_cmp(&b.t)).then(a.id.cmp(&b.id)));
    shots.dedup_by_key(|s| s.id);
    let mut out = Plan::default();
    let mut start = 0;
    for i in 1..=shots.len() {
        let breaks = match (shots.get(i - 1), shots.get(i)) {
            (Some(a), Some(b)) => {
                let gap = RUN_GAP + a.shutter.unwrap_or(0.0).min(60.0);
                !a.same_rig(b) || b.t - a.t > gap || b.t < a.t
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

/// A piece of a run.
enum Piece {
    Bracket(Vec<usize>),
    Same(Vec<usize>),
}

fn plan_run(run: &[Shot], out: &mut Plan) {
    let folder = run.first().map(|s| s.folder.clone()).unwrap_or_default();
    let n = run.len();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut i = 0;
    while i < n {
        // a bracket from i: each next frame at a level not taken yet, at the same aperture and ISO
        let mut j = i + 1;
        while j < n && j - i < MAX_BRACKET && run[j].same_base(&run[i]) && (i..j).all(|m| (run[j].level() - run[m].level()).abs() >= 0.25) {
            j += 1;
        }
        let levels: Vec<f64> = (i..j).map(|k| run[k].level()).collect();
        let spread = levels.iter().copied().fold(f64::MIN, f64::max) - levels.iter().copied().fold(f64::MAX, f64::min);
        if j - i >= 2 && spread >= MIN_SPREAD {
            pieces.push(Piece::Bracket((i..j).collect()));
            i = j;
            continue;
        }
        // frames at one exposure from i
        let mut j = i + 1;
        while j < n && run[j].same_exposure(&run[i]) {
            j += 1;
        }
        if j - i >= 2 {
            pieces.push(Piece::Same((i..j).collect()));
        }
        i = j.max(i + 1);
    }
    let ids = |v: &[usize]| v.iter().filter_map(|k| run.get(*k)).map(|s| s.id).collect::<Vec<_>>();
    let mut k = 0;
    while k < pieces.len() {
        match &pieces[k] {
            Piece::Same(v) => {
                let dists: Vec<f32> = v.iter().filter_map(|i| run[*i].dist).collect();
                let distinct = {
                    let mut d: Vec<i64> = dists.iter().map(|d| (d * 100.0).round() as i64).collect();
                    d.sort_unstable();
                    d.dedup();
                    d.len()
                };
                if v.len() >= 3 && dists.len() == v.len() && distinct >= 3 {
                    out.sure.push(Candidate { kind: MergeType::Stack, photos: ids(v), folder: folder.clone() });
                } else {
                    out.looks.push(Look::Same { folder: folder.clone(), photos: ids(v) });
                }
                k += 1;
            }
            Piece::Bracket(first) => {
                // brackets of the same pattern one after another
                let pattern = |v: &[usize]| v.iter().map(|i| run[*i].level()).collect::<Vec<f64>>();
                let p0 = pattern(first);
                let mut brackets = vec![ids(first)];
                let mut m = k + 1;
                while let Some(Piece::Bracket(next)) = pieces.get(m) {
                    let p = pattern(next);
                    if p.len() != p0.len() || p.iter().zip(&p0).any(|(a, b)| (a - b).abs() >= 0.3) {
                        break;
                    }
                    brackets.push(ids(next));
                    m += 1;
                }
                if brackets.len() >= 2 {
                    out.looks.push(Look::Brackets { folder: folder.clone(), brackets });
                } else {
                    out.sure.push(Candidate { kind: MergeType::Hdr, photos: ids(first), folder: folder.clone() });
                }
                k = m;
            }
        }
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Relation {
    /// Overlapping, but shifted: panorama neighbours.
    Shifted,
    /// The same framing.
    Same,
    Different,
}

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

/// How `b`'s framing relates to `a`'s (thumbnails of one shape; another shape is different).
pub fn relation(a: &Thumb, b: &Thumb) -> Relation {
    if a.w != b.w || a.h != b.h || a.px.len() != a.w * a.h || b.px.len() != b.w * b.h {
        return Relation::Different;
    }
    let zero = ncc(a, b, 0, 0, 0.9).unwrap_or(0.0);
    let (rx, ry) = ((a.w as f64 * 0.65) as i64, (a.h as f64 * 0.65) as i64);
    let mut best = (0i64, 0i64, f64::MIN);
    for dy in -ry..=ry {
        for dx in -rx..=rx {
            if let Some(r) = ncc(a, b, dx, dy, 0.3)
                && r > best.2
            {
                best = (dx, dy, r);
            }
        }
    }
    let shift = (best.0.abs() as f64 / a.w as f64).max(best.1.abs() as f64 / a.h as f64);
    if shift >= 0.1 && best.2 >= 0.6 && best.2 >= zero + 0.15 {
        Relation::Shifted
    } else if zero >= 0.85 {
        Relation::Same
    } else {
        Relation::Different
    }
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

/// What frames to look at are, from their thumbnails (a missing one breaks the frames apart).
pub fn decide(look: &Look, thumbs: &HashMap<PhotoId, Thumb>) -> Vec<Candidate> {
    let rel = |a: PhotoId, b: PhotoId| match (thumbs.get(&a), thumbs.get(&b)) {
        (Some(x), Some(y)) => relation(x, y),
        _ => Relation::Different,
    };
    let mut out = Vec::new();
    match look {
        Look::Same { folder, photos } => {
            let rels: Vec<Relation> = photos.windows(2).map(|w| rel(w[0], w[1])).collect();
            // runs of one relation: shifted → panorama, same framing with focus moving → stack
            let mut i = 0;
            while i < rels.len() {
                let r = rels[i];
                let mut j = i;
                while j < rels.len() && rels[j] == r {
                    j += 1;
                }
                let frames: Vec<PhotoId> = photos.get(i..=j).map(<[PhotoId]>::to_vec).unwrap_or_default();
                match r {
                    Relation::Shifted if frames.len() >= 2 => out.push(Candidate { kind: MergeType::Pano, photos: frames, folder: folder.clone() }),
                    Relation::Same if frames.len() >= 3 => {
                        let ts: Vec<&Thumb> = frames.iter().filter_map(|f| thumbs.get(f)).collect();
                        if ts.len() == frames.len() && focus_moves(&ts) {
                            out.push(Candidate { kind: MergeType::Stack, photos: frames, folder: folder.clone() });
                        }
                    }
                    _ => {}
                }
                i = j;
            }
        }
        Look::Brackets { folder, brackets } => {
            let firsts: Vec<PhotoId> = brackets.iter().filter_map(|b| b.first().copied()).collect();
            let shifted: Vec<bool> = firsts.windows(2).map(|w| rel(w[0], w[1]) == Relation::Shifted).collect();
            let mut i = 0;
            while i < brackets.len() {
                let mut j = i;
                while j < shifted.len() && shifted[j] {
                    j += 1;
                }
                // brackets i..=j are one shifted series
                if j > i {
                    let photos: Vec<PhotoId> = brackets.get(i..=j).map(|b| b.concat()).unwrap_or_default();
                    out.push(Candidate { kind: MergeType::HdrPano, photos, folder: folder.clone() });
                } else if let Some(b) = brackets.get(i) {
                    out.push(Candidate { kind: MergeType::Hdr, photos: b.clone(), folder: folder.clone() });
                }
                i = j + 1;
            }
        }
    }
    out
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

// ------------------------------------------------------------------------------------- session

/// A finder run on a worker thread (the app): thumbnails are loaded and compared off the UI
/// thread as background work; [`Session::merge_find_poll`] stores the groups when it is done.
pub struct FindJob {
    rx: std::sync::mpsc::Receiver<Vec<Candidate>>,
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
            return self.merge_found(found, scanned);
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
                    Some(found)
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
        let found = match job.rx.try_recv() {
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
        Some(self.merge_found(found, job.scanned.clone()))
    }

    /// The automatic finder (Settings ▸ find merges: auto.): start a background run over the
    /// folders whose photos changed since it last looked (all of them the first time). `None`:
    /// nothing new, or a run is going.
    pub fn merge_find_auto(&mut self) -> Option<Result<Value>> {
        if self.merge_find_job.is_some() {
            return None;
        }
        let now = fingerprints(&self.catalog);
        let seen = scanned(&self.catalog);
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
    /// shares a frame with a group already there; note the folders looked at.
    pub fn merge_found(&mut self, found: Vec<Candidate>, looked: BTreeMap<String, u64>) -> Result<Value> {
        let mut taken: HashSet<PhotoId> = groups(&self.catalog).iter().flat_map(|g| g.photos.iter().copied().chain(g.result)).collect();
        let now = (self.clock)();
        let mut ops = Vec::new();
        let mut out = Vec::new();
        for c in found {
            if c.photos.len() < 2 || c.photos.iter().any(|p| taken.contains(p) || self.catalog.photo(*p).is_none_or(|p| !p.in_library())) {
                continue;
            }
            taken.extend(c.photos.iter().copied());
            let g = MergeGroup { id: group_id(&c.photos), kind: c.kind, photos: c.photos, state: GroupState::Open, result: None, folder: c.folder, found: now.clone() };
            ops.push(g.op());
            out.push(g.to_json());
        }
        if !looked.is_empty() {
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
            let r = crate::merge::parse(k.command(), &params)
                .and_then(|(mk, finish)| self.merge_now(mk, finish, &frames))
                .and_then(|info| {
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

    /// Skip groups: never suggested again (one undo step).
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
            self.commit("Skip Merge", Op::Batch { ops })?;
        }
        Ok(json!({"dismissed": n}))
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
        assert_eq!(plan.sure, vec![Candidate { kind: MergeType::Hdr, photos: vec![a, b, c], folder: "/shoot".into() }]);
        assert!(plan.looks.is_empty(), "{:?}", plan.looks);
    }

    #[test]
    fn repeated_brackets_and_one_exposure_runs_need_a_look() {
        let mut cat = Catalog::new();
        let mut first = Vec::new();
        for (k, t) in ["10:00:00", "10:00:03"].iter().enumerate() {
            for (j, (s, b)) in [("1/125", 0.0), ("1/500", -2.0), ("1/30", 2.0)].iter().enumerate() {
                let id = photo(&mut cat, &format!("b{k}{j}.dng"), &format!("2026-11-01T{t}"), s, Some(*b));
                if j == 0 {
                    first.push(id);
                }
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
        assert!(matches!(&plan.looks[0], Look::Brackets { brackets, .. } if brackets.len() == 2));
        assert_eq!(plan.looks[1], Look::Same { folder: "/shoot".into(), photos: pano.clone() });
        assert_eq!(plan.to_load().len(), 6, "the first frame of each bracket and the pano's frames");
        // without thumbnails: the brackets are one HDR each, the pano is nothing
        let none = HashMap::new();
        let found: Vec<MergeType> = plan.looks.iter().flat_map(|l| decide(l, &none)).map(|c| c.kind).collect();
        assert_eq!(found, vec![MergeType::Hdr, MergeType::Hdr]);
        let _ = first;
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

    #[test]
    fn thumbnails_tell_a_pano_from_a_burst_and_a_focus_stack() {
        let t = |img: &Rgb32f| Thumb::of(img).unwrap();
        let a = t(&scene(480, 320, 0.0, false, false));
        let shifted = t(&scene(480, 320, 160.0, false, false));
        let same = t(&scene(480, 320, 0.0, false, false));
        assert_eq!(relation(&a, &shifted), Relation::Shifted);
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
        let ids: Vec<PhotoId> = (1..=3).map(PhotoId).collect();
        let mut thumbs = HashMap::new();
        for (id, th) in ids.iter().zip([near, same.clone(), far]) {
            thumbs.insert(*id, th);
        }
        let look = Look::Same { folder: "/s".into(), photos: ids.clone() };
        assert_eq!(decide(&look, &thumbs), vec![Candidate { kind: MergeType::Stack, photos: ids.clone(), folder: "/s".into() }]);
        let pano: Vec<PhotoId> = (11..=13).map(PhotoId).collect();
        for (k, id) in pano.iter().enumerate() {
            thumbs.insert(*id, t(&scene(480, 320, 150.0 * k as f32, false, false)));
        }
        let look = Look::Same { folder: "/s".into(), photos: pano.clone() };
        assert_eq!(decide(&look, &thumbs), vec![Candidate { kind: MergeType::Pano, photos: pano.clone(), folder: "/s".into() }]);
        // a burst: nothing
        let burst: Vec<PhotoId> = (21..=23).map(PhotoId).collect();
        for id in &burst {
            thumbs.insert(*id, same.clone());
        }
        assert!(decide(&Look::Same { folder: "/s".into(), photos: burst }, &thumbs).is_empty());
        // brackets at shifted framings: one HDR panorama
        let br = vec![vec![PhotoId(11), PhotoId(31)], vec![PhotoId(12), PhotoId(32)]];
        let got = decide(&Look::Brackets { folder: "/s".into(), brackets: br }, &thumbs);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, MergeType::HdrPano);
        assert_eq!(got[0].photos.len(), 4);
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
