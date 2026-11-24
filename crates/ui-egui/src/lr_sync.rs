//! File ▸ Sync from Lightroom Classic…: read the catalog (a copy) and the changed photos' history
//! on a worker thread, show what a sync would bring (`library.syncLightroom` dry run) and which
//! photos changed on both sides, then apply it with the chosen rule for those.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;

/// The task label while the catalog is read.
pub const READING: &str = "Reading the Lightroom catalog";

/// Open the dialog and start reading (`p`: `{catalog?}`).
pub fn start(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if app.tasks.is_running(READING) || app.import.is_some() {
        return Err("busy reading or importing; try again when it has finished".into());
    }
    let catalog = match p.get("catalog").and_then(Value::as_str).filter(|c| !c.trim().is_empty()) {
        Some(c) => std::path::PathBuf::from(c),
        None => lightcraft_engine::lr_migrate::default_catalog().ok_or("no Lightroom catalog (.lrcat) found in ~/Pictures/Lightroom")?,
    };
    let lib = app.session.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone());
    let paths: HashSet<String> = app
        .session
        .catalog
        .photos()
        .filter_map(|p| if let lightcraft_catalog::Source::File { path } = &p.source { Some(path.clone()) } else { None })
        .collect();
    let scratch = std::env::temp_dir();
    let pid = std::process::id();
    let records = scratch.join(format!("lightcraft-lightroom-sync-{pid}.json"));
    let extras = scratch.join(format!("lightcraft-lightroom-sync-history-{pid}.json"));
    let shown = catalog.display().to_string();
    let (r_out, e_out) = (records.clone(), extras.clone());
    let work = move || -> Result<(), String> {
        let (rec, ex) = lightcraft_engine::lr_migrate::with_catalog_copy(&catalog, &scratch, |db| {
            let rec = lightcraft_engine::lr_migrate::records_from_db(db, &catalog.display().to_string())?;
            let want = lightcraft_engine::lr_sync::wanted(&rec, lib.as_deref(), &paths);
            let ex = lightcraft_engine::lr_history::read_extras(db, &want)?;
            Ok((rec, ex))
        })?;
        let w = |path: &std::path::Path, bytes: Vec<u8>| std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()));
        w(&r_out, serde_json::to_vec(&rec).map_err(|e| e.to_string())?)?;
        w(&e_out, serde_json::to_vec(&ex).map_err(|e| e.to_string())?)
    };
    let (records, extras) = (records.display().to_string(), extras.display().to_string());
    app.ui.dialog = Some(Dialog::LightroomSync {
        catalog: shown.clone(),
        records: records.clone(),
        extras: extras.clone(),
        report: None,
        conflicts: "ask".into(),
        error: None,
    });
    let done = move |app: &mut LightcraftApp, _ctx: &egui::Context, r: Result<(), String>| {
        let r = r.and_then(|()| app.run("library.syncLightroom", json!({"records": records, "extras": extras, "dryRun": true})));
        if let Some(Dialog::LightroomSync { report, error, .. }) = &mut app.ui.dialog {
            match r {
                Ok(v) => *report = Some(v),
                Err(e) => *error = Some(e),
            }
        }
    };
    crate::tasks::spawn(app, READING, work, done)?;
    Ok(json!({"background": true, "catalog": shown}))
}

/// The dialog's body: what a sync brings, and the rule for photos changed on both sides.
pub fn body(ui: &mut egui::Ui, catalog: &str, report: Option<&Value>, conflicts: &mut String, error: Option<&str>) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(catalog).color(t.text_dim));
    if let Some(e) = error {
        ui.label(egui::RichText::new(e).color(t.text_label));
        return;
    }
    let Some(r) = report else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(egui::RichText::new(crate::i18n::tr("Reading the Lightroom catalog…")).color(t.text_label));
        });
        return;
    };
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let line = |ui: &mut egui::Ui, text: String| {
        ui.label(egui::RichText::new(text).color(t.text_label));
    };
    let since = r["since"].as_str().map(|s| s.replace('T', " ")).unwrap_or_else(|| "the migration".into());
    ui.label(egui::RichText::new(format!("Changed in Lightroom since {since}:")).color(t.text_dim));
    let np = &r["newPhotos"];
    line(ui, format!("{} to import", photos(n(&np["found"]), "new photo")));
    if n(&np["missingFiles"]) > 0 {
        line(ui, format!("{} whose files aren't there (drive offline?)", photos(n(&np["missingFiles"]), "photo")));
    }
    if n(&np["virtualCopies"]) > 0 {
        line(ui, format!("{} new virtual copies", n(&np["virtualCopies"])));
    }
    let d = &r["develop"];
    line(ui, format!("{} with new edits to bring over", photos(n(&d["fromLightroom"]), "photo")));
    let m = &r["metadata"];
    let meta = n(&m["rating"]) + n(&m["flag"]) + n(&m["label"]) + n(&m["keywords"]) + n(&m["text"]);
    line(ui, format!("{meta} ratings, flags, labels, keywords and captions"));
    let c = &r["collections"];
    if n(&c["added"]) + n(&c["removed"]) > 0 {
        line(ui, format!("{} added to collections, {} removed", n(&c["added"]), n(&c["removed"])));
    }
    if n(&d["keptMine"]) > 0 {
        line(ui, format!("{} edited only here: kept", n(&d["keptMine"])));
    }
    let both = n(&d["conflicts"]) + n(&m["conflicts"]);
    if both == 0 {
        if n(&np["found"]) + n(&d["fromLightroom"]) + meta + n(&c["added"]) + n(&c["removed"]) == 0 {
            ui.add_space(4.0);
            line(ui, crate::i18n::tr("Nothing new: this library is up to date with the catalog.").to_string());
        }
        return;
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new(format!("{both} changed both here and in Lightroom:")).font(t.semibold(12.5)).color(t.text));
    for x in d["conflictList"].as_array().into_iter().flatten().take(8) {
        let newer = if x["newer"] == "lightroom" { "Lightroom newer" } else { "edited here later" };
        ui.label(egui::RichText::new(format!("{}  ·  {}", x["file"].as_str().unwrap_or(""), crate::i18n::tr(newer))).color(t.text_dim));
    }
    if n(&d["conflicts"]) > 8 {
        ui.label(egui::RichText::new(format!("and {} more", n(&d["conflicts"]) - 8)).color(t.text_dim));
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (key, label) in [("ask", "mine."), ("lightroom", "lightroom."), ("newest", "newest.")] {
            let r = ui.selectable_label(conflicts == key, label);
            crate::widgets::register(ui.ctx(), format!("chip:lrSyncConflicts:{key}"), r.rect);
            if r.clicked() {
                *conflicts = key.to_string();
            }
        }
    });
    let why = match conflicts.as_str() {
        "lightroom" => "Lightroom's settings replace these; ours are kept as a version.",
        "newest" => "The side edited last wins; ours are kept as a version when Lightroom's win.",
        _ => "These stay as they are here (asked again next sync).",
    };
    ui.label(egui::RichText::new(crate::i18n::tr(why)).color(t.text_dim));
}

/// `3 photos`, `1 photo`.
fn photos(n: u64, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// Apply the sync the dialog showed.
pub fn confirm(app: &mut LightcraftApp, records: &str, extras: &str, conflicts: &str) -> Result<Value, String> {
    let r = app.run("library.syncLightroom", json!({"records": records, "extras": extras, "conflicts": conflicts}))?;
    for f in [records, extras] {
        let _ = std::fs::remove_file(f);
    }
    Ok(r)
}

/// The toast after a sync.
pub fn summary(v: &Value) -> String {
    let n = |x: &Value| x.as_u64().unwrap_or(0);
    let m = &v["metadata"];
    format!(
        "Synced from Lightroom Classic: {} new photos, {} edits, {} metadata changes, {} conflicts kept",
        n(&v["newPhotos"]["imported"]),
        n(&v["develop"]["fromLightroom"]),
        n(&m["rating"]) + n(&m["flag"]) + n(&m["label"]) + n(&m["keywords"]) + n(&m["text"]),
        n(&v["develop"]["conflicts"]).saturating_sub(n(&v["develop"]["resolved"]))
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn summary_counts_what_came_over() {
        let v = json!({"newPhotos": {"imported": 2}, "develop": {"fromLightroom": 1, "conflicts": 3, "resolved": 1}, "metadata": {"rating": 2, "keywords": 1}});
        assert_eq!(super::summary(&v), "Synced from Lightroom Classic: 2 new photos, 1 edits, 3 metadata changes, 2 conflicts kept");
        assert_eq!(super::summary(&json!({})), "Synced from Lightroom Classic: 0 new photos, 0 edits, 0 metadata changes, 0 conflicts kept");
        assert_eq!((super::photos(1, "photo"), super::photos(0, "photo")), ("1 photo".to_string(), "0 photos".to_string()));
    }
}
