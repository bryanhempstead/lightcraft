//! File ▸ Migrate from Lightroom Classic…: read the catalog and work out every photo's settings on a
//! worker thread, import the photos with the background import task, then apply the rest
//! (`library.migrateLightroom` with the prepared records and `import: false`) when it finishes.
//! The window stays responsive throughout; progress shows as the import's (`ui.inspect` →
//! `import`) and, before that, as a task (`ui.inspect` → `tasks`).

use serde_json::{Value, json};

use crate::LightcraftApp;

/// The task label while the catalog is read.
pub const READING: &str = "Reading the Lightroom catalog";

/// Whether `library.migrateLightroom` with these params runs in the background here (the app's
/// own migration); a dry run, or the apply step after the import (`import: false`), runs as the
/// plain engine command.
pub fn runs_in_background(p: &Value) -> bool {
    !p.get("dryRun").and_then(Value::as_bool).unwrap_or(false) && p.get("import").and_then(Value::as_bool).unwrap_or(true)
}

/// Start a migration: `p` = `library.migrateLightroom`'s params.
pub fn start(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if app.import.is_some() || app.tasks.is_running(READING) {
        return Err("an import is running; migrate when it has finished".into());
    }
    let catalog = match p.get("catalog").and_then(Value::as_str).filter(|c| !c.trim().is_empty()) {
        Some(c) => std::path::PathBuf::from(c),
        None => match lightcraft_engine::lr_migrate::default_catalog() {
            Some(c) => c,
            None => match app.services.pick_files.as_mut().and_then(|f| f().into_iter().next()) {
                Some(c) => std::path::PathBuf::from(c),
                None => return Err("no Lightroom catalog (.lrcat) found in ~/Pictures/Lightroom".into()),
            },
        },
    };
    let limit = p.get("limit").and_then(Value::as_u64).map(|l| l.min(usize::MAX as u64) as usize);
    let records: Option<String> = p.get("records").and_then(Value::as_str).map(str::to_string);
    let scratch = std::env::temp_dir();
    let out = scratch.join(format!("lightcraft-lightroom-records-{}.json", std::process::id()));
    let shown = catalog.display().to_string();
    let work = move || -> Result<(String, Vec<String>), String> {
        let mut rec = match records {
            Some(r) => lightcraft_engine::lr_migrate::read_records(&r)?,
            None => lightcraft_engine::lr_migrate::dump_catalog(&catalog, &scratch)?,
        };
        lightcraft_engine::lr_migrate::prepare(&mut rec, limit);
        let files = lightcraft_engine::lr_migrate::files_to_import(&rec, limit);
        let bytes = serde_json::to_vec(&rec).map_err(|e| e.to_string())?;
        std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
        Ok((out.display().to_string(), files))
    };
    let mut then = p.clone();
    let done = move |app: &mut LightcraftApp, ctx: &egui::Context, r: Result<(String, Vec<String>), String>| match r {
        Ok((records, files)) => {
            if let Some(o) = then.as_object_mut() {
                o.remove("catalog");
                o.insert("records".into(), json!(records));
                o.insert("import".into(), json!(false));
            }
            let n = files.len();
            match crate::import::start_paths_then(app, files, Some(("library.migrateLightroom".to_string(), then, summary))) {
                Ok(_) => app.toast(ctx, format!("Migrating from Lightroom Classic: importing {n} photos…")),
                Err(e) => app.toast_error(ctx, format!("Migrate from Lightroom Classic: {e}")),
            }
        }
        Err(e) => app.toast_error(ctx, format!("Migrate from Lightroom Classic: {e}")),
    };
    crate::tasks::spawn(app, READING, work, done)?;
    Ok(json!({"background": true, "catalog": shown}))
}

/// The toast after the apply step.
pub fn summary(v: &Value) -> String {
    let n = |k: &str| v[k].as_u64().unwrap_or(0);
    let mut msg = format!(
        "Migrated from Lightroom Classic: {} photos, {} with edits, {} albums",
        n("matched"),
        v["develop"]["applied"].as_u64().unwrap_or(0),
        v["albums"]["albums"].as_u64().unwrap_or(0) + v["albums"]["smart"].as_u64().unwrap_or(0)
    );
    if n("missing") > 0 {
        msg.push_str(&format!(" · {} not found (offline drives?)", n("missing")));
    }
    let looks = v["develop"]["creativeLooks"].as_object().map_or(0, |o| o.values().filter_map(Value::as_u64).sum::<u64>());
    let matched = v["develop"]["creativeLooksMatched"].as_u64().unwrap_or(0);
    if looks > matched {
        msg.push_str(&format!(" · {} use a creative profile we don't have", looks - matched));
    }
    if let Some(p) = v["presets"]["imported"].as_u64() {
        msg.push_str(&format!(" · {p} presets"));
    }
    msg
}
