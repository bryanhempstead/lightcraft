//! LC-SB-PIPELINE: Second Brain's shoot pipeline, seen from LightCraft (Bryan, 2026-11-23: "add an
//! area that marks that it came from the 2nd brain pipeline … and if it's not then I can
//! retroactively put it there … mark or ask if I want it backed up and then put it in the
//! currently editing area").
//!
//! **Reading (the marks).** Second Brain keeps its pipeline in `<userData>/shootpipe.json` (one row
//! per shoot: its folder, its stage, its camera folders). LightCraft only ever **reads** that file,
//! so the marks show whether or not Second Brain is running. [`load`] re-reads it only when its
//! modification time or size changed (and the inbox below), so the folder tree can ask every
//! frame from a worker thread for nothing.
//!
//! **Sending (retroactively).** LightCraft never writes `shootpipe.json` (Second Brain may be
//! writing it at that moment). A folder sent from here becomes one small request file in
//! `<userData>/lightcraft-inbox/<id>.json`, written to a temporary name and renamed, so Second
//! Brain never sees half a request. Second Brain takes the inbox at launch, whenever a file lands
//! in it while it runs, and when LightCraft nudges it over its local API
//! (`POST /v1/pipeline/inbox`, [`nudge`]): it adds the shoot with its own pipeline code (camera
//! folders join their shoot), runs or marks the backup, moves it to editing and files the request
//! under `lightcraft-inbox/done/`. Until then the folder shows `pipeline.pending`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use lightcraft_catalog::query::{folder_key, folder_within};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Second Brain's pipeline file in its userData folder.
pub const PIPE_FILE: &str = "shootpipe.json";
/// The folder (in Second Brain's userData) LightCraft's requests wait in.
pub const INBOX: &str = "lightcraft-inbox";
/// Largest pipeline file read (Bryan's is ~30 KB).
const MAX_FILE: u64 = 16 << 20;
/// Largest request file read.
const MAX_REQUEST: u64 = 64 << 10;
/// Most requests listed as pending.
const MAX_PENDING: usize = 500;

/// Second Brain's stages, in order, with the word LightCraft shows for each (full words, periods
/// for spaces: `pipeline.editing`).
pub const STAGES: [(&str, &str); 8] = [
    ("imp", "imported"),
    ("bkup", "backed.up"),
    ("cull", "culling"),
    ("edit", "editing"),
    ("finedit", "exporting"),
    ("dlvr", "delivering"),
    ("post", "posting"),
    ("done", "finished"),
];

/// The word for a stage id (`edit` → `editing`); an id this build doesn't know → `in.progress`.
pub fn stage_word(stage: &str) -> &'static str {
    STAGES.iter().find(|(id, _)| *id == stage).map_or("in.progress", |(_, w)| w)
}

/// The tag the folder tree shows for a stage: `pipeline.editing`.
pub fn stage_tag(stage: &str) -> String {
    format!("pipeline.{}", stage_word(stage))
}

/// The tag of a folder sent from LightCraft that Second Brain hasn't taken yet.
pub const PENDING_TAG: &str = "pipeline.pending";

/// One shoot on Second Brain's board.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shoot {
    pub id: String,
    /// The shoot's folder (a shoot IS its folder).
    pub path: String,
    pub name: String,
    /// Second Brain's stage id (`edit`); [`stage_word`] for the word.
    pub stage: String,
    /// Its camera folders and other parts (they may lie outside `path`: a part tied to its backup).
    pub parts: Vec<String>,
}

/// What Second Brain should do about the backup of a folder sent from LightCraft.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Backup {
    /// Run the pipeline's backup (to its backup places).
    #[default]
    Run,
    /// It is backed up already: mark the backup step done.
    Done,
    /// Leave the backup alone.
    Skip,
}

impl Backup {
    pub fn parse(s: &str) -> Option<Backup> {
        match s.trim().trim_end_matches('.') {
            "run" | "back.up" | "backup" | "backUp" => Some(Backup::Run),
            "done" | "already" | "already.backed.up" | "alreadyBackedUp" => Some(Backup::Done),
            "skip" | "none" => Some(Backup::Skip),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Backup::Run => "run",
            Backup::Done => "done",
            Backup::Skip => "skip",
        }
    }
}

/// A folder sent from LightCraft, waiting in the inbox.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Request {
    pub v: u32,
    pub id: String,
    /// Always `pipeline.add` for now.
    pub kind: String,
    pub path: String,
    pub name: String,
    pub backup: Backup,
    /// Where the shoot lands (Second Brain's stage id; `edit` = currently editing).
    pub stage: String,
    /// Milliseconds since 1970.
    pub at: u64,
    pub from: String,
}

/// Second Brain's pipeline as last read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pipeline {
    /// Its userData folder.
    pub dir: PathBuf,
    /// `shootpipe.json` was there and read.
    pub found: bool,
    /// Why it couldn't be read (a bad file = no marks, never a crash).
    pub error: Option<String>,
    /// The shoots on the board (archived ones left out).
    pub shoots: Vec<Shoot>,
    /// Requests from LightCraft Second Brain hasn't taken yet, oldest first.
    pub pending: Vec<Request>,
    /// Where its backups go (settings ▸ backup places); empty: it asks when one runs.
    pub backup_places: Vec<String>,
}

/// What a folder is to the pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark<'a> {
    /// On the board (the shoot, a folder inside it, or one of its camera folders).
    Shoot(&'a Shoot),
    /// Sent from LightCraft, not taken yet.
    Pending(&'a Request),
}

impl Mark<'_> {
    /// `pipeline.editing` / `pipeline.pending`.
    pub fn tag(&self) -> String {
        match self {
            Mark::Shoot(s) => stage_tag(&s.stage),
            Mark::Pending(_) => PENDING_TAG.to_string(),
        }
    }
}

impl Pipeline {
    /// The shoot `folder` belongs to: the deepest shoot it lies in, or the shoot that lists it as
    /// one of its parts. A folder that only holds shoots (a parent) is not marked.
    pub fn shoot_for(&self, folder: &str) -> Option<&Shoot> {
        let key = folder_key(folder);
        if key.is_empty() {
            return None;
        }
        self.shoots
            .iter()
            .filter(|s| folder_within(&key, &s.path))
            .max_by_key(|s| folder_key(&s.path).len())
            .or_else(|| self.shoots.iter().find(|s| s.parts.iter().any(|p| folder_key(p) == key)))
    }

    /// The request still waiting for `folder` (sent for it or a folder it lies in).
    pub fn pending_for(&self, folder: &str) -> Option<&Request> {
        let key = folder_key(folder);
        if key.is_empty() {
            return None;
        }
        self.pending.iter().find(|r| folder_within(&key, &r.path))
    }

    /// The folder's mark: its shoot, else a request waiting for it.
    pub fn mark(&self, folder: &str) -> Option<Mark<'_>> {
        self.shoot_for(folder).map(Mark::Shoot).or_else(|| self.pending_for(folder).map(Mark::Pending))
    }
}

/// Second Brain's userData folder (see [`crate::galleries::sb_dir`]).
pub fn sb_dir(dir: Option<&Path>) -> Option<PathBuf> {
    crate::galleries::sb_dir(dir)
}

fn read_capped(path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    f.take(cap.saturating_add(1)).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(std::io::Error::other(format!("larger than {} KB", cap >> 10)));
    }
    Ok(buf)
}

/// The shoots in a `shootpipe.json` (archived and malformed rows left out).
pub fn parse_shoots(bytes: &[u8]) -> Result<Vec<Shoot>, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    let rows = v.get("shoots").and_then(Value::as_array).ok_or("no shoots list")?;
    let text = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(rows
        .iter()
        .filter(|r| r.is_object())
        .filter(|r| r.get("archived").is_none_or(|a| a.is_null() || a == &json!(false) || a == &json!(0)))
        .filter_map(|r| {
            let path = text(r, "path");
            if path.is_empty() || !path.starts_with('/') {
                return None;
            }
            let parts = r
                .get("children")
                .and_then(Value::as_array)
                .map(|c| c.iter().filter_map(|c| c.get("path").and_then(Value::as_str)).filter(|p| !p.is_empty()).map(str::to_string).collect())
                .unwrap_or_default();
            Some(Shoot { id: text(r, "id"), name: text(r, "name"), stage: text(r, "stage"), path, parts })
        })
        .collect())
}

/// The requests waiting in `<dir>/lightcraft-inbox` (unreadable ones left out), oldest first.
pub fn pending(dir: &Path) -> Vec<Request> {
    let Ok(rd) = std::fs::read_dir(dir.join(INBOX)) else { return Vec::new() };
    let mut v: Vec<Request> = rd
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            !n.starts_with('.') && n.ends_with(".json") && e.file_type().is_ok_and(|t| t.is_file())
        })
        .take(MAX_PENDING)
        .filter_map(|e| read_capped(&e.path(), MAX_REQUEST).ok())
        .filter_map(|b| serde_json::from_slice::<Request>(&b).ok())
        .filter(|r| !r.path.is_empty())
        .collect();
    v.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
    v
}

/// Read the pipeline and the inbox now (no cache).
pub fn read(dir: &Path) -> Pipeline {
    let mut p = Pipeline { dir: dir.to_path_buf(), pending: pending(dir), ..Default::default() };
    let file = dir.join(PIPE_FILE);
    match read_capped(&file, MAX_FILE) {
        Ok(b) => {
            p.found = true;
            match parse_shoots(&b) {
                Ok(s) => p.shoots = s,
                Err(e) => p.error = Some(format!("{}: {e}", file.display())),
            }
            p.backup_places = serde_json::from_slice::<Value>(&b)
                .ok()
                .and_then(|v| v.pointer("/settings/backupPlaces").and_then(Value::as_array).cloned())
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => p.error = Some(format!("{}: {e}", file.display())),
    }
    p
}

/// What [`load`] compares to know nothing changed: the pipeline file's and the inbox's
/// modification times and the file's size.
type Stamp = (Option<SystemTime>, Option<u64>, Option<SystemTime>);

fn stamp(dir: &Path) -> Stamp {
    let f = std::fs::metadata(dir.join(PIPE_FILE)).ok();
    let inbox = std::fs::metadata(dir.join(INBOX)).ok().and_then(|m| m.modified().ok());
    (f.as_ref().and_then(|m| m.modified().ok()), f.map(|m| m.len()), inbox)
}

/// [`read`], kept until the pipeline file or the inbox changes (checked by modification time —
/// cheap enough to ask every couple of seconds).
pub fn load(dir: &Path) -> Arc<Pipeline> {
    type Cache = Option<(PathBuf, Stamp, Arc<Pipeline>)>;
    static CACHE: Mutex<Cache> = Mutex::new(None);
    let now = stamp(dir);
    {
        let g = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((d, s, p)) = g.as_ref()
            && d == dir
            && *s == now
        {
            return p.clone();
        }
    }
    let p = Arc::new(read(dir));
    *CACHE.lock().unwrap_or_else(PoisonError::into_inner) = Some((dir.to_path_buf(), now, p.clone()));
    p
}

/// [`load`] for a folder path given as text (what the UI's file-system cache calls).
pub fn load_str(dir: &str) -> Arc<Pipeline> {
    load(Path::new(dir))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

/// Queue "add `folder` to the pipeline" for Second Brain: a request file in its inbox (written
/// to a temporary name, then renamed). A folder already waiting gets its request back, unchanged.
pub fn queue_add(dir: &Path, folder: &str, name: Option<&str>, backup: Backup) -> Result<Request, String> {
    let folder = folder.trim().trim_end_matches('/');
    if !folder.starts_with('/') || folder.len() < 2 {
        return Err(format!("“{folder}” is not a full folder path"));
    }
    if !dir.is_dir() {
        return Err(format!("Second Brain's folder isn't there ({}) — is Second Brain installed?", dir.display()));
    }
    if let Some(r) = pending(dir).into_iter().find(|r| folder_key(&r.path) == folder_key(folder)) {
        return Ok(r);
    }
    let inbox = dir.join(INBOX);
    std::fs::create_dir_all(&inbox).map_err(|e| format!("{}: {e}", inbox.display()))?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let at = now_ms();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let id = format!("lc{at}-{}-{seq}", std::process::id());
    let base = Path::new(folder).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| folder.to_string());
    let name = name.map(str::trim).filter(|n| !n.is_empty()).map_or(base, str::to_string);
    let req = Request {
        v: 1,
        id: id.clone(),
        kind: "pipeline.add".into(),
        path: folder.to_string(),
        name,
        backup,
        stage: "edit".into(),
        at,
        from: "lightcraft".into(),
    };
    let body = serde_json::to_vec_pretty(&req).map_err(|e| e.to_string())?;
    let tmp = inbox.join(format!(".{id}.json.tmp"));
    let dest = inbox.join(format!("{id}.json"));
    std::fs::write(&tmp, &body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", dest.display()));
    }
    Ok(req)
}

/// Tell a running Second Brain to take its inbox now (`POST /v1/pipeline/inbox` on its local
/// API) → whether it answered. Not running (or an older Second Brain without the route): the
/// request waits in the inbox for its next launch.
pub fn nudge(dir: Option<&Path>) -> bool {
    let Ok(api) = crate::galleries::connect(dir) else { return false };
    api.post("/v1/pipeline/inbox", &json!({"from": "lightcraft"})).is_ok()
}

/// A shoot as the commands report it.
pub fn shoot_json(s: &Shoot) -> Value {
    json!({"id": s.id, "path": s.path, "name": s.name, "stage": s.stage, "stageWord": stage_word(&s.stage), "tag": stage_tag(&s.stage), "parts": s.parts})
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn stage_words_are_full_words() {
        assert_eq!(stage_tag("edit"), "pipeline.editing");
        assert_eq!(stage_tag("bkup"), "pipeline.backed.up");
        assert_eq!(stage_tag("whatever"), "pipeline.in.progress");
        assert!(STAGES.iter().all(|(_, w)| w.chars().all(|c| c.is_ascii_lowercase() || c == '.')));
    }

    #[test]
    fn bad_files_give_no_shoots_and_no_panic() {
        for b in [&b""[..], b"null", b"[]", b"{\"shoots\": 3}", b"{\"shoots\": [1, null, {\"path\": 4}, {\"path\": \"rel\"}]}", b"\xff\xfe"] {
            assert!(parse_shoots(b).map_or(true, |v| v.is_empty()), "{b:?}");
        }
    }

    #[test]
    fn backup_choices_parse() {
        assert_eq!(Backup::parse("back.up."), Some(Backup::Run));
        assert_eq!(Backup::parse("already.backed.up."), Some(Backup::Done));
        assert_eq!(Backup::parse("skip."), Some(Backup::Skip));
        assert_eq!(Backup::parse("maybe"), None);
    }
}
