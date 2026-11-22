//! Sync from Lightroom Classic again (`library.syncLightroom`): bring what changed in the catalog
//! since the migration (or the last sync) into a library migrated from it — photos added (or
//! whose drive is back), develop settings, ratings / flags / colour labels / keywords / IPTC
//! text, collection membership, history and snapshots — without overwriting what was changed in
//! LightCraft since, unless asked.
//!
//! **What changed where.** After each sync `<library>/lightroom-sync.json` keeps, per photo, a
//! digest of Lightroom's settings text and of ours, and the metadata both sides had, so the
//! next sync knows exactly which side moved. Before the first sync there is only the
//! migration's record: Lightroom changed a photo when its latest history step (or touch, for
//! metadata) is newer than anything the migration saw; LightCraft changed it when it has a
//! History step here (the migration writes none) or a newer edit time than the migration gave it.
//!
//! **Conflicts** (both sides changed the look): never overwritten by default — they are listed
//! (newer side included) and kept as they are here until a sync with `conflicts: lightroom` /
//! `newest`, or `take: [photo ids]`, resolves them; Lightroom's settings then replace ours and
//! ours are kept as a version ("LightCraft before Lightroom sync"). A dry run reports
//! everything and changes nothing. One undo step.
//!
//! Lightroom's catalog is only read, from a copy ([`crate::lr_migrate::with_catalog_copy`]).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lightcraft_catalog::{ColorLabel, Flag, Op, Photo, PhotoId, Version};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Session;
use crate::lr_history::{ExtrasOptions, Source, store_extras};
use crate::lr_migrate::{LrImage, MigrateOptions, RESUME_FILE, Records, library_ids, migrate, prepare_image};

/// The sync record in the library folder.
pub const SYNC_FILE: &str = "lightroom-sync.json";
/// The version our settings are kept as when Lightroom's replace them in a conflict.
pub const KEPT_VERSION: &str = "LightCraft before Lightroom sync";
/// Conflicts listed by name in a report.
const MAX_LISTED: usize = 200;

/// Metadata as Lightroom has it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LrMeta {
    pub rating: u8,
    pub pick: i8,
    pub label: String,
    /// Sorted.
    pub keywords: Vec<String>,
    /// Caption, copyright, creator, location, city, state, country.
    pub text: [String; 7],
}

/// Metadata as we have it (the fields a sync touches).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OurMeta {
    pub rating: u8,
    pub pick: i8,
    pub label: Option<ColorLabel>,
    pub keywords: Vec<String>,
    pub text: [String; 7],
}

/// One photo at the last sync.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoState {
    /// Lightroom's image id.
    pub lr: i64,
    /// Digest of Lightroom's settings text ("" = an open conflict: asked again next time).
    pub dev: String,
    /// Digest of our settings ("" with `dev`).
    pub ours: String,
    pub lr_meta: LrMeta,
    pub our_meta: OurMeta,
}

/// `lightroom-sync.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncState {
    pub catalog: String,
    pub synced: String,
    /// The newest Lightroom activity (history step or touch) the last sync saw.
    pub lr_latest: Option<String>,
    pub photos: BTreeMap<u64, PhotoState>,
    /// Lightroom collection id → our photos in it at the last sync.
    pub collections: BTreeMap<i64, Vec<u64>>,
    /// New masters' files a sync tried and couldn't add (byte-identical to a photo already
    /// here, or unreadable): not offered again.
    pub not_added: BTreeSet<String>,
}

/// How a photo changed on both sides is settled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Conflicts {
    /// Keep ours and list it (asked again next time).
    #[default]
    Ask,
    /// Lightroom's replace ours (ours kept as a version).
    Lightroom,
    /// Ours stay, and count as synced.
    LightCraft,
    /// The side edited last wins.
    Newest,
}

impl Conflicts {
    pub fn parse(s: &str) -> Option<Conflicts> {
        Some(match s.to_ascii_lowercase().as_str() {
            "ask" | "keep" => Conflicts::Ask,
            "lightroom" | "theirs" => Conflicts::Lightroom,
            "lightcraft" | "mine" | "ours" => Conflicts::LightCraft,
            "newest" | "newer" => Conflicts::Newest,
            _ => return None,
        })
    }
}

/// Options of [`sync`].
#[derive(Clone, Debug, Default)]
pub struct SyncOptions {
    pub dry_run: bool,
    pub conflicts: Conflicts,
    /// Photos whose conflict goes Lightroom's way / ours.
    pub take: HashSet<PhotoId>,
    pub keep: HashSet<PhotoId>,
    /// Import photos new in Lightroom (or whose files are back).
    pub import_new: bool,
    pub collections: bool,
    pub history: bool,
}

/// FNV-1a 64, hex (a change detector, not a security hash).
pub fn digest(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn our_digest(p: &Photo) -> String {
    digest(&serde_json::to_vec(&*p.develop).unwrap_or_default())
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v: Vec<String> = v.iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
    v.sort_by_key(|k| k.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

pub fn lr_meta(im: &LrImage) -> LrMeta {
    LrMeta {
        rating: im.rating,
        pick: im.pick,
        label: im.label.trim().to_string(),
        keywords: sorted(&im.keywords),
        text: [&im.caption, &im.copyright, &im.creator, &im.location, &im.city, &im.state, &im.country].map(|t| t.trim().to_string()),
    }
}

pub fn our_meta(p: &Photo) -> OurMeta {
    let m = &p.meta;
    OurMeta {
        rating: p.rating,
        pick: match p.flag {
            Flag::Pick => 1,
            Flag::Reject => -1,
            Flag::None => 0,
        },
        label: p.label,
        keywords: sorted(&m.keywords),
        text: [&m.caption, &m.copyright, &m.creator, &m.location, &m.city, &m.state, &m.country].map(|t| t.trim().to_string()),
    }
}

/// The newest Lightroom activity on a photo (its last history step or touch).
fn activity(im: &LrImage) -> Option<&str> {
    match (im.last_edit.as_deref(), im.touched.as_deref()) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 1 << 30 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The library's sync record (none before the first sync, or when it can't be read).
pub fn load_state(lib: &Path) -> Option<SyncState> {
    read_json(&lib.join(SYNC_FILE))
}

/// What the migration recorded: the newest Lightroom time it saw, and Lightroom's edit time per
/// photo (`library.resumePoint`'s record).
fn migration_record(lib: &Path) -> (Option<String>, HashMap<u64, String>) {
    let Some(doc) = read_json::<Value>(&lib.join(RESUME_FILE)) else { return (None, HashMap::new()) };
    let mut latest: Option<String> = None;
    let mut edited = HashMap::new();
    for k in ["touched", "edited"] {
        for (id, t) in doc[k].as_object().into_iter().flatten() {
            let Some(t) = t.as_str() else { continue };
            if latest.as_deref().is_none_or(|l| t > l) {
                latest = Some(t.to_string());
            }
            if k == "edited"
                && let Ok(id) = id.parse::<u64>()
            {
                edited.insert(id, t.to_string());
            }
        }
    }
    (latest, edited)
}

/// The Lightroom images a sync needs the history of, worked out without the session (the app does
/// this on a worker thread): photos whose settings changed in Lightroom, and masters whose file
/// isn't one of `library_paths` (new in Lightroom, or back online) with their copies.
pub fn wanted(rec: &Records, lib: Option<&Path>, library_paths: &HashSet<String>) -> HashSet<i64> {
    let state = lib.and_then(load_state);
    let cutoff = state.as_ref().and_then(|s| s.lr_latest.clone()).or_else(|| lib.and_then(|l| migration_record(l).0));
    let by_lr: HashMap<i64, &PhotoState> = state.iter().flat_map(|s| s.photos.values()).map(|p| (p.lr, p)).collect();
    let mut out = HashSet::new();
    for im in &rec.images {
        let changed = match by_lr.get(&im.id) {
            Some(st) => st.dev != digest(im.develop.as_bytes()),
            None => cutoff.as_deref().is_none_or(|c| im.last_edit.as_deref().is_some_and(|e| e > c)),
        };
        let new = im.master.is_none()
            && !library_paths.contains(&im.path)
            && state.as_ref().is_none_or(|s| !s.not_added.contains(&im.path))
            && Path::new(&im.path).is_file();
        if changed || new {
            out.insert(im.id);
        }
    }
    let masters: HashSet<i64> = out.clone();
    out.extend(rec.images.iter().filter(|i| i.master.is_some_and(|m| masters.contains(&m))).map(|i| i.id));
    out
}

/// Settle one field: `Some(true)` = take Lightroom's, `Some(false)` = keep ours, `None` = a
/// conflict to settle.
fn settle(lr_changed: bool, lc_changed: bool) -> Option<bool> {
    match (lr_changed, lc_changed) {
        (false, _) => Some(false),
        (true, false) => Some(true),
        (true, true) => None,
    }
}

/// A metadata field (an open conflict's).
#[derive(Clone, Copy)]
enum Field {
    Rating,
    Pick,
    Label,
    Text(usize),
}

#[derive(Default)]
struct Counts {
    rating: usize,
    flag: usize,
    label: usize,
    keywords: usize,
    text: usize,
    meta_conflicts: usize,
}

/// Lightroom's settings for photo `p` (as the migration maps them), turned like the photo.
fn lightroom_settings(s: &Session, p: &Photo, im: &LrImage) -> Option<lightcraft_develop::DevelopSettings> {
    let aspect = (p.width > 0 && p.height > 0).then(|| p.width as f64 / p.height as f64);
    let prep = prepare_image(im, aspect);
    let mut partial = prep.develop?;
    if let Some((name, amount)) = &prep.creative_look
        && let Some(id) = crate::lr_migrate::matching_profile(s, &HashMap::new(), name)
    {
        partial["profile"] = json!({"id": id, "amount": (amount * 100.0).clamp(0.0, 200.0)});
    }
    let mut d = lightcraft_develop::apply_partial(&p.import_defaults(), &partial, 1.0);
    d.orientation = match crate::lr_migrate::lr_quarter_turns(&im.orientation) {
        Some(q) => lightcraft_geom::Orientation::from_parts(false, (q + 4 - prep.file_turns % 4) % 4),
        None => p.develop.orientation,
    };
    Some(d)
}

/// Sync `rec` (a re-read of the catalog the library was migrated from) into the session. `src`:
/// where the changed photos' history and snapshots come from (none: not brought over).
pub fn sync(s: &mut Session, rec: &Records, src: Option<Source>, opts: &SyncOptions) -> crate::Result<Value> {
    let undo0 = s.undo.len();
    let now = (s.clock)();
    let lib: Option<PathBuf> = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone());
    let old = lib.as_deref().and_then(load_state);
    let (migrated_latest, migrated_edit) = lib.as_deref().map(migration_record).unwrap_or_default();
    let first = old.is_none();
    let cutoff = old.as_ref().and_then(|s| s.lr_latest.clone()).or(migrated_latest);
    let old_by_lr: HashMap<i64, (u64, &PhotoState)> = old.iter().flat_map(|s| s.photos.iter()).map(|(id, p)| (p.lr, (*id, p))).collect();
    let lr_after_cutoff = |t: Option<&str>| cutoff.as_deref().is_none_or(|c| t.is_some_and(|t| t > c));
    let lr_by_id: HashMap<i64, &LrImage> = rec.images.iter().map(|i| (i.id, i)).collect();

    // ---- photos new in Lightroom (or whose drive is back)
    let ids0 = library_ids(s, rec);
    let have_paths: HashSet<String> = s
        .catalog
        .photos()
        .filter_map(|p| if let lightcraft_catalog::Source::File { path } = &p.source { Some(path.clone()) } else { None })
        .collect();
    let mut missing = 0usize;
    let new_masters: Vec<&LrImage> = rec
        .images
        .iter()
        .filter(|i| i.master.is_none() && !ids0.contains_key(&i.id) && !have_paths.contains(&i.path))
        // (files an earlier sync couldn't add — copies of photos already here, unreadable files)
        .filter(|i| old.as_ref().is_none_or(|o| !o.not_added.contains(&i.path)))
        .filter(|i| {
            let there = Path::new(&i.path).is_file();
            missing += usize::from(!there);
            there
        })
        .collect();
    let new_ids: HashSet<i64> = new_masters.iter().map(|m| m.id).collect();
    let mut new_report = json!({"found": new_masters.len(), "missingFiles": missing});
    if opts.import_new && !new_masters.is_empty() {
        let sub = Records {
            catalog: rec.catalog.clone(),
            images: rec.images.iter().filter(|i| new_ids.contains(&i.id) || i.master.is_some_and(|m| new_ids.contains(&m))).cloned().collect(),
            collections: Vec::new(),
        };
        let r = migrate(s, &sub, &MigrateOptions { import: true, only_new: true, dry_run: opts.dry_run, ..Default::default() })?;
        new_report["imported"] = r["import"]["imported"].clone();
        new_report["duplicates"] = r["import"]["duplicates"].clone();
        new_report["failed"] = r["import"]["failed"].clone();
        new_report["withEdits"] = r["develop"]["applied"].clone();
    }
    // virtual copies made in Lightroom of photos we have
    let mut copies_made = 0usize;
    let mut fresh: HashSet<i64> = new_ids.clone();
    for c in rec.images.iter().filter(|i| i.master.is_some() && !ids0.contains_key(&i.id)) {
        let (Some(m), Some(mid)) = (c.master.and_then(|m| lr_by_id.get(&m)), c.master.and_then(|m| ids0.get(&m).copied())) else { continue };
        if c.sync_duplicate && c.develop == m.develop && c.rating == m.rating && c.pick == m.pick {
            continue;
        }
        fresh.insert(c.id);
        copies_made += 1;
        if !opts.dry_run {
            s.selection = crate::Selection::single(mid);
            s.execute("photo.virtualCopy", &json!({"ids": [mid.0], "name": c.copy_name.clone().unwrap_or_else(|| "Copy 1".into())}))?;
        }
    }
    new_report["virtualCopies"] = json!(copies_made);
    let ids = library_ids(s, rec);

    // ---- per photo: develop settings and metadata
    let mut ops = Vec::new();
    let mut state = SyncState { catalog: rec.catalog.clone(), synced: now.clone(), ..Default::default() };
    let (mut dev_lr, mut dev_kept, mut dev_resolved, mut unchanged) = (0usize, 0usize, 0usize, 0usize);
    let mut conflicts = Vec::new();
    let mut n_conflicts = 0usize;
    let mut counts = Counts::default();
    let mut history_ids: HashSet<i64> = HashSet::new();
    let mut ids_sorted: Vec<(&i64, &PhotoId)> = ids.iter().collect();
    ids_sorted.sort();
    for (lr_id, id) in ids_sorted {
        let (Some(im), Some(p)) = (lr_by_id.get(lr_id), s.catalog.photo(*id)) else { continue };
        let lr_text = digest(im.develop.as_bytes());
        let prev = old_by_lr.get(lr_id).map(|(_, st)| *st);
        let is_fresh = fresh.contains(lr_id) || new_ids.contains(lr_id) || im.master.is_some_and(|m| new_ids.contains(&m));
        // develop
        let lr_changed = match prev {
            Some(st) => st.dev != lr_text,
            None => is_fresh || lr_after_cutoff(im.last_edit.as_deref()),
        };
        let lc_changed = !is_fresh
            && match prev {
                Some(st) => st.ours != our_digest(p),
                None => {
                    !p.history.is_empty()
                        || (p.edited.is_some() && migrated_edit.get(&id.0).is_none_or(|t| Some(t) != p.edited.as_ref()) && !first_unedited(p))
                }
            };
        let mut take = settle(lr_changed, lc_changed);
        let mut open_conflict = false;
        if take.is_none() {
            n_conflicts += 1;
            let lr_at = im.last_edit.clone();
            let lc_at = p.edited.clone();
            let newer = if lr_at > lc_at { "lightroom" } else { "lightcraft" };
            if conflicts.len() < MAX_LISTED {
                conflicts.push(json!({"photoId": id.0, "file": p.file_name, "lightroomEdited": lr_at, "lightcraftEdited": lc_at, "newer": newer}));
            }
            take = if opts.take.contains(id) {
                Some(true)
            } else if opts.keep.contains(id) {
                Some(false)
            } else {
                match opts.conflicts {
                    Conflicts::Ask => {
                        open_conflict = true;
                        Some(false)
                    }
                    Conflicts::Lightroom => Some(true),
                    Conflicts::LightCraft => Some(false),
                    Conflicts::Newest => Some(newer == "lightroom"),
                }
            };
            if !open_conflict {
                dev_resolved += 1;
            }
        }
        let mut new_dev: Option<lightcraft_develop::DevelopSettings> = None;
        if take == Some(true) && lr_changed {
            if let Some(d) = lightroom_settings(s, p, im).filter(|d| *d != *p.develop) {
                dev_lr += 1;
                history_ids.insert(*lr_id);
                if lc_changed {
                    // ours stay as a version
                    let mut versions = p.versions.clone();
                    versions.push(Version { name: KEPT_VERSION.into(), created: now.clone(), settings: p.develop.clone(), auto: false });
                    ops.push(Op::SetVersions { id: *id, versions });
                }
                new_dev = Some(d);
            }
        } else if lr_changed {
            dev_kept += 1;
        } else if !is_fresh {
            unchanged += 1;
        }
        if lr_changed && is_fresh {
            history_ids.insert(*lr_id);
        }
        if let Some(d) = &new_dev {
            let settings = Arc::new(d.clone());
            let edited = im.last_edit.clone().or_else(|| Some(now.clone()));
            ops.push(Op::SetDevelop { id: *id, settings: settings.clone(), label: "Lightroom Sync".into(), edited });
            ops.push(Op::PushHistory { id: *id, step: lightcraft_catalog::HistoryStep { label: "Lightroom Sync".into(), settings } });
        }

        // metadata, field by field
        let lr_m = lr_meta(im);
        let ours = our_meta(p);
        let mut want = ours.clone();
        let lr_touched = lr_after_cutoff(activity(im));
        let base = prev.map(|st| (&st.lr_meta, &st.our_meta));
        let lr_label = crate::lr_migrate::label_of(s, &lr_m.label);
        let field = |lr_moved: bool, ours_moved: bool| settle(lr_moved, ours_moved);
        // (first sync: Lightroom moved when it touched the photo after the migration, and we did
        // when the photo has an edit here)
        let ours_any = lc_changed;
        let mut open: Vec<Field> = Vec::new();
        macro_rules! one {
            ($lr:expr, $ours:expr, $lr_base:expr, $our_base:expr, $count:ident, $apply:expr, $open:expr) => {
                if $lr != $ours {
                    let (lm, om) = match base {
                        Some(_) => ($lr != $lr_base, $ours != $our_base),
                        None => (lr_touched, ours_any),
                    };
                    match field(lm, om) {
                        Some(true) => {
                            $apply;
                            counts.$count += 1;
                        }
                        Some(false) => {}
                        None => {
                            counts.meta_conflicts += 1;
                            if matches!(opts.conflicts, Conflicts::Lightroom) || opts.take.contains(id) {
                                $apply;
                                counts.$count += 1;
                            } else if !(matches!(opts.conflicts, Conflicts::LightCraft) || opts.keep.contains(id)) {
                                // still open: asked again next time
                                open.push($open);
                            }
                        }
                    }
                }
            };
        }
        let (b_lr, b_our) = base.map(|(l, o)| (l.clone(), o.clone())).unwrap_or_default();
        one!(lr_m.rating, ours.rating, b_lr.rating, b_our.rating, rating, want.rating = lr_m.rating, Field::Rating);
        one!(lr_m.pick, ours.pick, b_lr.pick, b_our.pick, flag, want.pick = lr_m.pick, Field::Pick);
        if lr_label.is_some() || !lr_m.label.is_empty() || base.is_some() {
            one!(lr_label, ours.label, crate::lr_migrate::label_of(s, &b_lr.label), b_our.label, label, want.label = lr_label, Field::Label);
        }
        for i in 0..7 {
            // (Lightroom's empty fields don't clear ours)
            if !lr_m.text[i].is_empty() {
                one!(lr_m.text[i], ours.text[i], b_lr.text[i], b_our.text[i], text, want.text[i] = lr_m.text[i].clone(), Field::Text(i));
            }
        }
        // keywords: Lightroom's additions and (since the last sync) removals
        if lr_m.keywords != ours.keywords && (base.is_some() || lr_touched) {
            let has = |list: &[String], k: &str| list.iter().any(|x| x.eq_ignore_ascii_case(k));
            let mut kw = want.keywords.clone();
            for k in &lr_m.keywords {
                if !has(&kw, k) && (base.is_none() || !has(&b_lr.keywords, k)) {
                    kw.push(k.clone());
                }
            }
            if base.is_some() {
                kw.retain(|k| has(&lr_m.keywords, k) || !has(&b_lr.keywords, k));
            }
            let kw = sorted(&kw);
            if kw != want.keywords {
                counts.keywords += 1;
                want.keywords = kw;
            }
        }
        if want != ours {
            if want.rating != ours.rating {
                ops.push(Op::SetRating { id: *id, rating: want.rating });
            }
            if want.pick != ours.pick {
                let flag = match want.pick {
                    1 => Flag::Pick,
                    -1 => Flag::Reject,
                    _ => Flag::None,
                };
                ops.push(Op::SetFlag { id: *id, flag });
            }
            if want.label != ours.label {
                ops.push(Op::SetLabel { id: *id, label: want.label });
            }
            if want.keywords != ours.keywords || want.text != ours.text {
                let mut m = p.meta.clone();
                // keep our own spelling / order of keywords that stay
                m.keywords.retain(|k| want.keywords.iter().any(|w| w.eq_ignore_ascii_case(k.trim())));
                for k in &want.keywords {
                    if !m.keywords.iter().any(|x| x.trim().eq_ignore_ascii_case(k)) {
                        m.keywords.push(k.clone());
                    }
                }
                let t = &want.text;
                for (dst, src) in [
                    (&mut m.caption, &t[0]),
                    (&mut m.copyright, &t[1]),
                    (&mut m.creator, &t[2]),
                    (&mut m.location, &t[3]),
                    (&mut m.city, &t[4]),
                    (&mut m.state, &t[5]),
                    (&mut m.country, &t[6]),
                ] {
                    if dst.trim() != src.as_str() {
                        *dst = src.clone();
                    }
                }
                if m != p.meta {
                    ops.push(Op::SetMeta { id: *id, meta: Box::new(m) });
                }
            }
        }
        let after_dev = match &new_dev {
            Some(d) => digest(&serde_json::to_vec(d).unwrap_or_default()),
            None => our_digest(p),
        };
        // an open metadata conflict keeps the last sync's values, so it's asked again
        let (mut st_lr, mut st_our) = (lr_m, want);
        for f in open {
            match f {
                Field::Rating => (st_lr.rating, st_our.rating) = (b_lr.rating, b_our.rating),
                Field::Pick => (st_lr.pick, st_our.pick) = (b_lr.pick, b_our.pick),
                Field::Label => (st_lr.label, st_our.label) = (b_lr.label.clone(), b_our.label),
                Field::Text(i) => {
                    if let (Some(l), Some(o), Some(bl), Some(bo)) =
                        (st_lr.text.get_mut(i), st_our.text.get_mut(i), b_lr.text.get(i), b_our.text.get(i))
                    {
                        *l = bl.clone();
                        *o = bo.clone();
                    }
                }
            }
        }
        state.photos.insert(
            id.0,
            PhotoState {
                lr: *lr_id,
                dev: if open_conflict { String::new() } else { lr_text },
                ours: if open_conflict { String::new() } else { after_dev },
                lr_meta: st_lr,
                our_meta: st_our,
            },
        );
    }

    // ---- collections: members added / removed in Lightroom since the last sync (photos new
    // this time join theirs)
    let (mut added, mut removed, mut albums_made) = (0usize, 0usize, 0usize);
    if opts.collections {
        let member = |i: &i64| ids.get(i).copied().or_else(|| lr_by_id.get(i).and_then(|im| im.master).and_then(|m| ids.get(&m).copied()));
        for c in rec.collections.iter().filter(|c| matches!(c.kind.as_str(), "collection" | "quick")) {
            let now_members: Vec<PhotoId> = c.images.iter().filter_map(member).collect();
            let before: Option<HashSet<u64>> = old.as_ref().and_then(|o| o.collections.get(&c.id)).map(|v| v.iter().copied().collect());
            let album = if c.kind == "quick" {
                s.catalog.quick_collection()
            } else {
                let parent = c.parent.and_then(|pid| rec.collections.iter().find(|x| x.id == pid)).map(|x| x.name.clone());
                find_album(s, &c.name, parent.as_deref())
            };
            let to_add: Vec<PhotoId> = now_members
                .iter()
                .copied()
                .filter(|pid| match &before {
                    Some(b) => !b.contains(&pid.0),
                    None => album.is_none() || c.images.iter().any(|i| fresh.contains(i) && ids.get(i) == Some(pid)),
                })
                .collect();
            let to_remove: Vec<PhotoId> = match &before {
                Some(b) => b.iter().map(|x| PhotoId(*x)).filter(|pid| !now_members.contains(pid)).collect(),
                None => Vec::new(),
            };
            state.collections.insert(c.id, now_members.iter().map(|p| p.0).collect());
            if to_add.is_empty() && to_remove.is_empty() {
                continue;
            }
            let aid = match album {
                Some(a) => a,
                None if opts.dry_run => {
                    albums_made += 1;
                    added += to_add.len();
                    continue;
                }
                None => {
                    albums_made += 1;
                    let parent = c.parent.and_then(|pid| rec.collections.iter().find(|x| x.id == pid)).and_then(|x| find_album(s, &x.name, None));
                    crate::lr_migrate::find_or_make_album(s, if c.name.trim().is_empty() { "Untitled Collection" } else { &c.name }, parent, false)?
                }
            };
            let mut photos = s.catalog.album(aid).map(|a| a.photos.clone()).unwrap_or_default();
            for pid in &to_add {
                if !photos.contains(pid) {
                    photos.push(*pid);
                    added += 1;
                }
            }
            let n = photos.len();
            photos.retain(|pid| !to_remove.contains(pid));
            removed += n - photos.len();
            if s.catalog.album(aid).is_some_and(|a| a.photos != photos) {
                ops.push(Op::SetAlbumPhotos { id: aid, photos });
            }
        }
    }
    let changes = ops.len();
    if !opts.dry_run && !ops.is_empty() {
        s.commit("Sync from Lightroom Classic", Op::Batch { ops })?;
    }

    // ---- history and snapshots of the photos that changed or came in
    let mut extras = Value::Null;
    if opts.history
        && let Some(src) = src
        && !history_ids.is_empty()
    {
        let eo = ExtrasOptions { history: true, snapshots: true, dry_run: opts.dry_run, only: Some(history_ids.clone()) };
        extras = store_extras(s, src, rec, &eo)?;
    }
    let steps = s.undo.len().saturating_sub(undo0);
    s.merge_undo(steps, "Sync from Lightroom Classic");

    // ---- the record for next time
    state.lr_latest = rec.images.iter().filter_map(activity).max().map(str::to_string).or(cutoff.clone());
    state.not_added = old.as_ref().map(|o| o.not_added.clone()).unwrap_or_default();
    if opts.import_new {
        let now_paths: HashSet<&str> = s
            .catalog
            .photos()
            .filter_map(|p| if let lightcraft_catalog::Source::File { path } = &p.source { Some(path.as_str()) } else { None })
            .collect();
        state.not_added.extend(new_masters.iter().filter(|m| !now_paths.contains(m.path.as_str())).map(|m| m.path.clone()));
    }
    // photos this sync didn't see (not in the library now) keep their old record
    if let Some(o) = &old {
        for (k, v) in &o.photos {
            state.photos.entry(*k).or_insert_with(|| v.clone());
        }
    }
    if !opts.dry_run
        && let Some(dir) = &lib
    {
        let path = dir.join(SYNC_FILE);
        let tmp = dir.join(format!("{SYNC_FILE}.tmp"));
        let bytes = serde_json::to_vec(&state).unwrap_or_default();
        if std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
            log::warn!("lightroom sync: could not write {}", path.display());
        }
    }
    Ok(json!({
        "catalog": rec.catalog,
        "dryRun": opts.dry_run,
        "firstSync": first,
        "since": cutoff,
        "newPhotos": new_report,
        "develop": {
            "fromLightroom": dev_lr,
            "keptMine": dev_kept,
            "conflicts": n_conflicts,
            "resolved": dev_resolved,
            "conflictList": conflicts,
            "unchanged": unchanged,
        },
        "metadata": {
            "rating": counts.rating,
            "flag": counts.flag,
            "label": counts.label,
            "keywords": counts.keywords,
            "text": counts.text,
            "conflicts": counts.meta_conflicts,
        },
        "collections": {"added": added, "removed": removed, "albumsMade": albums_made},
        "history": extras,
        "changes": changes,
    }))
}

/// A photo the migration left at Lightroom's defaults (no edit time of Lightroom's recorded, no
/// History here): nothing of ours to protect.
fn first_unedited(p: &Photo) -> bool {
    p.history.is_empty() && !p.is_edited()
}

/// Our album named `name` (under a folder named `parent`, if given).
fn find_album(s: &Session, name: &str, parent: Option<&str>) -> Option<lightcraft_catalog::AlbumId> {
    let name = if name.trim().is_empty() { "Untitled Collection" } else { name };
    s.catalog
        .albums()
        .filter(|a| !a.is_smart() && !a.quick && a.name == name)
        .find(|a| match parent {
            None => true,
            Some(pn) => a.parent.and_then(|pid| s.catalog.album(pid)).is_some_and(|pa| pa.name == pn),
        })
        .map(|a| a.id)
}

#[cfg(test)]
#[path = "tests_lr_sync.rs"]
mod tests;
