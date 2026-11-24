//! LC-SB-PIPELINE: which library folders are shoots in Second Brain's pipeline (and at which
//! stage), and sending a folder that isn't there yet ([`crate::sb_pipeline`]). Works with Second
//! Brain closed: the pipeline is read from its file, a send waits in its inbox.

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, bool_or, cmd, str_param};
use crate::sb_pipeline::{self as sbp, Backup, Mark};
use crate::{Result, Session};

fn dir(s: &Session, c: &str) -> Result<std::path::PathBuf> {
    sbp::sb_dir(s.second_brain_dir.as_deref()).ok_or_else(|| bad(c, "no home folder to find Second Brain in"))
}

fn folder_param(p: &Value) -> Option<String> {
    str_param(p, "folder").or_else(|| str_param(p, "path")).map(|f| f.trim().trim_end_matches('/').to_string()).filter(|f| !f.is_empty())
}

fn request_json(r: &sbp::Request) -> Value {
    json!({"id": r.id, "path": r.path, "name": r.name, "backup": r.backup.id(), "stage": r.stage, "at": r.at})
}

fn status(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pipeline.status";
    let dir = dir(s, C)?;
    let pipe = sbp::load(&dir);
    let file = dir.join(sbp::PIPE_FILE);
    let Some(folder) = folder_param(p) else {
        return Ok(json!({
            "file": file,
            "found": pipe.found,
            "error": pipe.error,
            "shoots": pipe.shoots.iter().map(sbp::shoot_json).collect::<Vec<_>>(),
            "pending": pipe.pending.iter().map(request_json).collect::<Vec<_>>(),
        }));
    };
    let mark = pipe.mark(&folder);
    let mut out = json!({
        "folder": folder,
        "file": file,
        "found": pipe.found,
        "inPipeline": matches!(mark, Some(Mark::Shoot(_))),
        "pending": matches!(mark, Some(Mark::Pending(_))),
        "tag": mark.map(|m| m.tag()),
        "error": pipe.error,
    });
    match mark {
        Some(Mark::Shoot(sh)) => {
            out["stage"] = json!(sh.stage);
            out["stageWord"] = json!(sbp::stage_word(&sh.stage));
            out["shoot"] = sbp::shoot_json(sh);
        }
        Some(Mark::Pending(r)) => out["request"] = request_json(r),
        None => {}
    }
    Ok(out)
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pipeline.add";
    let folder = folder_param(p).ok_or_else(|| bad(C, "missing `folder`"))?;
    let backup = match str_param(p, "backup") {
        None => Backup::Run,
        Some(b) => Backup::parse(b).ok_or_else(|| bad(C, format!("`backup` is run, done or skip, not “{b}”")))?,
    };
    let dir = dir(s, C)?;
    if let Some(sh) = sbp::load(&dir).shoot_for(&folder) {
        return Err(bad(C, format!("“{}” is already in the second brain pipeline ({})", sh.name, sbp::stage_word(&sh.stage))));
    }
    let req = sbp::queue_add(&dir, &folder, str_param(p, "name"), backup).map_err(|e| bad(C, e))?;
    // a running Second Brain takes it now (on a worker: never wait on it here); closed, it takes
    // it at its next launch
    let nudge = bool_or(p, "nudge", true);
    #[cfg(target_arch = "wasm32")]
    let _ = nudge;
    #[cfg(not(target_arch = "wasm32"))]
    if nudge {
        let d = s.second_brain_dir.clone();
        let _ = std::thread::Builder::new().name("lc-sb-nudge".into()).spawn(move || sbp::nudge(d.as_deref()));
    }
    Ok(json!({
        "queued": true,
        "pending": true,
        "tag": sbp::PENDING_TAG,
        "request": request_json(&req),
        "file": dir.join(sbp::INBOX).join(format!("{}.json", req.id)),
    }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "pipeline.status",
            "Second Brain Pipeline Status",
            [],
            None,
            "{folder?} → with folder: {folder, inPipeline, pending, tag (pipeline.editing | pipeline.pending | null), stage?, stageWord?, shoot?: {id, path, name, stage, stageWord, tag, parts}, request?, found, error, file}; without: {shoots, pending, found, error, file} — Second Brain's shoot pipeline, read from its shootpipe.json (Second Brain need not be running; never written here)",
            always,
            status
        ),
        cmd!(
            query "pipeline.add",
            "Send to Second Brain Pipeline",
            [],
            None,
            "{folder, backup?: run | done | skip (default run: back.up. / already.backed.up. / skip.), name?, nudge?: bool (default true)} → {queued, pending, tag, request, file} — sends a folder that isn't in Second Brain's pipeline yet: a request in its inbox (lightcraft-inbox/), taken at once when Second Brain is running, else at its next launch; it lands in editing. Refused for a folder already on the board",
            always,
            add
        ),
    ]
}
