//! Favorites kept safe (Bryan, 2026-11-24: "if a folder is deleted that has marks with that button
//! keep them in lightcraft unless I manually delete them … I want the original photos tied with
//! the edits").
//!
//! A hearted photo (fav., `Photo.favorite`) gets a copy of its **original file** and a fresh XMP
//! **sidecar** with its current edits in the keep folder (the app's default:
//! [`DEFAULT_ROOT`], `/Volumes/LOT48/LightCraft Favorites`), at `<root>/<shoot>/<file name>`:
//! the shoot is the nearest folder above the file whose name says what it is of (`raw`, `M262`,
//! `100MSDCF` are skipped, as Library ▸ Shoots does). A file there already is never overwritten: a
//! different one makes the copy `IMG_1-1.CR3`, `IMG_1-2.CR3`…; the same bytes are simply reused.
//!
//! - **Where a copy is** is a library record per photo (`fav.kept.<id>` → [`Kept`]), journaled
//!   with the catalog like `xmp.pending`. What is still to copy needs no record of its own: a
//!   hearted photo without a `fav.kept` record *is* the queue, so a crash, a quit mid-copy or an
//!   offline LOT48 only means the next pass copies it. Copies are made on a worker thread
//!   ([`FavKeep::background`], the app) a moment after the heart, [`BATCH`] files at a time;
//!   a pass over the whole library runs at open and every [`CHECK_EVERY`] (every
//!   [`OFFLINE_RETRY`] while the keep folder's disk is not there). Nothing ever blocks a frame.
//! - **Edits follow**: a change to a kept photo writes the kept sidecar again (compared by the
//!   packet's hash, so only real changes are written).
//! - **The original gone** (its folder deleted, its disk not there): the photo silently points at
//!   the kept copy (a bookkeeping [`Op::Relink`], no undo step) and stays in the library with its
//!   edits — they are the catalog's record, not the file's. The record keeps the original's path
//!   (`from`): when it is back (a drive plugged in again), the photo switches back to it.
//! - **Nothing is removed from the keep folder** except by `fav.forget {ids}` (to the Trash) or a
//!   delete from the library that is told to (`keptCopies: true`; the app asks, default no).
//!   Unhearting a photo keeps its copy. A photo forgotten while hearted is not kept again until
//!   it is hearted again (`fav.forgot.<id>`).
//! - A virtual copy keeps its master's file (the edits of the master go in the sidecar). A photo
//!   already inside the keep folder is kept where it is (`inPlace`: never deleted by forget).
//!
//! The engine's default keep folder is `None` (off): only a frontend that says where (the app,
//! from Settings ▸ General ▸ Favorites) ever writes anywhere, so tests and scripts never touch a
//! real drive.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lightcraft_catalog::{Catalog, Op, Photo, PhotoId, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use web_time::Instant;

use crate::cmd::{CommandSpec, always, bad, cmd};
use crate::sidecar::{SidecarWrite, StemOwners};
use crate::{Result, Session};

/// Where the app keeps favorites unless Settings says otherwise.
pub const DEFAULT_ROOT: &str = "/Volumes/LOT48/LightCraft Favorites";
/// `fav.kept.<id>` → [`Kept`].
pub const KEPT_PREFIX: &str = "fav.kept.";
/// `fav.forgot.<id>` → true: forgotten while hearted, not kept again until hearted again.
pub const FORGOT_PREFIX: &str = "fav.forgot.";
/// A heart (or an edit to a kept photo) waits this long, so a quick heart / unheart or a run of
/// slider changes is one write.
pub const DEBOUNCE: Duration = Duration::from_secs(2);
/// A pass over every favorite: copies waiting, originals gone or back, sidecars behind.
pub const CHECK_EVERY: Duration = Duration::from_secs(180);
/// The keep folder's disk wasn't there: look again this soon.
pub const OFFLINE_RETRY: Duration = Duration::from_secs(60);
/// Files copied per worker run (the next run starts at once).
pub const BATCH: usize = 24;

/// Where a photo's kept copy is (the `fav.kept.<id>` record).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    /// The kept copy of the original.
    pub path: String,
    /// Its sidecar (none written: a virtual-copy-free photo whose sidecar couldn't be written).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<String>,
    /// The original's path (where the photo goes back to when it is there again).
    pub from: String,
    #[serde(default)]
    pub size: u64,
    /// When it was copied (ISO 8601).
    #[serde(default)]
    pub at: String,
    /// Hash of the sidecar packet last written beside it (edits since → written again).
    #[serde(default)]
    pub packet: u64,
    /// The original is inside the keep folder already: nothing was copied, forget never deletes it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_place: bool,
}

pub fn kept_key(id: PhotoId) -> String {
    format!("{KEPT_PREFIX}{}", id.0)
}

fn forgot_key(id: PhotoId) -> String {
    format!("{FORGOT_PREFIX}{}", id.0)
}

/// Photo `id`'s kept copy record.
pub fn kept(cat: &Catalog, id: PhotoId) -> Option<Kept> {
    cat.record(&kept_key(id)).and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// The photo whose file a hearted photo keeps: itself, or a virtual copy's master (a photo from a
/// file only).
pub fn keeper(cat: &Catalog, p: &Photo) -> Option<PhotoId> {
    let id = p.copy_of.unwrap_or(p.id);
    let m = cat.photo(id)?;
    matches!(m.source, Source::File { .. }).then_some(id)
}

/// Whether `p` (or, for a virtual copy, its master) has a kept copy — the grid's `kept.` mark.
pub fn is_kept(cat: &Catalog, p: &Photo) -> bool {
    keeper(cat, p).is_some_and(|k| cat.record(&kept_key(k)).is_some())
}

/// The photo points at its kept copy now (its original is gone).
pub fn uses_kept(cat: &Catalog, id: PhotoId) -> bool {
    match (cat.photo(id).map(|p| &p.source), kept(cat, id)) {
        (Some(Source::File { path }), Some(k)) => !k.in_place && *path == k.path,
        _ => false,
    }
}

fn hearted(p: &Photo) -> bool {
    p.favorite && !p.deleted && !p.local
}

/// The keepers of every hearted photo that wants keeping (not forgotten).
fn wanted(cat: &Catalog) -> BTreeSet<PhotoId> {
    cat.photos().filter(|p| hearted(p)).filter_map(|p| keeper(cat, p)).filter(|k| cat.record(&forgot_key(*k)).is_none()).collect()
}

/// The keep folder's subfolder for an original: the nearest folder above it whose name says what
/// it is of (not `raw`, `M262`, `100MSDCF`, a bare date), else the folder it is in.
pub fn group_name(path: &str) -> String {
    let clean = |n: &str| {
        let s: String = n.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
        s.trim().trim_start_matches('.').to_string()
    };
    let mut dir = Path::new(path).parent();
    let first = dir.and_then(Path::file_name).map(|n| clean(&n.to_string_lossy()));
    for _ in 0..64 {
        let Some(d) = dir else { break };
        let Some(name) = d.file_name().map(|n| n.to_string_lossy().to_string()) else { break };
        // a disk itself is no shoot
        if d.parent().is_none_or(|p| p == Path::new("/Volumes")) {
            break;
        }
        if !lightcraft_catalog::shoots::is_generic_name(&name) && name.chars().any(char::is_alphabetic) {
            let c = clean(&name);
            if !c.is_empty() {
                return c;
            }
        }
        dir = d.parent();
    }
    first.filter(|n| !n.is_empty()).unwrap_or_else(|| "Favorites".into())
}

/// A disk's name for messages (`LOT48` for `/Volumes/LOT48/…`), else the folder.
pub fn disk_name(root: &Path) -> String {
    let mut parts = root.components().map(|c| c.as_os_str().to_string_lossy().to_string());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(_), Some(v), Some(d)) if v == "Volumes" => d,
        _ => root.display().to_string(),
    }
}

/// What `fav.keepStatus` reports (and the app's favorites. row and Settings show).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// A keep folder is set.
    pub on: bool,
    pub root: String,
    /// The keep folder's disk (`LOT48`).
    pub disk: String,
    /// The keep folder's disk was there at the last look (`None`: not looked yet).
    pub online: Option<bool>,
    /// Photos to keep (hearted, not forgotten; a master counts once).
    pub favorites: usize,
    /// … with a kept copy.
    pub kept: usize,
    /// … still to copy.
    pub waiting: usize,
    /// … whose original is gone: they show their kept copy.
    pub using_kept: usize,
    /// … that couldn't be copied at the last try (original not found, a read error), retried later.
    pub failed: usize,
    /// A copy run is going.
    pub running: bool,
}

struct CopyItem {
    id: PhotoId,
    src: String,
    dir: PathBuf,
    name: String,
    sidecar: Option<SidecarWrite>,
}

struct RefreshItem {
    id: PhotoId,
    kept: String,
    sidecar: SidecarWrite,
}

struct CheckItem {
    id: PhotoId,
    current: String,
    kept: String,
    home: String,
}

struct Plan {
    root: PathBuf,
    stamp: String,
    copies: Vec<CopyItem>,
    refresh: Vec<RefreshItem>,
    checks: Vec<CheckItem>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.copies.is_empty() && self.refresh.is_empty() && self.checks.is_empty()
    }
}

enum Outcome {
    Kept(PhotoId, Kept),
    Refreshed(PhotoId, u64, String),
    Failed(PhotoId, String),
    /// The kept copy is gone from the keep folder (deleted there by hand).
    Lost(PhotoId),
    /// The original is gone and the kept copy is there: point the photo at it.
    UseKept(PhotoId),
    /// The original is back: point the photo at it again.
    Home(PhotoId),
}

struct Done {
    generation: u64,
    /// The keep folder (or the disk it goes on) was there.
    online: bool,
    /// Copies or sidecar writes were planned (an offline result only matters then).
    wrote: bool,
    outcomes: Vec<Outcome>,
}

struct Job {
    handle: std::thread::JoinHandle<Done>,
    cancel: Arc<AtomicBool>,
}

/// The session's favorites keeping (see the module docs).
#[derive(Default)]
pub struct FavKeep {
    /// The keep folder; `None` = off (the engine's default — only a frontend sets it).
    pub root: Option<PathBuf>,
    /// Copy on a worker thread (the app); off: [`Session::poll_fav_keep`] copies right there.
    pub background: bool,
    /// Keepers whose copy or sidecar is due (after [`DEBOUNCE`]).
    due: BTreeMap<PhotoId, Instant>,
    /// The next pass over every favorite (`None`: at the next poll).
    next_full: Option<Instant>,
    /// The next full pass compares every kept sidecar (once per library open).
    refresh_all: bool,
    job: Option<Job>,
    /// The keep folder's disk at the last look.
    online: Option<bool>,
    /// Keepers whose last copy failed: when, and why.
    failed: BTreeMap<PhotoId, (Instant, String)>,
}

impl FavKeep {
    /// Forget everything in flight (a library opened, the keep folder changed). A running copy
    /// is told to stop after its current file and its results are dropped (a copy that landed is
    /// found again by its bytes next time).
    fn reset(&mut self) {
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.due.clear();
        self.next_full = None;
        self.refresh_all = true;
        self.online = None;
        self.failed.clear();
    }

    /// A copy run is going.
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// The keep folder's disk at the last look (`None`: not looked yet).
    pub fn online(&self) -> Option<bool> {
        self.online
    }

    #[cfg(test)]
    pub(crate) fn full_pass_now_for_tests(&mut self) {
        self.next_full = Some(Instant::now());
        self.failed.clear();
    }

    #[cfg(test)]
    pub(crate) fn refresh_due_now_for_tests(&mut self) {
        let now = Instant::now();
        self.due.values_mut().for_each(|t| *t = now);
    }

    /// How long until the next poll has work (the app repaints then).
    pub fn wake_in(&self) -> Option<Duration> {
        self.root.as_ref()?;
        if self.job.is_some() {
            return Some(Duration::from_millis(250));
        }
        let now = Instant::now();
        let full = self.next_full.map_or(Duration::ZERO, |t| t.saturating_duration_since(now));
        let due = self.due.values().map(|t| t.saturating_duration_since(now)).min();
        Some(due.map_or(full, |d| d.min(full)))
    }
}

/// Whether the keep folder is there; `create`: make its last folder when the disk it goes on is
/// there (never the disk's folders themselves: `/Volumes/LOT48` missing means LOT48 is offline).
fn root_ready(root: &Path, create: bool) -> bool {
    if root.is_dir() {
        return true;
    }
    let parent_there = root.parent().is_some_and(Path::is_dir);
    if !create {
        return parent_there;
    }
    (parent_there && std::fs::create_dir(root).is_ok()) || root.is_dir()
}

/// `a` and `b` hold the same bytes.
fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    let (fa, fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let (mut ra, mut rb) = (std::io::BufReader::with_capacity(1 << 20, fa), std::io::BufReader::with_capacity(1 << 20, fb));
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let n = read_full(&mut ra, &mut ba)?;
        let m = read_full(&mut rb, &mut bb)?;
        if n != m || ba.get(..n) != bb.get(..m) {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let Some(rest) = buf.get_mut(n..) else { break };
        match r.read(rest) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Copy `src` into `dir` as `name`, never over a different file (`-1`, `-2`… instead); a file
/// there with the same bytes is the copy already. Copied under a temporary name first, so a copy
/// cut short never looks like a kept file.
fn keep_file(src: &Path, dir: &Path, name: &str) -> std::result::Result<(PathBuf, u64), String> {
    use crate::rename::MoveFs;
    let say = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let meta = std::fs::metadata(src).map_err(|e| say(src, e))?;
    if !meta.is_file() {
        return Err(format!("{}: not a file", src.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| say(dir, e))?;
    let base = Path::new(name);
    let stem = base.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    let ext = base.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.lightcraft-partial"));
    for k in 0..1000u32 {
        let cand = if k == 0 { dir.join(name) } else { dir.join(format!("{stem}-{k}{ext}")) };
        match std::fs::symlink_metadata(&cand) {
            Ok(m) => {
                if m.is_file() && m.len() == meta.len() && same_bytes(src, &cand).map_err(|e| say(&cand, e))? {
                    return Ok((cand, meta.len()));
                }
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(say(&cand, e)),
        }
        let _ = std::fs::remove_file(&tmp);
        let n = std::fs::copy(src, &tmp).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            say(&tmp, e)
        })?;
        if n != meta.len() {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{}: copied {n} of {} bytes", src.display(), meta.len()));
        }
        // the original's time stays with it; flushed before it gets its name
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&tmp) {
            if let Ok(t) = meta.modified() {
                let _ = f.set_modified(t);
            }
            let _ = f.sync_all();
        }
        match crate::rename::RealFs.rename_no_replace(&tmp, &cand) {
            Ok(()) => return Ok((cand, n)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&tmp);
                continue;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(say(&cand, e));
            }
        }
    }
    Err(format!("{}: no free name for {name}", dir.display()))
}

fn run_plan(plan: Plan, generation: u64, cancel: &AtomicBool) -> Done {
    let wrote = !plan.copies.is_empty() || !plan.refresh.is_empty();
    let online = root_ready(&plan.root, !plan.copies.is_empty());
    let mut outcomes = Vec::new();
    if online {
        for c in plan.copies {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let r = crate::guard::catch("keep a favorite", || keep_file(Path::new(&c.src), &c.dir, &c.name)).unwrap_or_else(Err);
            match r {
                Ok((dest, size)) => {
                    let dest = dest.to_string_lossy().to_string();
                    let saved = c
                        .sidecar
                        .map(|w| crate::guard::catch("kept sidecar", || w.beside(dest.clone()).run().map_err(|e| e.to_string())).unwrap_or_else(Err));
                    let (sidecar, packet) = match saved {
                        Some(Ok(s)) => (Some(s.path.to_string_lossy().to_string()), s.sync.packet),
                        Some(Err(e)) => {
                            log::warn!("favorites: sidecar for {dest}: {e}");
                            (None, 0)
                        }
                        None => (None, 0),
                    };
                    outcomes
                        .push(Outcome::Kept(c.id, Kept { path: dest, sidecar, from: c.src, size, at: plan.stamp.clone(), packet, in_place: false }));
                }
                Err(e) => outcomes.push(Outcome::Failed(c.id, e)),
            }
        }
        for r in plan.refresh {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if !Path::new(&r.kept).is_file() {
                outcomes.push(Outcome::Lost(r.id));
                continue;
            }
            match crate::guard::catch("kept sidecar", || r.sidecar.run().map_err(|e| e.to_string())).unwrap_or_else(Err) {
                Ok(s) => outcomes.push(Outcome::Refreshed(r.id, s.sync.packet, s.path.to_string_lossy().to_string())),
                Err(e) => log::warn!("favorites: kept sidecar for {}: {e}", r.kept),
            }
        }
    }
    for ch in plan.checks {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if ch.current == ch.kept {
            if ch.home != ch.kept && Path::new(&ch.home).is_file() {
                outcomes.push(Outcome::Home(ch.id));
            }
        } else if !Path::new(&ch.current).exists() && Path::new(&ch.kept).is_file() {
            outcomes.push(Outcome::UseKept(ch.id));
        }
    }
    Done { generation, online, wrote, outcomes }
}

fn relink_to(id: PhotoId, path: &str) -> Op {
    let file_name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    Op::Relink { id, file_name, source: Source::File { path: path.to_string() }, format: None }
}

fn record_op(id: PhotoId, k: Option<&Kept>) -> Op {
    Op::SetRecord { key: kept_key(id), value: k.and_then(|k| serde_json::to_value(k).ok()) }
}

impl Session {
    /// Set (or turn off, `None`) the keep folder. A change starts over: the next poll looks at
    /// every favorite.
    pub fn set_fav_keep_root(&mut self, root: Option<PathBuf>) {
        if self.fav_keep.root != root {
            self.fav_keep.reset();
            self.fav_keep.root = root;
        }
    }

    /// A library was opened: look at every favorite again (its ids mean other photos now).
    pub(crate) fn fav_keep_opened(&mut self) {
        self.fav_keep.reset();
    }

    /// A library is closing: a copy run in flight is stopped after its file (not waited for; what
    /// it copied is found by its bytes next time). Finished results are taken in first.
    pub(crate) fn fav_keep_closing(&mut self) {
        if self.fav_keep.job.as_ref().is_some_and(|j| j.handle.is_finished()) {
            self.fav_keep_take();
        }
        self.fav_keep.reset();
    }

    /// A command changed photos (`ops`): hearts are due to be kept, kept photos' sidecars to be
    /// written again.
    pub(crate) fn fav_keep_note(&mut self, from: usize) {
        if self.fav_keep.root.is_none() {
            return;
        }
        fn walk(op: &Op, out: &mut Vec<(PhotoId, bool)>) {
            match op {
                Op::SetFavorite { id, favorite } => out.push((*id, *favorite)),
                Op::SetRating { id, .. }
                | Op::SetFlag { id, .. }
                | Op::SetLabel { id, .. }
                | Op::SetDevelop { id, .. }
                | Op::SetMeta { id, .. }
                | Op::SetCaptured { id, .. }
                | Op::SetRegions { id, .. }
                | Op::Relink { id, .. }
                | Op::SetFile { id, .. } => out.push((*id, false)),
                Op::Batch { ops } => ops.iter().for_each(|o| walk(o, out)),
                _ => {}
            }
        }
        let mut touched = Vec::new();
        self.pending_log.get(from..).unwrap_or_default().iter().for_each(|o| walk(o, &mut touched));
        let at = Instant::now() + DEBOUNCE;
        let mut clear = Vec::new();
        for (id, hearted_now) in touched {
            let Some(k) = self.catalog.photo(id).and_then(|p| keeper(&self.catalog, p)) else { continue };
            if hearted_now && self.catalog.record(&forgot_key(k)).is_some() {
                // hearted again after a forget: kept again
                clear.push(Op::SetRecord { key: forgot_key(k), value: None });
            }
            if hearted_now || self.catalog.record(&kept_key(k)).is_some() {
                self.fav_keep.due.insert(k, at);
            }
        }
        if !clear.is_empty()
            && let Err(e) = self.apply_bookkeeping(Op::Batch { ops: clear })
        {
            log::warn!("favorites: {e}");
        }
    }

    /// What to do now: `full` looks at every favorite (copies waiting, originals gone or back);
    /// otherwise only `ids`. In memory only (the disk work is [`run_plan`]'s).
    fn fav_keep_plan(&mut self, full: bool, ids: &[PhotoId]) -> Option<(Plan, bool)> {
        let root = self.fav_keep.root.clone()?;
        let now = Instant::now();
        let wanted = wanted(&self.catalog);
        let mut look: BTreeSet<PhotoId> = ids.iter().copied().collect();
        if full {
            look.extend(wanted.iter().copied());
            look.extend(
                self.catalog
                    .records_with_prefix(KEPT_PREFIX)
                    .filter_map(|(k, _)| k.strip_prefix(KEPT_PREFIX).and_then(|n| n.parse().ok()).map(PhotoId)),
            );
        }
        let refresh_all = full && std::mem::take(&mut self.fav_keep.refresh_all);
        let copy_ids: Vec<PhotoId> = look.iter().copied().filter(|id| wanted.contains(id) && self.catalog.record(&kept_key(*id)).is_none()).collect();
        let owners = StemOwners::for_photos(&self.catalog, &copy_ids);
        let mut plan = Plan { root: root.clone(), stamp: (self.clock)(), copies: Vec::new(), refresh: Vec::new(), checks: Vec::new() };
        let mut fix = Vec::new();
        let mut more = false;
        for id in look {
            let Some(p) = self.catalog.photo(id) else { continue };
            let Source::File { path } = &p.source else { continue };
            match kept(&self.catalog, id) {
                None if wanted.contains(&id) => {
                    if self.fav_keep.failed.get(&id).is_some_and(|(t, _)| now.saturating_duration_since(*t) < CHECK_EVERY) && !ids.contains(&id) {
                        continue;
                    }
                    if Path::new(path).starts_with(&root) {
                        // in the keep folder already: kept where it is
                        let k = Kept {
                            path: path.clone(),
                            sidecar: None,
                            from: path.clone(),
                            size: p.file_size,
                            at: plan.stamp.clone(),
                            packet: 0,
                            in_place: true,
                        };
                        fix.push(record_op(id, Some(&k)));
                        continue;
                    }
                    if plan.copies.len() >= BATCH {
                        more = true;
                        continue;
                    }
                    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.file_name.clone());
                    plan.copies.push(CopyItem {
                        id,
                        src: path.clone(),
                        dir: root.join(group_name(path)),
                        name,
                        sidecar: self.prepare_sidecar(id, &owners).ok(),
                    });
                }
                None => {}
                Some(k) if k.in_place => {}
                Some(mut k) => {
                    let using = *path == k.path;
                    if full {
                        plan.checks.push(CheckItem { id, current: path.clone(), kept: k.path.clone(), home: k.from.clone() });
                    }
                    if !using && *path != k.from {
                        // the original moved (renamed, relinked): home follows it
                        k.from = path.clone();
                        fix.push(record_op(id, Some(&k)));
                    }
                    if !using && (refresh_all || ids.contains(&id)) && p.copy_of.is_none() {
                        let packet = lightcraft_catalog::fnv64(crate::sidecar::sidecar_packet(p, &self.catalog).as_bytes());
                        if packet != k.packet
                            && let Ok(w) = self.prepare_sidecar(id, &StemOwners::for_photos(&self.catalog, &[id]))
                        {
                            plan.refresh.push(RefreshItem { id, kept: k.path.clone(), sidecar: w.beside(k.path.clone()) });
                        }
                    }
                }
            }
        }
        if !fix.is_empty()
            && let Err(e) = self.apply_bookkeeping(Op::Batch { ops: fix })
        {
            log::warn!("favorites: {e}");
        }
        Some((plan, more))
    }

    /// Once a frame (the app; [`Session::persist_if_dirty`]): take in a finished copy run, start
    /// the next when something is due. Cheap when nothing is.
    pub fn poll_fav_keep(&mut self) {
        if self.fav_keep.job.as_ref().is_some_and(|j| j.handle.is_finished()) {
            self.fav_keep_take();
        }
        if self.fav_keep.root.is_none() || self.fav_keep.job.is_some() {
            return;
        }
        let now = Instant::now();
        let full = self.fav_keep.next_full.is_none_or(|t| now >= t);
        let ready: Vec<PhotoId> = self.fav_keep.due.iter().filter(|(_, t)| **t <= now).map(|(id, _)| *id).collect();
        if !full && ready.is_empty() {
            return;
        }
        for id in &ready {
            self.fav_keep.due.remove(id);
        }
        let Some((plan, more)) = self.fav_keep_plan(full, &ready) else { return };
        if full {
            self.fav_keep.next_full = Some(now + CHECK_EVERY);
        }
        if more {
            self.fav_keep.next_full = Some(now);
        }
        if plan.is_empty() {
            return;
        }
        let generation = self.library_generation();
        let cancel = Arc::new(AtomicBool::new(false));
        if self.fav_keep.background {
            let c = cancel.clone();
            match std::thread::Builder::new().name("fav-keep".into()).spawn(move || run_plan(plan, generation, &c)) {
                Ok(handle) => self.fav_keep.job = Some(Job { handle, cancel }),
                Err(e) => log::warn!("favorites: no worker thread ({e}); trying again later"),
            }
        } else {
            let done = run_plan(plan, generation, &cancel);
            self.fav_keep_apply(done);
        }
    }

    fn fav_keep_take(&mut self) {
        let Some(j) = self.fav_keep.job.take() else { return };
        match j.handle.join() {
            Ok(done) => self.fav_keep_apply(done),
            Err(_) => log::error!("favorites: the copy thread panicked"),
        }
    }

    fn fav_keep_apply(&mut self, done: Done) {
        if done.generation != self.library_generation() {
            return;
        }
        let now = Instant::now();
        if done.wrote || self.fav_keep.online.is_none() || !done.online {
            self.fav_keep.online = Some(done.online);
        }
        if !done.online && done.wrote {
            self.fav_keep.next_full = Some(now + OFFLINE_RETRY);
        }
        let mut ops = Vec::new();
        let mut home = Vec::new();
        for o in done.outcomes {
            match o {
                Outcome::Kept(id, k) => {
                    self.fav_keep.failed.remove(&id);
                    if self.catalog.photo(id).is_some() {
                        log::info!("favorites: kept {} as {}", k.from, k.path);
                        ops.push(record_op(id, Some(&k)));
                    }
                }
                Outcome::Refreshed(id, packet, sidecar) => {
                    if let Some(mut k) = kept(&self.catalog, id) {
                        k.packet = packet;
                        k.sidecar = Some(sidecar);
                        ops.push(record_op(id, Some(&k)));
                    }
                }
                Outcome::Failed(id, why) => {
                    log::warn!("favorites: {why}");
                    self.fav_keep.failed.insert(id, (now, why));
                }
                Outcome::Lost(id) => {
                    log::warn!("favorites: the kept copy of photo {} is gone from the keep folder; kept again", id.0);
                    ops.push(record_op(id, None));
                }
                Outcome::UseKept(id) => {
                    if let Some(k) = kept(&self.catalog, id) {
                        log::info!("favorites: {} is gone; the photo shows its kept copy {}", k.from, k.path);
                        ops.push(relink_to(id, &k.path));
                    }
                }
                Outcome::Home(id) => {
                    if let Some(k) = kept(&self.catalog, id) {
                        log::info!("favorites: {} is back; the photo shows it again", k.from);
                        ops.push(relink_to(id, &k.from));
                        home.push(id);
                    }
                }
            }
        }
        if !ops.is_empty()
            && let Err(e) = self.apply_bookkeeping(Op::Batch { ops })
        {
            log::warn!("favorites: {e}");
        }
        // back on its original: the edits made meanwhile go into its own sidecar too
        if !home.is_empty() && self.xmp.auto_write {
            if self.xmp_queue.on {
                self.queue_sidecars(home);
            } else {
                let synced = home.iter().filter_map(|id| self.save_sidecar(*id).ok().map(|s| (*id, s.sync))).collect();
                self.record_xmp_sync(synced);
            }
        }
    }

    /// Keep these photos' files now (or every one waiting): on the worker in the app (queued), at
    /// once elsewhere. → what happened.
    fn fav_keep_now(&mut self, ids: Option<Vec<PhotoId>>) -> Value {
        let explicit = ids.is_some();
        let keepers: Vec<PhotoId> = match ids {
            Some(ids) => ids.iter().filter_map(|id| self.catalog.photo(*id)).filter_map(|p| keeper(&self.catalog, p)).collect(),
            None => wanted(&self.catalog).into_iter().filter(|id| self.catalog.record(&kept_key(*id)).is_none()).collect(),
        };
        for id in &keepers {
            self.fav_keep.failed.remove(id);
        }
        if self.fav_keep.background {
            let now = Instant::now();
            for id in &keepers {
                self.fav_keep.due.insert(*id, now);
            }
            if !explicit {
                self.fav_keep.next_full = Some(now);
            }
            return json!({"queued": keepers.len()});
        }
        // here and now, batch after batch
        let mut kept_n = 0usize;
        let mut failed: Vec<Value> = Vec::new();
        let mut online = true;
        let todo: Vec<PhotoId> = keepers;
        for batch in todo.chunks(BATCH) {
            let Some((plan, _)) = self.fav_keep_plan(false, batch) else { break };
            let n_copies = plan.copies.len();
            let done = run_plan(plan, self.library_generation(), &AtomicBool::new(false));
            for o in &done.outcomes {
                match o {
                    Outcome::Kept(..) => kept_n += 1,
                    Outcome::Failed(id, why) => failed.push(json!({"id": id.0, "error": why})),
                    _ => {}
                }
            }
            let offline = n_copies > 0 && !done.online;
            self.fav_keep_apply(done);
            if offline {
                online = false;
                break;
            }
        }
        json!({"kept": kept_n, "failed": failed, "offline": !online})
    }

    /// The favorites. counts (see [`Status`]).
    pub fn fav_keep_status(&self) -> Status {
        let root = self.fav_keep.root.as_ref();
        let mut st = Status {
            on: root.is_some(),
            root: root.map(|r| r.display().to_string()).unwrap_or_default(),
            disk: root.map(|r| disk_name(r)).unwrap_or_default(),
            online: self.fav_keep.online,
            running: self.fav_keep.job.is_some(),
            ..Status::default()
        };
        for id in wanted(&self.catalog) {
            st.favorites += 1;
            match kept(&self.catalog, id) {
                Some(k) => {
                    st.kept += 1;
                    if !k.in_place && self.catalog.photo(id).is_some_and(|p| matches!(&p.source, Source::File { path } if *path == k.path)) {
                        st.using_kept += 1;
                    }
                }
                None => {
                    st.waiting += 1;
                    if self.fav_keep.failed.contains_key(&id) {
                        st.failed += 1;
                    }
                }
            }
        }
        st
    }

    /// Take photos' kept copies (and their sidecars) to the Trash and forget them; `ids` are any
    /// photos (a virtual copy forgets its master's). One that shows its kept copy because its
    /// original is gone is skipped (the copy is its only file). → (trashed files, forgotten ids,
    /// skipped [{id, why}]).
    pub(crate) fn fav_forget(&mut self, ids: &[PhotoId], keep_out_while_hearted: bool) -> (Vec<crate::trash::TrashedFile>, Vec<PhotoId>, Vec<Value>) {
        let mut keepers: Vec<PhotoId> = ids.iter().filter_map(|id| self.catalog.photo(*id)).filter_map(|p| keeper(&self.catalog, p)).collect();
        // a removed photo: its own record still says where its copy is
        keepers.extend(ids.iter().copied().filter(|id| self.catalog.photo(*id).is_none()));
        keepers.sort_unstable();
        keepers.dedup();
        let (mut trashed, mut gone, mut skipped, mut ops) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let wanted = wanted(&self.catalog);
        for id in keepers {
            let Some(k) = kept(&self.catalog, id) else { continue };
            if !k.in_place && self.catalog.photo(id).is_some_and(|p| matches!(&p.source, Source::File { path } if *path == k.path)) {
                skipped.push(json!({"id": id.0, "why": "its original is gone: the kept copy is its only file (delete the photo to delete it)"}));
                continue;
            }
            if !k.in_place {
                let files: Vec<PathBuf> =
                    std::iter::once(k.path.clone()).chain(k.sidecar.clone()).map(PathBuf::from).filter(|f| f.is_file()).collect();
                let results = crate::trash::trash(&files, self.trash_dir.as_deref());
                let mut ok = true;
                for (f, r) in files.iter().zip(results) {
                    match r {
                        Ok(to) => trashed.push(crate::trash::TrashedFile { original: f.display().to_string(), trash: to.display().to_string() }),
                        Err(e) => {
                            ok &= Some(f) != files.first();
                            skipped.push(json!({"id": id.0, "why": format!("{}: {e}", f.display())}));
                        }
                    }
                }
                if !ok {
                    continue;
                }
            }
            ops.push(record_op(id, None));
            if keep_out_while_hearted && wanted.contains(&id) {
                ops.push(Op::SetRecord { key: forgot_key(id), value: Some(json!(true)) });
            }
            self.fav_keep.due.remove(&id);
            gone.push(id);
        }
        if !ops.is_empty()
            && let Err(e) = self.apply_bookkeeping(Op::Batch { ops })
        {
            log::warn!("favorites: {e}");
        }
        (trashed, gone, skipped)
    }
}

fn ids_param(s: &Session, p: &Value, c: &str, required: bool) -> Result<Option<Vec<PhotoId>>> {
    match p.get("ids") {
        Some(Value::Array(a)) => Ok(Some(a.iter().filter_map(Value::as_u64).map(PhotoId).collect())),
        None | Some(Value::Null) if required => {
            let t = s.targets(p);
            if t.is_empty() { Err(bad(c, "give `ids` (or select the photos)")) } else { Ok(Some(t)) }
        }
        None | Some(Value::Null) => Ok(None),
        Some(_) => Err(bad(c, "`ids` is a list of photo ids")),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "fav.keep",
            "Keep Favorites Now",
            [],
            None,
            "{ids?} — copy hearted photos' originals + a fresh XMP sidecar with their edits to the keep folder now (default: every one still waiting); in the app it is queued on the copy worker → {queued}; elsewhere done at once → {kept, failed: [{id, error}], offline}. Off (no keep folder) → error",
            always,
            |s, p| {
                const C: &str = "fav.keep";
                if s.fav_keep.root.is_none() {
                    return Err(bad(C, "favorites keep is off (Settings ▸ General ▸ Favorites: keep. on. and a folder)"));
                }
                let ids = ids_param(s, p, C, false)?;
                Ok(s.fav_keep_now(ids))
            }
        ),
        cmd!(
            query "fav.keepStatus",
            "Favorites Kept Status",
            [],
            None,
            "{ids?} → {on, root, disk, online, favorites, kept, waiting, usingKept, failed, running, photos?: [{id, kept?: {path, sidecar, from, size, at, inPlace}, usingKept, error?}]} — how many hearted photos have their original kept safe in the keep folder, and (with ids) where each one's copy is",
            always,
            |s, p| {
                let mut v = serde_json::to_value(s.fav_keep_status()).unwrap_or_default();
                if let Some(ids) = ids_param(s, p, "fav.keepStatus", false)? {
                    let photos: Vec<Value> = ids
                        .iter()
                        .map(|id| {
                            let k = s.catalog.photo(*id).and_then(|ph| keeper(&s.catalog, ph)).unwrap_or(*id);
                            json!({
                                "id": id.0,
                                "kept": kept(&s.catalog, k),
                                "usingKept": uses_kept(&s.catalog, k),
                                "error": s.fav_keep.failed.get(&k).map(|(_, e)| e.clone()),
                            })
                        })
                        .collect();
                    v["photos"] = json!(photos);
                }
                Ok(v)
            }
        ),
        cmd!(
            "fav.forget",
            "Forget Kept Copies",
            [],
            None,
            "{ids} — the photos' kept copies and their sidecars go to the Trash and are forgotten (the only way anything leaves the keep folder); a photo still hearted is not kept again until it is hearted again; one showing its kept copy because its original is gone is skipped → {forgot, trashed: [path], skipped: [{id, why}]}",
            always,
            |s, p| {
                let Some(ids) = ids_param(s, p, "fav.forget", true)? else { return Err(bad("fav.forget", "give `ids`")) };
                let (trashed, gone, skipped) = s.fav_forget(&ids, true);
                let paths: Vec<&str> = trashed.iter().map(|t| t.original.as_str()).collect();
                Ok(json!({"forgot": gone.len(), "trashed": paths, "skipped": skipped}))
            }
        ),
    ]
}
