//! LRC-LIB-PUBLISH: Publish Services (Lightroom Classic's Library ▸ Publish Services panel).
//!
//! A **service** is a destination with its own export settings ([`Service`]: a name, a kind, the
//! `app.export` params its files are made with). Under it are **published collections** (and
//! collection sets to arrange them). Each photo in a published collection is in one of four
//! [`State`]s:
//!
//! - **New** — never published;
//! - **Modified** — published, but its develop settings or metadata changed since (its
//!   [`fingerprint`] differs from the one stored when it was published): to re-publish;
//! - **Published** — the published copy is up to date;
//! - **Deleted** — taken out of the collection (or the library) after it was published: its copy
//!   is removed from the destination on the next publish.
//!
//! Everything is kept in the catalog as library records ([`lightcraft_catalog::Op::SetRecord`]):
//! `publish.service.<id>` ([`Service`]) and `publish.collection.<id>` ([`Collection`], with each
//! published photo's fingerprint and where its copy went). Creating, editing and deleting
//! services and collections, adding and removing photos are undo steps; what a publish run
//! records is bookkeeping.
//!
//! **Running a publish** is three steps so a front end can do the heavy part on a worker thread:
//! [`prepare`] (needs the session: the photos to export, the removals), [`PublishJob::run`] (no
//! session: render, write, deliver, remove) and [`finish`] (records the outcome). The
//! `publish.run` command does all three at once.
//!
//! # Adding a service
//!
//! Services plug in through [`Publisher`]: add a [`ServiceKind`] variant, implement [`Publisher`]
//! for it (where files are written, how they are delivered — copied in place, uploaded, handed to
//! an app — and how a published copy is removed), and return it from [`publisher_for`]. Two ship:
//! [`ServiceKind::HardDrive`] (a folder per collection) and [`ServiceKind::Photos`] (an album in
//! Apple Photos per collection, through [`crate::apple_photos`]).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use lightcraft_catalog::{Catalog, Op, PhotoId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::export::{Conflict, Destination, ExportOptions, PreparedExport};

pub const SERVICE_PREFIX: &str = "publish.service.";
pub const COLLECTION_PREFIX: &str = "publish.collection.";

/// The kind of destination a service publishes to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceKind {
    /// A folder on a disk: each published collection is a folder in it (sets are folders too).
    #[default]
    HardDrive,
    /// Apple Photos (macOS): each published collection is an album there.
    Photos,
}

impl ServiceKind {
    pub fn parse(s: &str) -> Option<ServiceKind> {
        match s.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "harddrive" | "folder" | "disk" => Some(ServiceKind::HardDrive),
            "photos" | "applephotos" => Some(ServiceKind::Photos),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ServiceKind::HardDrive => "Hard Drive",
            ServiceKind::Photos => "Apple Photos",
        }
    }
}

/// A publish service.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Service {
    pub id: u64,
    pub kind: ServiceKind,
    pub name: String,
    /// Hard Drive: the folder the collections' folders go in.
    pub dir: String,
    /// The `app.export` params its files are made with (format, size, naming, metadata…).
    pub export: Value,
}

/// Where a photo's published copy went and what it looked like then.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Published {
    /// [`fingerprint`] of the photo when it was published.
    pub fingerprint: u64,
    /// The service's id of the copy: Hard Drive = its path. Empty: marked up to date without a
    /// copy LightCraft knows of.
    pub remote: String,
    /// When (ISO 8601).
    pub at: String,
}

/// A published copy waiting to be removed from the destination.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Removal {
    pub photo: u64,
    pub remote: String,
}

/// A published collection, or (`set`) a collection set that holds others.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Collection {
    pub id: u64,
    pub service: u64,
    pub name: String,
    pub set: bool,
    pub parent: Option<u64>,
    /// Its photos, in the order they were added.
    pub photos: Vec<u64>,
    /// Photos published from it (photo id → copy).
    pub published: BTreeMap<u64, Published>,
    /// Copies of photos taken out of it, removed from the destination on the next publish.
    pub removed: Vec<Removal>,
}

/// Where a photo of a published collection stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    New,
    Modified,
    Published,
    Deleted,
}

impl State {
    /// The grid's group heading.
    pub fn heading(self) -> &'static str {
        match self {
            State::New => "New Photos to Publish",
            State::Modified => "Modified Photos to Re-Publish",
            State::Published => "Published Photos",
            State::Deleted => "Deleted Photos to Remove",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            State::New => "new",
            State::Modified => "modified",
            State::Published => "published",
            State::Deleted => "deleted",
        }
    }
    pub fn from_key(k: &str) -> Option<State> {
        [State::New, State::Modified, State::Published, State::Deleted].into_iter().find(|s| s.key() == k)
    }
}

/// What a published copy depends on: the photo's develop settings, metadata and file name. A
/// change to any of them makes a published photo Modified. FNV-1a 64, stable across builds.
pub fn fingerprint(p: &lightcraft_catalog::Photo) -> u64 {
    let meta = serde_json::to_string(&p.meta).unwrap_or_default();
    let s = format!("{}|{}|{}|{}", p.develop.hash64(), meta, p.file_name, p.copy_name.as_deref().unwrap_or(""));
    lightcraft_catalog::fnv64(s.as_bytes())
}

// ------------------------------------------------------------------------------------ reads

pub fn services(cat: &Catalog) -> Vec<Service> {
    let mut v: Vec<Service> = cat.records_with_prefix(SERVICE_PREFIX).filter_map(|(_, v)| serde_json::from_value(v.clone()).ok()).collect();
    v.sort_by_key(|s| s.id);
    v
}

pub fn service(cat: &Catalog, id: u64) -> Option<Service> {
    cat.record(&format!("{SERVICE_PREFIX}{id}")).and_then(|v| serde_json::from_value(v.clone()).ok())
}

pub fn collections(cat: &Catalog) -> Vec<Collection> {
    let mut v: Vec<Collection> = cat.records_with_prefix(COLLECTION_PREFIX).filter_map(|(_, v)| serde_json::from_value(v.clone()).ok()).collect();
    v.sort_by_key(|c| (c.service, !c.set, c.name.to_lowercase(), c.id));
    v
}

pub fn collection(cat: &Catalog, id: u64) -> Option<Collection> {
    cat.record(&format!("{COLLECTION_PREFIX}{id}")).and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// The next free id (services and collections share one sequence).
fn next_id(cat: &Catalog) -> u64 {
    let ids = cat.records_with_prefix("publish.").filter_map(|(k, _)| k.rsplit('.').next().and_then(|n| n.parse::<u64>().ok()));
    ids.max().unwrap_or(0).saturating_add(1)
}

/// A photo of the collection that is still in the library (not deleted, not a Local record).
fn alive(cat: &Catalog, id: u64) -> Option<&std::sync::Arc<lightcraft_catalog::Photo>> {
    cat.photo(PhotoId(id)).filter(|p| p.in_library())
}

/// Where `photo` stands in `c`.
pub fn state_of(cat: &Catalog, c: &Collection, photo: u64) -> Option<State> {
    if c.removed.iter().any(|r| r.photo == photo) && !c.photos.contains(&photo) {
        return Some(State::Deleted);
    }
    if !c.photos.contains(&photo) {
        return None;
    }
    let Some(p) = alive(cat, photo) else {
        // gone from the library: its copy goes too (if there is one)
        return Some(if c.published.contains_key(&photo) { State::Deleted } else { State::New });
    };
    Some(match c.published.get(&photo) {
        None => State::New,
        Some(pb) if pb.fingerprint == fingerprint(p) => State::Published,
        Some(_) => State::Modified,
    })
}

/// Every photo of `c` with its state, members first (in order), then pending removals.
pub fn states(cat: &Catalog, c: &Collection) -> Vec<(u64, State)> {
    let mut v: Vec<(u64, State)> = c.photos.iter().filter_map(|id| state_of(cat, c, *id).map(|s| (*id, s))).collect();
    for r in &c.removed {
        if !c.photos.contains(&r.photo) && !v.iter().any(|(id, _)| *id == r.photo) {
            v.push((r.photo, State::Deleted));
        }
    }
    v
}

/// `{new, modified, published, deleted, total}` of a collection (a set: of every collection in it).
pub fn counts(cat: &Catalog, c: &Collection) -> Value {
    let all = collections(cat);
    let mut n: HashMap<State, usize> = HashMap::new();
    let mut total = 0;
    let mut add = |c: &Collection| {
        for (_, s) in states(cat, c) {
            *n.entry(s).or_default() += 1;
            if s != State::Deleted {
                total += 1;
            }
        }
    };
    if c.set {
        for k in descendants(&all, c.id) {
            add(k);
        }
    } else {
        add(c);
    }
    let g = |s| n.get(&s).copied().unwrap_or(0);
    json!({"new": g(State::New), "modified": g(State::Modified), "published": g(State::Published), "deleted": g(State::Deleted), "total": total})
}

/// The collections (not sets) inside set `id`, at any depth.
fn descendants(all: &[Collection], id: u64) -> Vec<&Collection> {
    let mut out = Vec::new();
    let mut stack = vec![(id, 0usize)];
    while let Some((p, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        for c in all.iter().filter(|c| c.parent == Some(p)) {
            if c.set {
                stack.push((c.id, depth + 1));
            } else {
                out.push(c);
            }
        }
    }
    out
}

/// The names of `c`'s sets, outermost first, then its own name (its folder path on a Hard Drive).
pub fn path_of(all: &[Collection], c: &Collection) -> Vec<String> {
    let mut names = vec![c.name.clone()];
    let mut parent = c.parent;
    let mut seen = 0;
    while let Some(p) = parent {
        seen += 1;
        let Some(s) = all.iter().find(|x| x.id == p).filter(|_| seen < 64) else { break };
        names.push(s.name.clone());
        parent = s.parent;
    }
    names.reverse();
    names
}

/// The library view of published collection `id`: the photos of `visible` (the query's result,
/// filters applied) that are in it, grouped New, Modified, Published, Deleted, each group in the
/// order the photos were added.
pub fn view_order(cat: &Catalog, id: u64, visible: &[PhotoId]) -> Vec<PhotoId> {
    let Some(c) = collection(cat, id) else { return Vec::new() };
    let shown: std::collections::HashSet<u64> = visible.iter().map(|p| p.0).collect();
    let mut v: Vec<(State, usize, u64)> =
        states(cat, &c).into_iter().enumerate().filter(|(_, (id, _))| shown.contains(id)).map(|(i, (id, s))| (s, i, id)).collect();
    v.sort();
    v.into_iter().map(|(_, _, id)| PhotoId(id)).collect()
}

/// The state groups of a view made by [`view_order`]: `(state, first index, count)`.
pub fn view_runs(cat: &Catalog, id: u64, ids: &[PhotoId]) -> Vec<(State, usize, usize)> {
    let Some(c) = collection(cat, id) else { return Vec::new() };
    let mut runs: Vec<(State, usize, usize)> = Vec::new();
    for (i, p) in ids.iter().enumerate() {
        let s = state_of(cat, &c, p.0).unwrap_or(State::New);
        match runs.last_mut() {
            Some(r) if r.0 == s => r.2 += 1,
            _ => runs.push((s, i, 1)),
        }
    }
    runs
}

// ------------------------------------------------------------------------------------ edits

pub(crate) fn set_service(s: &Service) -> Op {
    Op::SetRecord { key: format!("{SERVICE_PREFIX}{}", s.id), value: serde_json::to_value(s).ok() }
}

pub(crate) fn set_collection(c: &Collection) -> Op {
    Op::SetRecord { key: format!("{COLLECTION_PREFIX}{}", c.id), value: serde_json::to_value(c).ok() }
}

pub(crate) fn new_service(cat: &Catalog, kind: ServiceKind, name: &str, dir: &str, export: Value) -> Service {
    Service { id: next_id(cat), kind, name: name.to_string(), dir: dir.to_string(), export }
}

pub(crate) fn new_collection(cat: &Catalog, service: u64, name: &str, set: bool, parent: Option<u64>) -> Collection {
    Collection { id: next_id(cat), service, name: name.to_string(), set, parent, ..Default::default() }
}

/// Add photos to `c` (once each); a photo whose copy was waiting to be removed keeps it instead.
pub fn add_photos(c: &mut Collection, ids: &[u64]) -> usize {
    let mut n = 0;
    for id in ids {
        if !c.photos.contains(id) {
            c.photos.push(*id);
            n += 1;
        }
        if let Some(i) = c.removed.iter().position(|r| r.photo == *id) {
            let r = c.removed.remove(i);
            c.published.entry(*id).or_insert(Published { fingerprint: 0, remote: r.remote, at: String::new() });
        }
    }
    n
}

/// Take photos out of `c`: a published one's copy waits to be removed on the next publish.
pub fn remove_photos(c: &mut Collection, ids: &[u64]) -> usize {
    let mut n = 0;
    for id in ids {
        if let Some(i) = c.photos.iter().position(|x| x == id) {
            c.photos.remove(i);
            n += 1;
        }
        if let Some(pb) = c.published.remove(id)
            && !pb.remote.is_empty()
        {
            c.removed.push(Removal { photo: *id, remote: pb.remote });
        }
    }
    n
}

/// Mark photos of `c` as published as they are now, without publishing them.
pub fn mark_up_to_date(cat: &Catalog, c: &mut Collection, ids: &[u64], at: &str) -> usize {
    let mut n = 0;
    for id in ids.iter().filter(|id| c.photos.contains(id)) {
        let Some(p) = alive(cat, *id) else { continue };
        let fp = fingerprint(p);
        let e = c.published.entry(*id).or_default();
        if e.fingerprint != fp {
            e.fingerprint = fp;
            e.at = at.to_string();
            n += 1;
        }
    }
    n
}

// ------------------------------------------------------------------------------- publishers

/// A destination photos are published to (see the module docs → *Adding a service*).
pub trait Publisher: Send {
    /// Where the rendered files of the collection at `path` (its sets' names, then its own) are
    /// written: the destination itself, or a staging folder the service delivers from.
    fn folder(&self, path: &[String]) -> Result<PathBuf, String>;
    /// The files written for the collection at `path`: hand them to the destination. One result
    /// per file: the copy's id at the destination (its path, an upload id…), or why not.
    fn deliver(&self, path: &[String], files: &[String]) -> Vec<Result<String, String>>;
    /// Remove published copies (ids from [`Publisher::deliver`]). One result per copy.
    fn remove(&self, remote: &[String]) -> Vec<Result<(), String>>;
    /// The written files are only a staging copy (removed after delivery).
    fn staging(&self) -> bool {
        false
    }
}

/// A file or folder name made safe from a collection's name.
fn safe_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() { '-' } else { c }).collect();
    let t = t.trim().trim_matches('.').trim().to_string();
    if t.is_empty() { "Untitled".into() } else { t }
}

/// [`ServiceKind::HardDrive`]: `dir/<set>/…/<collection>/file`; a removed copy goes to the Trash.
pub struct HardDrive {
    pub dir: PathBuf,
    /// The session's test Trash folder ([`crate::Session::trash_dir`]).
    pub trash_dir: Option<PathBuf>,
}

impl Publisher for HardDrive {
    fn folder(&self, path: &[String]) -> Result<PathBuf, String> {
        if self.dir.as_os_str().is_empty() || !self.dir.is_absolute() {
            return Err("the service has no folder to publish to (publish.editService {dir})".into());
        }
        Ok(path.iter().fold(self.dir.clone(), |d, n| d.join(safe_name(n))))
    }
    fn deliver(&self, _: &[String], files: &[String]) -> Vec<Result<String, String>> {
        files.iter().map(|f| Ok(f.clone())).collect()
    }
    fn remove(&self, remote: &[String]) -> Vec<Result<(), String>> {
        let paths: Vec<PathBuf> = remote.iter().map(PathBuf::from).collect();
        // a copy that is gone already needs no removing
        let here: Vec<bool> = paths.iter().map(|p| p.is_file()).collect();
        let present: Vec<PathBuf> = paths.iter().zip(&here).filter(|(_, h)| **h).map(|(p, _)| p.clone()).collect();
        let mut done = crate::trash::trash(&present, self.trash_dir.as_deref()).into_iter();
        here.iter().map(|h| if *h { done.next().unwrap_or(Ok(PathBuf::new())).map(|_| ()) } else { Ok(()) }).collect()
    }
}

/// [`ServiceKind::Photos`]: an album per collection in Apple Photos. Photos' scripting can't
/// delete items, so removals are reported for the user to do there.
pub struct PhotosService {
    pub staging: PathBuf,
}

impl Publisher for PhotosService {
    fn folder(&self, path: &[String]) -> Result<PathBuf, String> {
        Ok(self.staging.join(safe_name(&path.join(" - "))))
    }
    fn deliver(&self, path: &[String], files: &[String]) -> Vec<Result<String, String>> {
        let album = path.last().cloned().unwrap_or_default();
        match crate::apple_photos::import(&album, files, &crate::apple_photos::run_osascript) {
            Ok(_) => files
                .iter()
                .map(|f| Ok(format!("photos:{album}/{}", Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())))
                .collect(),
            Err(e) => files.iter().map(|_| Err(e.clone())).collect(),
        }
    }
    fn remove(&self, remote: &[String]) -> Vec<Result<(), String>> {
        remote.iter().map(|r| Err(format!("{r}: remove it in Photos (Photos can't delete items for other apps)"))).collect()
    }
    fn staging(&self) -> bool {
        true
    }
}

/// The [`Publisher`] for a service.
pub fn publisher_for(s: &crate::Session, svc: &Service) -> Result<Box<dyn Publisher>, String> {
    Ok(match svc.kind {
        ServiceKind::HardDrive => Box::new(HardDrive { dir: PathBuf::from(svc.dir.trim()), trash_dir: s.trash_dir.clone() }),
        ServiceKind::Photos => {
            let stamp: String = (s.clock)().chars().filter(char::is_ascii_digit).collect();
            Box::new(PhotosService { staging: std::env::temp_dir().join(format!("LightCraft Publish {stamp}-{}", std::process::id())) })
        }
    })
}

/// The export options a service's files are made with: its params, made safe for publishing
/// (no Same Folder, no Add to Catalog, no post-processing, no subfolder: the publisher decides).
pub fn service_options(svc: &Service) -> Result<ExportOptions, String> {
    let params = if svc.export.is_object() { svc.export.clone() } else { json!({}) };
    let mut o = ExportOptions::from_params(&ExportOptions::known_keys_only(&params)).map_err(|e| e.to_string())?;
    o.export_to = crate::export::ExportTo::SpecificFolder;
    o.add_to_catalog = false;
    o.add_to_stack = None;
    o.subfolder.clear();
    o.after = crate::export::AfterExport::default();
    o.conflict = Conflict::Unique;
    Ok(o)
}

// ------------------------------------------------------------------------------------- runs

/// A publish run set up from the session ([`prepare`]); [`PublishJob::run`] needs no session.
pub struct PublishJob {
    pub collection: u64,
    pub name: String,
    items: Vec<PreparedExport>,
    /// Photo → (fingerprint now, its previous copy).
    facts: HashMap<u64, (u64, Option<String>)>,
    /// Copies of other photos of the collection (their file names stay theirs).
    others: Vec<String>,
    removals: Vec<Removal>,
    opts: ExportOptions,
    path: Vec<String>,
    publisher: Box<dyn Publisher>,
    at: String,
}

impl PublishJob {
    /// Photos to export.
    pub fn exports(&self) -> usize {
        self.items.len()
    }
    /// Copies to remove.
    pub fn removals(&self) -> usize {
        self.removals.len()
    }
}

/// What a run did, for [`finish`].
#[derive(Debug, Default)]
pub struct PublishOutcome {
    pub collection: u64,
    /// (photo, fingerprint, copy).
    pub published: Vec<(u64, u64, String)>,
    /// Removals done (also those whose copy was already gone).
    pub removed: Vec<Removal>,
    pub errors: Vec<Value>,
    pub at: String,
    pub cancelled: bool,
}

/// Set up publishing collection `id` (New and Modified photos, or only `only` among them;
/// pending removals always). Errors: no such collection, a set, an unusable service.
pub fn prepare(s: &mut crate::Session, id: u64, only: Option<&[u64]>) -> Result<PublishJob, String> {
    let c = collection(&s.catalog, id).ok_or_else(|| format!("no published collection {id}"))?;
    if c.set {
        return Err(format!("“{}” is a collection set: publish the collections in it", c.name));
    }
    let svc = service(&s.catalog, c.service).ok_or_else(|| format!("the service of “{}” is gone", c.name))?;
    let opts = service_options(&svc)?;
    let publisher = publisher_for(s, &svc)?;
    let all = collections(&s.catalog);
    let path = path_of(&all, &c);
    // fail early (no folder chosen) rather than after rendering everything
    publisher.folder(&path)?;
    let mut ids = Vec::new();
    let mut facts = HashMap::new();
    for (pid, st) in states(&s.catalog, &c) {
        if !matches!(st, State::New | State::Modified) || only.is_some_and(|o| !o.contains(&pid)) {
            continue;
        }
        let Some(p) = alive(&s.catalog, pid) else { continue };
        facts.insert(pid, (fingerprint(p), c.published.get(&pid).map(|pb| pb.remote.clone()).filter(|r| !r.is_empty())));
        ids.push(PhotoId(pid));
    }
    // a member gone from the library with a copy: remove the copy too
    let mut removals = c.removed.clone();
    for (pid, st) in states(&s.catalog, &c) {
        if st == State::Deleted
            && c.photos.contains(&pid)
            && let Some(pb) = c.published.get(&pid).filter(|pb| !pb.remote.is_empty())
        {
            removals.push(Removal { photo: pid, remote: pb.remote.clone() });
        }
    }
    let others = c.published.iter().filter(|(pid, _)| !facts.contains_key(pid)).map(|(_, pb)| pb.remote.clone()).collect();
    let items = crate::export::prepare_batch(s, &ids, &opts)?;
    Ok(PublishJob { collection: id, name: c.name.clone(), items, facts, others, removals, opts, path, publisher, at: (s.clock)() })
}

impl PublishJob {
    /// Render, write and deliver the photos, then remove the copies waiting to go. Never fails as
    /// a whole: each photo's error is in the outcome. `progress(done, file)` before each photo;
    /// false cancels the rest.
    pub fn run(self, write: &mut dyn FnMut(&str, &[u8]) -> Result<(), String>, progress: &mut dyn FnMut(usize, &str) -> bool) -> PublishOutcome {
        let mut out = PublishOutcome { collection: self.collection, at: self.at.clone(), ..Default::default() };
        let folder = match self.publisher.folder(&self.path) {
            Ok(f) => f,
            Err(e) => {
                out.errors.push(json!({"error": e}));
                return out;
            }
        };
        let to = Destination { dir: folder.to_string_lossy().to_string(), exact: None };
        let mut taken: std::collections::HashSet<String> = self.others.iter().cloned().collect();
        let mut written: Vec<(u64, String)> = Vec::new();
        for (i, item) in self.items.into_iter().enumerate() {
            let photo = item.photo.0;
            let own = self.facts.get(&photo).and_then(|f| f.1.clone());
            // its own previous copy is replaced in place; other photos' copies keep their names
            let exists = |p: &str| own.as_deref() != Some(p) && (taken.contains(p) || Path::new(p).exists());
            let mut prog = |_: usize, f: &str| progress(i, f);
            match crate::export::run_batch(vec![item], &self.opts, &to, write, &exists, false, &mut prog) {
                Ok(r) if r.is_empty() => {
                    out.cancelled = true;
                    break;
                }
                Ok(r) => {
                    for v in r {
                        match v.get("path").and_then(Value::as_str) {
                            Some(p) => {
                                taken.insert(p.to_string());
                                written.push((photo, p.to_string()));
                            }
                            None => out.errors.push(v),
                        }
                    }
                }
                Err(e) => out.errors.push(json!({"photo": photo, "error": e})),
            }
        }
        // hand the files over
        let files: Vec<String> = written.iter().map(|w| w.1.clone()).collect();
        let delivered = if files.is_empty() { Vec::new() } else { self.publisher.deliver(&self.path, &files) };
        let mut stale = Vec::new();
        for ((photo, path), r) in written.iter().zip(delivered) {
            match r {
                Ok(remote) => {
                    let (fp, old) = self.facts.get(photo).cloned().unwrap_or((0, None));
                    // re-published under another name: the old copy goes
                    if let Some(old) = old.filter(|o| *o != remote) {
                        stale.push(Removal { photo: *photo, remote: old });
                    }
                    out.published.push((*photo, fp, remote));
                }
                Err(e) => out.errors.push(json!({"photo": photo, "file": path, "error": e})),
            }
        }
        if self.publisher.staging() {
            let _ = std::fs::remove_dir_all(&folder);
        }
        // removals: the copies of photos taken out of the collection, and replaced copies
        let mut removals = self.removals;
        let pending = removals.len();
        removals.extend(stale);
        if !out.cancelled && !removals.is_empty() {
            let remote: Vec<String> = removals.iter().map(|r| r.remote.clone()).collect();
            for (i, (r, res)) in removals.into_iter().zip(self.publisher.remove(&remote)).enumerate() {
                match res {
                    Ok(()) if i < pending => out.removed.push(r),
                    Ok(()) => {}
                    Err(e) => out.errors.push(json!({"photo": r.photo, "file": r.remote, "error": e})),
                }
            }
        }
        out
    }
}

/// Record a run's outcome in the collection (bookkeeping, not an undo step) →
/// `{published, removed, errors, counts}`.
pub fn finish(s: &mut crate::Session, out: PublishOutcome) -> crate::Result<Value> {
    let Some(mut c) = collection(&s.catalog, out.collection) else {
        return Ok(json!({"published": 0, "removed": 0, "errors": out.errors, "note": "the collection was deleted meanwhile"}));
    };
    for (photo, fp, remote) in &out.published {
        let pb = Published { fingerprint: *fp, remote: remote.clone(), at: out.at.clone() };
        if c.photos.contains(photo) {
            c.published.insert(*photo, pb);
        } else {
            // taken out while it was publishing: its new copy goes next time
            c.removed.push(Removal { photo: *photo, remote: remote.clone() });
        }
    }
    for r in &out.removed {
        c.removed.retain(|x| x != r);
        if !c.photos.contains(&r.photo) || alive(&s.catalog, r.photo).is_none() {
            c.published.remove(&r.photo);
        }
        // a member gone from the library: nothing left to publish
        if alive(&s.catalog, r.photo).is_none() {
            c.photos.retain(|x| *x != r.photo);
        }
    }
    s.apply_bookkeeping(set_collection(&c))?;
    let counts = counts(&s.catalog, &c);
    Ok(json!({
        "published": out.published.len(),
        "removed": out.removed.len(),
        "errors": out.errors,
        "cancelled": out.cancelled,
        "counts": counts,
    }))
}

/// [`prepare`] + [`PublishJob::run`] (native file writes) + [`finish`].
pub fn publish_now(s: &mut crate::Session, id: u64, only: Option<&[u64]>) -> crate::Result<Value> {
    let job = prepare(s, id, only).map_err(|e| crate::cmd::bad("publish.run", e))?;
    let out = job.run(&mut |p, b| crate::export::write_file(p, b), &mut |_, _| true);
    finish(s, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names_and_kinds() {
        assert_eq!(safe_name("a/b:c"), "a-b-c");
        assert_eq!(safe_name(" .. "), "Untitled");
        assert_eq!(ServiceKind::parse("Hard Drive"), Some(ServiceKind::HardDrive));
        assert_eq!(ServiceKind::parse("photos"), Some(ServiceKind::Photos));
        assert_eq!(ServiceKind::parse("flickr"), None);
        assert_eq!(State::from_key("modified"), Some(State::Modified));
    }

    #[test]
    fn path_of_nested_sets_is_bounded() {
        let a = Collection { id: 1, set: true, name: "Weddings".into(), parent: Some(2), ..Default::default() };
        let b = Collection { id: 2, set: true, name: "Loop".into(), parent: Some(1), ..Default::default() };
        let c = Collection { id: 3, name: "Ann".into(), parent: Some(1), ..Default::default() };
        let p = path_of(&[a, b, c.clone()], &c);
        assert_eq!(p.last().map(String::as_str), Some("Ann"));
        assert!(p.len() <= 65);
    }
}
