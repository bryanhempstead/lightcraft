//! Filter presets: the filter bar's settings saved under a name ("Five stars", "Unedited raws")
//! and applied in one step. Saved with the library.

use lightcraft_catalog::Filter;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::{Result, Session};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterPreset {
    pub name: String,
    pub filter: Filter,
}

fn list(s: &Session) -> Value {
    json!(s.filter_presets)
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "filter.savePreset";
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(C, "missing `name`"))?.to_string();
    if s.filter == Filter::default() {
        return Err(bad(C, "the filter is empty: set some filters first"));
    }
    let preset = FilterPreset { name: name.clone(), filter: s.filter.clone() };
    match s.filter_presets.iter_mut().find(|f| f.name.eq_ignore_ascii_case(&name)) {
        Some(f) => *f = preset,
        None => s.filter_presets.push(preset),
    }
    s.save_prefs()?;
    Ok(list(s))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "filter.applyPreset";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    let f = s.filter_presets.iter().find(|f| f.name.eq_ignore_ascii_case(name)).ok_or_else(|| bad(C, format!("no filter preset `{name}`")))?;
    s.filter = f.filter.clone();
    Ok(json!({"photos": s.visible().len()}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "filter.deletePreset";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    let n = s.filter_presets.len();
    s.filter_presets.retain(|f| !f.name.eq_ignore_ascii_case(name));
    if n == s.filter_presets.len() {
        return Err(bad(C, format!("no filter preset `{name}`")));
    }
    s.save_prefs()?;
    Ok(list(s))
}

/// The metadata column browser (L07): each column's values with counts over the photos the view
/// shows when only the columns to its left narrow it (Lightroom's behaviour).
fn column_browser(s: &mut Session, p: &Value) -> Result<Value> {
    use lightcraft_catalog::columns::{COLUMNS, ColumnPick, DEFAULT_COLUMNS, counts};
    let cols: Vec<String> = match p.get("columns").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => DEFAULT_COLUMNS.iter().map(|c| c.to_string()).collect(),
    };
    if let Some(bad_col) = cols.iter().find(|c| !COLUMNS.iter().any(|(id, _)| id == c)) {
        return Err(bad(
            "library.columnBrowser",
            format!("unknown column `{bad_col}` ({})", COLUMNS.iter().map(|c| c.0).collect::<Vec<_>>().join("|")),
        ));
    }
    let saved = s.filter.clone();
    let picks = saved.columns.clone();
    let mut out = Vec::new();
    for (i, c) in cols.iter().enumerate() {
        let left: Vec<ColumnPick> = cols[..i].iter().filter_map(|lc| picks.iter().find(|p| &p.column == lc).cloned()).collect();
        // picks in columns not shown still narrow every column
        let hidden = picks.iter().filter(|p| !cols.contains(&p.column)).cloned();
        s.filter.columns = left.into_iter().chain(hidden).collect();
        let ids = s.visible_cloned();
        let values = counts(ids.iter().filter_map(|id| s.catalog.photo(*id)).map(|a| &**a), &s.catalog, c);
        let label = COLUMNS.iter().find(|(id, _)| id == c).map(|x| x.1).unwrap_or("");
        let picked = picks.iter().find(|p| &p.column == c).map(|p| p.values.clone()).unwrap_or_default();
        out.push(json!({
            "column": c,
            "label": label,
            "total": ids.len(),
            "values": values.iter().map(|(v, n)| json!({"value": v, "count": n})).collect::<Vec<_>>(),
            "picked": picked,
        }));
    }
    s.filter = saved;
    Ok(json!({"columns": out, "available": COLUMNS.iter().map(|(id, l)| json!({"id": id, "label": l})).collect::<Vec<_>>()}))
}

fn column_pick(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.columnPick";
    let column = str_param(p, "column").ok_or_else(|| bad(C, "missing `column`"))?.to_string();
    if !lightcraft_catalog::columns::COLUMNS.iter().any(|(id, _)| *id == column) {
        return Err(bad(C, format!("unknown column `{column}`")));
    }
    let values: Vec<String> =
        p.get("values").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    s.filter.columns.retain(|c| c.column != column);
    if !values.is_empty() {
        s.filter.columns.push(lightcraft_catalog::columns::ColumnPick { column, values });
    }
    Ok(json!({"count": s.visible().len()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "filter.presets", "Filter Presets", [], None, "{} → [{name, filter}]", always, |s, _| Ok(list(s))),
        cmd!(
            query "library.exportStatus",
            "Export Status",
            [],
            None,
            "{ids?} — when each photo was last exported (null = never; LightCraft's own exports, kept with the library) → [{id, exported}]",
            always,
            |s, p| {
                let ids = s.targets(p);
                Ok(json!(ids.iter().map(|id| json!({"id": id.0, "exported": s.export_log.get(&id.0)})).collect::<Vec<_>>()))
            }
        ),
        cmd!(
            "library.resetExportStatus",
            "Reset Export Status",
            ["Library"],
            None,
            "{ids?} — forgets that the photos were exported (the Exported filter no longer shows them) → {reset}",
            super::has_selection,
            |s, p| {
                let ids = s.targets(p);
                let n = ids.iter().filter(|id| s.export_log.remove(&id.0).is_some()).count();
                if n > 0 {
                    s.save_exports();
                }
                Ok(json!({"reset": n}))
            }
        ),
        cmd!(
            query "library.columnBrowser",
            "Metadata Columns",
            [],
            None,
            "{columns?: [date|year|camera|lens|label|keyword|fileType|flag|rating|iso|focalLength|aperture|shutter|country|city|location|creator|treatment|aspect|copies|folder] (default date, camera, lens, label)} — the Library Filter's metadata columns: each column's values with photo counts, narrowed by the picks in the columns to its left → {columns: [{column, label, total, values: [{value (\"\" = unknown), count}], picked}], available: [{id, label}]}",
            always,
            column_browser
        ),
        cmd!(
            "library.columnPick",
            "Pick Metadata Values",
            [],
            None,
            "{column, values: [..] (empty = all)} — shows only photos with one of these values in that column (several columns: all of them) → {count}",
            always,
            column_pick
        ),
        cmd!(
            "filter.savePreset",
            "Save Filter Preset",
            [],
            None,
            "{name} — the current filter (library.filter), saved with the library",
            always,
            save
        ),
        cmd!("filter.applyPreset", "Apply Filter Preset", [], None, "{name} — replaces the current filter → {photos}", always, apply),
        cmd!("filter.deletePreset", "Delete Filter Preset", [], None, "{name}", always, delete),
    ]
}
