//! Shoots: the library's photo work in one flat list, whatever disk it is on.
//!
//! A *shoot* is the folder a body of work lives in ("Erika and Connor wedding"), found in the
//! library's folder tree (see [`crate::folders`]) by its names alone, never by reading the disk:
//!
//! * Camera and plumbing folders are not shoots: a folder whose name is only generic words —
//!   `raw`, `jpeg`, `photos`, `DCIM`, `100MSDCF`, camera makes and models (`M262`, `GR III`,
//!   `X-T2 raw`, `Canon R6`, `ILCE-7SM3`), `Day 1` — belongs to the shoot above it
//!   ([`is_generic_name`]). So `/Volumes/X/Erika and Connor wedding/raw/M262` and `…/raw/GR3`
//!   are one shoot, "Erika and Connor wedding".
//! * A folder that only gathers other work is a *container*, not a shoot: well-known places
//!   (`Pictures`, `Downloads`, the home folder, `Lightroom`…) and a folder with no photos of its
//!   own whose subfolders (two or more) all have meaningful names (`- High Priority Edit` holding
//!   `Bismarck Street Photos` and `Iceland…`). Its subfolders are looked at in turn; photos lying
//!   loose in a container are a shoot of their own that shows only those photos (`deep: false`).
//! * Anything else that holds photos is a shoot, with everything inside it.
//!
//! The rules only read names: a shoot that is split or merged the wrong way is still exactly the
//! photos of its folder, and the Folders section lists every folder as it is.

use std::collections::HashMap;

use serde::Serialize;

use crate::folders::{FolderNode, MAX_DEPTH, place};
use crate::query::folder_key;
use crate::{Catalog, Source};

/// One shoot: a folder of the library's photos (see the module docs).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shoot {
    /// What the row says: the folder's name, its parent's name in front when the name says
    /// nothing on its own (`Pictures › raw`, `2022 › 09-23`).
    pub name: String,
    /// The folder (as the folder tree spells it; hand it to `Filter::library_folder`).
    pub path: String,
    /// Its photos include the folders inside it (`false`: the loose photos of a container).
    pub deep: bool,
    /// Where its disk is mounted (`/`, `/Volumes/X`), as the folder tree's volume row says.
    pub disk: String,
    /// The disk's name as the folder tree has it (`/` for the startup disk).
    pub disk_name: String,
    /// Its photos.
    pub count: usize,
    /// The newest import or edit time among its photos (ISO 8601), empty when unknown.
    pub latest: String,
}

/// Words that only say how or with what a photo was made, not what it is of.
const GENERIC_WORDS: &[&str] = &[
    // formats
    "raw",
    "raws",
    "jpeg",
    "jpegs",
    "jpg",
    "jpgs",
    "dng",
    "dngs",
    "heic",
    "heif",
    "hif",
    "tif",
    "tiff",
    "png",
    "cr2",
    "cr3",
    "crw",
    "nef",
    "nrw",
    "arw",
    "srf",
    "sr2",
    "raf",
    "orf",
    "rw2",
    "pef",
    "x3f",
    "3fr",
    "iiq",
    "mov",
    "mp4",
    // plumbing
    "photos",
    "photo",
    "pics",
    "pic",
    "images",
    "image",
    "files",
    "file",
    "originals",
    "original",
    "camera",
    "cameras",
    "cam",
    "card",
    "cards",
    "sd",
    "cf",
    "cfexpress",
    "dcim",
    "private",
    "misc",
    "video",
    "videos",
    "clips",
    "footage",
    "edits",
    "edit",
    "edited",
    "export",
    "exports",
    "exported",
    "selects",
    "select",
    "final",
    "finals",
    "proofs",
    "backup",
    "import",
    "imports",
    "new",
    "old",
    "day",
    "part",
    "pt",
    "roll",
    "set",
    "batch",
    "take",
    "card1",
    "card2",
    "a",
    "b",
    "c",
    "and",
    // model suffixes
    "pro",
    "max",
    "mini",
    "plus",
    "ultra",
    "lite",
    "i",
    "ii",
    "iii",
    "iv",
    "mk",
    "mkii",
    "mkiii",
    "mkiv",
    "mark",
    "grii",
    "griii",
    "griiix",
    "griv",
];

/// Camera and phone makes (a folder named after the camera that filled it).
const BRANDS: &[&str] = &[
    "canon",
    "nikon",
    "sony",
    "fuji",
    "fujifilm",
    "leica",
    "ricoh",
    "gr",
    "olympus",
    "om",
    "panasonic",
    "lumix",
    "pentax",
    "hasselblad",
    "sigma",
    "gopro",
    "dji",
    "mavic",
    "osmo",
    "drone",
    "iphone",
    "ipad",
    "phone",
    "pixel",
    "samsung",
    "insta360",
    "eos",
    "alpha",
    "minolta",
    "contax",
    "kodak",
    "polaroid",
    "zeiss",
    "mamiya",
    "phase",
    "blackmagic",
    "bmpcc",
    "red",
    "arri",
];

/// Places that gather work rather than being a piece of it.
const CONTAINERS: &[&str] = &[
    "users",
    "home",
    "volumes",
    "pictures",
    "my pictures",
    "desktop",
    "downloads",
    "documents",
    "my documents",
    "movies",
    "music",
    "dropbox",
    "google drive",
    "my drive",
    "onedrive",
    "icloud drive",
    "creative cloud files",
    "lightroom",
    "lightroom catalog",
    "lightroom library",
    "photos library",
    "mobile downloads",
    "shoots",
    "photo shoots",
    "sessions",
    "clients",
    "projects",
    "jobs",
    "archive",
    "photography",
];

/// The words of a folder name: lower case, split at anything but letters, digits and inner
/// dashes (`X-T2` stays one word).
fn words(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .map(|w| w.trim_matches('-'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether one word only names a format, a camera or a card folder.
fn generic_word(w: &str) -> bool {
    if GENERIC_WORDS.contains(&w) || BRANDS.contains(&w) {
        return true;
    }
    let digits = w.chars().filter(char::is_ascii_digit).count();
    let letters = w.chars().filter(|c| c.is_alphabetic()).count();
    if letters == 0 {
        // a card or roll number (`1`, `02`, `100`); a year or a date (`2026`, `09-23`) says when
        return digits > 0 && digits <= 3 && digits == w.chars().count();
    }
    if digits == 0 {
        return false;
    }
    // `3rd`, `19th`: part of a date
    let ordinal = ["st", "nd", "rd", "th"].iter().any(|s| w.strip_suffix(s).is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())));
    // a model or card folder: letters and digits, short (`m262`, `x-t2`, `r6`, `a7iii`, `ilce-7sm3`, `100msdcf`)
    !ordinal && w.chars().count() <= 10
}

/// Whether a folder's name says nothing about what is in it: only formats, camera makes and
/// models, card folders and the like (`raw`, `M262`, `X-T2 raw`, `Canon R6`, `100MSDCF`, `Day 1`).
/// An empty or punctuation-only name is generic too.
pub fn is_generic_name(name: &str) -> bool {
    words(name).iter().all(|w| generic_word(w))
}

/// Whether a folder name is a well-known place that gathers work (`Pictures`, `Downloads`…).
fn is_container_name(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    CONTAINERS.contains(&n.as_str())
}

/// Whether `path` is a user's home folder on the startup disk (`/Users/me`, `/home/me`).
fn is_home(path: &str) -> bool {
    place(path).is_some_and(|p| p.mount == "/" && p.names.len() == 2 && p.names.first().is_some_and(|n| n == "Users" || n == "home"))
}

/// A folder that gathers other work (see the module docs).
fn is_container(n: &FolderNode) -> bool {
    !n.selectable
        || is_container_name(&n.name)
        || is_home(&n.path)
        || (n.own == 0 && n.children.len() >= 2 && n.children.iter().all(|c| !is_generic_name(&c.name)))
}

/// A shoot's name: its folder's, with the parent's in front when the folder's own says nothing
/// (only generic words, or only digits like a date folder).
fn shoot_name(n: &FolderNode, parent: Option<&str>) -> String {
    let vague = is_generic_name(&n.name) || !n.name.chars().any(char::is_alphabetic);
    let own = if n.name.trim().is_empty() { n.path.clone() } else { n.name.clone() };
    match parent {
        Some(p) if vague && !p.trim().is_empty() => format!("{p} › {own}"),
        _ => own,
    }
}

/// The name a Folders row shows for a folder: its own, and when that says nothing on its own
/// (`M262`, `raw`), the nearest folder above it that does, in front
/// (`Erika and Connor wedding › M262`). Folders above the disk's top are never looked at.
pub fn context_label(path: &str, name: &str) -> String {
    if !is_generic_name(name) {
        return name.to_string();
    }
    let Some(placed) = place(path) else { return name.to_string() };
    // the folders above this one, nearest first
    let above = placed.names.iter().rev().skip(1);
    for n in above {
        if is_container_name(n) {
            break;
        }
        if !is_generic_name(n) {
            return format!("{n} › {name}");
        }
    }
    name.to_string()
}

/// The shoots in a folder tree (as [`Catalog::folder_tree`] builds it), in tree order, without
/// their `latest` time (see [`Catalog::shoots`]).
pub fn find_shoots(tree: &[FolderNode]) -> Vec<Shoot> {
    let mut out = Vec::new();
    for v in tree {
        for c in &v.children {
            walk(c, None, v, &mut out, 0);
        }
    }
    out
}

fn walk(n: &FolderNode, parent: Option<&str>, disk: &FolderNode, out: &mut Vec<Shoot>, depth: usize) {
    let shoot = |deep: bool, count: usize| Shoot {
        name: shoot_name(n, parent),
        path: n.path.clone(),
        deep,
        disk: disk.path.clone(),
        disk_name: disk.name.clone(),
        count,
        latest: String::new(),
    };
    if depth < MAX_DEPTH && is_container(n) {
        if n.own > 0 && n.selectable {
            out.push(shoot(false, n.own));
        }
        for c in &n.children {
            walk(c, Some(&n.name), disk, out, depth + 1);
        }
    } else if n.count > 0 && n.selectable {
        out.push(shoot(true, n.count));
    }
}

impl Catalog {
    /// The library's shoots (see the module docs) with the newest import or edit time of each
    /// one's photos; `extra` as for [`Catalog::folder_tree_with`].
    pub fn shoots(&self, extra: &[String]) -> Vec<Shoot> {
        let tree = self.folder_tree_with(extra);
        let mut shoots = find_shoots(&tree);
        // newest time per folder (its own photos), by identity key
        let mut own: HashMap<String, String> = HashMap::new();
        let mut by_dir: HashMap<&str, &str> = HashMap::new();
        for p in self.photos().filter(|p| p.in_library()) {
            if let Source::File { path } = &p.source
                && let Some(i) = path.rfind(['/', '\\'])
                && let Some(dir) = path.get(..i.max(1))
            {
                let t = p.edited.as_deref().filter(|e| *e > p.imported.as_str()).unwrap_or(p.imported.as_str());
                let e = by_dir.entry(dir).or_insert(t);
                if t > *e {
                    *e = t;
                }
            }
        }
        for (dir, t) in by_dir {
            let e = own.entry(folder_key(dir)).or_default();
            if t > e.as_str() {
                *e = t.to_string();
            }
        }
        fn newest(n: &FolderNode, own: &HashMap<String, String>, deep: bool, depth: usize) -> String {
            let mut best = own.get(&folder_key(&n.path)).cloned().unwrap_or_default();
            if deep && depth < MAX_DEPTH {
                for c in &n.children {
                    let t = newest(c, own, true, depth + 1);
                    if t > best {
                        best = t;
                    }
                }
            }
            best
        }
        fn find<'a>(nodes: &'a [FolderNode], key: &str, depth: usize) -> Option<&'a FolderNode> {
            for n in nodes {
                let k = folder_key(&n.path);
                if k == key {
                    return Some(n);
                }
                if depth < MAX_DEPTH
                    && crate::query::folder_within(key, &n.path)
                    && let Some(f) = find(&n.children, key, depth + 1)
                {
                    return Some(f);
                }
            }
            None
        }
        for s in &mut shoots {
            if let Some(n) = find(&tree, &folder_key(&s.path), 0) {
                s.latest = newest(n, &own, s.deep, 0);
            }
        }
        shoots
    }
}
