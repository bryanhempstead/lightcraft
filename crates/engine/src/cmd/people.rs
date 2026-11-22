//! People (faces), AI culling's face analysis and AI search: finding faces, naming people, confirming or
//! rejecting suggestions, the models they need, and natural-language search. See [`crate::people`] and
//! [`crate::vision`].

use lightcraft_catalog::PhotoId;
use lightcraft_vision::models::{self, Package};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, bool_or, cmd, str_param};
use crate::people::FaceRef;
use crate::vision::Kind;
use crate::{Result, Session};

/// `faces: [{photo, index}]` or `[[photo, index]]`.
fn faces(p: &Value, c: &str) -> Result<Vec<FaceRef>> {
    let list = p.get("faces").and_then(Value::as_array).ok_or_else(|| bad(c, "missing `faces`: [{photo, index}]"))?;
    list.iter()
        .map(|f| {
            let (photo, index) = match f {
                Value::Array(a) => (a.first().and_then(Value::as_u64), a.get(1).and_then(Value::as_u64)),
                _ => (f.get("photo").and_then(Value::as_u64), f.get("index").and_then(Value::as_u64)),
            };
            match (photo, index.and_then(|i| usize::try_from(i).ok())) {
                (Some(photo), Some(index)) => Ok(FaceRef { photo: PhotoId(photo), index }),
                _ => Err(bad(c, "each face is {photo, index}")),
            }
        })
        .collect()
}

/// The photos a run covers: `ids`, else `scope` (`selection`, `view` — the default — or `all`).
fn scope_ids(s: &mut Session, p: &Value) -> Vec<PhotoId> {
    if p.get("ids").is_some() {
        return s.targets(p);
    }
    match str_param(p, "scope").unwrap_or("view") {
        "selection" => s.targets(p),
        "all" => s.catalog.photos().filter(|q| q.in_library()).map(|q| q.id).collect(),
        _ => s.visible_cloned(),
    }
}

fn kind_param(p: &Value, c: &str, default: Kind) -> Result<Kind> {
    match str_param(p, "kind") {
        None => Ok(default),
        Some(k) => Kind::parse(k).ok_or_else(|| bad(c, "kind is `faces` or `search`")),
    }
}

fn package_param(p: &Value, c: &str) -> Result<Package> {
    let k = str_param(p, "model").ok_or_else(|| bad(c, "missing `model`: faces or search"))?;
    models::by_key(k).ok_or_else(|| bad(c, format!("unknown model `{k}` (faces, search)")))
}

fn index_run(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let ids = scope_ids(s, p);
    let force = bool_or(p, "force", false);
    // wait: run to the end even in the app (scripts, the control channel)
    let background = s.vision.background;
    if bool_or(p, "wait", false) {
        s.vision.background = false;
    }
    let queued = s.vision_start(kind, &ids, force);
    s.vision.background = background;
    let queued = queued?;
    let progress = s.vision.progress(kind);
    Ok(json!({"queued": queued, "photos": ids.len(), "background": s.vision.background, "progress": progress}))
}

fn people_list(s: &mut Session, p: &Value) -> Result<Value> {
    let filter = s.view_filter();
    let people = s.people(&filter);
    let limit = p.get("limit").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(usize::MAX);
    let faces = bool_or(p, "faces", false);
    let named: Vec<Value> = people
        .named
        .iter()
        .take(limit)
        .map(|n| {
            let mut v = json!({"name": n.name, "photos": n.photos, "faces": n.faces.len(), "similar": n.similar.len(), "cover": n.cover});
            if faces {
                v["faceList"] = json!(n.faces);
                v["similarList"] = json!(n.similar);
            }
            v
        })
        .collect();
    let unnamed: Vec<Value> = people
        .unnamed
        .iter()
        .take(limit)
        .map(|st| {
            let mut v = json!({"faces": st.faces.len(), "suggestion": st.suggestion, "cover": st.cover});
            if faces {
                v["faceList"] = json!(st.faces);
            }
            v
        })
        .collect();
    Ok(json!({
        "named": named, "unnamed": unnamed, "namedCount": people.named.len(), "stacks": people.unnamed.len(),
        "unindexed": people.unindexed, "facesInView": people.faces, "clustering": s.people_cache.busy(),
    }))
}

fn semantic(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "search.ai";
    let q = str_param(p, "query").map(str::trim).unwrap_or("");
    if q.is_empty() {
        s.filter.semantic = None;
        return Ok(json!({"query": null}));
    }
    // ranking first, so a missing model or index is an error rather than an empty grid
    let ranked = s.semantic_rank(q).map_err(|e| bad(C, e.to_string()))?;
    let top: Vec<Value> =
        ranked.iter().take(p.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize).map(|(id, v)| json!({"id": id.0, "score": v})).collect();
    if bool_or(p, "filter", true) {
        s.filter.semantic = Some(q.to_string());
    }
    let shown = s.visible_cloned().len();
    Ok(json!({"query": q, "matches": shown, "top": top}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "faces.find",
            "Find Faces",
            ["Library"],
            None,
            "{scope?: view|selection|all (view), ids?, force?: bool, wait?: bool} — find faces in the photos (those not analysed yet; all with force) in the background: each becomes an unnamed face region, recognised for People and judged for eyes and focus (Assisted Culling). Needs the face models (vision.model.download {model: \"faces\"}) → {queued, photos, progress}",
            always,
            |s, p| index_run(s, p, Kind::Faces)
        ),
        cmd!(query "vision.status", "Recognition Status", [], None, "{} → the face and search models (installed, size, licence, folder, download) and indexes (photos indexed, run progress: done / total / failed / faces / rate)", always, |s, _| {
            s.vision_poll();
            Ok(crate::vision::status_json(s))
        }),
        cmd!(
            "vision.cancel",
            "Stop Finding Faces",
            [],
            None,
            "{kind?: faces|search} — stop the running indexing (what's done is kept; the next run goes on from there)",
            always,
            |s, p| {
                let kind = match str_param(p, "kind") {
                    Some(k) => Some(Kind::parse(k).ok_or_else(|| bad("vision.cancel", "kind is `faces` or `search`"))?),
                    None => None,
                };
                s.vision.cancel(kind);
                Ok(json!({"cancelled": true}))
            }
        ),
        cmd!(
            "vision.model.download",
            "Download Recognition Models",
            [],
            None,
            "{model: faces|search, acknowledged: true} — download a model package (faces: YuNet + SFace + open-closed-eye, 39 MB, MIT / Apache-2.0; search: CLIP ViT-B/32 LAION-2B, 607 MB, MIT) after the user has seen its size and licence; size- and SHA-256-checked, resumes",
            always,
            |s, p| {
                const C: &str = "vision.model.download";
                let pkg = package_param(p, C)?;
                if !bool_or(p, "acknowledged", false) {
                    let i = models::info(pkg);
                    return Err(bad(
                        C,
                        format!(
                            "{} is {:.0} MB ({}; {}): pass acknowledged: true once the user agreed",
                            i.name,
                            i.size() as f64 / 1e6,
                            i.licence,
                            i.models
                        ),
                    ));
                }
                s.vision.download(pkg).map_err(|e| bad(C, e))?;
                Ok(json!({"download": s.vision.download_state(pkg), "installed": s.vision.installed(pkg)}))
            }
        ),
        cmd!("vision.model.cancel", "Stop Model Download", [], None, "{model: faces|search}", always, |s, p| {
            let pkg = package_param(p, "vision.model.cancel")?;
            s.vision.cancel_download(pkg);
            Ok(json!({"cancelled": true}))
        }),
        cmd!(query "people.list", "People", [], None, "{faces?: bool, limit?} — the People view of the current view's photos: named people (name, photos, cover face, faces, similar: suggested faces) and unnamed stacks of the same person (cover, faces, suggestion), with faces: true each face {photo, index, rect, name, suggestion, score}", always, people_list),
        cmd!(
            "people.name",
            "Name Person",
            [],
            None,
            "{faces: [{photo, index}], name} — name the faces (one undo step): their regions, and the person keyword on their photos",
            always,
            |s, p| {
                let f = faces(p, "people.name")?;
                let name = str_param(p, "name").ok_or_else(|| bad("people.name", "missing `name`"))?;
                Ok(json!({"named": s.name_faces(&f, Some(name), "Name Person")?}))
            }
        ),
        cmd!(
            "people.confirm",
            "Confirm Suggestion",
            [],
            None,
            "{faces: [{photo, index}]} — ✓: each face becomes the person it's suggested as",
            always,
            |s, p| {
                let f = faces(p, "people.confirm")?;
                let filter = s.view_filter();
                Ok(json!({"confirmed": s.confirm_faces(&f, &filter)?}))
            }
        ),
        cmd!(
            "people.reject",
            "Reject Suggestion",
            [],
            None,
            "{faces: [{photo, index}], name?} — ✗: the face isn't `name` (default: who it's suggested as); never suggested again",
            always,
            |s, p| {
                let f = faces(p, "people.reject")?;
                let filter = s.view_filter();
                Ok(json!({"rejected": s.reject_faces(&f, str_param(p, "name"), &filter)?}))
            }
        ),
        cmd!(
            "people.unname",
            "Remove Name",
            [],
            None,
            "{faces: [{photo, index}]} — the faces become unnamed again (their person keyword leaves photos where nobody else has that name)",
            always,
            |s, p| {
                let f = faces(p, "people.unname")?;
                Ok(json!({"unnamed": s.name_faces(&f, None, "Remove Name")?}))
            }
        ),
        cmd!(
            "people.removeFaces",
            "Remove Face",
            [],
            None,
            "{faces: [{photo, index}]} — delete the face regions (not a face of anybody)",
            always,
            |s, p| {
                let f = faces(p, "people.removeFaces")?;
                Ok(json!({"removed": s.remove_faces(&f)?}))
            }
        ),
        cmd!(
            "people.rename",
            "Rename Person",
            [],
            None,
            "{from, to} — rename a person on every face (a name someone has already merges the two)",
            always,
            |s, p| {
                let from = str_param(p, "from").ok_or_else(|| bad("people.rename", "missing `from`"))?;
                let to = str_param(p, "to").ok_or_else(|| bad("people.rename", "missing `to`"))?;
                Ok(json!({"faces": s.rename_person(from, to)?}))
            }
        ),
        cmd!(
            "photo.addFace",
            "Draw Face Region",
            [],
            None,
            "{id?, rect: {x0, y0, x1, y1}, name?} — add a face region (normalized, upright frame) to the photo, optionally named; undoable → {index}",
            always,
            |s, p| {
                const C: &str = "photo.addFace";
                let id = p.get("id").and_then(Value::as_u64).map(PhotoId).or_else(|| s.active()).ok_or_else(|| bad(C, "no photo"))?;
                let r = p.get("rect").ok_or_else(|| bad(C, "missing `rect`"))?;
                let num = |k: &str| {
                    r.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(C, format!("`rect.{k}` must be a number")))
                };
                let (a, b, c, d) = (num("x0")?, num("y0")?, num("x1")?, num("y1")?);
                let rect = lightcraft_meta::Rect {
                    x0: a.min(c).clamp(0.0, 1.0),
                    y0: b.min(d).clamp(0.0, 1.0),
                    x1: a.max(c).clamp(0.0, 1.0),
                    y1: b.max(d).clamp(0.0, 1.0),
                };
                if rect.x1 - rect.x0 < 0.005 || rect.y1 - rect.y0 < 0.005 {
                    return Err(bad(C, "the box would be too small"));
                }
                Ok(json!({"index": s.add_face(id, rect, str_param(p, "name"))?}))
            }
        ),
        cmd!(
            "search.index",
            "Index for AI Search",
            ["Library"],
            None,
            "{scope?: view|selection|all (all), ids?, force?, wait?: bool} — compute the photos' CLIP embeddings for natural-language search, in the background (needs vision.model.download {model: \"search\"}) → {queued, photos, progress}",
            always,
            |s, p| {
                let mut p = p.clone();
                if p.get("scope").is_none() && p.get("ids").is_none() {
                    p["scope"] = json!("all");
                }
                index_run(s, &p, kind_param(&p, "search.index", Kind::Search)?)
            }
        ),
        cmd!(
            "search.ai",
            "AI Search",
            [],
            None,
            "{query, limit?, filter?: bool (true)} — natural-language search (\"dog on the beach\", \"red car at night\"): the view shows the photos that match best, best first, with the other filters still applying; an empty query clears it → {query, matches, top: [{id, score}]}",
            always,
            semantic
        ),
    ]
}
