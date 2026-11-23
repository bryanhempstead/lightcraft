//! Lightroom's two kinds of delete (L03): **Remove** (out of the library, the files stay) and
//! **Delete from Disk** (the original and its XMP sidecar go to the system Trash, then the photo
//! leaves the library), plus Delete Rejected Photos. A virtual copy has no file of its own: it is
//! only ever removed. Removing a master removes its virtual copies with it. A photo whose file
//! can't be moved to the Trash stays in the library (reported in `failed`).
//!
//! Photo ▸ Delete Photo (`photo.delete`) still moves photos to Recently Deleted (LightCraft's own
//! soft delete); what the app's Delete key does is a setting (Settings ▸ General ▸ Delete key:
//! the Trash, Recently Deleted, the album only, or Lightroom's question).
//!
//! Undo really brings a Delete from Disk back: the step remembers where each file went in the
//! Trash, and ⌘Z moves them back to their exact paths before the photos return (a file emptied
//! from the Trash, or whose path is taken now, stays out and is reported; nothing is
//! overwritten); redo trashes them again. The undo history lives in memory only, so after a
//! reopen Put Back Last Delete (`photo.putBackLastDelete`, from the library's
//! `last-delete.json`) does the same for the last delete.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use lightcraft_catalog::{Flag, Op, PhotoId, Source};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, bool_or, cmd, has_selection};
use crate::trash::{LastDelete, TrashedFile};
use crate::{EngineError, LibrarySource, Result, Selection, Session};

/// `ids` plus the virtual copies of the masters among them (sorted, no repeats).
fn with_copies(s: &Session, ids: &[PhotoId]) -> Vec<PhotoId> {
    let set: HashSet<PhotoId> = ids.iter().copied().filter(|id| s.catalog.photo(*id).is_some()).collect();
    let mut out: Vec<PhotoId> = set.iter().copied().collect();
    out.extend(s.catalog.photos().filter(|p| p.copy_of.is_some_and(|m| set.contains(&m)) && !set.contains(&p.id)).map(|p| p.id));
    out.sort();
    out.dedup();
    out
}

/// Remove `ids` (and their virtual copies) from the library as one undo step.
fn remove(s: &mut Session, label: &str, ids: &[PhotoId]) -> Result<usize> {
    let all = with_copies(s, ids);
    if all.is_empty() {
        return Ok(0);
    }
    let op = s.catalog.delete_photos_permanently_ops(&all);
    s.commit(label, op)?;
    let gone: HashSet<PhotoId> = all.iter().copied().collect();
    let vis = s.visible_cloned();
    s.selection = vis.iter().find(|id| !gone.contains(id)).map(|f| Selection::single(*f)).unwrap_or_default();
    Ok(all.len())
}

/// The files a photo owns on disk: its original and the XMP sidecar(s) no other photo shares
/// (`IMG_1.xmp` next to both `IMG_1.CR3` and `IMG_1.JPG` stays while one of them is kept).
fn owned_files(s: &Session, id: PhotoId, leaving: &HashSet<PhotoId>) -> Vec<PathBuf> {
    let Some(p) = s.catalog.photo(id) else { return vec![] };
    if p.copy_of.is_some() {
        return vec![];
    }
    let Source::File { path } = &p.source else { return vec![] };
    let mut files = vec![PathBuf::from(path)];
    let stem_key = |path: &str| {
        let p = Path::new(path);
        (p.parent().map(Path::to_path_buf), p.file_stem().map(|s| s.to_ascii_lowercase()))
    };
    let mine = stem_key(path);
    let shared_stem = s
        .catalog
        .photos()
        .filter(|o| !leaving.contains(&o.id) && o.copy_of.is_none())
        .any(|o| matches!(&o.source, Source::File { path: op } if op != path && stem_key(op) == mine));
    for naming in [crate::sidecar::SidecarNaming::Full, crate::sidecar::SidecarNaming::Stem] {
        if naming == crate::sidecar::SidecarNaming::Stem && shared_stem {
            continue;
        }
        for c in [crate::sidecar::sidecar_path(path, naming), crate::sidecar::sidecar_path(path, naming).with_extension("XMP")] {
            // .xmp and .XMP are one file on a case-insensitive volume
            if c.is_file() && !files.iter().any(|f| crate::rename::same_file(f, &c)) {
                files.push(c);
            }
        }
    }
    files
}

/// Move the photos' files to the Trash, then remove from the library those whose files went (or
/// that have none), as one undo step that remembers where the files went. Returns `{trashed,
/// removed, failed}`.
fn delete_from_disk(s: &mut Session, label: &str, ids: &[PhotoId]) -> Result<Value> {
    let all = with_copies(s, ids);
    let leaving: HashSet<PhotoId> = all.iter().copied().collect();
    let mut ok_ids = Vec::new();
    let mut trashed: Vec<TrashedFile> = Vec::new();
    let mut failed = Vec::new();
    // a master's virtual copies go only if the master does
    let masters: Vec<PhotoId> = all.iter().copied().filter(|id| s.catalog.photo(*id).is_some_and(|p| p.copy_of.is_none())).collect();
    let mut gone_masters: HashSet<PhotoId> = HashSet::new();
    for id in &masters {
        let files = owned_files(s, *id, &leaving);
        let missing_original = files.first().is_some_and(|f| !f.exists());
        // a photo whose original is already gone (offline, deleted elsewhere): just remove it
        let to_trash: Vec<PathBuf> = files.into_iter().filter(|f| f.exists()).collect();
        let results = crate::trash::trash(&to_trash, s.trash_dir.as_deref());
        let mut all_ok = true;
        let mut mine = Vec::new();
        for (f, r) in to_trash.iter().zip(results) {
            match r {
                Ok(to) => mine.push(TrashedFile { original: f.display().to_string(), trash: to.display().to_string() }),
                Err(e) => {
                    // only the original's failure keeps the photo; a sidecar that stayed is reported
                    if Some(f) == to_trash.first() && !missing_original {
                        all_ok = false;
                    }
                    failed.push(json!({"id": id.0, "path": f.display().to_string(), "error": e}));
                }
            }
        }
        if all_ok {
            ok_ids.push(*id);
            gone_masters.insert(*id);
            trashed.extend(mine);
        } else {
            // the photo stays: so do its sidecars
            put_back_all(&mine);
        }
    }
    // virtual copies: with their master, or on their own when the master stays
    ok_ids.extend(
        all.iter()
            .copied()
            .filter(|id| s.catalog.photo(*id).and_then(|p| p.copy_of).is_some_and(|m| !leaving.contains(&m) || gone_masters.contains(&m))),
    );
    let gone = (!trashed.is_empty()).then(|| LastDelete::of(s, label, &with_copies(s, &ok_ids), trashed.clone()));
    let removed = match remove(s, label, &ok_ids) {
        Ok(n) => n,
        Err(e) => {
            // the library didn't change: the files come back
            put_back_all(&trashed);
            return Err(e);
        }
    };
    if removed > 0
        && let Some(step) = s.undo.last_mut()
    {
        step.trashed = trashed.clone();
    }
    if gone.is_some() {
        remember_last_delete(s, gone);
    }
    let trashed: Vec<Value> = trashed.iter().map(|f| json!({"path": f.original, "trash": f.trash})).collect();
    Ok(json!({"trashed": trashed, "removed": removed, "failed": failed}))
}

/// Put files that went to the Trash back where they were (a delete that didn't happen).
fn put_back_all(files: &[TrashedFile]) {
    for f in files {
        if let Err(e) = crate::trash::put_back(f) {
            log::warn!("delete: {e}");
        }
    }
}

/// An undo whose catalog change failed after its files came back: they go back into the Trash
/// where they were (the step stays as it was).
pub(crate) fn retrash(files: &[TrashedFile]) {
    for f in files {
        if let Err(e) = std::fs::rename(&f.original, &f.trash) {
            log::warn!("undo: {}: can't move it back to the Trash ({e})", f.original);
        }
    }
}

/// Redo of a delete: the files that are at their originals go to the Trash again — all of them,
/// or none (those that went come back) and the redo fails.
pub(crate) fn trash_again(s: &Session, files: &[TrashedFile]) -> Result<Vec<TrashedFile>> {
    let present: Vec<&TrashedFile> = files.iter().filter(|f| Path::new(&f.original).exists()).collect();
    if present.is_empty() {
        return Ok(Vec::new());
    }
    let paths: Vec<PathBuf> = present.iter().map(|f| PathBuf::from(&f.original)).collect();
    let mut again = Vec::new();
    let mut errors = Vec::new();
    for (f, r) in present.iter().zip(crate::trash::trash(&paths, s.trash_dir.as_deref())) {
        match r {
            Ok(to) => again.push(TrashedFile { original: f.original.clone(), trash: to.display().to_string() }),
            Err(e) => errors.push(e),
        }
    }
    if let Some(e) = errors.first() {
        put_back_all(&again);
        return Err(EngineError::Other(format!("can't move the files to the Trash again: {e}")));
    }
    Ok(again)
}

/// Remember (or forget) the last delete for Put Back Last Delete, saved with the library.
pub(crate) fn remember_last_delete(s: &mut Session, last: Option<LastDelete>) {
    s.last_delete = last;
    s.save_last_delete();
}

/// After an undo that brought a delete's photos back: Put Back has nothing left to do for it.
pub(crate) fn forget_last_delete_if_back(s: &mut Session) {
    let back = s.last_delete.as_ref().is_some_and(|l| l.photos.iter().all(|p| s.catalog.photo(p.id).is_some()));
    if back {
        remember_last_delete(s, None);
    }
}

/// Put Back Last Delete: the last delete's files come out of the Trash to their paths (those that
/// can) and its photos return to the library and their albums, as one undo step.
fn put_back_last(s: &mut Session) -> Result<Value> {
    let Some(last) = s.last_delete.clone().filter(|l| !l.photos.is_empty()) else {
        return Err(bad("photo.putBackLastDelete", "there is no delete to put back"));
    };
    let returning: Vec<lightcraft_catalog::Photo> = last.photos.iter().filter(|p| s.catalog.photo(p.id).is_none()).cloned().collect();
    if returning.is_empty() {
        remember_last_delete(s, None);
        return Err(bad("photo.putBackLastDelete", "those photos are already back in the library"));
    }
    let ids: HashSet<PhotoId> = returning.iter().map(|p| p.id).collect();
    let mut back = Vec::new();
    let mut not_back = Vec::new();
    for f in &last.files {
        match crate::trash::put_back(f) {
            Ok(()) => back.push(f.clone()),
            Err(e) => not_back.push(e),
        }
    }
    let mut ops: Vec<Op> = returning.iter().map(|p| Op::AddPhoto { photo: Box::new(p.clone()) }).collect();
    for (album, photos) in &last.albums {
        let Some(a) = s.catalog.album(*album).filter(|a| !a.is_smart()) else { continue };
        let mut list = a.photos.clone();
        list.extend(photos.iter().copied().filter(|p| ids.contains(p) && !a.photos.contains(p)));
        ops.push(Op::SetAlbumPhotos { id: *album, photos: list });
    }
    if let Err(e) = s.commit("Put Back Last Delete", Op::Batch { ops }) {
        retrash(&back);
        return Err(e);
    }
    remember_last_delete(s, None);
    let mut sel: Vec<PhotoId> = ids.iter().copied().collect();
    sel.sort();
    if let Some(first) = sel.first() {
        s.selection = Selection { ids: sel.clone(), active: Some(*first) };
    }
    let restored: Vec<&str> = back.iter().map(|f| f.original.as_str()).collect();
    Ok(json!({"photos": returning.len(), "restored": restored, "notBack": not_back}))
}

/// Settings ▸ Delete key ▸ album.: in an album, the photos leave that album only (files and
/// library untouched); anywhere else they leave the library (files kept), as Remove.
fn remove_from_album(s: &mut Session, p: &Value) -> Result<Value> {
    let t = targets_or_none(s, p, "photo.removeFromAlbum")?;
    if let LibrarySource::Album(id) = s.source
        && let Some(a) = s.catalog.album(id).filter(|a| !a.is_smart())
    {
        let name = a.name.clone();
        let photos: Vec<PhotoId> = a.photos.iter().copied().filter(|x| !t.contains(x)).collect();
        let n = a.photos.len().saturating_sub(photos.len());
        s.commit("Remove from Album", Op::SetAlbumPhotos { id, photos })?;
        let vis = s.visible_cloned();
        s.selection = vis.iter().find(|id| !t.contains(id)).map(|f| Selection::single(*f)).unwrap_or_default();
        return Ok(json!({"album": id.0, "name": name, "removed": n}));
    }
    Ok(json!({"removed": remove(s, "Remove from Library", &t)?, "fromLibrary": true}))
}

/// The rejected photos of the current view (source + filter), Lightroom's Delete Rejected Photos.
fn rejected(s: &mut Session) -> Vec<PhotoId> {
    let vis = s.visible_cloned();
    vis.into_iter().filter(|id| s.catalog.photo(*id).is_some_and(|p| p.flag == Flag::Reject)).collect()
}

fn targets_or_none(s: &Session, p: &Value, c: &str) -> Result<Vec<PhotoId>> {
    let t = s.targets(p);
    if t.is_empty() {
        return Err(bad(c, "no photos selected"));
    }
    Ok(t)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "photo.removeFromLibrary",
            "Remove from Library",
            ["Photo"],
            None,
            "{ids?} — takes the photos (and their virtual copies) out of the library; the files stay on disk (Lightroom's Remove) → {removed}",
            has_selection,
            |s, p| {
                let t = targets_or_none(s, p, "photo.removeFromLibrary")?;
                Ok(json!({"removed": remove(s, "Remove from Library", &t)?}))
            }
        ),
        cmd!(
            "photo.deleteFromDisk",
            "Remove and Trash Photo",
            ["Photo"],
            Some("Alt+Cmd+Shift+Delete"),
            "{ids?} — moves each original and its XMP sidecar to the system Trash, then removes the photo (and its virtual copies) from the library; a virtual copy is only removed; a photo whose file can't be trashed stays → {trashed: [{path, trash}], removed, failed: [{id, path, error}]}",
            has_selection,
            |s, p| {
                let t = targets_or_none(s, p, "photo.deleteFromDisk")?;
                delete_from_disk(s, "Delete from Disk", &t)
            }
        ),
        cmd!(
            "photo.putBackLastDelete",
            "Put Back Last Delete",
            ["Photo"],
            None,
            "{} — the last Delete from Disk's files come back out of the Trash to their paths (not one emptied from the Trash or whose path is taken now: nothing is overwritten) and its photos return to the library and their albums; works after the undo history moved on and after a reopen → {photos, restored: [path], notBack: [why]}",
            always,
            |s, _| put_back_last(s)
        ),
        cmd!(
            "photo.removeFromAlbum",
            "Remove from This Album",
            ["Photo"],
            None,
            "{ids?} — in an album: the photos leave that album only (files and library untouched) → {album, name, removed}; elsewhere (All Photographs, folders, shoots, smart albums): Remove from Library, files kept → {removed, fromLibrary: true}",
            has_selection,
            remove_from_album
        ),
        cmd!(
            query "library.rejected",
            "Rejected Photos",
            [],
            None,
            "{} — the rejected photos in the current view (what Delete Rejected Photos acts on) → {ids, count, virtualCopies}",
            always,
            |s, _| {
                let ids = rejected(s);
                let vcs = ids.iter().filter(|id| s.catalog.photo(**id).is_some_and(|p| p.copy_of.is_some())).count();
                Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "count": ids.len(), "virtualCopies": vcs}))
            }
        ),
        cmd!(
            "library.deleteRejected",
            "Delete Rejected Photos",
            ["Photo"],
            None,
            "{fromDisk?: bool (default false = Remove)} — the rejected photos of the current view: removed from the library, or with fromDisk their files go to the Trash too → {removed, trashed?, failed?}",
            always,
            |s, p| {
                let ids = rejected(s);
                if ids.is_empty() {
                    return Ok(json!({"removed": 0}));
                }
                if bool_or(p, "fromDisk", false) {
                    delete_from_disk(s, "Delete Rejected Photos", &ids)
                } else {
                    Ok(json!({"removed": remove(s, "Remove Rejected Photos", &ids)?}))
                }
            }
        ),
    ]
}
