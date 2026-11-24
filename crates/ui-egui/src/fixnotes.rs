//! LC-UI-FIXNOTES: notes about what isn't working, sent to the Claude Code session that builds
//! LightCraft.
//!
//! The top bar's `notes.` button (⌘⌥⇧N, `notes.toggle`) opens a small floating panel that stays
//! open while you work. Each note records where it was written — module, view, open tool or panel,
//! the active photo (id, file, camera), the selection, the last warnings and errors — and, with
//! `shot.` on, a screenshot of the window (taken through the `ui.screenshot` path, the panel
//! hidden). `fix.` sends the picked open notes (or every open one): one JSON request file in the
//! fix inbox, which the Claude session watches; the notes are marked sent. After fixing, the
//! session calls `notes.markFixed {ids}` over the control port. The request format is documented
//! in `docs/fix-notes.md`.
//!
//! Files (never in the repo, never in the photo library), under LightCraft's settings folder
//! (`~/Library/Application Support/LightCraft/` on macOS):
//! - `fix-notes.json` — the notes, kept across restarts;
//! - `fix-notes-shots/` — their screenshots;
//! - `fix-inbox/<YYYYMMDD-HHMMSS>-fix.json` — the requests (UTC time; `LIGHTCRAFT_FIX_INBOX` names
//!   another folder). Written as `….json.tmp`, then renamed, so a watcher never reads half a file.
//!
//! Without a settings folder (`--memory`, `LIGHTCRAFT_NO_PREFS`, tests) notes live in memory.
//! Never crashes: a damaged notes file is set aside and reported, a full disk or an unwritable
//! inbox is an error notice and the notes stay unsent.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::{register, text_button};

/// The notes file, in the settings folder.
pub const FILE: &str = "fix-notes.json";
/// The request folder, in the settings folder.
pub const INBOX: &str = "fix-inbox";
/// The screenshots folder, in the settings folder.
pub const SHOTS: &str = "fix-notes-shots";
/// Names another request folder (tests, a different watcher).
pub const INBOX_ENV: &str = "LIGHTCRAFT_FIX_INBOX";
/// The request file format's version (`docs/fix-notes.md`).
pub const REQUEST_VERSION: u32 = 1;
/// Notes kept at most (the oldest fixed ones go first).
const MAX_NOTES: usize = 2000;
/// A note's text is cut at this many characters.
const MAX_TEXT: usize = 20_000;
/// Warnings and errors recorded with a note.
const RECENT_N: usize = 8;
/// The panel's width (points).
const W: f32 = 360.0;
/// The notes list's fixed height (it scrolls inside, never grows with the pile).
const LIST_H: f32 = 250.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    #[default]
    Open,
    Sent,
    Fixed,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Open => "open.",
            Status::Sent => "sent.",
            Status::Fixed => "fixed.",
        }
    }
}

/// One note.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Note {
    pub id: u64,
    pub text: String,
    /// Unix seconds.
    pub at: i64,
    /// `at` as ISO 8601 UTC (`2026-11-22T14:05:00Z`).
    pub time: String,
    pub status: Status,
    /// Where it was written ([`context`]).
    pub context: Value,
    /// The screenshot's path, once saved.
    pub screenshot: Option<String>,
    /// The request file it went out in.
    pub request: Option<String>,
    pub sent_at: Option<i64>,
    pub fixed_at: Option<i64>,
    /// What the fixer said it did (`notes.markFixed {note}`).
    pub fix_note: Option<String>,
}

/// `fix-notes.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Store {
    pub version: u32,
    pub next_id: u64,
    pub notes: Vec<Note>,
}

/// A screenshot being taken for a note.
pub struct Shot {
    id: u64,
    path: String,
    rx: Receiver<Value>,
}

/// The notes and the panel's state.
pub struct FixNotes {
    /// `fix-notes.json` (`None`: in memory).
    pub file: Option<PathBuf>,
    /// Where requests go (`None`: `fix.` says there is no inbox).
    pub inbox: Option<PathBuf>,
    /// Where screenshots go (`None`: no screenshots).
    pub shots: Option<PathBuf>,
    /// The app's log file, named in requests.
    pub log_file: Option<PathBuf>,
    pub store: Store,
    /// The panel is shown.
    pub open: bool,
    /// The note being typed.
    pub draft: String,
    /// New notes take a screenshot.
    pub shot: bool,
    /// Picked notes (click / ⌘-click / ⇧-click, all. / none.).
    pub picked: BTreeSet<u64>,
    anchor: Option<u64>,
    /// A note being edited: its id and the text so far.
    pub editing: Option<(u64, String)>,
    /// Screenshots in progress (the panel hides until they're taken).
    pub pending: Vec<Shot>,
    /// Errors shown to the user lately (toasts, failed commands), for the notes' context.
    pub recent: VecDeque<(i64, String)>,
    /// The panel's last message and whether it is an error ("sent — Claude will pick it up").
    pub message: Option<(String, bool)>,
    /// Give the note field the keyboard focus when the panel is drawn next (after a screenshot
    /// hid it).
    refocus: bool,
    /// The panel was the last thing clicked (Esc then closes it, [`has_focus`]).
    clicked_in: bool,
    /// The note field had the keyboard when the panel was last drawn (egui lets go of the focus
    /// as soon as Esc is pressed, before the shortcuts look).
    field_focused: bool,
}

impl Default for FixNotes {
    fn default() -> Self {
        FixNotes {
            file: None,
            inbox: None,
            shots: None,
            log_file: None,
            store: Store { version: 1, next_id: 1, notes: vec![] },
            open: false,
            draft: String::new(),
            shot: true,
            picked: BTreeSet::new(),
            anchor: None,
            editing: None,
            pending: vec![],
            recent: VecDeque::new(),
            message: None,
            refocus: false,
            clicked_in: false,
            field_focused: false,
        }
    }
}

fn now() -> i64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// This process's id (none in the browser, where std would panic).
#[cfg(not(target_arch = "wasm32"))]
fn pid() -> u32 {
    std::process::id()
}
#[cfg(target_arch = "wasm32")]
fn pid() -> u32 {
    0
}

/// `2026-11-22T14:05:00Z`.
pub fn iso(secs: i64) -> String {
    format!("{}Z", lightcraft_catalog::dates::civil(secs))
}

/// `20261122-140500` (UTC), the request file's name stem.
pub fn stamp(secs: i64) -> String {
    lightcraft_catalog::dates::civil(secs).chars().filter_map(|c| if c == 'T' { Some('-') } else { c.is_ascii_digit().then_some(c) }).collect()
}

/// Write `bytes` to `path` through `<path>.tmp` and a rename (a reader never sees half a file).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("can't write {}: {e}", path.display()));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("can't write {}: {e}", path.display())
    })
}

impl FixNotes {
    /// The notes in `config` (`fix-notes.json`, `fix-notes-shots/`, `fix-inbox/`), loaded;
    /// `inbox` (from [`INBOX_ENV`]) wins over the default inbox. Also returns a notice for the
    /// user when the notes file couldn't be used.
    pub fn at(config: Option<&Path>, inbox: Option<PathBuf>) -> (FixNotes, Option<String>) {
        let mut n = FixNotes {
            file: config.map(|c| c.join(FILE)),
            shots: config.map(|c| c.join(SHOTS)),
            inbox: inbox.or_else(|| config.map(|c| c.join(INBOX))),
            ..Default::default()
        };
        let notice = n.load();
        (n, notice)
    }

    /// The desktop app's notes: in LightCraft's settings folder unless `in_memory`; the inbox
    /// from [`INBOX_ENV`] if set.
    pub fn for_app(in_memory: bool) -> (FixNotes, Option<String>) {
        let inbox = std::env::var_os(INBOX_ENV).filter(|v| !v.is_empty()).map(PathBuf::from);
        let config = if in_memory { None } else { lightcraft_engine::camera_profiles::config_dir() };
        Self::at(config.as_deref(), inbox)
    }

    /// Read [`Self::file`]. A damaged file is renamed `fix-notes.json.damaged-<unix time>` (never
    /// lost) and the notes start empty; one that can't be read keeps the notes in memory this
    /// session. Returns the notice for the user.
    pub fn load(&mut self) -> Option<String> {
        let path = self.file.clone()?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                self.file = None;
                return Some(crate::i18n::tr_format!(
                    "The fix notes in {} can't be read ({e}). New notes are kept until LightCraft quits.",
                    path.display(),
                    e = e
                ));
            }
        };
        match serde_json::from_slice::<Store>(&bytes) {
            Ok(mut s) => {
                s.version = s.version.max(1);
                let top = s.notes.iter().map(|n| n.id).max().unwrap_or(0);
                s.next_id = s.next_id.max(top.saturating_add(1)).max(1);
                self.store = s;
                None
            }
            Err(e) => {
                let mut aside = path.as_os_str().to_os_string();
                aside.push(format!(".damaged-{}", now()));
                let aside = PathBuf::from(aside);
                let kept = std::fs::rename(&path, &aside).is_ok();
                if !kept {
                    // can't move it: don't write over it either
                    self.file = None;
                }
                Some(if kept {
                    crate::i18n::tr_format!(
                        "The fix notes file was damaged ({e}); it was kept as {} and new notes start empty.",
                        aside.display(),
                        e = e
                    )
                } else {
                    crate::i18n::tr_format!(
                        "The fix notes file {} is damaged ({e}). New notes are kept until LightCraft quits.",
                        path.display(),
                        e = e
                    )
                })
            }
        }
    }

    /// Write [`Self::file`] (nothing in memory-only sessions).
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.file else { return Ok(()) };
        let bytes = serde_json::to_vec_pretty(&self.store).map_err(|e| format!("fix notes: {e}"))?;
        write_atomic(path, &bytes).map_err(|e| format!("The fix notes weren't saved: {e}"))
    }

    pub fn note(&self, id: u64) -> Option<&Note> {
        self.store.notes.iter().find(|n| n.id == id)
    }

    fn note_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.store.notes.iter_mut().find(|n| n.id == id)
    }

    pub fn count(&self, status: Status) -> usize {
        self.store.notes.iter().filter(|n| n.status == status).count()
    }

    /// Remember an error the user was shown (for the next notes' context).
    pub fn saw_error(&mut self, text: &str) {
        let text: String = text.chars().take(600).collect();
        if self.recent.back().is_some_and(|(_, t)| *t == text) {
            return;
        }
        self.recent.push_back((now(), text));
        while self.recent.len() > RECENT_N * 2 {
            self.recent.pop_front();
        }
    }

    /// The notes in the order the panel lists them (newest first).
    fn listed(&self) -> Vec<u64> {
        self.store.notes.iter().rev().map(|n| n.id).collect()
    }

    /// Click (just this one), ⌘-click (add / remove it), ⇧-click (the run from the last click).
    pub fn pick(&mut self, id: u64, command: bool, shift: bool) {
        let order = self.listed();
        match (shift, self.anchor.and_then(|a| order.iter().position(|x| *x == a)), order.iter().position(|x| *x == id)) {
            (true, Some(a), Some(b)) => {
                let (lo, hi) = (a.min(b), a.max(b));
                if !command {
                    self.picked.clear();
                }
                self.picked.extend(order.get(lo..=hi).unwrap_or_default().iter().copied());
                return;
            }
            _ if command => {
                if !self.picked.remove(&id) {
                    self.picked.insert(id);
                }
            }
            _ => {
                self.picked.clear();
                self.picked.insert(id);
            }
        }
        self.anchor = Some(id);
    }

    /// Notes `fix.` sends: the picked open ones, else every open one.
    pub fn to_send(&self) -> Vec<u64> {
        let open = |n: &&Note| n.status == Status::Open;
        let picked: Vec<u64> = self.store.notes.iter().filter(open).filter(|n| self.picked.contains(&n.id)).map(|n| n.id).collect();
        if picked.is_empty() { self.store.notes.iter().filter(open).map(|n| n.id).collect() } else { picked }
    }
}

/// Where the user is: module, view, open tool / panel / dialog, the active photo, the selection,
/// the source, the library, and the last warnings and errors.
pub fn context(app: &LightcraftApp) -> Value {
    let name = |v: Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
    let photo = app.session.active().and_then(|id| app.session.catalog.photo(id)).map(|p| {
        let path = match &p.source {
            lightcraft_catalog::Source::File { path } => path.clone(),
            lightcraft_catalog::Source::Demo { scene } => format!("demo:{scene}"),
        };
        json!({
            "id": p.id.0,
            "fileName": p.file_name,
            "path": path,
            "format": p.format,
            "camera": p.meta.camera,
            "lens": p.meta.lens,
            "size": [p.width, p.height],
            "captured": p.captured,
            "edited": p.is_edited(),
            "previewOnly": p.preview_only,
        })
    });
    let dialog = app.ui.dialog.as_ref().map(|d| format!("{d:?}").chars().take_while(|c| c.is_alphanumeric()).collect::<String>());
    // the logger's warnings and errors, and what the user was shown, oldest first
    let mut recent: Vec<(i64, String)> = lightcraft_engine::logging::recent(RECENT_N);
    recent.extend(app.fix_notes.recent.iter().cloned());
    recent.sort_by_key(|(at, _)| *at);
    let recent: Vec<Value> = recent.iter().skip(recent.len().saturating_sub(RECENT_N)).map(|(at, t)| json!({"time": iso(*at), "text": t})).collect();
    json!({
        "module": name(serde_json::to_value(app.ui.module).unwrap_or(Value::Null)),
        "view": name(serde_json::to_value(app.ui.view).unwrap_or(Value::Null)),
        // the right panel (in Library only when it isn't a Develop tool left selected there)
        "panel": (app.ui.module == crate::state::Module::Develop || !app.ui.right.is_edit_tool())
            .then(|| name(serde_json::to_value(app.ui.right).unwrap_or(Value::Null))),
        "dialog": dialog,
        "photo": photo,
        "selected": app.session.selection.ids.len(),
        "source": crate::leftoff::source_key(app),
        "library": app.session.library.as_ref().map(|l| l.dir.display().to_string()),
        "status": (!app.ui.status.is_empty()).then(|| app.ui.status.clone()),
        "notices": app.notices,
        "recent": recent,
        "window": app.canvas_rect.map(|r| [r.width().round(), r.height().round()]),
    })
}

fn ids_param(p: &Value) -> Result<Option<Vec<u64>>, String> {
    match p.get("ids").or_else(|| p.get("id")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(|i| Some(vec![i])).ok_or_else(|| "`ids` must be note ids".to_string()),
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_u64().ok_or_else(|| "`ids` must be note ids".to_string())).collect::<Result<Vec<_>, _>>().map(Some)
        }
        Some(_) => Err("`ids` must be a list of note ids".into()),
    }
}

/// Save, or tell the user (the panel and a log line) why not. Returns the error.
fn persist(app: &mut LightcraftApp) -> Option<String> {
    match app.fix_notes.save() {
        Ok(()) => None,
        Err(e) => {
            log::warn!("{e}");
            app.fix_notes.message = Some((e.clone(), true));
            Some(e)
        }
    }
}

/// `notes.add {text, shot?}`: a note with the context of now; `shot` (default: the panel's
/// `shot.` toggle) also takes a screenshot of the window.
pub fn add(app: &mut LightcraftApp, text: &str, shot: Option<bool>) -> Result<Value, String> {
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        return Err("notes.add: the note is empty".into());
    }
    let n = &mut app.fix_notes;
    if n.store.notes.len() >= MAX_NOTES {
        // the oldest fixed notes make room
        if let Some(i) = n.store.notes.iter().position(|x| x.status == Status::Fixed) {
            n.store.notes.remove(i);
        } else {
            return Err(format!("notes.add: {MAX_NOTES} notes already; remove or send some first"));
        }
    }
    let context = context(app);
    let n = &mut app.fix_notes;
    let id = n.store.next_id.max(1);
    n.store.next_id = id.saturating_add(1);
    let at = now();
    n.store.notes.push(Note { id, text, at, time: iso(at), status: Status::Open, context, ..Default::default() });
    let shot = shot.unwrap_or(n.shot);
    let shot_path = match (&n.shots, shot) {
        (Some(dir), true) => Some(dir.join(format!("{}-note{id}.png", stamp(at))).to_string_lossy().to_string()),
        _ => None,
    };
    if let Some(path) = &shot_path {
        let made = n.shots.as_ref().map(std::fs::create_dir_all);
        match made {
            Some(Err(e)) => {
                if let Some(note) = n.note_mut(id) {
                    note.context["screenshotError"] = json!(format!("can't create the screenshots folder: {e}"));
                }
            }
            _ => {
                let rx = app.queue_screenshot(path.clone());
                app.fix_notes.pending.push(Shot { id, path: path.clone(), rx });
            }
        }
    }
    app.fix_notes.message = None;
    let save_error = persist(app);
    let mut out = json!({"id": id, "note": app.fix_notes.note(id), "open": app.fix_notes.count(Status::Open)});
    if let Some(p) = shot_path {
        out["screenshot"] = json!(p);
    }
    if let Some(e) = save_error {
        out["saveError"] = json!(e);
    }
    Ok(out)
}

/// Screenshots taken since the last frame go with their notes.
pub fn poll(app: &mut LightcraftApp) {
    if app.fix_notes.pending.is_empty() {
        return;
    }
    let mut done = false;
    let mut pending = std::mem::take(&mut app.fix_notes.pending);
    pending.retain(|s| match s.rx.try_recv() {
        Ok(v) => {
            if let Some(note) = app.fix_notes.note_mut(s.id) {
                if v["ok"] == true {
                    note.screenshot = Some(s.path.clone());
                } else {
                    let e = v["error"].as_str().unwrap_or("no screenshot");
                    note.context["screenshotError"] = json!(e);
                }
            }
            done = true;
            false
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => true,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
    });
    app.fix_notes.pending = pending;
    if done {
        persist(app);
    }
}

/// The request file for `ids` (`docs/fix-notes.md`).
pub fn request(app: &LightcraftApp, ids: &[u64], at: i64) -> Value {
    let n = &app.fix_notes;
    let notes: Vec<&Note> = ids.iter().filter_map(|id| n.note(*id)).collect();
    json!({
        "kind": "lightcraft.fixRequest",
        "version": REQUEST_VERSION,
        "created": iso(at),
        "at": at,
        "app": {
            "version": env!("CARGO_PKG_VERSION"),
            "commit": option_env!("LIGHTCRAFT_GIT_COMMIT").unwrap_or("unknown"),
            "os": std::env::consts::OS,
            "pid": pid(),
            "logFile": n.log_file.as_ref().map(|p| p.display().to_string()),
        },
        "library": app.session.library.as_ref().map(|l| l.dir.display().to_string()),
        "notesFile": n.file.as_ref().map(|p| p.display().to_string()),
        "notes": notes,
        "reply": {"command": "notes.markFixed", "params": {"ids": ids}},
    })
}

/// `notes.send {ids?}`: one request file in the inbox with the given notes (open or sent), else
/// the picked open ones, else every open one; they're marked sent. A write that fails leaves
/// them as they were.
pub fn send(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let ids = match ids_param(p)? {
        Some(ids) => {
            let mut out = vec![];
            for id in ids {
                match app.fix_notes.note(id) {
                    None => return Err(format!("notes.send: no note {id}")),
                    Some(n) if n.status == Status::Fixed => return Err(format!("notes.send: note {id} is fixed already")),
                    Some(_) if out.contains(&id) => {}
                    Some(_) => out.push(id),
                }
            }
            out
        }
        None => app.fix_notes.to_send(),
    };
    if ids.is_empty() {
        return Err("notes.send: no open notes to send".into());
    }
    let Some(inbox) = app.fix_notes.inbox.clone() else {
        return Err("notes.send: no fix inbox folder (LightCraft runs without a settings folder); the notes stay unsent".into());
    };
    let at = now();
    let body = serde_json::to_vec_pretty(&request(app, &ids, at)).map_err(|e| format!("notes.send: {e}"))?;
    // two sends in one second don't overwrite each other
    let stem = stamp(at);
    let path = (1..1000u32)
        .map(|i| inbox.join(if i == 1 { format!("{stem}-fix.json") } else { format!("{stem}-{i}-fix.json") }))
        .find(|p| !p.exists())
        .ok_or_else(|| format!("notes.send: too many requests named {stem} in {}", inbox.display()))?;
    if let Err(e) = write_atomic(&path, &body) {
        let e = crate::i18n::tr_format!("Couldn't send the notes: {e}. They stay unsent.", e = e);
        app.fix_notes.message = Some((e.clone(), true));
        return Err(e);
    }
    let file = path.display().to_string();
    for id in &ids {
        if let Some(n) = app.fix_notes.note_mut(*id) {
            n.status = Status::Sent;
            n.sent_at = Some(at);
            n.request = Some(file.clone());
        }
    }
    app.fix_notes.picked.retain(|id| !ids.contains(id));
    let what = if ids.len() == 1 { crate::i18n::tr("1 note").to_string() } else { crate::i18n::tr_format!("{n} notes", n = ids.len()) };
    app.fix_notes.message = Some((crate::i18n::tr_format!("sent {what} — Claude will pick it up", what = what), false));
    let save_error = persist(app);
    let mut out = json!({"path": file, "ids": ids, "count": ids.len()});
    if let Some(e) = save_error {
        out["saveError"] = json!(e);
    }
    Ok(out)
}

fn list(app: &LightcraftApp, p: &Value) -> Result<Value, String> {
    let status = match p.get("status").and_then(Value::as_str) {
        None | Some("all") => None,
        Some(s) => {
            Some(serde_json::from_value::<Status>(json!(s)).map_err(|_| format!("notes.list: `status` {s:?} is not open, sent, fixed or all"))?)
        }
    };
    let n = &app.fix_notes;
    let notes: Vec<&Note> = n.store.notes.iter().filter(|x| status.is_none_or(|s| x.status == s)).collect();
    Ok(json!({
        "notes": notes,
        "open": n.count(Status::Open),
        "sent": n.count(Status::Sent),
        "fixed": n.count(Status::Fixed),
        "picked": n.picked,
        "panel": n.open,
        "file": n.file.as_ref().map(|p| p.display().to_string()),
        "inbox": n.inbox.as_ref().map(|p| p.display().to_string()),
    }))
}

/// UI commands: `notes.toggle`, `notes.add`, `notes.list`, `notes.send`, `notes.markFixed`,
/// `notes.reopen`, `notes.edit`, `notes.remove`, `notes.select`.
pub fn run_command(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let r = match id {
        "notes.toggle" => {
            let open = p.get("open").and_then(Value::as_bool).unwrap_or(!app.fix_notes.open);
            app.fix_notes.open = open;
            app.fix_notes.clicked_in = false;
            app.fix_notes.field_focused = false;
            // opened: the note field takes the keys (type straight away, Esc closes it again)
            app.fix_notes.refocus = open;
            if open {
                app.fix_notes.message = None;
            }
            Ok(json!({"open": open}))
        }
        "notes.add" => {
            let text = p.get("text").and_then(Value::as_str).unwrap_or("").to_string();
            add(app, &text, p.get("shot").and_then(Value::as_bool))
        }
        "notes.list" => list(app, p),
        "notes.send" => send(app, p),
        "notes.markFixed" | "notes.reopen" => (|| {
            let Some(ids) = ids_param(p)? else { return Err(format!("{id}: missing `ids`")) };
            let fixed = id == "notes.markFixed";
            let comment = p.get("note").and_then(Value::as_str).map(|s| s.chars().take(MAX_TEXT).collect::<String>());
            if let Some(missing) = ids.iter().find(|i| app.fix_notes.note(**i).is_none()) {
                return Err(format!("{id}: no note {missing}"));
            }
            let at = now();
            for i in &ids {
                if let Some(n) = app.fix_notes.note_mut(*i) {
                    if fixed {
                        n.status = Status::Fixed;
                        n.fixed_at = Some(at);
                        if comment.is_some() {
                            n.fix_note = comment.clone();
                        }
                    } else {
                        n.status = Status::Open;
                        n.fixed_at = None;
                    }
                }
            }
            if fixed {
                let what = if ids.len() == 1 { crate::i18n::tr("1 note").to_string() } else { crate::i18n::tr_format!("{n} notes", n = ids.len()) };
                app.fix_notes.message = Some((crate::i18n::tr_format!("{what} fixed", what = what), false));
            }
            let save_error = persist(app);
            Ok(json!({"ids": ids, "open": app.fix_notes.count(Status::Open), "saveError": save_error}))
        })(),
        "notes.edit" => (|| {
            let Some(i) = p.get("id").and_then(Value::as_u64) else { return Err("notes.edit: missing `id`".into()) };
            let text: String = p.get("text").and_then(Value::as_str).unwrap_or("").trim().chars().take(MAX_TEXT).collect();
            if text.is_empty() {
                return Err("notes.edit: the note is empty (notes.remove removes it)".into());
            }
            let Some(n) = app.fix_notes.note_mut(i) else { return Err(format!("notes.edit: no note {i}")) };
            n.text = text;
            let save_error = persist(app);
            Ok(json!({"id": i, "saveError": save_error}))
        })(),
        "notes.remove" => (|| {
            let Some(ids) = ids_param(p)? else { return Err("notes.remove: missing `ids`".into()) };
            let before = app.fix_notes.store.notes.len();
            app.fix_notes.store.notes.retain(|n| !ids.contains(&n.id));
            app.fix_notes.picked.retain(|i| !ids.contains(i));
            if app.fix_notes.editing.as_ref().is_some_and(|(i, _)| ids.contains(i)) {
                app.fix_notes.editing = None;
            }
            let removed = before - app.fix_notes.store.notes.len();
            let save_error = persist(app);
            Ok(json!({"removed": removed, "saveError": save_error}))
        })(),
        "notes.select" => (|| {
            // {ids} picks exactly those; {all: true} every note; {} or {ids: []} none
            let ids = if p.get("all").and_then(Value::as_bool) == Some(true) { Some(app.fix_notes.listed()) } else { ids_param(p)? };
            app.fix_notes.picked = ids.unwrap_or_default().into_iter().filter(|i| app.fix_notes.note(*i).is_some()).collect();
            app.fix_notes.anchor = app.fix_notes.picked.iter().next().copied();
            Ok(json!({"picked": app.fix_notes.picked}))
        })(),
        _ => return None,
    };
    Some(r)
}

/// The top bar's `notes.` button, with the number of open notes.
pub fn top_button(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let label = crate::i18n::tr("notes.");
    let font = t.semibold(11.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.text);
    let open = app.fix_notes.count(Status::Open);
    let badge = (open > 0).then(|| ui.painter().layout_no_wrap(open.to_string(), t.semibold(10.0), t.text));
    let badge_w = badge.as_ref().map_or(0.0, |b| (b.size().x + 10.0).max(16.0) + 6.0);
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 18.0 + badge_w, 24.0), Sense::click());
    register(ui.ctx(), "button:fixNotes", r);
    let fill = if app.fix_notes.open {
        t.pressed
    } else if resp.hovered() {
        t.hover
    } else {
        t.button
    };
    let p = ui.painter();
    p.rect(r, CornerRadius::same(4), fill, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    p.galley(pos2(r.left() + 9.0, r.center().y - galley.size().y / 2.0), galley, t.text);
    if let Some(b) = badge {
        let br = Rect::from_min_size(pos2(r.right() - badge_w + 0.0, r.center().y - 8.0), vec2(badge_w - 6.0, 16.0));
        p.rect_filled(br, CornerRadius::same(8), t.accent);
        p.galley(br.center() - b.size() / 2.0, b, egui::Color32::WHITE);
    }
    let tip = if open == 0 {
        crate::i18n::tr("Notes about what isn't working (⌘⌥⇧N); fix. sends them to Claude").to_string()
    } else {
        crate::i18n::tr_format!("{open} open — {}", crate::i18n::tr("fix. sends them to Claude (⌘⌥⇧N)"), open = open)
    };
    if resp.on_hover_text(tip).clicked() {
        let _ = app.run("notes.toggle", json!({}));
    }
}

fn status_colors(t: &Tokens, s: Status) -> (egui::Color32, egui::Color32) {
    match s {
        Status::Open => (t.accent, egui::Color32::WHITE),
        Status::Sent => (t.caution, egui::Color32::BLACK),
        Status::Fixed => (egui::Color32::from_rgb(0x3d, 0x8b, 0x4f), egui::Color32::WHITE),
    }
}

/// How long ago, roughly: `just now`, `4 min ago`, `3 h ago`, `2 d ago`.
fn ago(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

/// A short line under a note: module · photo · time · shot.
fn context_line(n: &Note) -> String {
    let c = &n.context;
    let mut parts: Vec<String> = vec![];
    if let Some(m) = c["module"].as_str() {
        parts.push(m.to_string());
    }
    if let Some(f) = c["photo"]["fileName"].as_str() {
        parts.push(f.to_string());
    }
    if let Some(p) = c["panel"].as_str().filter(|p| *p != "none") {
        parts.push(p.to_string());
    }
    parts.push(ago(now().saturating_sub(n.at)));
    if n.screenshot.is_some() {
        parts.push("shot.".into());
    }
    parts.retain(|s| !s.is_empty());
    parts.join(" · ")
}

/// The floating notes panel (non-modal, movable, open while you work).
pub fn show(app: &mut LightcraftApp, ctx: &egui::Context) {
    // hidden while a screenshot for a note is taken (the shot shows the app, not the panel)
    if !app.fix_notes.open || !app.fix_notes.pending.is_empty() {
        return;
    }
    let t = Tokens::get(ctx);
    let frame = egui::Frame::window(&ctx.global_style()).fill(t.chrome).inner_margin(egui::Margin::same(12));
    let left = if app.integrated_titlebar { 90.0 } else { 20.0 };
    let shown = egui::Window::new(crate::i18n::tr("notes."))
        .id(egui::Id::new("fix-notes-panel"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .movable(true)
        .default_pos(pos2(left, t.top_bar_h + 10.0))
        .min_width(W)
        .max_width(W)
        .frame(frame)
        .show(ctx, |ui| body(app, ui));
    if let Some(s) = shown {
        register(ctx, "panel:fixNotes", s.response.rect);
        // a click in the panel gives it the keyboard's Esc, a click anywhere else takes it back
        if let Some(at) = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten()) {
            app.fix_notes.clicked_in = s.response.rect.contains(at);
        }
    }
}

/// Does the notes. panel have the keyboard (its note field focused, or the panel the last thing
/// clicked)? Esc then closes it ([`crate::shortcuts::handle`]); not while a note is being edited
/// (Esc cancels the edit) or a screenshot hides it.
pub fn has_focus(app: &LightcraftApp, ctx: &egui::Context) -> bool {
    let n = &app.fix_notes;
    n.open
        && n.pending.is_empty()
        && n.editing.is_none()
        && (n.clicked_in || n.field_focused || ctx.memory(|m| m.has_focus(egui::Id::new("fix-notes-field"))))
}

fn body(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.set_width(W);
    ui.spacing_mut().item_spacing = vec2(8.0, 12.0);
    let hovered_panel = ui.rect_contains_pointer(ui.max_rect().expand(12.0));
    // title · open count · ✕ (on hover)
    ui.horizontal(|ui| {
        ui.set_height(24.0);
        ui.label(egui::RichText::new(crate::i18n::tr("notes.")).font(t.semibold(13.0)).color(t.text));
        let open = app.fix_notes.count(Status::Open);
        let sent = app.fix_notes.count(Status::Sent);
        ui.label(
            egui::RichText::new(crate::i18n::tr_format!("{open} open · {sent} sent", open = open, sent = sent)).font(t.font(11.5)).color(t.text_dim),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            register(ui.ctx(), "button:fixNotesClose", r);
            if hovered_panel {
                if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, t.hover);
                }
                crate::icons::paint(ui.painter(), r.shrink(6.0), crate::icons::Icon::Close, t.icon);
            }
            if resp.on_hover_text(crate::i18n::tr("Close (⌘⌥⇧N)")).clicked() {
                let _ = app.run("notes.toggle", json!({"open": false}));
            }
        });
    });
    // the note being typed: Enter adds, ⇧Enter starts a new line
    let field = egui::Id::new("fix-notes-field");
    let resp = ui.add(
        egui::TextEdit::multiline(&mut app.fix_notes.draft)
            .id(field)
            .desired_rows(3)
            .desired_width(W)
            .font(t.font(13.0))
            .text_color(t.text)
            .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))
            .hint_text(crate::i18n::tr("What isn't working?")),
    );
    register(ui.ctx(), "field:fixNote", resp.rect);
    app.fix_notes.field_focused = resp.has_focus();
    let enter = resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    let mut add_now = enter;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let shot = app.fix_notes.shot;
        if text_button(ui, "fixNotesShot", "shot.", shot)
            .on_hover_text(crate::i18n::tr("Take a screenshot of the window with each new note"))
            .clicked()
        {
            app.fix_notes.shot = !shot;
        }
        if text_button(ui, "fixNotesAdd", "add.", false).clicked() {
            add_now = true;
        }
        ui.label(egui::RichText::new(crate::i18n::tr("⏎ adds · ⇧⏎ new line")).font(t.font(11.0)).color(t.text_dim));
    });
    if add_now {
        let text = std::mem::take(&mut app.fix_notes.draft);
        if text.trim().is_empty() {
            app.fix_notes.draft = text.trim().to_string();
        } else if let Err(e) = app.run("notes.add", json!({"text": text})) {
            app.fix_notes.draft = text;
            app.toast_error(ui.ctx(), e);
        }
        app.fix_notes.refocus = true;
    }
    // (once the panel is back: a screenshot for the new note hides it for a few frames)
    if app.fix_notes.pending.is_empty() && std::mem::take(&mut app.fix_notes.refocus) {
        ui.memory_mut(|m| m.request_focus(field));
    }
    list_ui(app, ui, &t);
    // all. none. · fix.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if text_button(ui, "fixNotesAll", "all.", false).clicked() {
            let _ = app.run("notes.select", json!({"all": true}));
        }
        if text_button(ui, "fixNotesNone", "none.", false).clicked() {
            let _ = app.run("notes.select", json!({"ids": []}));
        }
        let n = app.fix_notes.to_send().len();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if n == 0 { "fix.".to_string() } else { format!("fix. {n}") };
            let galley = ui.painter().layout_no_wrap(label, t.semibold(12.0), egui::Color32::WHITE);
            let (r, resp) = ui.allocate_exact_size(vec2((galley.size().x + 24.0).max(64.0), 24.0), Sense::click());
            register(ui.ctx(), "button:fixNotesSend", r);
            let fill = if resp.hovered() { t.accent.gamma_multiply(1.15) } else { t.accent };
            ui.painter().rect_filled(r, 4.0, fill);
            ui.painter().galley(r.center() - galley.size() / 2.0, galley, egui::Color32::WHITE);
            let tip = if app.fix_notes.picked.is_empty() { "Send every open note to Claude" } else { "Send the picked open notes to Claude" };
            if resp.on_hover_text(crate::i18n::tr(tip)).clicked() {
                match app.run("notes.send", json!({})) {
                    Ok(r) => {
                        let msg = app
                            .fix_notes
                            .message
                            .clone()
                            .map(|m| m.0)
                            .unwrap_or_else(|| crate::i18n::tr_format!("sent {} — Claude will pick it up", r["count"]));
                        app.toast(ui.ctx(), msg);
                    }
                    Err(e) => {
                        app.fix_notes.message = Some((e.clone(), true));
                        app.toast_error(ui.ctx(), e);
                    }
                }
            }
        });
    });
    if let Some((m, error)) = app.fix_notes.message.clone() {
        let text = if error { format!("⚠ {m}") } else { m };
        ui.add(egui::Label::new(egui::RichText::new(text).font(t.font(11.5)).color(if error { t.caution } else { t.text_label })).wrap());
    }
}

fn list_ui(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let (area, _) = ui.allocate_exact_size(vec2(W, LIST_H), Sense::hover());
    ui.painter().rect(area, 4.0, t.inset, Stroke::new(1.0, t.divider), StrokeKind::Inside);
    register(ui.ctx(), "list:fixNotes", area);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area.shrink(10.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    if app.fix_notes.store.notes.is_empty() {
        child.painter().text(
            area.center(),
            Align2::CENTER_CENTER,
            crate::i18n::tr("No notes yet: type what isn't working above"),
            t.font(12.0),
            t.text_dim,
        );
        return;
    }
    let order = app.fix_notes.listed();
    let mut action: Option<(&'static str, u64)> = None;
    egui::ScrollArea::vertical().id_salt("fix-notes-list").max_height(LIST_H - 20.0).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        let inner_w = ui.available_width() - 4.0;
        for id in order {
            let Some(note) = app.fix_notes.note(id).cloned() else { continue };
            if app.fix_notes.editing.as_ref().is_some_and(|(e, _)| *e == id) {
                edit_row(app, ui, t, id, inner_w);
                continue;
            }
            let chip_w = 46.0;
            let text_w = inner_w - 10.0 - chip_w - 8.0 - 8.0 - 36.0 - 8.0 - 20.0 - 10.0;
            let galley = ui.painter().layout(note.text.clone(), t.font(12.5), t.text, text_w.max(80.0));
            let line = context_line(&note);
            let sub = ui.painter().layout(line, t.font(10.5), t.text_dim, text_w.max(80.0));
            let h = (galley.size().y + 4.0 + sub.size().y + 16.0).max(40.0);
            let (r, resp) = ui.allocate_exact_size(vec2(inner_w, h), Sense::click());
            register(ui.ctx(), format!("row:fixNote-{id}"), r);
            let picked = app.fix_notes.picked.contains(&id);
            let hovered = ui.rect_contains_pointer(r);
            let fill = if picked {
                t.cell_selected
            } else if hovered {
                t.hover
            } else {
                t.cell
            };
            let p = ui.painter();
            p.rect_filled(r, 4.0, fill);
            if picked {
                p.rect_stroke(r, 4.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
            }
            let (bg, fg) = status_colors(t, note.status);
            let chip = Rect::from_min_size(pos2(r.left() + 10.0, r.top() + 8.0), vec2(chip_w, 18.0));
            p.rect_filled(chip, 9.0, bg);
            p.text(chip.center(), Align2::CENTER_CENTER, crate::i18n::tr(note.status.label()), t.semibold(10.5), fg);
            let tx = chip.right() + 8.0;
            p.galley(pos2(tx, r.top() + 8.0), galley.clone(), t.text);
            p.galley(pos2(tx, r.top() + 8.0 + galley.size().y + 4.0), sub, t.text_dim);
            // ed. and ✕ show on hover (they keep their room)
            let ed = Rect::from_min_size(pos2(r.right() - 36.0 - 8.0 - 20.0 - 10.0, r.top() + 8.0), vec2(36.0, 20.0));
            let rm = Rect::from_min_size(pos2(r.right() - 20.0 - 10.0, r.top() + 8.0), vec2(20.0, 20.0));
            let ed_resp = ui.interact(ed, egui::Id::new(("fix-note-edit", id)), Sense::click());
            let rm_resp = ui.interact(rm, egui::Id::new(("fix-note-remove", id)), Sense::click());
            register(ui.ctx(), format!("button:fixNoteEdit-{id}"), ed);
            register(ui.ctx(), format!("button:fixNoteRemove-{id}"), rm);
            if hovered {
                let p = ui.painter();
                p.rect(ed, 4.0, if ed_resp.hovered() { t.hover } else { t.button }, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
                p.text(ed.center(), Align2::CENTER_CENTER, crate::i18n::tr("edit."), t.semibold(10.5), t.text);
                if rm_resp.hovered() {
                    p.rect_filled(rm, 4.0, t.hover);
                }
                crate::icons::paint(p, rm.shrink(5.0), crate::icons::Icon::Close, t.icon);
            }
            if ed_resp.on_hover_text(crate::i18n::tr("Edit the note")).clicked() {
                action = Some(("edit", id));
            } else if rm_resp.on_hover_text(crate::i18n::tr("Remove (the picked notes, when this one is picked)")).clicked() {
                action = Some(("remove", id));
            } else if resp.clicked() {
                let m = ui.input(|i| i.modifiers);
                app.fix_notes.pick(id, m.command, m.shift);
            }
            if let Some(path) = &note.screenshot {
                let _ = resp.on_hover_text(format!("{}\n{path}", note.text));
            }
        }
    });
    match action {
        Some(("edit", id)) => {
            let text = app.fix_notes.note(id).map(|n| n.text.clone()).unwrap_or_default();
            app.fix_notes.editing = Some((id, text));
        }
        Some(("remove", id)) => {
            let ids: Vec<u64> = if app.fix_notes.picked.contains(&id) { app.fix_notes.picked.iter().copied().collect() } else { vec![id] };
            let _ = app.run("notes.remove", json!({"ids": ids}));
        }
        _ => {}
    }
}

fn edit_row(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, id: u64, w: f32) {
    let Some((_, text)) = app.fix_notes.editing.as_mut() else { return };
    let field = egui::Id::new(("fix-note-editing", id));
    let resp = ui.add(
        egui::TextEdit::multiline(text)
            .id(field)
            .desired_rows(2)
            .desired_width(w)
            .font(t.font(12.5))
            .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)),
    );
    register(ui.ctx(), format!("field:fixNoteEdit-{id}"), resp.rect);
    if !resp.has_focus() && !resp.lost_focus() {
        ui.memory_mut(|m| m.request_focus(field));
    }
    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter) && !i.modifiers.shift, i.key_pressed(egui::Key::Escape)));
    let mut save = resp.has_focus() && enter;
    let mut cancel = esc;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        save |= text_button(ui, "fixNoteEditSave", "save.", false).clicked();
        cancel |= text_button(ui, "fixNoteEditCancel", "cancel.", false).clicked();
    });
    if save {
        let text = app.fix_notes.editing.as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
        match app.run("notes.edit", json!({"id": id, "text": text})) {
            Ok(_) => app.fix_notes.editing = None,
            Err(e) => app.toast_error(ui.ctx(), e),
        }
    } else if cancel {
        app.fix_notes.editing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_and_iso_times() {
        // 2026-11-22T14:05:09Z
        let t = 1_795_356_309;
        assert_eq!(iso(t), "2026-11-22T14:05:09Z");
        assert_eq!(stamp(t), "20261122-140509");
    }

    #[test]
    fn picking_click_command_shift() {
        let mut n = FixNotes::default();
        for id in 1..=5 {
            n.store.notes.push(Note { id, text: format!("n{id}"), ..Default::default() });
        }
        // listed newest first: 5 4 3 2 1
        n.pick(4, false, false);
        assert_eq!(n.picked.iter().copied().collect::<Vec<_>>(), vec![4]);
        n.pick(2, false, true);
        assert_eq!(n.picked.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
        n.pick(5, true, false);
        assert_eq!(n.picked.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        n.pick(3, true, false);
        assert_eq!(n.picked.iter().copied().collect::<Vec<_>>(), vec![2, 4, 5]);
        n.pick(1, false, false);
        assert_eq!(n.picked.iter().copied().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn a_damaged_notes_file_is_kept_aside() {
        let d = std::env::temp_dir().join(format!("lc-fixnotes-damaged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FILE), b"{ not json").unwrap();
        let (n, notice) = FixNotes::at(Some(&d), None);
        assert!(notice.unwrap().contains("damaged"));
        assert!(n.store.notes.is_empty());
        assert!(n.file.is_some(), "a new file can be written");
        let kept: Vec<_> =
            std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains(".damaged-")).collect();
        assert_eq!(kept.len(), 1, "the damaged file is kept");
        let _ = std::fs::remove_dir_all(&d);
    }
}
