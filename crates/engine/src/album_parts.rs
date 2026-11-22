//! LRC-LIB-COLL-PARTS: a collection split by the folders its photos come from or the cameras that
//! took them — the rows under a Collections row's disclosure triangle (beyond Lightroom Classic,
//! whose collections only list photos). A wedding imported as `raw/M262`, `raw/R6`, `jpg` shows
//! those three folders under its collection; a click (`library.source {kind: album, id, folders}`)
//! shows the collection narrowed to them, so one camera can be worked on at a time.
//!
//! Folder rows are named relative to the folder all of the collection's photos share (the shoot),
//! with the camera after the name when the folder holds one camera whose name the folder's does not
//! already say (`M262 · LEICA M (Typ 262)`, but just `R6` for a Canon EOS R6).

use std::collections::BTreeMap;

use lightcraft_catalog::{AlbumId, Catalog, Photo};
use serde::{Deserialize, Serialize};

/// What a collection's photos are split by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PartBy {
    /// The folder each photo's file is in.
    #[default]
    Folder,
    /// The camera model that took it.
    Camera,
}

impl PartBy {
    pub fn parse(s: &str) -> Option<PartBy> {
        match s {
            "folder" | "folders" => Some(PartBy::Folder),
            "camera" | "cameras" => Some(PartBy::Camera),
            _ => None,
        }
    }
}

/// A collection narrowed to some of its folders or cameras ([`crate::Session::album_part`]); it
/// applies only while [`crate::LibrarySource::Album`] shows that same collection.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumPart {
    pub album: AlbumId,
    pub by: PartBy,
    /// [`part_key`]s of the rows picked (folder keys or camera names).
    pub keys: Vec<String>,
    /// What the view is called after the collection's name ([`title`]: `raw/M262`), worked out
    /// when the rows were picked.
    #[serde(default)]
    pub title: String,
}

impl AlbumPart {
    /// Whether the photo belongs to one of the picked rows.
    pub fn matches(&self, p: &Photo) -> bool {
        let k = part_key(p, self.by);
        self.keys.contains(&k)
    }
}

/// One row under a collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartRow {
    /// What picks it ([`AlbumPart::keys`]): the folder's key, or the camera's name (`""`: none
    /// recorded).
    pub key: String,
    /// What the row says: `raw/M262 · LEICA M (Typ 262)`, `jpg`, `Canon EOS R6`.
    pub label: String,
    /// The folder alone, relative to the shoot (folder rows; the camera's name on camera rows).
    pub name: String,
    pub count: usize,
    /// The cameras of its photos, most photos first.
    pub cameras: Vec<String>,
}

/// The row a photo falls under: its folder ([`lightcraft_catalog::local::folder_of`], `""` for a
/// photo without a file) or its camera's name.
pub fn part_key(p: &Photo, by: PartBy) -> String {
    match by {
        PartBy::Folder => lightcraft_catalog::local::folder_of(p).unwrap_or_default(),
        PartBy::Camera => p.meta.camera.trim().to_string(),
    }
}

/// The deepest that a relative folder name goes before it is shortened to its last two folders.
const MAX_LABEL_DEPTH: usize = 3;

/// The rows of collection `album` (none for a set or an album that is gone): its photos that are
/// not deleted, by folder (by relative name) or by camera (by name).
pub fn parts(cat: &Catalog, album: AlbumId, by: PartBy) -> Vec<PartRow> {
    if cat.album(album).is_none_or(|a| a.folder) {
        return Vec::new();
    }
    // key → (photos, camera → photos)
    let mut groups: BTreeMap<String, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
    for id in cat.album_photos(album) {
        let Some(p) = cat.photo(id).filter(|p| !p.deleted) else { continue };
        let g = groups.entry(part_key(p, by)).or_default();
        g.0 = g.0.saturating_add(1);
        let c = g.1.entry(p.meta.camera.trim().to_string()).or_default();
        *c = c.saturating_add(1);
    }
    let names = match by {
        PartBy::Folder => relative_names(&groups.keys().cloned().collect::<Vec<_>>()),
        PartBy::Camera => groups.keys().map(|k| if k.is_empty() { "Unknown camera".to_string() } else { k.clone() }).collect(),
    };
    let mut rows: Vec<PartRow> = groups
        .into_iter()
        .zip(names)
        .map(|((key, (count, cams)), name)| {
            let mut cameras: Vec<(String, usize)> = cams.into_iter().filter(|(c, _)| !c.is_empty()).collect();
            cameras.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let cameras: Vec<String> = cameras.into_iter().map(|(c, _)| c).collect();
            let label = match (by, cameras.as_slice()) {
                (PartBy::Folder, [one]) if !says_camera(&name, one) => format!("{name} · {one}"),
                _ => name.clone(),
            };
            PartRow { key, label, name, count, cameras }
        })
        .collect();
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.key.cmp(&b.key)));
    rows
}

/// What a narrowed collection is called after its name: one row's name, two joined by `+`, or how
/// many (`3 folders`, `2 cameras`). `None` when nothing is picked.
pub fn title(rows: &[PartRow], part: &AlbumPart) -> Option<String> {
    let picked: Vec<&str> = rows.iter().filter(|r| part.keys.contains(&r.key)).map(|r| r.name.as_str()).collect();
    match picked.as_slice() {
        [] => None,
        [one] => Some((*one).to_string()),
        [a, b] => Some(format!("{a} + {b}")),
        many => Some(format!("{} {}", many.len(), if part.by == PartBy::Folder { "folders" } else { "cameras" })),
    }
}

/// Folder keys named from the folder they all share: `raw/M262`, `raw/R6`, `jpg` under one shoot
/// (a lone folder by its own name; a photo without a file: `no folder`).
fn relative_names(keys: &[String]) -> Vec<String> {
    let split: Vec<Vec<&str>> = keys.iter().map(|k| k.split('/').filter(|s| !s.is_empty()).collect()).collect();
    let real: Vec<&Vec<&str>> = split.iter().filter(|s| !s.is_empty()).collect();
    // the folders all of them lie in (component by component)…
    let mut common = real.first().map_or(0, |f| f.len());
    for s in &real {
        common = common.min(s.len()).min(s.iter().zip(real.first().map_or(&[][..], |f| f.as_slice())).take_while(|(a, b)| a == b).count());
    }
    // …but never one of the folders itself: each keeps a name of its own
    if real.iter().any(|s| s.len() <= common) {
        common = real.iter().map(|s| s.len()).min().unwrap_or(1).saturating_sub(1);
    }
    split
        .iter()
        .map(|s| {
            if s.is_empty() {
                return "no folder".to_string();
            }
            let rest = s.get(common..).unwrap_or(&[]);
            let rest = if rest.len() > MAX_LABEL_DEPTH { rest.get(rest.len() - 2..).unwrap_or(rest) } else { rest };
            if rest.is_empty() { s.last().map_or_else(String::new, |n| n.to_string()) } else { rest.join("/") }
        })
        .collect()
}

/// Whether a folder's name already says which camera it holds: its last folder's name (letters
/// and digits only, any case) is in the camera's (`R6` in `Canon EOS R6`).
fn says_camera(folder: &str, camera: &str) -> bool {
    let norm = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect::<String>();
    let last = norm(folder.rsplit('/').next().unwrap_or(folder));
    !last.is_empty() && norm(camera).contains(&last)
}

// ---------------------------------------------------------------- folders beside, not imported

/// At most this many folders are looked at beside a collection's folders, and this many files in
/// each: a stray huge folder never stalls the scan.
const MAX_BESIDE: usize = 48;
const MAX_FILES: usize = 50_000;

/// One folder inside a collection folder's parent, with the photo files in it ([`scan_beside`]).
pub type Scanned = (String, Vec<String>);

/// The folders inside `parent` (a collection folder's parent, `<shoot>/raw`) with the files an
/// import would take from each (raws, images, videos; sidecars never), read from disk. Slow on a
/// sleeping drive: the UI runs it on a worker thread and keeps the answer.
pub fn scan_beside(parent: &str) -> std::sync::Arc<Vec<Scanned>> {
    let Ok(rd) = std::fs::read_dir(parent) else { return Default::default() };
    let mut dirs: Vec<std::path::PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    dirs.sort();
    dirs.truncate(MAX_BESIDE);
    std::sync::Arc::new(
        dirs.into_iter()
            .map(|d| {
                let d = d.to_string_lossy().to_string();
                let mut files = crate::import::expand(std::slice::from_ref(&d), None);
                files.truncate(MAX_FILES);
                (d, files)
            })
            .collect(),
    )
}

/// A folder beside a collection's folders whose photos are not in the library (`raw/GR3`, 401).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BesideRow {
    /// The folder on disk (what `add.` imports in place).
    pub path: String,
    /// Named like the collection's rows (`raw/GR3`).
    pub name: String,
    /// Photos in it not in the library: a raw + JPEG pair (same name) once, sidecars never, and
    /// none whose raw or JPEG twin is in the library already.
    pub count: usize,
}

/// The parents of collection `album`'s folders: where [`scan_beside`] looks.
pub fn beside_parents(cat: &Catalog, album: AlbumId) -> Vec<String> {
    parents_of(&parts(cat, album, PartBy::Folder))
}

/// The parents of the folders of a collection's folder rows ([`parts`] by [`PartBy::Folder`]).
pub fn parents_of(rows: &[PartRow]) -> Vec<String> {
    let mut v: Vec<String> = rows
        .iter()
        .filter(|r| !r.key.is_empty())
        .filter_map(|r| r.key.rsplit_once('/').map(|(p, _)| if p.is_empty() { "/".to_string() } else { p.to_string() }))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The folders in `scans` (beside collection `album`'s folders) that hold photos not in the
/// library, named from the shoot like the collection's rows.
pub fn beside(cat: &Catalog, album: AlbumId, scans: &[Scanned]) -> Vec<BesideRow> {
    use std::collections::HashSet;
    let own: Vec<String> = parts(cat, album, PartBy::Folder).into_iter().map(|r| r.key).filter(|k| !k.is_empty()).collect();
    // a folder of the collection, or one holding one (`raw` beside `jpg`), is no candidate
    let candidates: Vec<(&String, String, &Vec<String>)> = scans
        .iter()
        .map(|(path, files)| (path, lightcraft_catalog::query::folder_key(path), files))
        .filter(|(_, key, _)| !key.is_empty() && !own.iter().any(|o| lightcraft_catalog::query::folder_within(o, key)))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let stem_of = |path: &str| -> Option<(String, String)> {
        let key = lightcraft_catalog::query::folder_key(path);
        let (dir, file) = key.rsplit_once('/')?;
        let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
        Some((dir.to_string(), stem.to_lowercase()))
    };
    // the library's photos in those folders, by folder and name without extension
    let dirs: HashSet<&str> = candidates.iter().map(|(_, k, _)| k.as_str()).collect();
    let mut known: HashSet<(String, String)> = HashSet::new();
    for p in cat.photos().filter(|p| !p.local) {
        let lightcraft_catalog::Source::File { path } = &p.source else { continue };
        if let Some((dir, stem)) = stem_of(path)
            && dirs.iter().any(|d| lightcraft_catalog::query::folder_within(&dir, d))
        {
            known.insert((dir, stem));
        }
    }
    let mut all: Vec<String> = own.clone();
    all.extend(candidates.iter().map(|(_, k, _)| k.clone()));
    let names = relative_names(&all);
    candidates
        .iter()
        .enumerate()
        .filter_map(|(i, (path, _, files))| {
            let missing: HashSet<(String, String)> = files.iter().filter_map(|f| stem_of(f)).filter(|s| !known.contains(s)).collect();
            let name = names.get(own.len() + i).cloned().unwrap_or_default();
            (!missing.is_empty()).then(|| BesideRow { path: (*path).clone(), name, count: missing.len() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_catalog::{Album, Op, Source};

    fn library() -> (Catalog, AlbumId) {
        let mut cat = Catalog::default();
        let files = [
            ("/Volumes/A/Erika and Connor/raw/M262/a.dng", "LEICA M (Typ 262)"),
            ("/Volumes/A/Erika and Connor/raw/M262/b.dng", "LEICA M (Typ 262)"),
            ("/Volumes/A/Erika and Connor/raw/R6/c.cr3", "Canon EOS R6"),
            ("/Volumes/A/Erika and Connor/jpg/d.jpg", "Canon EOS R6"),
            ("/Volumes/A/Erika and Connor/jpg/e.jpg", "LEICA M (Typ 262)"),
        ];
        let mut ids = Vec::new();
        for (path, cam) in files {
            let id = cat.alloc_photo_id();
            let mut p = Photo::new(id, Source::File { path: path.into() }, "x", "JPEG", 10, 10, "2026-01-01T00:00:00");
            p.meta.camera = cam.into();
            cat.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
            ids.push(id);
        }
        let a = cat.alloc_album_id();
        let mut album = Album::new(a, "Erika and Connor");
        album.photos = ids;
        cat.apply(Op::AddAlbum { album }).unwrap();
        (cat, a)
    }

    #[test]
    fn folders_are_named_from_the_shoot_with_their_camera_when_the_name_does_not_say_it() {
        let (cat, a) = library();
        let rows = parts(&cat, a, PartBy::Folder);
        let got: Vec<(&str, usize)> = rows.iter().map(|r| (r.label.as_str(), r.count)).collect();
        assert_eq!(got, vec![("jpg", 2), ("raw/M262 · LEICA M (Typ 262)", 2), ("raw/R6", 1)]);
        assert_eq!(rows[0].cameras.len(), 2, "two cameras: none named on the row");
    }

    #[test]
    fn cameras_are_one_row_each_across_folders() {
        let (cat, a) = library();
        let rows = parts(&cat, a, PartBy::Camera);
        let got: Vec<(&str, usize)> = rows.iter().map(|r| (r.label.as_str(), r.count)).collect();
        assert_eq!(got, vec![("Canon EOS R6", 2), ("LEICA M (Typ 262)", 3)]);
    }

    #[test]
    fn a_part_matches_only_its_rows() {
        let (cat, a) = library();
        let rows = parts(&cat, a, PartBy::Folder);
        let part = AlbumPart { album: a, by: PartBy::Folder, keys: vec![rows[1].key.clone()], title: String::new() };
        assert_eq!(cat.photos().filter(|p| part.matches(p)).count(), 2);
        assert_eq!(title(&rows, &part).as_deref(), Some("raw/M262"));
        let both = AlbumPart { keys: vec![rows[1].key.clone(), rows[2].key.clone()], ..part };
        assert_eq!(title(&rows, &both).as_deref(), Some("raw/M262 + raw/R6"));
    }

    #[test]
    fn relative_names_keep_a_name_for_every_folder() {
        let k = |v: &[&str]| relative_names(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(k(&["/V/shoot/raw"]), vec!["raw"]);
        assert_eq!(k(&["/V/shoot", "/V/shoot/raw"]), vec!["shoot", "shoot/raw"]);
        assert_eq!(k(&["", "/V/a/b"]), vec!["no folder", "b"]);
        assert_eq!(k(&["/V/a/b/c/d/e", "/V/x"]), vec!["d/e", "x"]);
        assert_eq!(k(&[]), Vec::<String>::new());
    }

    #[test]
    fn a_set_or_a_missing_album_has_no_rows() {
        let (cat, _) = library();
        assert!(parts(&cat, AlbumId(9999), PartBy::Folder).is_empty());
    }

    #[test]
    fn camera_folders_beside_the_collection_that_hold_photos_not_in_the_library() {
        let dir = std::env::temp_dir().join(format!("lc-beside-{}", std::process::id()));
        let shoot = dir.join("Erika and Connor");
        for f in [
            "raw/M262/a.dng",
            "raw/GR3/x.dng",
            "raw/GR3/x.jpg",
            "raw/GR3/x.dng.xmp",
            "raw/GR3/x.xmp",
            "raw/GR3/y.dng",
            "raw/GR3/.hidden.dng",
            "raw/X-T2/z.raf",
            "raw/X-T2/w.jpg",
            "raw/empty/notes.txt",
        ] {
            let p = shoot.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"").unwrap();
        }
        let path = |f: &str| shoot.join(f).to_string_lossy().to_string();
        let mut cat = Catalog::default();
        let mut ids = Vec::new();
        // the collection holds raw/M262; raw/X-T2's z.raf is in the library but in no collection
        for f in ["raw/M262/a.dng", "raw/X-T2/z.raf"] {
            let id = cat.alloc_photo_id();
            let p = Photo::new(id, Source::File { path: path(f) }, "x", "DNG", 10, 10, "2026-01-01T00:00:00");
            cat.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
            ids.push(id);
        }
        let a = cat.alloc_album_id();
        let mut album = Album::new(a, "Erika and Connor");
        album.photos = vec![ids[0]];
        cat.apply(Op::AddAlbum { album }).unwrap();
        let parents = beside_parents(&cat, a);
        assert_eq!(parents, vec![lightcraft_catalog::query::folder_key(&path("raw"))]);
        let scans = scan_beside(&parents[0]);
        let rows = beside(&cat, a, &scans);
        let got: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.count)).collect();
        // GR3: x (raw + JPEG once, sidecars never) and y; X-T2: w (z is in the library); empty: none
        assert_eq!(got, vec![("GR3", 2), ("X-T2", 1)], "named like the collection's own row (M262: all under raw/)");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
