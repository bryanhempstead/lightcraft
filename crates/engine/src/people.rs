//! People (Lightroom Classic's People view): named people, the unnamed faces grouped into stacks of the same
//! person, and suggestions — an unnamed face that looks like someone already named.
//!
//! A person is a name on face regions ([`lightcraft_meta::Region::name`]) and, as in Classic, a **person keyword**
//! on their photos (`KeywordInfo::person`: exported or not by the keyword's own Include on Export option, left out
//! with Remove Person Info). Naming a face, confirming a suggestion (✓) or rejecting one (✗, remembered on the
//! region so it isn't suggested again) are ordinary undoable edits.
//!
//! Matching uses the faces index ([`crate::vision`]): each region's embedding is the indexed face at the same
//! place. Unnamed faces are grouped by [`lightcraft_vision::cluster::cluster`] over every indexed face of the
//! library; the grouping is recomputed only when the index changes (on a worker thread in the app), so naming a
//! face just takes it out of its stack.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, mpsc};

use lightcraft_catalog::{Filter, Op, PhotoId};
use lightcraft_meta::{Rect, RegionKind};
use lightcraft_vision::cluster;
use lightcraft_vision::index::FaceRecord;
use serde::Serialize;

use crate::vision::entry_for;
use crate::{EngineError, Result, Session};

/// One face region: the photo and its position in the photo's regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize)]
pub struct FaceRef {
    pub photo: PhotoId,
    pub index: usize,
}

/// A face as the People view shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FaceInfo {
    pub photo: PhotoId,
    pub index: usize,
    /// Normalized, in the photo's upright frame.
    pub rect: Rect,
    /// The name it's confirmed as.
    pub name: Option<String>,
    /// For an unnamed face: who it most likely is, and how sure (0..1).
    pub suggestion: Option<String>,
    pub score: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NamedPerson {
    pub name: String,
    /// Photos they are confirmed in.
    pub photos: usize,
    /// The face shown on their card (their largest).
    pub cover: FaceInfo,
    pub faces: Vec<FaceInfo>,
    /// Unnamed faces that look like them, best first (Classic's Similar).
    pub similar: Vec<FaceInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Stack {
    /// The face shown on the stack (its largest).
    pub cover: FaceInfo,
    pub faces: Vec<FaceInfo>,
    /// The name most of its faces are suggested as.
    pub suggestion: Option<String>,
}

/// The People view of the photos a filter lets through.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct People {
    pub named: Vec<NamedPerson>,
    pub unnamed: Vec<Stack>,
    /// Unnamed faces not in the faces index yet (no stack until they are).
    pub unindexed: usize,
    /// Faces in view.
    pub faces: usize,
}

/// Where a face's embedding is in the index: the photo's content key and the face's position in its record.
type EntryId = (String, u16);

/// Clusters of the whole index, by entry.
#[derive(Default)]
pub struct Clusters {
    of: HashMap<EntryId, u32>,
}

/// The People view's caches (on the session).
#[derive(Default)]
pub struct PeopleCache {
    clusters: Option<(u64, Arc<Clusters>)>,
    pending: Option<(u64, mpsc::Receiver<Clusters>)>,
    /// The grouping before there is one (one value, so the view cache keeps).
    empty: Arc<Clusters>,
    /// When the last regrouping started (while indexing, at most every few seconds).
    started: Option<web_time::Instant>,
    /// The last view: catalog revision, index generation, grouping, filter.
    view: Option<(u64, u64, usize, String, Arc<People>)>,
}

impl PeopleCache {
    /// A clustering is being computed.
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

fn cluster_index(entries: Vec<(EntryId, Vec<f32>)>) -> Clusters {
    let refs: Vec<&[f32]> = entries.iter().map(|(_, e)| e.as_slice()).collect();
    // faces of one photo are different people
    let owners: Vec<u64> = entries
        .iter()
        .map(|((key, _), _)| key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)))
        .collect();
    let groups = cluster::cluster(&refs, &owners, cluster::CLUSTER);
    let mut of = HashMap::new();
    for (g, members) in groups.iter().enumerate() {
        for &i in members {
            if let Some((id, _)) = entries.get(i) {
                of.insert(id.clone(), g as u32);
            }
        }
    }
    Clusters { of }
}

fn face_area(r: &Rect) -> f64 {
    (r.x1 - r.x0).max(0.0) * (r.y1 - r.y0).max(0.0)
}

impl Session {
    /// The clustering of the faces index, recomputed when the index changed (in the background with
    /// `vision.background`: the previous one is used until it's done).
    fn clusters(&mut self) -> Arc<Clusters> {
        let generation = self.vision.generation;
        if let Some((g, rx)) = &self.people_cache.pending {
            match rx.try_recv() {
                Ok(c) => {
                    let g = *g;
                    self.people_cache.clusters = Some((g, Arc::new(c)));
                    self.people_cache.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.people_cache.pending = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let current = self.people_cache.clusters.as_ref().is_some_and(|(g, _)| *g == generation);
        // while faces are being found, regroup every few seconds rather than after every photo
        let due = self.people_cache.clusters.is_none()
            || !self.vision.busy()
            || self.people_cache.started.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(8));
        if !current && self.people_cache.pending.is_none() && due {
            self.people_cache.started = Some(web_time::Instant::now());
            let entries: Vec<(EntryId, Vec<f32>)> = match self.faces_index() {
                Ok(idx) => {
                    let idx = idx.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut v: Vec<(EntryId, Vec<f32>)> = idx
                        .iter()
                        .flat_map(|(k, r)| r.faces.iter().enumerate().map(move |(i, f)| ((k.clone(), i as u16), f.embedding.clone())))
                        .filter(|(_, e)| e.len() == lightcraft_vision::sface::DIM)
                        .collect();
                    // a stable order: the same index always clusters the same way
                    v.sort_by(|a, b| a.0.cmp(&b.0));
                    v
                }
                Err(_) => Vec::new(),
            };
            if self.vision.background && entries.len() > 500 {
                let (tx, rx) = mpsc::channel();
                let spawned = std::thread::Builder::new().name("people-cluster".into()).spawn(move || {
                    let _ = tx.send(cluster_index(entries));
                });
                if spawned.is_ok() {
                    self.people_cache.pending = Some((generation, rx));
                }
            } else {
                self.people_cache.clusters = Some((generation, Arc::new(cluster_index(entries))));
            }
        }
        self.people_cache.clusters.as_ref().map(|(_, c)| c.clone()).unwrap_or_else(|| self.people_cache.empty.clone())
    }

    /// The People view for the photos `filter` lets through (its `person` ignored). Cached by catalog revision and
    /// index generation.
    pub fn people(&mut self, filter: &Filter) -> Arc<People> {
        let key = format!("{filter:?}");
        let clusters = self.clusters();
        let (rev, generation, grouping) = (self.catalog.revision, self.vision.generation, Arc::as_ptr(&clusters) as usize);
        if let Some((r, g, c, k, v)) = &self.people_cache.view
            && *r == rev
            && *g == generation
            && *c == grouping
            && *k == key
        {
            return v.clone();
        }
        let people = Arc::new(self.compute_people(filter, &clusters));
        self.people_cache.view = Some((rev, generation, grouping, key, people.clone()));
        people
    }

    fn compute_people(&mut self, filter: &Filter, clusters: &Clusters) -> People {
        let filter = Filter { person: None, ..filter.clone() };
        let index = self.faces_index().ok();
        let index = index.as_ref().map(|i| i.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let record_of = |key: &str| -> Option<&FaceRecord> { index.as_ref().and_then(|i| i.get(key)) };
        // every named face of the library (the people suggestions are made from), and the faces in view
        let mut known: BTreeMap<String, (String, Vec<Vec<f32>>)> = BTreeMap::new();
        struct Seen {
            face: FaceInfo,
            entry: Option<(EntryId, Vec<f32>)>,
            /// The photo is in the faces index.
            indexed: bool,
            rejected: Vec<String>,
            area: f64,
        }
        let mut in_view: Vec<Seen> = Vec::new();
        for p in self.catalog.photos().filter(|p| p.in_library()) {
            let key = crate::media::content_key(p);
            let record = record_of(&key);
            let shown = filter.matches(p, &self.catalog);
            for (i, r) in p.meta.regions.iter().enumerate() {
                if r.kind != RegionKind::Face {
                    continue;
                }
                let name = r.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
                let entry = record.and_then(|rec| {
                    let e = entry_for(rec, &r.rect)?;
                    let at = rec.faces.iter().position(|f| std::ptr::eq(f, e))?;
                    Some(((key.clone(), at as u16), e.embedding.clone()))
                });
                if let (Some(n), Some((_, e))) = (&name, &entry) {
                    known.entry(n.to_lowercase()).or_insert_with(|| (n.clone(), Vec::new())).1.push(e.clone());
                }
                if shown {
                    in_view.push(Seen {
                        face: FaceInfo { photo: p.id, index: i, rect: r.rect, name, suggestion: None, score: None },
                        entry,
                        indexed: record.is_some(),
                        rejected: r.rejected.iter().map(|n| n.to_lowercase()).collect(),
                        area: face_area(&r.rect) * f64::from(p.width) * f64::from(p.height),
                    });
                }
            }
        }
        // suggestions: centroid first (cheap), then the person's best matches
        let people: Vec<(&String, &(String, Vec<Vec<f32>>))> = known.iter().collect();
        let centroids: Vec<Vec<f32>> = people
            .iter()
            .map(|(_, (_, es))| {
                let mut c = vec![0f32; lightcraft_vision::sface::DIM];
                for e in es {
                    for (a, b) in c.iter_mut().zip(e) {
                        *a += b;
                    }
                }
                lightcraft_vision::sface::normalize(c)
            })
            .collect();
        for s in in_view.iter_mut().filter(|s| s.face.name.is_none()) {
            let Some((_, e)) = &s.entry else { continue };
            let mut best: Option<(usize, f32)> = None;
            for (pi, c) in centroids.iter().enumerate() {
                if lightcraft_vision::sface::cosine(c, e) < cluster::SUGGEST - 0.12 || s.rejected.contains(people[pi].0) {
                    continue;
                }
                let known: Vec<&[f32]> = people[pi].1.1.iter().map(Vec::as_slice).collect();
                let score = cluster::match_score(e, &known);
                if score >= cluster::SUGGEST && best.is_none_or(|(_, b)| score > b) {
                    best = Some((pi, score));
                }
            }
            if let Some((pi, score)) = best {
                s.face.suggestion = Some(people[pi].1.0.clone());
                s.face.score = Some((score * 1000.0).round() / 1000.0);
            }
        }
        let faces = in_view.len();
        // named people
        let mut named: BTreeMap<String, (NamedPerson, f64, std::collections::BTreeSet<PhotoId>)> = BTreeMap::new();
        for s in in_view.iter().filter(|s| s.face.name.is_some()) {
            let Some(name) = &s.face.name else { continue };
            let e = named.entry(name.to_lowercase()).or_insert_with(|| {
                (
                    NamedPerson { name: name.clone(), photos: 0, cover: s.face.clone(), faces: Vec::new(), similar: Vec::new() },
                    -1.0,
                    Default::default(),
                )
            });
            if s.area > e.1 {
                e.0.cover = s.face.clone();
                e.1 = s.area;
            }
            e.2.insert(s.face.photo);
            e.0.faces.push(s.face.clone());
        }
        // a person named only outside the view still gets their suggestions in it
        for s in in_view.iter().filter(|s| s.face.name.is_none()) {
            let Some(sug) = &s.face.suggestion else { continue };
            if let Some(e) = named.get_mut(&sug.to_lowercase()) {
                e.0.similar.push(s.face.clone());
            }
        }
        let mut named: Vec<NamedPerson> = named
            .into_values()
            .map(|(mut p, _, photos)| {
                p.photos = photos.len();
                p.similar.sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
                p
            })
            .collect();
        named.sort_by(|a, b| b.photos.cmp(&a.photos).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        // unnamed stacks
        let mut stacks: BTreeMap<u32, (Vec<&Seen>, f64, usize)> = BTreeMap::new();
        let mut unindexed = 0;
        let mut lone = u32::MAX;
        for s in in_view.iter().filter(|s| s.face.name.is_none()) {
            let g = match s.entry.as_ref().and_then(|(id, _)| clusters.of.get(id)) {
                Some(g) => *g,
                None if s.entry.is_some() => {
                    // indexed after the last clustering: on its own for now
                    lone -= 1;
                    lone
                }
                // an unnamed box from another app where the finder saw no face: not shown
                None if s.indexed => continue,
                None => {
                    unindexed += 1;
                    continue;
                }
            };
            let e = stacks.entry(g).or_insert_with(|| (Vec::new(), -1.0, 0));
            if s.area > e.1 {
                e.1 = s.area;
                e.2 = e.0.len();
            }
            e.0.push(s);
        }
        let mut unnamed: Vec<Stack> = stacks
            .into_values()
            .filter_map(|(members, _, cover)| {
                let cover = members.get(cover)?.face.clone();
                let mut votes: BTreeMap<String, (usize, String)> = BTreeMap::new();
                for m in &members {
                    if let Some(n) = &m.face.suggestion {
                        votes.entry(n.to_lowercase()).or_insert((0, n.clone())).0 += 1;
                    }
                }
                let suggestion = votes.into_values().filter(|(n, _)| n * 2 >= members.len()).max_by_key(|(n, _)| *n).map(|(_, s)| s);
                Some(Stack { cover, faces: members.iter().map(|m| m.face.clone()).collect(), suggestion })
            })
            .collect();
        unnamed.sort_by(|a, b| b.faces.len().cmp(&a.faces.len()).then(a.cover.photo.cmp(&b.cover.photo)));
        People { named, unnamed, unindexed, faces }
    }

    /// The faces `refs` names, checked.
    fn face_regions(&self, refs: &[FaceRef]) -> Result<()> {
        for r in refs {
            let p = self.catalog.photo(r.photo).ok_or(EngineError::Catalog(lightcraft_catalog::CatalogError::NoPhoto(r.photo)))?;
            if p.meta.regions.get(r.index).is_none_or(|g| g.kind != RegionKind::Face) {
                return Err(EngineError::Other(format!("photo {} has no face {}", r.photo.0, r.index)));
            }
        }
        Ok(())
    }

    /// The person keyword for `name`: an existing person keyword of that name anywhere in the keyword list
    /// (`People|Ann`), else `name` itself; and the op that marks it a person keyword when it isn't yet.
    fn person_keyword(&self, name: &str) -> (String, Option<Op>) {
        let lower = name.to_lowercase();
        let listed: Vec<&lightcraft_catalog::keywords::ListedKeyword> = self.catalog.listed_keywords().collect();
        if let Some(k) = listed.iter().find(|k| k.info.person && k.path.rsplit('|').next().is_some_and(|l| l.trim().to_lowercase() == lower)) {
            return (k.path.clone(), None);
        }
        let path = self.catalog.keyword_path(name).unwrap_or_else(|| name.trim().to_string());
        let mut info = self.catalog.keyword_info(&path).cloned().unwrap_or_default();
        if info.person && self.catalog.keyword_info(&path).is_some() {
            return (path, None);
        }
        info.person = true;
        (path.clone(), Some(Op::SetKeyword { path, info: Some(info) }))
    }

    /// Name the faces `refs` `name` (`None`: take their names off): their regions, and the person keyword on their
    /// photos (removed from a photo where no face keeps an old name). One undo step.
    pub fn name_faces(&mut self, refs: &[FaceRef], name: Option<&str>, label: &str) -> Result<usize> {
        self.face_regions(refs)?;
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        if let Some(n) = name
            && (n.contains('|') || !lightcraft_catalog::keywords::valid_name(n))
        {
            return Err(EngineError::Other(format!("“{n}” can't be a person's name (no | and no trailing *)")));
        }
        let mut ops = Vec::new();
        let keyword = name.map(|n| self.person_keyword(n));
        if let Some((_, Some(op))) = &keyword {
            ops.push(op.clone());
        }
        let mut by_photo: BTreeMap<PhotoId, Vec<usize>> = BTreeMap::new();
        for r in refs {
            by_photo.entry(r.photo).or_default().push(r.index);
        }
        for (id, idx) in by_photo {
            let Some(p) = self.catalog.photo(id) else { continue };
            let mut meta = p.meta.clone();
            let mut old: Vec<String> = Vec::new();
            for i in idx {
                if let Some(r) = meta.regions.get_mut(i) {
                    if let Some(o) = r.name.take() {
                        old.push(o);
                    }
                    if let Some(n) = name {
                        r.rejected.retain(|x| !x.eq_ignore_ascii_case(n));
                        r.name = Some(n.to_string());
                    }
                }
            }
            // old names nobody on the photo has any more lose their person keyword
            for o in old {
                if meta.regions.iter().any(|r| r.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&o))) {
                    continue;
                }
                let (path, _) = self.person_keyword(&o);
                if self.catalog.keyword_info(&path).is_some_and(|i| i.person) {
                    meta.keywords.retain(|k| !k.eq_ignore_ascii_case(&path));
                }
            }
            if let Some((path, _)) = &keyword
                && !meta.keywords.iter().any(|k| k.eq_ignore_ascii_case(path))
            {
                meta.keywords.push(path.clone());
            }
            ops.push(Op::SetMeta { id, meta: Box::new(meta) });
        }
        let n = refs.len();
        self.commit(label, Op::Batch { ops })?;
        Ok(n)
    }

    /// Confirm the suggestions on `refs` (each face as the person it's suggested as). Returns how many.
    pub fn confirm_faces(&mut self, refs: &[FaceRef], filter: &Filter) -> Result<usize> {
        let people = self.people(filter);
        let mut by_name: BTreeMap<String, Vec<FaceRef>> = BTreeMap::new();
        let all = people.named.iter().flat_map(|p| p.similar.iter()).chain(people.unnamed.iter().flat_map(|s| s.faces.iter()));
        for f in all {
            let r = FaceRef { photo: f.photo, index: f.index };
            if refs.contains(&r)
                && let Some(s) = &f.suggestion
            {
                by_name.entry(s.clone()).or_default().push(r);
            }
        }
        let mut n = 0;
        let steps = by_name.len();
        for (name, faces) in by_name {
            n += self.name_faces(&faces, Some(&name), "Confirm Person")?;
        }
        if steps > 1 {
            self.merge_undo(steps, "Confirm Person");
        }
        Ok(n)
    }

    /// Reject `name` (or each face's own suggestion) for `refs`: never suggested for them again. One undo step.
    pub fn reject_faces(&mut self, refs: &[FaceRef], name: Option<&str>, filter: &Filter) -> Result<usize> {
        self.face_regions(refs)?;
        let suggestions: HashMap<FaceRef, String> = if name.is_none() {
            let people = self.people(filter);
            people
                .named
                .iter()
                .flat_map(|p| p.similar.iter())
                .chain(people.unnamed.iter().flat_map(|s| s.faces.iter()))
                .filter_map(|f| Some((FaceRef { photo: f.photo, index: f.index }, f.suggestion.clone()?)))
                .collect()
        } else {
            HashMap::new()
        };
        let mut regions: BTreeMap<PhotoId, Vec<lightcraft_meta::Region>> = BTreeMap::new();
        let mut n = 0;
        for r in refs {
            let Some(who) = name.map(str::to_string).or_else(|| suggestions.get(r).cloned()) else { continue };
            let list = match regions.entry(r.photo) {
                std::collections::btree_map::Entry::Occupied(o) => o.into_mut(),
                std::collections::btree_map::Entry::Vacant(v) => match self.catalog.photo(r.photo) {
                    Some(p) => v.insert(p.meta.regions.clone()),
                    None => continue,
                },
            };
            if let Some(g) = list.get_mut(r.index)
                && !g.rejected.iter().any(|x| x.eq_ignore_ascii_case(&who))
            {
                g.rejected.push(who);
                n += 1;
            }
        }
        let ops: Vec<Op> = regions.into_iter().map(|(id, regions)| Op::SetRegions { id, regions }).collect();
        if !ops.is_empty() {
            self.commit("Reject Suggestion", Op::Batch { ops })?;
        }
        Ok(n)
    }

    /// Remove the face regions `refs` (and the person keyword of a named one, when no other face on the photo has
    /// that name). One undo step.
    pub fn remove_faces(&mut self, refs: &[FaceRef]) -> Result<usize> {
        self.face_regions(refs)?;
        let mut by_photo: BTreeMap<PhotoId, Vec<usize>> = BTreeMap::new();
        for r in refs {
            by_photo.entry(r.photo).or_default().push(r.index);
        }
        let mut ops = Vec::new();
        for (id, mut idx) in by_photo {
            let Some(p) = self.catalog.photo(id) else { continue };
            let mut meta = p.meta.clone();
            idx.sort_unstable();
            idx.dedup();
            let mut gone: Vec<String> = Vec::new();
            for i in idx.into_iter().rev() {
                if i < meta.regions.len() {
                    gone.extend(meta.regions.remove(i).name);
                }
            }
            for o in gone {
                if meta.regions.iter().any(|r| r.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&o))) {
                    continue;
                }
                let (path, _) = self.person_keyword(&o);
                if self.catalog.keyword_info(&path).is_some_and(|i| i.person) {
                    meta.keywords.retain(|k| !k.eq_ignore_ascii_case(&path));
                }
            }
            ops.push(Op::SetMeta { id, meta: Box::new(meta) });
        }
        let n = refs.len();
        self.commit("Remove Face", Op::Batch { ops })?;
        Ok(n)
    }

    /// Rename a person everywhere: their faces and their person keyword (a name that is someone else's already
    /// merges the two). One undo step.
    pub fn rename_person(&mut self, from: &str, to: &str) -> Result<usize> {
        let refs: Vec<FaceRef> = self
            .catalog
            .photos()
            .flat_map(|p| {
                p.meta
                    .regions
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.kind == RegionKind::Face && r.name.as_deref().is_some_and(|n| n.trim().eq_ignore_ascii_case(from.trim())))
                    .map(|(i, _)| FaceRef { photo: p.id, index: i })
                    .collect::<Vec<_>>()
            })
            .collect();
        if refs.is_empty() {
            return Err(EngineError::Other(format!("nobody is named “{from}”")));
        }
        self.name_faces(&refs, Some(to), "Rename Person")
    }

    /// Draw a face region on photo `id` (optionally named); its embedding is computed with the photo's next
    /// indexing (queued at once when the face models are there).
    pub fn add_face(&mut self, id: PhotoId, rect: Rect, name: Option<&str>) -> Result<usize> {
        let p = self.catalog.photo(id).ok_or(EngineError::Catalog(lightcraft_catalog::CatalogError::NoPhoto(id)))?;
        let mut regions = p.meta.regions.clone();
        regions.push(lightcraft_meta::Region::face(rect));
        let index = regions.len() - 1;
        self.commit("Add Face", Op::SetRegions { id, regions })?;
        if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
            self.name_faces(&[FaceRef { photo: id, index }], Some(n), "Add Face")?;
            self.merge_undo(2, "Add Face");
        }
        if self.vision.installed(lightcraft_vision::models::Package::Faces) && !self.vision.busy() {
            let _ = self.vision_start(crate::vision::Kind::Faces, &[id], true);
        }
        Ok(index)
    }
}
