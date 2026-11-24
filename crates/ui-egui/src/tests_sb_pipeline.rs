//! LC-SB-PIPELINE, headless (Bryan, 2026-11-23): "add an area that marks that it came from the 2nd
//! brain pipeline … and if it's not then I can retroactively put it there … and then it will go
//! through that pipeline and mark or ask if I want it backed up and then put it in the currently
//! editing area".
//!
//! * A shoot in Second Brain's pipeline (a sample `shootpipe.json` in a temp folder, never the real
//!   one) carries a `pipeline.editing` row under its folder in Folders — only on its topmost row —
//!   and Info says "from the second brain pipeline · editing". A stage changed in the file shows.
//! * A folder not in it: `send.to.pipeline.` opens a small dialog (back.up. / already.backed.up. /
//!   skip., cancel. / the accent ok.); Return = ok.; the request lands in Second Brain's inbox and
//!   the folder shows `pipeline.pending`. Nothing clipped or overlapping in the rows or the dialog.
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes screenshots there.

use std::time::Duration;

use lightcraft_catalog::{Op, Photo, PhotoId, Source};
use serde_json::{Value, json};

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

struct Fixture {
    h: Headless,
    root: std::path::PathBuf,
    sb: std::path::PathBuf,
    wedding: String,
    street: String,
    wedding_photo: PhotoId,
    street_photo: PhotoId,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn pipe(wedding: &str, stage: &str) -> Value {
    json!({"v": 1, "clock": null, "bjobs": [], "dismissed": [], "settings": {"backupPlaces": ["/Volumes/BONUSBOY backup"]}, "shoots": [
        {"id": "s1", "path": wedding, "name": "Wedding", "stage": stage, "archived": null,
         "children": [{"path": format!("{wedding}/raw/M262"), "name": "raw/M262"}, {"path": format!("{wedding}/raw/GR3"), "name": "raw/GR3"}]},
    ]})
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("lc-ui-sbpipe-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let shoots = root.join("shoots");
    let (wedding, street) = (shoots.join("Wedding"), shoots.join("Street"));
    for d in [wedding.join("raw/M262"), wedding.join("raw/GR3"), street.clone()] {
        std::fs::create_dir_all(&d).unwrap();
    }
    let sb = root.join("sb");
    std::fs::create_dir_all(&sb).unwrap();
    let (wedding, street) = (wedding.to_string_lossy().to_string(), street.to_string_lossy().to_string());
    std::fs::write(sb.join("shootpipe.json"), serde_json::to_vec(&pipe(&wedding, "edit")).unwrap()).unwrap();
    let mut session = lightcraft_engine::Session::new();
    session.second_brain_dir = Some(sb.clone());
    let mut ids = Vec::new();
    for (i, f) in [format!("{wedding}/raw/M262/a.dng"), format!("{wedding}/raw/GR3/b.dng"), format!("{street}/c.jpg")].iter().enumerate() {
        let id = session.catalog.alloc_photo_id();
        let name = std::path::Path::new(f).file_name().unwrap().to_string_lossy().to_string();
        let p = Photo::new(id, Source::File { path: f.clone() }, &name, "JPEG", 60, 40, &format!("2026-11-0{}T10:00:00", 1 + i));
        session.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
        ids.push(id);
    }
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = LightcraftApp::new(session, services);
    app.fix_notes = crate::fixnotes::FixNotes::default();
    // every folder on the way down open, so the shoot's rows show
    for p in [format!("{wedding}/raw/M262"), street.clone()] {
        for a in std::path::Path::new(&p).ancestors() {
            app.ui.folders_open.insert(lightcraft_catalog::query::folder_key(&a.to_string_lossy()), true);
        }
    }
    let mut h = Headless::new(app, [1400.0, 1500.0], 1.0);
    let r = h.request("ui.set", json!({"view": "photoGrid", "leftPanel": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.app.ui.toggle_sidebar_section("navigator");
    h.app.ui.library_sections = vec!["metadata".into()];
    h.settle(SETTLE);
    Fixture { h, root, sb, wedding, street, wedding_photo: ids[0], street_photo: ids[2] }
}

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn pipeline_rows(h: &Headless) -> Vec<String> {
    h.app.widgets.iter().filter_map(|(w, _)| w.strip_prefix("source:pipeline:")).map(str::to_string).collect()
}

fn shows(h: &Headless, text: &str) -> bool {
    h.view.texts().iter().any(|(s, _, _)| s == text)
}

/// `text` drawn, perhaps wrapped over a few lines.
fn shows_wrapped(h: &Headless, text: &str) -> bool {
    let all: String = h.view.texts().into_iter().map(|(s, _, _)| s).collect::<Vec<_>>().join("").split_whitespace().collect();
    all.contains(&text.split_whitespace().collect::<String>())
}

fn dump(h: &Headless) -> String {
    let texts: Vec<String> = h.view.texts().into_iter().map(|(s, _, _)| s).filter(|s| s.len() < 60).collect();
    let panels: Vec<&String> = h.app.widgets.iter().map(|(w, _)| w).filter(|w| w.starts_with("panel:")).collect();
    format!("active {:?} sections {:?} panels {panels:?} texts {texts:?}", h.app.session.active(), h.app.ui.library_sections)
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        let p = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": p.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

#[test]
fn a_pipeline_shoot_is_tagged_in_folders_and_info_and_follows_its_stage() {
    let mut f = fixture("tag");
    let (wedding, sb, photo) = (f.wedding.clone(), f.sb.clone(), f.wedding_photo);
    let h = &mut f.h;
    assert!(h.step_until(T, |h| !pipeline_rows(h).is_empty()), "a pipeline row shows");
    h.settle(SETTLE);
    // one row, on the shoot's topmost folder row (not on each camera folder inside it)
    let rows = pipeline_rows(h);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(lightcraft_catalog::query::folder_within(&rows[0], &wedding), "{rows:?}");
    let id = format!("pipeline:{}", rows[0]);
    assert!(shows(h, "pipeline.editing"));
    assert!((widget(h, &format!("source:{id}")).height() - 29.0).abs() < 0.5, "one row height");
    let left = widget(h, "panel:left_panel");
    assert!(left.contains_rect(widget(h, &format!("label:{id}"))), "inside the panel");
    let faults = crate::tests_panels::layout_faults(h, left);
    assert!(faults.is_empty(), "{faults:#?}");
    shot(h, "sbpipe-1-folders.png");

    // Info: where the photo's shoot stands
    h.app.run("library.select", json!({"ids": [photo.0]})).unwrap();
    h.settle(SETTLE);
    assert!(h.step_until(T, |h| shows_wrapped(h, "from the second brain pipeline · editing")), "the Info line: {}", dump(h));
    let right = widget(h, "panel:right_panel");
    assert!(right.contains_rect(widget(h, "info:pipeline")), "{right:?} {:?}", widget(h, "info:pipeline"));
    let faults = crate::tests_panels::layout_faults(h, right);
    assert!(faults.is_empty(), "{faults:#?}");
    shot(h, "sbpipe-2-info.png");

    // a stage changed in Second Brain shows after the next read
    std::fs::write(sb.join("shootpipe.json"), serde_json::to_vec(&pipe(&wedding, "dlvr")).unwrap()).unwrap();
    assert!(h.step_until(T, |h| shows(h, "pipeline.delivering")), "the new stage");
    assert!(h.step_until(T, |h| shows_wrapped(h, "from the second brain pipeline · delivering")));
    // a bad file: no tags, no crash
    std::fs::write(sb.join("shootpipe.json"), b"{not json").unwrap();
    assert!(h.step_until(T, |h| pipeline_rows(h).is_empty()), "no tags from a bad file");
}

#[test]
fn send_to_pipeline_asks_about_the_backup_and_queues_it_for_second_brain() {
    let mut f = fixture("send");
    let (street, sb, photo) = (f.street.clone(), f.sb.clone(), f.street_photo);
    let h = &mut f.h;
    h.app.run("library.select", json!({"ids": [photo.0]})).unwrap();
    assert!(
        h.step_until(T, |h| shows_wrapped(h, "not in the second brain pipeline") && has(h, "button:infoSendToPipeline")),
        "Info offers it: {}",
        dump(h)
    );
    h.settle(SETTLE);
    let r = h.request("ui.clickWidget", json!({"id": "button:infoSendToPipeline"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(
        matches!(&h.app.ui.dialog, Some(crate::state::Dialog::SendToPipeline { path, .. }) if *path == street),
        "{:?} {r} {:?} {:?}",
        h.app.ui.dialog,
        widget(h, "panel:right_panel"),
        h.app.widgets.iter().find(|(w, _)| w == "button:infoSendToPipeline")
    );
    // the dialog: three choices, cancel. and the accent ok., 8 px apart, nothing clipped
    let dialog = widget(h, "dialog:window");
    let choices: Vec<egui::Rect> = ["run", "done", "skip"].iter().map(|c| widget(h, &format!("button:pipelineBackup-{c}"))).collect();
    for w in choices.windows(2) {
        assert!(w[1].left() - w[0].right() >= 8.0 - 0.5, "{w:?}");
        assert!((w[1].height() - w[0].height()).abs() < 0.5);
    }
    let (cancel, ok) = (widget(h, "button:dialogCancel"), widget(h, "button:dialogOk"));
    assert!(ok.left() - cancel.right() >= 8.0 - 0.5, "{cancel:?} {ok:?}");
    assert!((ok.height() - cancel.height()).abs() < 0.5, "one height in the row");
    assert!(shows(h, "back.up.") && shows(h, "already.backed.up.") && shows(h, "skip.") && shows(h, "cancel.") && shows(h, "ok."));
    assert!(choices.iter().chain([&cancel, &ok]).all(|r| dialog.contains_rect(*r)));
    let faults = crate::tests_panels::layout_faults(h, dialog);
    assert!(faults.is_empty(), "{faults:#?}");
    shot(h, "sbpipe-3-dialog.png");

    // already.backed.up., then Return = ok.
    let r = h.request("ui.clickWidget", json!({"id": "button:pipelineBackup-done"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    let r = h.request("ui.key", json!({"key": "Enter"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(h.app.ui.dialog.is_none(), "closed");
    // the request waits in Second Brain's inbox
    let inbox = sb.join(lightcraft_engine::sb_pipeline::INBOX);
    let files: Vec<_> = std::fs::read_dir(&inbox).unwrap().flatten().map(|e| e.path()).collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let req: Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
    assert_eq!((req["path"].as_str(), req["backup"].as_str(), req["stage"].as_str()), (Some(street.as_str()), Some("done"), Some("edit")));
    // and the folder shows it as pending until Second Brain takes it
    assert!(h.step_until(T, |h| shows(h, "pipeline.pending")), "pending row");
    assert!(h.step_until(T, |h| shows_wrapped(h, "sent to the second brain pipeline · waiting for second brain")));
    let faults = crate::tests_panels::layout_faults(h, widget(h, "panel:left_panel"));
    assert!(faults.is_empty(), "{faults:#?}");
    shot(h, "sbpipe-4-pending.png");
}
