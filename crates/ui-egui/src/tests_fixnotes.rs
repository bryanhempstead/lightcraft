//! LC-UI-FIXNOTES headless, in Bryan's words: "a little notes area that I can click on and add
//! notes to things that aren't working, and then … hit the fix button at the end, and then it
//! sends it to this chat and we fix the things."
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes the panel's screenshot there.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::fixnotes::{FixNotes, Status};
use crate::headless::Headless;

const T: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(120);

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-fixnotes-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The demo library with its notes in `dir` (notes file, screenshots, inbox `dir/inbox`).
fn demo(dir: &Path) -> Headless {
    let png = |img: &lightcraft_raster::Rgba8| {
        lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
    };
    let services = crate::Services {
        png: Some(Box::new(png)),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.view = crate::state::ViewMode::PhotoGrid;
    let (notes, notice) = FixNotes::at(Some(dir), Some(dir.join("inbox")));
    assert!(notice.is_none(), "{notice:?}");
    app.fix_notes = notes;
    Headless::new(app, [1400.0, 900.0], 1.0)
}

fn run(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["result"].clone()
}

/// With `LIGHTCRAFT_TEST_SHOTS=<dir>`: a screenshot of the window there.
fn snap(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        h.settle(SETTLE);
        let p = Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": p.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

fn requests(inbox: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(inbox).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}

/// A note written in Develop records the module, the view, the open panel and the active photo;
/// fix. writes one valid request file into the inbox and marks the notes sent; they survive a
/// reopen; markFixed marks them fixed.
#[test]
fn notes_record_where_they_were_written_and_fix_sends_them() {
    let d = tmp("send");
    let inbox = d.join("inbox");
    let mut h = demo(&d);
    assert!(h.settle(SETTLE));
    run(&mut h, "view.develop", json!({}));
    let active = h.app.session.active().expect("a photo in Develop");
    let r = run(&mut h, "notes.add", json!({"text": "the exposure slider jumps", "shot": false}));
    let id = r["id"].as_u64().unwrap();
    let c = &r["note"]["context"];
    assert_eq!(c["module"], "develop", "{c}");
    assert_eq!(c["view"], "detail", "{c}");
    assert_eq!(c["panel"], "edit", "{c}");
    assert_eq!(c["photo"]["id"], active.0, "{c}");
    assert!(c["photo"]["path"].as_str().unwrap().starts_with("demo:"), "{c}");
    assert!(c["photo"]["fileName"].as_str().is_some_and(|f| !f.is_empty()), "{c}");
    assert_eq!(r["note"]["status"], "open");
    // a failed command shows up among the recent errors of the next note
    assert!(h.request("engine.execute", json!({"command": "notes.edit", "params": {"id": 999, "text": "x"}}), T)["ok"] == false);
    let r2 = run(&mut h, "notes.add", json!({"text": "second\nline two", "shot": false}));
    let id2 = r2["id"].as_u64().unwrap();
    let recent = r2["note"]["context"]["recent"].as_array().unwrap();
    assert!(recent.iter().any(|e| e["text"].as_str().unwrap().contains("no note 999")), "{recent:?}");
    assert_eq!(run(&mut h, "notes.list", json!({}))["open"], 2);

    // fix.
    let s = run(&mut h, "notes.send", json!({}));
    assert_eq!(s["count"], 2, "{s}");
    let files = requests(&inbox);
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().to_string();
    assert!(name.ends_with("-fix.json") && name.len() == "20261122-140500-fix.json".len(), "{name}");
    let req: Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
    assert_eq!(req["kind"], "lightcraft.fixRequest");
    assert_eq!(req["version"], 1);
    assert!(req["app"]["version"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(req["app"]["commit"].as_str().is_some_and(|v| !v.is_empty()));
    let notes = req["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0]["text"], "the exposure slider jumps");
    assert_eq!(notes[0]["context"]["module"], "develop");
    assert_eq!(req["reply"]["command"], "notes.markFixed");
    assert_eq!(req["reply"]["params"]["ids"], json!([id, id2]));
    let listed = run(&mut h, "notes.list", json!({"status": "sent"}));
    assert_eq!(listed["notes"].as_array().unwrap().len(), 2, "{listed}");
    assert_eq!(listed["open"], 0);
    assert!(h.app.fix_notes.message.as_ref().is_some_and(|(m, err)| !err && m.contains("Claude will pick it up")));
    // nothing more to send
    assert!(h.request("engine.execute", json!({"command": "notes.send", "params": {}}), T)["ok"] == false);

    // they survive a reopen
    let (again, notice) = FixNotes::at(Some(&d), Some(inbox.clone()));
    assert!(notice.is_none());
    assert_eq!(again.store.notes.len(), 2);
    assert!(again.store.notes.iter().all(|n| n.status == Status::Sent && n.request.as_deref() == Some(files[0].to_string_lossy().as_ref())));

    // Claude fixed the first one
    run(&mut h, "notes.markFixed", json!({"ids": [id], "note": "debounced the slider"}));
    let (again, _) = FixNotes::at(Some(&d), Some(inbox));
    let n = again.note(id).unwrap();
    assert_eq!(n.status, Status::Fixed);
    assert_eq!(n.fix_note.as_deref(), Some("debounced the slider"));
    assert_eq!(again.note(id2).unwrap().status, Status::Sent);
    let _ = std::fs::remove_dir_all(&d);
}

/// An inbox that can't be written (here: a file where the folder should be) is an error; the
/// notes stay open, nothing is half-written.
#[test]
fn an_unwritable_inbox_leaves_the_notes_unsent() {
    let d = tmp("unwritable");
    let mut h = demo(&d);
    std::fs::write(d.join("inbox"), b"not a folder").unwrap();
    run(&mut h, "notes.add", json!({"text": "crop handles vanish", "shot": false}));
    let r = h.request("engine.execute", json!({"command": "notes.send", "params": {}}), T);
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap().contains("stay unsent"), "{r}");
    assert_eq!(h.app.fix_notes.count(Status::Open), 1);
    assert!(h.app.fix_notes.message.as_ref().is_some_and(|(_, err)| *err));
    // no inbox at all (a memory-only session): an error too
    h.app.fix_notes.inbox = None;
    assert_eq!(h.request("engine.execute", json!({"command": "notes.send", "params": {}}), T)["ok"], false);
    assert_eq!(h.app.fix_notes.count(Status::Open), 1);
    let _ = std::fs::remove_dir_all(&d);
}

/// The panel end to end: notes. in the top bar (with the open count) opens it, Enter adds what
/// was typed (with a screenshot), ⌘-click / ⇧-click / all. / none. pick, fix. sends the picked
/// ones; ⌘⌥⇧N closes and opens it.
#[test]
fn the_panel_adds_picks_and_sends() {
    let d = tmp("panel");
    let inbox = d.join("inbox");
    let mut h = demo(&d);
    assert!(h.settle(SETTLE));
    assert!(!h.app.fix_notes.open);
    let r = h.request("ui.clickWidget", json!({"id": "button:fixNotes"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    assert!(h.app.fix_notes.open, "notes. opens the panel");
    assert!(h.app.widgets.iter().any(|(w, _)| w == "panel:fixNotes"));
    snap(&mut h, "fix-notes-empty.png");
    // type a note, Enter adds it (with a screenshot of the window)
    for text in ["first: the grid flickers", "second: map pins drift", "third: export hangs"] {
        assert_eq!(h.request("ui.clickWidget", json!({"id": "field:fixNote"}), T)["ok"], true);
        assert_eq!(h.request("ui.text", json!({"text": text}), T)["ok"], true);
        assert_eq!(h.request("ui.key", json!({"key": "Enter"}), T)["ok"], true);
        assert!(h.step_until(SETTLE, |h| h.app.fix_notes.pending.is_empty()), "the screenshot is taken");
        h.step();
    }
    let notes = &h.app.fix_notes.store.notes;
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert_eq!(notes[0].text, "first: the grid flickers");
    assert!(h.app.fix_notes.draft.is_empty(), "the field is cleared");
    let shot = notes[0].screenshot.clone().expect("a screenshot with the note");
    assert!(std::fs::metadata(&shot).unwrap().len() > 1000, "{shot}");
    // ⇧Enter is a new line, not a new note
    assert_eq!(h.request("ui.clickWidget", json!({"id": "field:fixNote"}), T)["ok"], true);
    assert_eq!(h.request("ui.text", json!({"text": "draft"}), T)["ok"], true);
    assert_eq!(h.request("ui.key", json!({"key": "Enter", "shift": true}), T)["ok"], true);
    assert_eq!(h.app.fix_notes.store.notes.len(), 3);
    assert_eq!(h.app.fix_notes.draft, "draft\n");
    h.app.fix_notes.draft.clear();
    h.app.fix_notes.shot = false;
    h.settle(SETTLE);
    let ids: Vec<u64> = h.app.fix_notes.store.notes.iter().map(|n| n.id).collect();
    // click the newest, ⇧-click the oldest: all three; ⌘-click the middle one off
    let row = |id: u64| format!("row:fixNote-{id}");
    assert_eq!(h.request("ui.clickWidget", json!({"id": row(ids[2]), "fx": 0.6}), T)["ok"], true);
    assert_eq!(h.request("ui.clickWidget", json!({"id": row(ids[0]), "fx": 0.6, "shift": true}), T)["ok"], true);
    assert_eq!(h.app.fix_notes.picked.len(), 3);
    assert_eq!(h.request("ui.clickWidget", json!({"id": row(ids[1]), "fx": 0.6, "cmd": true}), T)["ok"], true);
    assert_eq!(h.app.fix_notes.picked.iter().copied().collect::<Vec<_>>(), vec![ids[0], ids[2]]);
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:fixNotesNone"}), T)["ok"], true);
    assert!(h.app.fix_notes.picked.is_empty());
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:fixNotesAll"}), T)["ok"], true);
    assert_eq!(h.app.fix_notes.picked.len(), 3);
    assert_eq!(h.request("ui.clickWidget", json!({"id": row(ids[1]), "fx": 0.6}), T)["ok"], true);
    assert_eq!(h.app.fix_notes.picked.iter().copied().collect::<Vec<_>>(), vec![ids[1]]);
    // ed. on the oldest note: typed text goes into it, Enter saves
    assert_eq!(h.request("ui.clickWidget", json!({"id": format!("button:fixNoteEdit-{}", ids[0])}), T)["ok"], true);
    h.step();
    assert!(h.app.fix_notes.editing.as_ref().is_some_and(|(i, _)| *i == ids[0]));
    snap(&mut h, "fix-notes-edit.png");
    assert_eq!(h.request("ui.text", json!({"text": " (again)"}), T)["ok"], true);
    assert_eq!(h.request("ui.key", json!({"key": "Enter"}), T)["ok"], true);
    h.step();
    assert!(h.app.fix_notes.editing.is_none());
    let edited = &h.app.fix_notes.note(ids[0]).unwrap().text;
    assert!(edited.contains("(again)") && edited.contains("grid flickers"), "{edited}");
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        h.settle(SETTLE);
        let p = Path::new(&dir).join("fix-notes-panel.png");
        let r = h.request("ui.screenshot", json!({"path": p.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
    // fix. sends the picked open note only
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:fixNotesSend"}), T)["ok"], true);
    let files = requests(&inbox);
    assert_eq!(files.len(), 1, "{files:?}");
    let req: Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
    assert_eq!(req["notes"].as_array().unwrap().len(), 1);
    assert_eq!(req["notes"][0]["text"], "second: map pins drift");
    assert_eq!(h.app.fix_notes.note(ids[1]).unwrap().status, Status::Sent);
    assert_eq!(h.app.fix_notes.count(Status::Open), 2);
    assert!(h.app.ui.toast.as_ref().is_some_and(|t| t.0.contains("Claude will pick it up")), "{:?}", h.app.ui.toast);
    // the first note's screenshot went out with it when sent: the path is in the request
    h.settle(SETTLE);
    let s = run(&mut h, "notes.send", json!({}));
    assert_eq!(s["count"], 2);
    let req: Value = serde_json::from_slice(&std::fs::read(s["path"].as_str().unwrap()).unwrap()).unwrap();
    assert!(req["notes"].as_array().unwrap().iter().any(|n| n["screenshot"].as_str() == Some(shot.as_str())), "{req}");
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        h.settle(SETTLE);
        let p = Path::new(&dir).join("fix-notes-sent.png");
        let r = h.request("ui.screenshot", json!({"path": p.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
    // ⌘⌥⇧N closes and opens it
    assert_eq!(h.request("ui.key", json!({"key": "N", "cmd": true, "alt": true, "shift": true}), T)["ok"], true);
    h.step();
    assert!(!h.app.fix_notes.open, "⌘⌥⇧N closes the panel");
    assert_eq!(h.request("ui.key", json!({"key": "N", "cmd": true, "alt": true, "shift": true}), T)["ok"], true);
    h.step();
    assert!(h.app.fix_notes.open, "⌘⌥⇧N opens it");
    let _ = std::fs::remove_dir_all(&d);
}

/// Edit and remove through the commands (the panel's ed. and ✕ call them).
#[test]
fn notes_are_edited_removed_and_reopened() {
    let d = tmp("edit");
    let mut h = demo(&d);
    let a = run(&mut h, "notes.add", json!({"text": "one", "shot": false}))["id"].as_u64().unwrap();
    let b = run(&mut h, "notes.add", json!({"text": "two", "shot": false}))["id"].as_u64().unwrap();
    run(&mut h, "notes.edit", json!({"id": a, "text": "one, clearer"}));
    assert_eq!(h.app.fix_notes.note(a).unwrap().text, "one, clearer");
    assert_eq!(h.request("engine.execute", json!({"command": "notes.add", "params": {"text": "   "}}), T)["ok"], false, "empty notes are refused");
    run(&mut h, "notes.markFixed", json!({"ids": [b]}));
    run(&mut h, "notes.reopen", json!({"ids": [b]}));
    assert_eq!(h.app.fix_notes.note(b).unwrap().status, Status::Open);
    assert_eq!(run(&mut h, "notes.remove", json!({"ids": [a]}))["removed"], 1);
    let (again, _) = FixNotes::at(Some(&d), None);
    assert_eq!(again.store.notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), vec!["two"]);
    // ids keep counting up after a reopen
    assert!(again.store.next_id > b);
    let _ = std::fs::remove_dir_all(&d);
}

/// Esc closes the notes. panel when it has the keyboard (its field focused, or the panel last
/// clicked) — and only the panel: the loupe stays. Esc with the focus elsewhere leaves it open,
/// and while a note is edited Esc cancels the edit.
#[test]
fn esc_closes_the_notes_panel_when_it_has_focus() {
    use crate::state::ViewMode;
    let d = tmp("esc");
    let mut h = demo(&d);
    h.app.ui.view = ViewMode::Detail;
    assert!(h.settle(SETTLE));
    let esc = |h: &mut Headless| {
        assert_eq!(h.request("ui.key", json!({"key": "Escape"}), T)["ok"], true);
        h.step();
    };
    // opened: the field has the keys; Esc closes the panel, the loupe stays
    run(&mut h, "notes.toggle", json!({"open": true}));
    h.step();
    h.step();
    assert!(h.view.ctx.memory(|m| m.has_focus(egui::Id::new("fix-notes-field"))), "the field takes the keys when the panel opens");
    esc(&mut h);
    assert!(!h.app.fix_notes.open, "Esc in the field closes the panel");
    assert_eq!(h.app.ui.view, ViewMode::Detail, "and only the panel");
    // the panel clicked (not its field): Esc closes it too
    run(&mut h, "notes.toggle", json!({"open": true}));
    h.settle(SETTLE);
    let panel = h.app.widgets.iter().find(|(w, _)| w == "panel:fixNotes").map(|(_, r)| *r).expect("the panel");
    let at = egui::pos2(panel.left() + 6.0, panel.bottom() - 6.0);
    assert_eq!(h.request("ui.click", json!({"x": at.x, "y": at.y}), T)["ok"], true);
    h.view.ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("fix-notes-field")));
    h.step();
    esc(&mut h);
    assert!(!h.app.fix_notes.open, "Esc after a click in the panel closes it");
    assert_eq!(h.app.ui.view, ViewMode::Detail);
    // a click elsewhere takes the keyboard back: Esc is the loupe's again
    run(&mut h, "notes.toggle", json!({"open": true}));
    h.settle(SETTLE);
    let canvas = h.app.canvas_rect.expect("the loupe").center();
    h.view.ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("fix-notes-field")));
    assert_eq!(h.request("ui.click", json!({"x": canvas.x, "y": canvas.y}), T)["ok"], true);
    h.settle(SETTLE);
    run(&mut h, "view.zoomFit", json!({}));
    esc(&mut h);
    assert!(h.app.fix_notes.open, "the panel stays open");
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid, "Esc went to the loupe");
    // editing a note: Esc cancels the edit, the panel stays
    let id = run(&mut h, "notes.add", json!({"text": "one", "shot": false}))["id"].as_u64().unwrap();
    h.app.fix_notes.editing = Some((id, "one".into()));
    h.settle(SETTLE);
    esc(&mut h);
    assert!(h.app.fix_notes.open && h.app.fix_notes.editing.is_none(), "Esc cancelled the edit only");
    let _ = std::fs::remove_dir_all(&d);
}
