//! Photo Merge commands: `merge.hdr`, `merge.panorama`, `merge.hdrPanorama`, `merge.focus`, `merge.take` (+ `merge.faces`).
//!
//! Run synchronously here (CLI, MCP, control channel); the desktop UI plans the same job with
//! [`Session::plan_merge`] and runs it on a worker thread. `preview: true` merges ≤ 1024 px frames
//! and (with `previewPath`) writes the rendered preview as PNG instead of adding a photo.

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::{CommandSpec, bad, bool_or, cmd, str_param};
use crate::{Result, Session};

fn can_merge(s: &Session) -> std::result::Result<(), String> {
    if cfg!(target_arch = "wasm32") && s.media.file_bytes.is_none() {
        return Err("Photo Merge needs access to the photo files".into());
    }
    // the photo count is checked when run (an explicit `ids` parameter may name them)
    if s.catalog.photos().nth(1).is_none() { Err("the library has fewer than 2 photos".into()) } else { Ok(()) }
}

fn run(s: &mut Session, id: &str, p: &Value) -> Result<Value> {
    let (kind, finish) = crate::merge::parse(id, p)?;
    let ids: Vec<PhotoId> = s.targets(p);
    if !bool_or(p, "preview", false) {
        return s.merge_now(kind, finish, &ids);
    }
    let job = s.plan_merge(kind, finish, &ids, true)?;
    let out = job.run(&|_, _| true).map_err(|e| bad(id, e))?;
    let mut info = out.info;
    if let (Some(path), Some(img)) = (str_param(p, "previewPath"), &out.preview) {
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default())
            .map_err(|e| bad(id, e.to_string()))?;
        s.check_write_target(path).map_err(|e| bad(id, e))?;
        crate::export::write_file(path, &png).map_err(|e| bad(id, e))?;
        info["previewPath"] = json!(path);
    }
    if let Some(img) = &out.preview {
        info["previewSize"] = json!([img.width, img.height]);
    }
    Ok(info)
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// `ids`: group ids (strings).
fn group_ids(c: &str, p: &Value) -> Result<Vec<String>> {
    let v = p.get("ids").and_then(Value::as_array).ok_or_else(|| bad(c, "`ids`: the ids of merge groups (merge.candidates)"))?;
    Ok(v.iter().filter_map(|x| x.as_str().map(str::to_string).or_else(|| x.as_u64().map(|n| n.to_string()))).collect())
}

/// `ids`: group ids or merged photo ids.
fn merged_ids(c: &str, p: &Value) -> Result<Vec<Value>> {
    p.get("ids").and_then(Value::as_array).cloned().ok_or_else(|| bad(c, "`ids`: merge group ids or merged photo ids"))
}

fn kind_param(c: &str, p: &Value, key: &str) -> Result<Option<lightcraft_catalog::MergeType>> {
    match str_param(p, key) {
        None | Some("auto") | Some("auto.") => Ok(None),
        Some(k) => lightcraft_catalog::MergeType::parse(k).map(Some).ok_or_else(|| bad(c, format!("{key}: hdr, pano, stack or hdrPano"))),
    }
}

/// merges.: the finder, the groups and what a merge did (see [`crate::merge_find`]).
fn merges_specs() -> Vec<CommandSpec> {
    use crate::merge_find::{GroupState, Scope, groups};
    vec![
        cmd!(
            "merge.find",
            "Find Merges",
            [],
            None,
            "{scope?: {kind: all|folder|album, path?, deep?, id?} | path?, deep?, album?, background?: bool} — merges.: look for photo merges (brackets → hdr., shifted frames at one exposure → pano., one framing with the focus moving → stack., brackets at shifted framings → hdr.pano.) among the photos of a folder, a collection or the library, and keep them as open groups to review (frames already in a group, skipped ones too, are never suggested again); background: true looks on a worker thread in the app (→ {queued}) → {found, groups: [{id, kind, tag, photos, state, result, folder, found}]}",
            always,
            |s, p| {
                let scope = Scope::from_params(p).map_err(|e| bad("merge.find", e))?;
                let background = bool_or(p, "background", false) && s.vision.background;
                s.merge_find(&scope, background)
            }
        ),
        cmd!(
            query "merge.candidates",
            "Merge Groups",
            [],
            None,
            "{scope? | path?, deep?, album?, state?: open|merged|dismissed|all (default: open and merged)} → {groups: [{id, kind, tag, photos, state, result, folder, found}], running: bool} the merges found (merges.)",
            always,
            |s, p| {
                let scope = Scope::from_params(p).map_err(|e| bad("merge.candidates", e))?;
                let want = str_param(p, "state").unwrap_or("live");
                let list: Vec<Value> = groups(&s.catalog)
                    .into_iter()
                    .filter(|g| scope.holds(&s.catalog, g))
                    .filter(|g| match want {
                        "all" => true,
                        "open" => g.state == GroupState::Open,
                        "merged" => g.state == GroupState::Merged,
                        "dismissed" => g.state == GroupState::Dismissed,
                        _ => g.state != GroupState::Dismissed,
                    })
                    .map(|g| g.to_json())
                    .collect();
                Ok(json!({"groups": list, "running": s.merge_find_running()}))
            }
        ),
        cmd!(
            "merge.mergeGroups",
            "Merge Groups",
            [],
            None,
            "{ids: [group id], kind?: hdr|pano|stack|hdrPano (else each group's own), frames?: [photo id] (only these frames of each group, when two or more of it), autoSettings?: bool (true)} — merges.: merge each group into a DNG next to its first frame; its frames are then hidden from every view (not deleted, files untouched), the merged photo shows in merges. and where the frames were (their folder and collections), tagged with its kind → {merged: [{group, id, hidden}], failed: [{group, error}]}",
            can_merge,
            |s, p| {
                let ids = group_ids("merge.mergeGroups", p)?;
                let kind = kind_param("merge.mergeGroups", p, "kind")?;
                let only: Vec<PhotoId> = p.get("frames").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_u64).map(PhotoId).collect()).unwrap_or_default();
                s.merge_groups_now(&ids, kind, &only, bool_or(p, "autoSettings", true))
            }
        ),
        cmd!(
            "merge.dismiss",
            "Skip Merge",
            [],
            None,
            "{ids: [group id]} — merges. skip.: the open groups are never suggested again (⌘Z brings them back) → {dismissed}",
            always,
            |s, p| {
                let ids = group_ids("merge.dismiss", p)?;
                s.merge_dismiss(&ids)
            }
        ),
        cmd!(
            "merge.setKind",
            "Merge As",
            [],
            None,
            "{id: group id, kind: hdr|pano|stack|hdrPano} — what an open group will be merged as",
            always,
            |s, p| {
                let id = str_param(p, "id").ok_or_else(|| bad("merge.setKind", "`id`: a merge group id"))?;
                let kind = kind_param("merge.setKind", p, "kind")?.ok_or_else(|| bad("merge.setKind", "`kind`: hdr, pano, stack or hdrPano"))?;
                s.set_group_kind(id, kind)?;
                Ok(json!({"id": id, "kind": kind.id()}))
            }
        ),
        cmd!(
            "merge.unmerge",
            "Unmerge",
            [],
            None,
            "{ids: [group id | merged photo id]} — merges. unmerge.: the frames show again where they were and the group is open again; the merged photo stays (with its tag) → {unmerged, shown}",
            always,
            |s, p| {
                let ids = merged_ids("merge.unmerge", p)?;
                s.merge_unmerge(&ids)
            }
        ),
        cmd!(
            "merge.showIn",
            "Show Merged Photo In",
            [],
            None,
            "{ids: [group id | merged photo id], where: both|merges} — both.: in merges. and in its folder / collections (the default after a merge); merges.: in merges. only → {changed}",
            always,
            |s, p| {
                let ids = merged_ids("merge.showIn", p)?;
                let only = match str_param(p, "where").unwrap_or("both").trim_end_matches('.') {
                    "both" => false,
                    "merges" | "merges.only" | "mergesOnly" => true,
                    other => return Err(bad("merge.showIn", format!("where: both or merges, not `{other}`"))),
                };
                s.merge_show_in(&ids, only)
            }
        ),
    ]
}

pub fn specs() -> Vec<CommandSpec> {
    let mut v = base_specs();
    v.extend(merges_specs());
    v
}

fn base_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "merge.hdr",
            "HDR Merge",
            [],
            None,
            "{ids?, align=true, deghost=none|low|medium|high, autoSettings=true, stack=false, preview=false, showOverlay=false, previewPath?}",
            can_merge,
            |s, p| run(s, "merge.hdr", p)
        ),
        cmd!(
            "merge.panorama",
            "Panorama Merge",
            [],
            None,
            "{ids?, projection=auto|spherical|cylindrical|perspective, boundaryWarp=0..100, autoCrop=false, fillEdges=false, autoSettings=true, stack=false, maxMegapixels=40, preview=false, previewPath?}",
            can_merge,
            |s, p| run(s, "merge.panorama", p)
        ),
        cmd!(
            "merge.hdrPanorama",
            "HDR Panorama Merge",
            [],
            None,
            "{ids?, bracket=0 (auto from EXIF), align, deghost, projection, boundaryWarp, autoCrop, fillEdges, autoSettings, stack, preview, previewPath?}",
            can_merge,
            |s, p| run(s, "merge.hdrPanorama", p)
        ),
        cmd!(
            "merge.focus",
            "Focus Stack Merge",
            [],
            None,
            "{ids?, align=true, autoSettings=true, stack=false, preview=false, previewPath?} — photos focused at different distances → one DNG sharp throughout (`<name>-Focus.dng`)",
            can_merge,
            |s, p| run(s, "merge.focus", p)
        ),
        cmd!(
            "merge.faces",
            "Find Faces for Best Take",
            [],
            None,
            "{ids?} — the people in the photos, matched across them by position → {people: [{name, boxes: [[x, y, w, h] | null per photo]}]} (tagged faces, else Apple Vision on macOS)",
            can_merge,
            |s, p| {
                let ids: Vec<PhotoId> = s.targets(p);
                let people = s.take_faces(&ids).map_err(|e| bad("merge.faces", e))?;
                Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "people": people}))
            }
        ),
        cmd!(
            "merge.take",
            "Best Take Merge",
            [],
            None,
            "{ids?, base: index of the photo to start from, swaps: [{donor: photo index, baseBox: [x, y, w, h], donorBox}] (boxes from merge.faces), autoSettings=true, stack=false, preview=false, previewPath?} — each picked face onto the base photo → `<name>-Take.dng`",
            can_merge,
            |s, p| run(s, "merge.take", p)
        ),
    ]
}
