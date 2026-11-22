//! Lightroom's two kinds of delete (L03): **Remove** (out of the library, the files stay) and
//! **Delete from Disk** (the original and its XMP sidecar go to the system Trash, then the photo
//! leaves the library), plus Delete Rejected Photos. A virtual copy has no file of its own: it is
//! only ever removed. Removing a master removes its virtual copies with it. A photo whose file
//! can't be moved to the Trash stays in the library (reported in `failed`).
//!
//! Photo ▸ Delete Photo (`photo.delete`) still moves photos to Recently Deleted (LightCraft's own
//! soft delete); the app's Delete key asks first, like Lightroom (Delete from Disk / Remove).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use lightcraft_catalog::{Flag, PhotoId, Source};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, bool_or, cmd, has_selection};
use crate::{Result, Selection, Session};

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
/// that have none). Returns `{trashed, removed, failed}`.
fn delete_from_disk(s: &mut Session, label: &str, ids: &[PhotoId]) -> Result<Value> {
    let all = with_copies(s, ids);
    let leaving: HashSet<PhotoId> = all.iter().copied().collect();
    let mut ok_ids = Vec::new();
    let mut trashed = Vec::new();
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
        for (f, r) in to_trash.iter().zip(results) {
            match r {
                Ok(to) => trashed.push(json!({"path": f.display().to_string(), "trash": to.display().to_string()})),
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
        }
    }
    // virtual copies: with their master, or on their own when the master stays
    ok_ids.extend(
        all.iter()
            .copied()
            .filter(|id| s.catalog.photo(*id).and_then(|p| p.copy_of).is_some_and(|m| !leaving.contains(&m) || gone_masters.contains(&m))),
    );
    let removed = remove(s, label, &ok_ids)?;
    Ok(json!({"trashed": trashed, "removed": removed, "failed": failed}))
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
