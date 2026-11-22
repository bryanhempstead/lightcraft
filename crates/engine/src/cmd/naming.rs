//! Named file-name templates (E01, Lightroom's Filename Template Editor presets): the ten built-in
//! templates plus the user's own, shared by Rename Photos, import renaming and export naming;
//! Custom Text / Shoot Name / extension case params; the Import # / Image # counters.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::rename::{BUILTIN_TEMPLATES, Batch, ExtCase, NamingTemplate};
use crate::{Result, Session};

/// Imports made and photos imported so far (Import # / Image #).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Counters {
    pub imports: u64,
    pub images: u64,
}

/// The template a command names: `key` (a template string) or `name_key` (a template by name).
pub fn template_param(s: &Session, p: &Value, key: &str, name_key: &str, c: &str) -> Result<Option<String>> {
    if let Some(n) = str_param(p, name_key).map(str::trim).filter(|n| !n.is_empty()) {
        return crate::rename::named_template(&s.naming_templates, n)
            .map(|t| Some(t.into_owned()))
            .ok_or_else(|| bad(c, format!("unknown file-name template `{n}` (see naming.templates)")));
    }
    Ok(str_param(p, key).map(str::to_string))
}

/// `text`, `shoot`, `extCase` → a [`Batch`].
pub fn batch_param(p: &Value, c: &str) -> Result<Batch> {
    let ext_case = match str_param(p, "extCase") {
        None => ExtCase::Keep,
        Some(x) => ExtCase::parse(x).ok_or_else(|| bad(c, format!("extCase must be keep|lower|upper, not `{x}`")))?,
    };
    Ok(Batch {
        text: str_param(p, "text").map(Batch::clean).unwrap_or_default(),
        shoot: str_param(p, "shoot").map(Batch::clean).unwrap_or_default(),
        ext_case,
        ..Default::default()
    })
}

/// The import's naming values: `renameText`, `shootName`, `renameExtCase`, and the next Import # /
/// Image #.
pub fn import_batch(s: &Session, p: &Value) -> Result<Batch> {
    let c = "library.import";
    let ext_case = match str_param(p, "renameExtCase") {
        None => ExtCase::Keep,
        Some(x) => ExtCase::parse(x).ok_or_else(|| bad(c, format!("renameExtCase must be keep|lower|upper, not `{x}`")))?,
    };
    Ok(Batch {
        text: str_param(p, "renameText").map(Batch::clean).unwrap_or_default(),
        shoot: str_param(p, "shootName").map(Batch::clean).unwrap_or_default(),
        import_no: s.naming_counters.imports.saturating_add(1),
        image_start: s.naming_counters.images.saturating_add(1),
        ext_case,
        ..Default::default()
    })
}

/// An import added `n` photos: Import # and Image # move on (saved with the library).
pub fn count_import(s: &mut Session, n: usize) {
    if n == 0 {
        return;
    }
    s.naming_counters.imports = s.naming_counters.imports.saturating_add(1);
    s.naming_counters.images = s.naming_counters.images.saturating_add(n as u64);
    if let Err(e) = s.save_prefs() {
        log::warn!("naming counters: {e}");
    }
}

fn example(s: &Session, template: &str) -> String {
    let b = Batch {
        total: 24,
        text: "Wedding".into(),
        shoot: "Smith".into(),
        import_no: s.naming_counters.imports + 1,
        image_start: 1,
        ..Default::default()
    };
    crate::rename::expand_in(template, &crate::rename::sample_photo(), 1, &b)
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let builtin: Vec<Value> =
        BUILTIN_TEMPLATES.iter().map(|(n, t)| json!({"name": n, "template": t, "builtin": true, "example": example(s, t)})).collect();
    let user: Vec<Value> = s
        .naming_templates
        .iter()
        .map(|u| json!({"name": u.name, "template": u.template, "builtin": false, "example": example(s, &u.template)}))
        .collect();
    Ok(json!({"templates": builtin.into_iter().chain(user).collect::<Vec<_>>(), "counters": s.naming_counters}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "naming.saveTemplate";
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(C, "missing `name`"))?.to_string();
    let template = str_param(p, "template").map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad(C, "missing `template`"))?.to_string();
    if name.chars().count() > 80 || template.chars().count() > 400 {
        return Err(bad(C, "name (80) or template (400 characters) too long"));
    }
    if BUILTIN_TEMPLATES.iter().any(|(b, _)| b.eq_ignore_ascii_case(&name)) {
        return Err(bad(C, format!("`{name}` is a built-in template; save under another name")));
    }
    let unknown = crate::rename::unknown_tokens(&template);
    match s.naming_templates.iter_mut().find(|u| u.name.eq_ignore_ascii_case(&name)) {
        Some(u) => u.template = template.clone(),
        None => s.naming_templates.push(NamingTemplate { name: name.clone(), template: template.clone() }),
    }
    s.save_prefs()?;
    Ok(json!({"name": name, "template": template, "example": example(s, &template), "unknownTokens": unknown}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "naming.deleteTemplate";
    let name = str_param(p, "name").map(str::trim).unwrap_or_default().to_string();
    let before = s.naming_templates.len();
    s.naming_templates.retain(|u| !u.name.eq_ignore_ascii_case(&name));
    if s.naming_templates.len() == before {
        return Err(bad(C, format!("no saved template `{name}` (built-in templates can't be deleted)")));
    }
    s.save_prefs()?;
    Ok(json!({"deleted": name}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "naming.templates",
            "File-Name Templates",
            [],
            None,
            "{} — Lightroom's ten built-in filename templates and the saved ones, each with an example for a sample photo (Custom Text \"Wedding\", Shoot Name \"Smith\", 24 photos) → {templates: [{name, template, builtin, example}], counters: {imports, images}}",
            always,
            list
        ),
        cmd!(
            "naming.saveTemplate",
            "Save File-Name Template",
            [],
            None,
            "{name, template} — saves (or updates) a named template for Rename Photos, import and export naming → {name, template, example, unknownTokens}",
            always,
            save
        ),
        cmd!("naming.deleteTemplate", "Delete File-Name Template", [], None, "{name} — deletes a saved template (built-ins stay)", always, delete),
    ]
}
