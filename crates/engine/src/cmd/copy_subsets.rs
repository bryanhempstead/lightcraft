//! Copy / Sync / Paste / New Preset checklists: Lightroom Classic's per-setting groups
//! ([`SettingsGroup`]), its built-in subsets (All, None, Default, Modified) and saved subsets
//! (New / Rename / Delete), persisted in the library's `prefs.json`.

use lightcraft_catalog::PhotoId;
use lightcraft_develop::{HEADINGS, SettingsGroup};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, has_active, str_param};
use crate::{Result, Session};

/// A saved checklist (Lightroom's custom copy subsets).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CopySubset {
    pub name: String,
    /// Group ids (kept as text so a file from another version never fails to load).
    pub groups: Vec<String>,
}

impl CopySubset {
    pub fn groups(&self) -> Vec<SettingsGroup> {
        SettingsGroup::parse_ids_lenient(&self.groups)
    }
}

/// The built-in subsets, in menu order.
pub const BUILTIN: [&str; 4] = ["all", "none", "default", "modified"];

/// The groups in which photo `id` differs from how it came in (its camera defaults).
pub fn modified_groups(s: &Session, id: PhotoId) -> Vec<SettingsGroup> {
    let base = s.catalog.photo(id).map(|p| p.camera_defaults()).unwrap_or_default();
    SettingsGroup::modified_in(&s.develop_of(id).unwrap_or_default(), &base)
}

/// A subset by name: all / none / default / modified (on the active photo) or a saved one.
pub fn resolve(s: &Session, name: &str) -> std::result::Result<Vec<SettingsGroup>, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "all" => Ok(SettingsGroup::ALL.to_vec()),
        "none" => Ok(Vec::new()),
        "default" => Ok(SettingsGroup::default_copy()),
        "modified" => s.active().map(|id| modified_groups(s, id)).ok_or_else(|| "`modified` needs an active photo".to_string()),
        n => s
            .copy_subsets
            .iter()
            .find(|x| x.name.trim().eq_ignore_ascii_case(n))
            .map(CopySubset::groups)
            .ok_or_else(|| format!("no subset `{}`", name.trim())),
    }
}

/// `groups` (per-setting ids, heading ids or older coarse ids) or else `subset` (a name for
/// [`resolve`]); `None` when neither is given.
pub fn groups_param(s: &Session, p: &Value, c: &str) -> Result<Option<Vec<SettingsGroup>>> {
    if let Some(g) = p.get("groups").filter(|g| !g.is_null()) {
        let ids: Vec<String> = serde_json::from_value(g.clone()).map_err(|e| bad(c, format!("`groups` must be a list of ids: {e}")))?;
        return SettingsGroup::parse_ids(&ids).map(Some).map_err(|e| bad(c, e));
    }
    match str_param(p, "subset") {
        Some(name) => resolve(s, name).map(Some).map_err(|e| bad(c, e)),
        None => Ok(None),
    }
}

/// `masks`: only these mask ids of the photo go along (Lightroom's choose-masks list).
pub fn masks_param(p: &Value, c: &str) -> Result<Option<Vec<u32>>> {
    match p.get("masks").filter(|m| !m.is_null()) {
        Some(m) => serde_json::from_value(m.clone()).map(Some).map_err(|e| bad(c, format!("`masks` must be a list of mask ids: {e}"))),
        None => Ok(None),
    }
}

fn ids(groups: &[SettingsGroup]) -> Vec<String> {
    groups.iter().map(|g| g.id()).collect()
}

fn list(s: &Session) -> Value {
    let saved: Vec<Value> = s.copy_subsets.iter().map(|x| json!({"name": x.name, "groups": ids(&x.groups())})).collect();
    json!({"builtin": BUILTIN, "subsets": saved})
}

fn name_param(p: &Value, key: &str, c: &str) -> Result<String> {
    let n = str_param(p, key).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, format!("missing `{key}`")))?;
    if BUILTIN.iter().any(|b| b.eq_ignore_ascii_case(n)) {
        return Err(bad(c, format!("`{n}` is a built-in subset: choose another name")));
    }
    Ok(n.to_string())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "develop.settingsGroups",
            "Settings Groups",
            [],
            None,
            "{} — the Copy / Sync / Paste / New Preset checklist: headings with their per-setting group ids and labels, the default subset → {headings: [{id, label, items: [{id, label}]}], default}",
            always,
            |_s, _p| {
                let headings: Vec<Value> = HEADINGS
                    .iter()
                    .map(|h| json!({"id": h.id, "label": h.label, "items": h.items.iter().map(|g| json!({"id": g.id(), "label": g.label()})).collect::<Vec<_>>()}))
                    .collect();
                Ok(json!({"headings": headings, "default": ids(&SettingsGroup::default_copy())}))
            }
        ),
        cmd!(
            "develop.modifiedGroups",
            "Modified Settings Groups",
            [],
            None,
            "{id?} — the groups in which the photo (default: the active one) differs from how it came in (Lightroom's Modified subset) → {groups}",
            has_active,
            |s, p| {
                let c = "develop.modifiedGroups";
                let id = match p.get("id").and_then(Value::as_u64) {
                    Some(id) => PhotoId(id),
                    None => s.active().ok_or_else(|| bad(c, "no active photo"))?,
                };
                if s.catalog.photo(id).is_none() {
                    return Err(bad(c, format!("no photo {}", id.0)));
                }
                Ok(json!({"groups": ids(&modified_groups(s, id))}))
            }
        ),
        cmd!(
            "develop.subsets",
            "Settings Subsets",
            [],
            None,
            "{} — the built-in subsets (all, none, default, modified) and the saved ones → {builtin, subsets: [{name, groups}]}",
            always,
            |s, _p| Ok(list(s))
        ),
        cmd!(
            "develop.saveSubset",
            "Save Settings Subset",
            [],
            None,
            "{name, groups: [settingsGroup]} — save (or replace, same name) a checklist for Copy / Sync / Paste / New Preset → the subsets",
            always,
            |s, p| {
                let c = "develop.saveSubset";
                let name = name_param(p, "name", c)?;
                let groups = groups_param(s, p, c)?.ok_or_else(|| bad(c, "missing `groups`"))?;
                let sub = CopySubset { name: name.clone(), groups: ids(&groups) };
                match s.copy_subsets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)) {
                    Some(x) => *x = sub,
                    None => s.copy_subsets.push(sub),
                }
                s.save_prefs()?;
                Ok(list(s))
            }
        ),
        cmd!("develop.renameSubset", "Rename Settings Subset", [], None, "{name, to} → the subsets", always, |s, p| {
            let c = "develop.renameSubset";
            let (name, to) = (name_param(p, "name", c)?, name_param(p, "to", c)?);
            if !to.eq_ignore_ascii_case(&name) && s.copy_subsets.iter().any(|x| x.name.eq_ignore_ascii_case(&to)) {
                return Err(bad(c, format!("a subset `{to}` already exists")));
            }
            let x = s.copy_subsets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)).ok_or_else(|| bad(c, format!("no subset `{name}`")))?;
            x.name = to;
            s.save_prefs()?;
            Ok(list(s))
        }),
        cmd!("develop.deleteSubset", "Delete Settings Subset", [], None, "{name} → the subsets", always, |s, p| {
            let c = "develop.deleteSubset";
            let name = name_param(p, "name", c)?;
            let n = s.copy_subsets.len();
            s.copy_subsets.retain(|x| !x.name.eq_ignore_ascii_case(&name));
            if n == s.copy_subsets.len() {
                return Err(bad(c, format!("no subset `{name}`")));
            }
            s.save_prefs()?;
            Ok(list(s))
        }),
    ]
}
