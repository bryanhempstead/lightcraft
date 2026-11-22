//! LRC-LIB-COLL-PARTS: a shoot or a collection split by the folders its photos come from or the
//! cameras that took them — the rows under a shoots. or Collections row's disclosure triangle
//! (beyond Lightroom Classic). A wedding imported as `raw/M262`, `raw/GR3`, `raw/X-T2` shows those
//! three folders under its row; a click (`shoot.show {path, folders}`, `library.source {kind:
//! album, id, folders}`) shows it narrowed to them, so one camera can be worked on at a time.
//!
//! Folder rows are named relative to the shoot's folder (a collection: the folder all its photos
//! share), with the camera after the name when the folder holds one camera whose name the
//! folder's does not already say (`raw/M262 · LEICA M (Typ 262)`, but `raw/R6` for a Canon EOS R6).
//! Folders beside them on disk whose photos never came into the library are listed too
//! ([`beside`]).

use std::collections::BTreeMap;

use lightcraft_catalog::{AlbumId, Catalog, Photo, PhotoId};
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

/// What is split: a collection, or a shoot (its folder, the folders inside it with `deep`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PartOf {
    Album { id: AlbumId },
    Shoot { path: String, deep: bool },
}

impl PartOf {
    /// Its photos (deleted ones too; [`parts`] leaves them out).
    pub fn photos(&self, cat: &Catalog) -> Vec<PhotoId> {
        match self {
            PartOf::Album { id } => {
                if cat.album(*id).is_none_or(|a| a.folder) {
                    return Vec::new();
                }
                cat.album_photos(*id)
            }
            PartOf::Shoot { path, deep } => {
                if lightcraft_catalog::query::folder_key(path).is_empty() {
                    return Vec::new();
                }
                let f = lightcraft_catalog::Filter { library_folder: Some(path.clone()), library_folder_only: !deep, ..Default::default() };
                cat.query(&f, &lightcraft_catalog::Sort::default())
            }
        }
    }

    /// The folder its rows are named from: a shoot's own; a collection's, the one its photos share.
    fn root(&self) -> Option<String> {
        match self {
            PartOf::Album { .. } => None,
            PartOf::Shoot { path, .. } => Some(lightcraft_catalog::query::folder_key(path)),
        }
    }
}

/// A shoot or collection narrowed to some of its folders or cameras
/// ([`crate::Session::album_part`]); it applies only while that shoot or collection is shown.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumPart {
    pub of: PartOf,
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

/// The rows of a shoot or collection (none for a set or an album that is gone): its photos that
/// are not deleted, by folder (by relative name) or by camera (by name).
pub fn parts(cat: &Catalog, of: &PartOf, by: PartBy) -> Vec<PartRow> {
    // key → (photos, camera → photos)
    let mut groups: BTreeMap<String, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
    for id in of.photos(cat) {
        let Some(p) = cat.photo(id).filter(|p| !p.deleted) else { continue };
        let g = groups.entry(part_key(p, by)).or_default();
        g.0 = g.0.saturating_add(1);
        let c = g.1.entry(p.meta.camera.trim().to_string()).or_default();
        *c = c.saturating_add(1);
    }
    let names = match by {
        PartBy::Folder => names_from(&groups.keys().cloned().collect::<Vec<_>>(), of.root().as_deref()),
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

/// Folder keys named from `root` (a shoot's folder: `raw/M262`; the folder itself by its name),
/// else from the folder they all share ([`relative_names`]).
fn names_from(keys: &[String], root: Option<&str>) -> Vec<String> {
    let Some(root) = root.map(|r| r.trim_end_matches('/')).filter(|r| !r.is_empty()) else { return relative_names(keys) };
    keys.iter()
        .map(|k| {
            let k = k.trim_end_matches('/');
            match k.strip_prefix(root).and_then(|rest| rest.strip_prefix('/')) {
                Some(rest) if !rest.is_empty() => {
                    let parts: Vec<&str> = rest.split('/').collect();
                    let tail = if parts.len() > MAX_LABEL_DEPTH { parts.get(parts.len() - 2..).unwrap_or(&parts) } else { &parts };
                    tail.join("/")
                }
                _ if k.is_empty() => "no folder".to_string(),
                _ => k.rsplit('/').next().unwrap_or(k).to_string(),
            }
        })
        .collect()
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

/// Where [`scan_beside`] looks: the parents of the folder rows' folders ([`parts`] by
/// [`PartBy::Folder`]) that lie in one of `roots` (the shoots they belong to: never a disk's
/// top, whose folders are other shoots).
pub fn parents_of(rows: &[PartRow], roots: &[String]) -> Vec<String> {
    let mut v: Vec<String> = rows
        .iter()
        .filter(|r| !r.key.is_empty())
        .filter_map(|r| r.key.rsplit_once('/').map(|(p, _)| if p.is_empty() { "/".to_string() } else { p.to_string() }))
        .filter(|p| roots.iter().any(|r| lightcraft_catalog::query::folder_within(p, r)))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The roots a shoot's or collection's folders are looked beside in ([`parents_of`]): a shoot's
/// own folder (when it takes the folders inside it); for a collection, the shoots its folders lie in.
pub fn roots_of(of: &PartOf, rows: &[PartRow], shoots: &[lightcraft_catalog::Shoot]) -> Vec<String> {
    match of {
        PartOf::Shoot { path, deep: true } => vec![lightcraft_catalog::query::folder_key(path)],
        PartOf::Shoot { .. } => Vec::new(),
        PartOf::Album { .. } => {
            let mut v: Vec<String> = shoots
                .iter()
                .filter(|s| s.deep && rows.iter().any(|r| !r.key.is_empty() && lightcraft_catalog::query::folder_within(&r.key, &s.path)))
                .map(|s| lightcraft_catalog::query::folder_key(&s.path))
                .collect();
            v.sort();
            v.dedup();
            v
        }
    }
}

/// The folders in `scans` (beside the folders of a shoot or collection, `of`) that hold photos
/// not in the library, named like its rows.
pub fn beside(cat: &Catalog, of: &PartOf, scans: &[Scanned]) -> Vec<BesideRow> {
    use std::collections::HashSet;
    let own: Vec<String> = parts(cat, of, PartBy::Folder).into_iter().map(|r| r.key).filter(|k| !k.is_empty()).collect();
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
    let names = names_from(&all, of.root().as_deref());
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

// ---------------------------------------------------------------- commands

/// `folders` / `cameras` of a command's params: the rows to narrow to.
pub(crate) fn part_param(c: &str, p: &serde_json::Value) -> crate::Result<Option<(PartBy, Vec<String>)>> {
    let (by, v) = match (p.get("folders").filter(|v| !v.is_null()), p.get("cameras").filter(|v| !v.is_null())) {
        (Some(_), Some(_)) => return Err(crate::cmd::bad(c, "narrow by `folders` or by `cameras`, not both")),
        (Some(v), None) => (PartBy::Folder, v),
        (None, Some(v)) => (PartBy::Camera, v),
        (None, None) => return Ok(None),
    };
    let keys: Option<Vec<String>> = v.as_array().and_then(|a| a.iter().map(|k| k.as_str().map(str::to_string)).collect());
    let keys = keys.ok_or_else(|| crate::cmd::bad(c, "`folders` / `cameras` is a list of keys from album.parts / shoot.parts"))?;
    Ok(Some((by, keys)))
}

/// `of` narrowed to rows `keys` (checked against its rows: an unknown one is an error, not an
/// empty grid under a name that says something); `None` for no keys.
pub(crate) fn make_part(c: &str, cat: &Catalog, of: PartOf, by: PartBy, mut keys: Vec<String>) -> crate::Result<Option<AlbumPart>> {
    keys.sort();
    keys.dedup();
    if keys.is_empty() {
        return Ok(None);
    }
    let rows = parts(cat, &of, by);
    if let Some(k) = keys.iter().find(|k| !rows.iter().any(|r| r.key == **k)) {
        return Err(crate::cmd::bad(c, format!("`{k}`: no such row (see album.parts / shoot.parts)")));
    }
    let mut part = AlbumPart { of, by, keys, title: String::new() };
    part.title = title(&rows, &part).unwrap_or_default();
    Ok(Some(part))
}

/// What album.parts and shoot.parts answer: the rows, those picked, and with `scanDisk` the
/// folders beside them holding photos not in the library (read from disk here).
pub(crate) fn parts_json(c: &str, s: &crate::Session, of: &PartOf, p: &serde_json::Value) -> crate::Result<serde_json::Value> {
    let by = match crate::cmd::str_param(p, "by") {
        None => PartBy::Folder,
        Some(b) => PartBy::parse(b).ok_or_else(|| crate::cmd::bad(c, "`by` is folder or camera"))?,
    };
    let rows = parts(&s.catalog, of, by);
    let picked: Vec<String> = s.active_album_part().filter(|x| x.of == *of && x.by == by).map(|x| x.keys.clone()).unwrap_or_default();
    let mut out = serde_json::json!({"rows": rows, "picked": picked});
    if crate::cmd::bool_or(p, "scanDisk", false) {
        let folders = parts(&s.catalog, of, PartBy::Folder);
        let roots = roots_of(of, &folders, &s.catalog.shoots(&s.empty_folders));
        let scans: Vec<Scanned> = parents_of(&folders, &roots).iter().flat_map(|parent| scan_beside(parent).as_ref().clone()).collect();
        out["beside"] = serde_json::json!(beside(&s.catalog, of, &scans));
    }
    Ok(out)
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
        let rows = parts(&cat, &PartOf::Album { id: a }, PartBy::Folder);
        let got: Vec<(&str, usize)> = rows.iter().map(|r| (r.label.as_str(), r.count)).collect();
        assert_eq!(got, vec![("jpg", 2), ("raw/M262 · LEICA M (Typ 262)", 2), ("raw/R6", 1)]);
        assert_eq!(rows[0].cameras.len(), 2, "two cameras: none named on the row");
    }

    #[test]
    fn cameras_are_one_row_each_across_folders() {
        let (cat, a) = library();
        let rows = parts(&cat, &PartOf::Album { id: a }, PartBy::Camera);
        let got: Vec<(&str, usize)> = rows.iter().map(|r| (r.label.as_str(), r.count)).collect();
        assert_eq!(got, vec![("Canon EOS R6", 2), ("LEICA M (Typ 262)", 3)]);
    }

    #[test]
    fn a_part_matches_only_its_rows() {
        let (cat, a) = library();
        let rows = parts(&cat, &PartOf::Album { id: a }, PartBy::Folder);
        let part = AlbumPart { of: PartOf::Album { id: a }, by: PartBy::Folder, keys: vec![rows[1].key.clone()], title: String::new() };
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
        assert!(parts(&cat, &PartOf::Album { id: AlbumId(9999) }, PartBy::Folder).is_empty());
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
        let of = PartOf::Album { id: a };
        let shoots = cat.shoots(&[]);
        let roots = roots_of(&of, &parts(&cat, &of, PartBy::Folder), &shoots);
        assert_eq!(roots, vec![lightcraft_catalog::query::folder_key(&path(""))], "the shoot: {shoots:?}");
        let parents = parents_of(&parts(&cat, &of, PartBy::Folder), &roots);
        assert_eq!(parents, vec![lightcraft_catalog::query::folder_key(&path("raw"))]);
        let scans = scan_beside(&parents[0]);
        let rows = beside(&cat, &of, &scans);
        let got: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.count)).collect();
        // GR3: x (raw + JPEG once, sidecars never) and y; X-T2: w (z is in the library); empty: none
        assert_eq!(got, vec![("GR3", 2), ("X-T2", 1)], "named like the collection's own row (M262: all under raw/)");
        // the shoot: named from its folder
        let shoot = PartOf::Shoot { path: path(""), deep: true };
        let rows = parts(&cat, &shoot, PartBy::Folder);
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["raw/M262", "raw/X-T2"]);
        let got: Vec<(String, usize)> = beside(&cat, &shoot, &scans).into_iter().map(|r| (r.name, r.count)).collect();
        assert_eq!(got, vec![("raw/GR3".to_string(), 2)], "X-T2 is the shoot's own folder now");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
