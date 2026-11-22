//! LRC-LIB-PUBLISH / LR-EXP-PHOTOS in the app: publishing a collection and exporting to Apple
//! Photos run on a worker thread ([`crate::tasks`]), the engine's three steps
//! ([`lightcraft_engine::publish`]: prepare here, run there, finish here). The Publish Services
//! panel itself is in [`crate::panels::left`].

use serde_json::{Value, json};

use crate::LightcraftApp;

/// The task label while a collection publishes (`ui.inspect` → `tasks`).
pub const PUBLISHING: &str = "publish";
/// … and while photos go to Apple Photos.
pub const TO_PHOTOS: &str = "export to Photos";

fn summary(name: &str, r: &Value) -> String {
    let (p, d) = (r["published"].as_u64().unwrap_or(0), r["removed"].as_u64().unwrap_or(0));
    let errors = r["errors"].as_array().map_or(0, Vec::len);
    let mut s = format!("Published “{name}”: {p} photo{}", if p == 1 { "" } else { "s" });
    if d > 0 {
        s.push_str(&format!(", {d} removed"));
    }
    if errors > 0 {
        let first = r["errors"][0]["error"].as_str().unwrap_or("");
        s.push_str(&format!(" — {errors} failed: {first}"));
    }
    s
}

/// `dialog.publish {id?}`: publish collection `id` (default: the one on screen) in the background.
pub fn start_publish(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let id = match p.get("id").and_then(Value::as_u64) {
        Some(id) => id,
        None => match app.session.source {
            lightcraft_engine::LibrarySource::Published(c) => c,
            _ => return Err("choose a published collection to publish".into()),
        },
    };
    if app.tasks.is_running(PUBLISHING) {
        return Err("a publish is running already".into());
    }
    // no thread-safe writer (the browser build): publish here and now
    let Some(write) = app.services.write_shared.clone() else {
        let r = app.run("publish.run", json!({"id": id}))?;
        let name = lightcraft_engine::publish::collection(&app.session.catalog, id).map(|c| c.name).unwrap_or_default();
        app.toast_soon(summary(&name, &r), 6.0);
        return Ok(r);
    };
    let job = lightcraft_engine::publish::prepare(&mut app.session, id, None)?;
    let (name, n, gone) = (job.name.clone(), job.exports(), job.removals());
    let short = job_name_short(&name);
    if n == 0 && gone == 0 {
        app.toast_soon(format!("“{name}” is up to date"), 4.0);
        return Ok(json!({"published": 0, "removed": 0}));
    }
    crate::tasks::spawn(
        app,
        PUBLISHING,
        move || job.run(&mut |path, bytes| write(path, bytes), &mut |_, _| true),
        move |app, _ctx, out| match lightcraft_engine::publish::finish(&mut app.session, out) {
            Ok(r) => app.toast_soon(summary(&name, &r), 6.0),
            Err(e) => app.toast_soon(e.to_string(), 6.0),
        },
    )?;
    app.toast_soon(format!("Publishing “{}”… {n} photo{}", short, if n == 1 { "" } else { "s" }), 3.0);
    Ok(json!({"background": true, "photos": n, "removals": gone}))
}

fn job_name_short(n: &str) -> String {
    if n.chars().count() > 40 { format!("{}…", n.chars().take(39).collect::<String>()) } else { n.to_string() }
}

/// `dialog.exportToPhotos {album?}`: without `album`, ask for one (empty = none); then export the
/// selection to Apple Photos in the background.
pub fn start_photos(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let Some(album) = p.get("album").and_then(Value::as_str).map(str::to_string) else {
        crate::panels::dialogs::prompt(
            app,
            "Export to Photos",
            "Album in Photos (empty: none)",
            "LightCraft",
            "dialog.exportToPhotos",
            json!({}),
            "album",
        );
        return Ok(Value::Null);
    };
    if !lightcraft_engine::apple_photos::available() {
        return Err("Export to Photos needs macOS (Apple Photos)".into());
    }
    if app.tasks.is_running(TO_PHOTOS) {
        return Err("an export to Photos is running already".into());
    }
    let Some(write) = app.services.write_shared.clone() else { return Err("Export to Photos needs the desktop app".into()) };
    let ids = app.session.targets(&json!({}));
    let job = lightcraft_engine::apple_photos::prepare(&mut app.session, &ids, &album, p)?;
    let n = job.len();
    crate::tasks::spawn(
        app,
        TO_PHOTOS,
        move || job.run(&mut |path, bytes| write(path, bytes), &lightcraft_engine::apple_photos::run_osascript),
        move |app, _ctx, r| match r {
            Ok(v) => {
                let k = v["imported"].as_u64().unwrap_or(0);
                let to = v["album"].as_str().filter(|a| !a.is_empty()).map(|a| format!(" (album “{a}”)")).unwrap_or_default();
                app.toast_soon(format!("Added {k} photo{} to Photos{to}", if k == 1 { "" } else { "s" }), 6.0);
            }
            Err(e) => app.toast_soon(e, 8.0),
        },
    )?;
    app.toast_soon(format!("Exporting {n} photo{} to Photos…", if n == 1 { "" } else { "s" }), 3.0);
    Ok(json!({"background": true, "photos": n}))
}

/// Create a service of `kind` (Hard Drive asks for its folder).
pub fn create_service(app: &mut LightcraftApp, kind: &str) -> Result<Value, String> {
    let mut p = json!({"kind": kind});
    if kind == "hardDrive" {
        let Some(dir) = app.services.pick_folder.as_mut().and_then(|f| f()) else { return Ok(Value::Null) };
        let name = std::path::Path::new(&dir).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Hard Drive".into());
        p["dir"] = json!(dir);
        p["name"] = json!(unique_name(app, &name));
    } else {
        p["name"] = json!(unique_name(app, "Apple Photos"));
    }
    app.run("publish.createService", p)
}

fn unique_name(app: &LightcraftApp, base: &str) -> String {
    let taken: Vec<String> = lightcraft_engine::publish::services(&app.session.catalog).into_iter().map(|s| s.name.to_lowercase()).collect();
    (1..1000)
        .map(|i| if i == 1 { base.to_string() } else { format!("{base} {i}") })
        .find(|n| !taken.contains(&n.to_lowercase()))
        .unwrap_or_else(|| base.to_string())
}
