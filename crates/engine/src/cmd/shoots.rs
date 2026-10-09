//! Library ▸ Shoots: the library's photo work in one flat list, whatever disk it is on (the
//! shoots themselves are the catalog's, [`lightcraft_catalog::shoots`]), with what the
//! photographer made of the list: pinned shoots first, shoots taken off it, names of their own,
//! newest first or by name. Kept per library in `prefs.json`; never touches a photo or a file.
//!
//! (Not to be confused with [`super::shoot`]: folder-scoped commands for pipelines.)

use std::collections::BTreeMap;

use lightcraft_catalog::Shoot;
use lightcraft_catalog::query::folder_key;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::{LibrarySource, Result, Selection, Session};

/// How Shoots is ordered (pinned shoots first either way).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShootSort {
    /// Newest import or edit first.
    #[default]
    Date,
    /// By name, A to Z.
    Name,
}

/// What the photographer made of the list (folders are compared however they are spelled).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShootPrefs {
    /// Pinned to the top, in the order they were pinned.
    pub pinned: Vec<String>,
    /// Taken off the list ("Remove from Shoots"); their photos stay in the library.
    pub hidden: Vec<String>,
    /// Names of their own (folder → name); the folder on disk keeps its name.
    pub names: BTreeMap<String, String>,
    pub sort: ShootSort,
}

impl ShootPrefs {
    fn pinned_at(&self, path: &str) -> Option<usize> {
        let k = folder_key(path);
        self.pinned.iter().position(|p| folder_key(p) == k)
    }
    fn is_hidden(&self, path: &str) -> bool {
        let k = folder_key(path);
        self.hidden.iter().any(|p| folder_key(p) == k)
    }
    fn name(&self, path: &str) -> Option<&str> {
        let k = folder_key(path);
        self.names.iter().find(|(p, _)| folder_key(p) == k).map(|(_, n)| n.as_str())
    }

    /// A folder moved from `from` to `to` (Find Missing Folder): its pin, removal and name follow.
    pub fn follow(&mut self, from: &str, to: &str) {
        let moved = |p: &str| -> Option<String> {
            if !lightcraft_catalog::query::folder_within(p, from) {
                return None;
            }
            let rest = folder_key(p).strip_prefix(folder_key(from).as_str()).map(str::to_string)?;
            Some(format!("{}{rest}", to.trim_end_matches(['/', '\\'])))
        };
        for list in [&mut self.pinned, &mut self.hidden] {
            for p in list.iter_mut() {
                if let Some(n) = moved(p) {
                    *p = n;
                }
            }
        }
        let names = std::mem::take(&mut self.names);
        self.names = names.into_iter().map(|(p, n)| (moved(&p).unwrap_or(p), n)).collect();
    }
}

/// One row of Shoots: the catalog's shoot and the photographer's choices for it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShootRow {
    #[serde(flatten)]
    pub shoot: Shoot,
    /// The name the folders give it (`shoot.name` is the one shown: its own name when renamed).
    pub auto_name: String,
    pub pinned: bool,
    pub hidden: bool,
}

/// The order names sort in: case-insensitive, leading marks (`*`, `-`, `_`) ignored.
fn name_key(n: &str) -> String {
    n.trim_start_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

/// Shoots as the panel lists them, from the catalog's `shoots`: pinned first (in pin order),
/// then by the chosen sort; removed ones only with `hidden`.
pub fn rows(shoots: Vec<Shoot>, prefs: &ShootPrefs, hidden: bool) -> Vec<ShootRow> {
    let mut v: Vec<ShootRow> = shoots
        .into_iter()
        .map(|s| {
            let auto_name = s.name.clone();
            let name = prefs.name(&s.path).map(str::to_string).unwrap_or_else(|| s.name.clone());
            ShootRow { pinned: prefs.pinned_at(&s.path).is_some(), hidden: prefs.is_hidden(&s.path), auto_name, shoot: Shoot { name, ..s } }
        })
        .filter(|r| hidden || !r.hidden)
        .collect();
    let by_name = |a: &ShootRow, b: &ShootRow| name_key(&a.shoot.name).cmp(&name_key(&b.shoot.name)).then_with(|| a.shoot.path.cmp(&b.shoot.path));
    v.sort_by(|a, b| {
        let pa = prefs.pinned_at(&a.shoot.path).unwrap_or(usize::MAX);
        let pb = prefs.pinned_at(&b.shoot.path).unwrap_or(usize::MAX);
        pa.cmp(&pb).then_with(|| match prefs.sort {
            ShootSort::Date => b.shoot.latest.cmp(&a.shoot.latest).then_with(|| by_name(a, b)),
            ShootSort::Name => by_name(a, b),
        })
    });
    v
}

/// The library's Shoots rows (see [`rows`]).
pub fn list(s: &Session, hidden: bool) -> Vec<ShootRow> {
    rows(s.catalog.shoots(&s.empty_folders), &s.shoot_prefs, hidden)
}

/// The shoot at `path` (removed ones too), or an error naming the command.
fn find(s: &Session, c: &str, p: &Value) -> Result<ShootRow> {
    let path =
        str_param(p, "path").filter(|d| !d.trim().is_empty()).ok_or_else(|| bad(c, "missing `path` (a shoot's folder, from library.shoots)"))?;
    let k = folder_key(path);
    list(s, true).into_iter().find(|r| folder_key(&r.shoot.path) == k).ok_or_else(|| bad(c, format!("{path}: not a shoot (see library.shoots)")))
}

/// `on` as given, else the opposite of `now`.
fn on_or_toggle(c: &str, p: &Value, now: bool) -> Result<bool> {
    match p.get("on") {
        None | Some(Value::Null) => Ok(!now),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(bad(c, "`on` is true or false")),
    }
}

fn pin(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shoot.pin";
    let r = find(s, C, p)?;
    let on = on_or_toggle(C, p, r.pinned)?;
    let k = folder_key(&r.shoot.path);
    s.shoot_prefs.pinned.retain(|x| folder_key(x) != k);
    if on {
        s.shoot_prefs.pinned.push(r.shoot.path.clone());
    }
    s.save_prefs()?;
    Ok(json!({"path": r.shoot.path, "pinned": on}))
}

fn hide(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shoot.hide";
    let r = find(s, C, p)?;
    let on = on_or_toggle(C, p, r.hidden)?;
    let k = folder_key(&r.shoot.path);
    s.shoot_prefs.hidden.retain(|x| folder_key(x) != k);
    if on {
        s.shoot_prefs.hidden.push(r.shoot.path.clone());
        // a removed shoot is not pinned any more
        s.shoot_prefs.pinned.retain(|x| folder_key(x) != k);
    }
    s.save_prefs()?;
    Ok(json!({"path": r.shoot.path, "hidden": on}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shoot.rename";
    let r = find(s, C, p)?;
    let name = match p.get("name") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(n)) => n.trim().to_string(),
        Some(_) => return Err(bad(C, "`name` is text (empty: the folder's name again)")),
    };
    if name.chars().count() > 300 {
        return Err(bad(C, "a name of at most 300 characters"));
    }
    let k = folder_key(&r.shoot.path);
    s.shoot_prefs.names.retain(|x, _| folder_key(x) != k);
    if !name.is_empty() && name != r.auto_name {
        s.shoot_prefs.names.insert(r.shoot.path.clone(), name.clone());
    }
    s.save_prefs()?;
    let shown = if name.is_empty() { r.auto_name.clone() } else { name };
    Ok(json!({"path": r.shoot.path, "name": shown}))
}

fn sort(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shoot.sort";
    s.shoot_prefs.sort = match str_param(p, "by") {
        Some("date") => ShootSort::Date,
        Some("name") => ShootSort::Name,
        _ => return Err(bad(C, "`by` is date or name")),
    };
    s.save_prefs()?;
    Ok(json!({"by": s.shoot_prefs.sort}))
}

fn show(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shoot.show";
    let r = find(s, C, p)?;
    s.library_folder = Some(r.shoot.path.clone());
    s.library_folder_subfolders = Some(r.shoot.deep);
    s.filter.library_folder = None;
    s.source = LibrarySource::LibraryFolder;
    let vis = s.visible_cloned();
    if s.selection.active.is_none_or(|a| !vis.contains(&a)) {
        s.selection = vis.first().map(|f| Selection::single(*f)).unwrap_or_default();
    }
    Ok(json!({"path": r.shoot.path, "count": vis.len()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "library.shoots",
            "Shoots",
            [],
            None,
            "{hidden?: bool (include shoots removed from the list)} → {sort, shoots: [{name, autoName, path, deep, disk, diskName, count, latest, pinned, hidden}]} — the library's photo work, one row per shoot folder on any disk (camera/format folders like raw, M262, X-T2 raw belong to the shoot above them), pinned first, then newest import/edit first or by name; show one with shoot.show",
            always,
            |s, p| Ok(json!({"sort": s.shoot_prefs.sort, "shoots": list(s, super::bool_or(p, "hidden", false))}))
        ),
        cmd!(
            "shoot.show",
            "Show Shoot",
            [],
            None,
            "{path} — show a shoot's photos (all of its folder, the folders inside it included; a container's loose photos only its own) → {path, count}",
            always,
            show
        ),
        cmd!(
            "shoot.pin",
            "Pin to Top",
            [],
            None,
            "{path, on?: bool (default: toggle)} — keep a shoot at the top of Shoots → {path, pinned}",
            always,
            pin
        ),
        cmd!(
            "shoot.hide",
            "Remove from Shoots",
            [],
            None,
            "{path, on?: bool (default: toggle; false puts it back)} — take a shoot off the Shoots list; its photos stay in the library and in Folders → {path, hidden}",
            always,
            hide
        ),
        cmd!(
            "shoot.rename",
            "Rename Shoot",
            [],
            None,
            "{path, name (empty: the folder's name again)} — the name Shoots shows; the folder on disk is not renamed → {path, name}",
            always,
            rename
        ),
        cmd!(
            "shoot.sort",
            "Sort Shoots",
            [],
            None,
            "{by: date|name} — newest import/edit first, or by name (pinned shoots stay on top) → {by}",
            always,
            sort
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shoot(name: &str, path: &str, latest: &str) -> Shoot {
        Shoot { name: name.into(), path: path.into(), deep: true, disk: "/Volumes/X".into(), disk_name: "X".into(), count: 1, latest: latest.into() }
    }

    fn names(v: &[ShootRow]) -> Vec<&str> {
        v.iter().map(|r| r.shoot.name.as_str()).collect()
    }

    #[test]
    fn pinned_first_then_newest_or_by_name_and_removed_ones_left_out() {
        let all = vec![
            shoot("*Beta", "/Volumes/X/*Beta", "2026-02-01"),
            shoot("alpha", "/Volumes/X/alpha", "2026-01-01"),
            shoot("Gamma", "/Volumes/X/Gamma", "2026-03-01"),
            shoot("Delta", "/Volumes/X/Delta", "2026-04-01"),
        ];
        let mut prefs = ShootPrefs::default();
        assert_eq!(names(&rows(all.clone(), &prefs, false)), ["Delta", "Gamma", "*Beta", "alpha"]);
        prefs.sort = ShootSort::Name;
        assert_eq!(names(&rows(all.clone(), &prefs, false)), ["alpha", "*Beta", "Delta", "Gamma"]);
        prefs.pinned = vec!["/Volumes/X/Gamma/".into(), "/Volumes/X//alpha".into()];
        prefs.hidden = vec!["/Volumes/X/Delta".into()];
        prefs.names.insert("/Volumes/X/*Beta".into(), "Zed".into());
        let v = rows(all.clone(), &prefs, false);
        assert_eq!(names(&v), ["Gamma", "alpha", "Zed"]);
        assert!(v[0].pinned && v[1].pinned && !v[2].pinned);
        assert_eq!(v[2].auto_name, "*Beta");
        let with_hidden = rows(all, &prefs, true);
        assert!(with_hidden.iter().any(|r| r.hidden && r.shoot.name == "Delta"));
    }

    #[test]
    fn choices_follow_a_folder_that_moved() {
        let mut prefs = ShootPrefs { pinned: vec!["/Volumes/X/A/B".into()], hidden: vec!["/Volumes/Y/C".into()], ..Default::default() };
        prefs.names.insert("/Volumes/X/A".into(), "Mine".into());
        prefs.follow("/Volumes/X/A", "/Volumes/Z/A");
        assert_eq!(prefs.pinned, ["/Volumes/Z/A/B"]);
        assert_eq!(prefs.hidden, ["/Volumes/Y/C"]);
        assert_eq!(prefs.names.get("/Volumes/Z/A").map(String::as_str), Some("Mine"));
    }

    #[test]
    fn commands_pin_remove_rename_sort_and_show() {
        use lightcraft_catalog::{Op, Photo, Source};
        let mut s = Session::new();
        for (i, path) in [
            "/Volumes/X/Erika and Connor wedding/raw/M262/a.jpg",
            "/Volumes/X/Erika and Connor wedding/raw/GR3/b.jpg",
            "/Volumes/Y/Shoot 2/photos/c.jpg",
        ]
        .iter()
        .enumerate()
        {
            let id = s.catalog.alloc_photo_id();
            let p = Photo::new(id, Source::File { path: (*path).into() }, "x.jpg", "JPEG", 60, 40, &format!("2026-01-0{}T10:00:00", i + 1));
            s.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
        }
        let r = s.execute("library.shoots", &json!({})).unwrap();
        let shown: Vec<&str> = r["shoots"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(shown, ["Shoot 2", "Erika and Connor wedding"]);
        let erika = "/Volumes/X/Erika and Connor wedding";
        // Show Photos in Subfolders off: a shoot still shows all of its folder
        s.execute("library.showSubfolders", &json!({"on": false})).unwrap();
        assert_eq!(s.execute("shoot.show", &json!({"path": erika})).unwrap()["count"], 2);
        assert_eq!(s.source, LibrarySource::LibraryFolder);
        // choosing the folder itself goes by the setting again
        s.execute("library.source", &json!({"kind": "libraryFolder", "path": erika})).unwrap();
        assert_eq!(s.visible_cloned().len(), 0);
        s.execute("shoot.pin", &json!({"path": erika})).unwrap();
        s.execute("shoot.rename", &json!({"path": erika, "name": "E & C"})).unwrap();
        let r = s.execute("library.shoots", &json!({})).unwrap();
        assert_eq!(r["shoots"][0]["name"], "E & C");
        assert_eq!(r["shoots"][0]["pinned"], true);
        s.execute("shoot.hide", &json!({"path": "/Volumes/Y/Shoot 2"})).unwrap();
        assert_eq!(s.execute("library.shoots", &json!({})).unwrap()["shoots"].as_array().unwrap().len(), 1);
        s.execute("shoot.hide", &json!({"path": "/Volumes/Y/Shoot 2", "on": false})).unwrap();
        s.execute("shoot.sort", &json!({"by": "name"})).unwrap();
        s.execute("shoot.rename", &json!({"path": erika, "name": ""})).unwrap();
        let r = s.execute("library.shoots", &json!({})).unwrap();
        assert_eq!(r["sort"], "name");
        assert_eq!(r["shoots"][0]["name"], "Erika and Connor wedding");
        // bad arguments are errors, never a panic
        for (c, p) in [
            ("shoot.pin", json!({})),
            ("shoot.pin", json!({"path": "/nowhere"})),
            ("shoot.pin", json!({"path": erika, "on": 3})),
            ("shoot.rename", json!({"path": erika, "name": 5})),
            ("shoot.sort", json!({"by": "size"})),
            ("shoot.show", json!({"path": ""})),
            ("library.source", json!({"kind": "libraryFolder", "path": erika, "subfolders": "yes"})),
        ] {
            assert!(s.execute(c, &p).is_err(), "{c} {p}");
        }
    }
}
