//! Photo management commands: batch rename, capture time, colour label names and sets.

use serde_json::{Value, json};

use lightcraft_catalog::{ColorLabel, Op};

use super::{CommandSpec, always, bad, bool_or, cmd, f64_or, has_selection, str_param};
use crate::Result;

fn rename_args(s: &crate::Session, p: &Value, c: &str) -> Result<(Vec<lightcraft_catalog::PhotoId>, String, usize, crate::rename::Batch)> {
    let template =
        super::naming::template_param(s, p, "template", "templateName", c)?.ok_or_else(|| bad(c, "missing `template` (or `templateName`)"))?;
    let start = p.get("start").and_then(Value::as_u64).unwrap_or(1) as usize;
    let batch = super::naming::batch_param(p, c)?;
    Ok((s.targets(p), template, start, batch))
}

/// A named set of colour-label names (red, yellow, green, blue, purple; empty = the colour's name).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LabelSet {
    pub name: String,
    pub names: [String; 5],
}

/// Our built-in sets.
pub fn builtin_label_sets() -> Vec<LabelSet> {
    let set = |name: &str, n: [&str; 5]| LabelSet { name: name.into(), names: n.map(str::to_string) };
    vec![set("Colors", ["", "", "", "", ""]), set("Review", ["Reject", "Needs Work", "Approved", "Retouch", "Print"])]
}

fn all_label_sets(s: &crate::Session) -> Vec<LabelSet> {
    builtin_label_sets().into_iter().chain(s.label_sets.iter().cloned()).collect()
}

/// The set whose names are the catalog's current ones.
fn current_label_set(s: &crate::Session) -> Option<String> {
    let cur: [String; 5] = ColorLabel::ALL.map(|l| s.catalog.custom_label_name(l).unwrap_or("").to_string());
    all_label_sets(s)
        .into_iter()
        .find(|x| x.names.iter().zip(&cur).all(|(a, b)| a.trim() == b.trim() || (a.trim().is_empty() && b.is_empty())))
        .map(|x| x.name)
}

/// Every set (built-ins first) and the one in use: `{sets: [{name, names, builtin}], current}`.
pub fn label_sets_json(s: &crate::Session) -> Value {
    let builtin = builtin_label_sets().len();
    let sets: Vec<Value> =
        all_label_sets(s).iter().enumerate().map(|(i, x)| json!({"name": x.name, "names": x.names, "builtin": i < builtin})).collect();
    json!({"sets": sets, "current": current_label_set(s)})
}

fn set_names_ops(s: &crate::Session, names: &[String; 5]) -> Vec<Op> {
    ColorLabel::ALL
        .iter()
        .zip(names)
        .filter_map(|(l, n)| {
            let n = n.trim();
            let name = (!n.is_empty() && !n.eq_ignore_ascii_case(&format!("{l:?}"))).then(|| n.to_string());
            (s.catalog.custom_label_name(*l) != name.as_deref()).then_some(Op::SetLabelName { label: *l, name })
        })
        .collect()
}

/// Geotag photos from a GPX track log by capture time (Lightroom Classic's Map ▸ Tracklog ▸
/// Auto-Tag Photos). GPX times are UTC; capture times are the camera's local clock, so a photo's
/// recorded zone (Exif `OffsetTimeOriginal`) or the `offset` parameter converts them.
fn auto_tag_tracklog(s: &mut crate::Session, p: &Value) -> Result<Value> {
    use lightcraft_meta::{DateTime, Match, parse_gpx};
    const C: &str = "photo.autoTagTracklog";
    let text = match (str_param(p, "gpx"), str_param(p, "path")) {
        (Some(t), _) => t.to_string(),
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| bad(C, format!("can't read {path}: {e}")))?,
        (None, None) => return Err(bad(C, "missing `path` (a .gpx file) or `gpx` (its text)")),
    };
    let log = parse_gpx(&text).map_err(|e| bad(C, e.to_string()))?;
    let (start, end) = log.span().ok_or_else(|| bad(C, "the track log has no timed track points"))?;
    // minutes east of UTC; `None` = not given (zone-less capture times are then read as UTC)
    let offset: Option<i64> = match p.get("offset") {
        None | Some(Value::Null) => None,
        Some(Value::String(o)) if o.trim().is_empty() => None,
        Some(Value::Number(h)) => {
            let h = h.as_f64().unwrap_or(0.0);
            if h.abs() > 18.0 {
                return Err(bad(C, "offset is at most ±18 hours"));
            }
            Some((h * 60.0).round() as i64)
        }
        Some(Value::String(o)) => {
            Some(DateTime::parse_offset(o).ok_or_else(|| bad(C, format!("can't read offset `{o}` (e.g. +02:00 or -5)")))? as i64)
        }
        Some(_) => return Err(bad(C, "offset is `+HH:MM` or a number of hours")),
    };
    let max_gap = f64_or(p, "maxGap", 600.0);
    if max_gap.is_nan() || max_gap < 0.0 {
        return Err(bad(C, "maxGap must be ≥ 0 seconds"));
    }
    let replace = bool_or(p, "replace", false);
    let (mut no_time, mut outside, mut has_gps, mut assumed_utc, mut interpolated) = (0, 0, 0, 0, 0);
    let mut ops = Vec::new();
    let mut photos = Vec::new();
    for id in s.targets(p) {
        let Some(ph) = s.catalog.photo(id) else { continue };
        if ph.meta.gps.is_some() && !replace {
            has_gps += 1;
            continue;
        }
        let Some(dt) = ph.captured.as_deref().and_then(DateTime::parse_iso) else {
            no_time += 1;
            continue;
        };
        // UTC seconds: the recorded zone wins, else the caller's offset
        let utc = match dt.offset_minutes {
            Some(_) => dt.unix_seconds(),
            None => {
                if offset.is_none() {
                    assumed_utc += 1;
                }
                dt.unix_seconds() - offset.unwrap_or(0) * 60
            }
        } as f64
            + dt.millis as f64 / 1000.0;
        let Some((g, m)) = log.locate(utc, max_gap) else {
            outside += 1;
            continue;
        };
        if m == Match::Interpolated {
            interpolated += 1;
        }
        let pos = ((g.latitude * 1e7).round() / 1e7, (g.longitude * 1e7).round() / 1e7);
        photos.push(json!({"id": id.0, "gps": [pos.0, pos.1], "match": if m == Match::Interpolated { "interpolated" } else { "nearest" }}));
        if ph.meta.gps != Some(pos) {
            let mut meta = ph.meta.clone();
            meta.gps = Some(pos);
            ops.push(Op::SetMeta { id, meta: Box::new(meta) });
        }
    }
    let tagged = photos.len();
    if !bool_or(p, "dryRun", false) && !ops.is_empty() {
        s.commit("Auto-Tag from Tracklog", Op::Batch { ops })?;
    }
    let iso = |t: f64| format!("{}Z", lightcraft_catalog::dates::civil(t.floor() as i64));
    Ok(json!({
        "tagged": tagged,
        "interpolated": interpolated,
        "nearest": tagged - interpolated,
        "skipped": {"noTime": no_time, "outside": outside, "hasGps": has_gps},
        "assumedUtc": assumed_utc,
        "points": log.points.len(),
        "start": iso(start),
        "end": iso(end),
        "photos": photos,
    }))
}

/// Edit Capture Time ▸ Revert to Original / Change to File's Creation Date (L16): each photo's time
/// from its file (the metadata it records, or the file system's creation date). Photos without a
/// readable file are skipped. One undo step.
fn capture_time_from_files(s: &mut crate::Session, targets: &[lightcraft_catalog::PhotoId], file_date: bool) -> Result<Value> {
    let mut ops = Vec::new();
    let mut skipped = Vec::new();
    for id in targets {
        let Some(ph) = s.catalog.photo(*id) else { continue };
        let lightcraft_catalog::Source::File { path } = &ph.source else {
            skipped.push(json!({"id": id.0, "reason": "no file"}));
            continue;
        };
        let path = path.clone();
        let new = if file_date {
            std::fs::metadata(&path)
                .and_then(|m| m.created().or_else(|_| m.modified()))
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| lightcraft_catalog::dates::civil(d.as_secs() as i64))
        } else {
            crate::import::probe_paths(s, std::slice::from_ref(&path)).into_iter().next().and_then(|r| r.ok()).and_then(|i| i.captured)
        };
        match new {
            Some(t) if s.catalog.photo(*id).and_then(|p| p.captured.as_deref()) != Some(t.as_str()) => {
                ops.push(Op::SetCaptured { id: *id, captured: Some(t) })
            }
            Some(_) => {}
            None => {
                skipped.push(json!({"id": id.0, "reason": if file_date { "the file can't be read" } else { "the file records no capture time" }}))
            }
        }
    }
    let n = ops.len();
    if n > 0 {
        s.commit(if file_date { "Change Capture Time to File Date" } else { "Revert Capture Time" }, Op::Batch { ops })?;
    }
    Ok(json!({"changed": n, "skipped": skipped}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "photo.renameTokens", "Rename Template Tags", [], None, "{} → {tokens: [{tag, aliases, meaning, example}], dateDirectives: [{directive, meaning}], notes: [..], sample} — the file-name template tags shared by photo.rename, library.import (rename) and app.export (naming); examples are for a sample photo", always, |_, _| {
            Ok(crate::rename::token_help_json())
        }),
        cmd!(query "photo.renamePreview", "Rename Preview", [], None, "{template: e.g. `{date}_{name}`, `Trip-{seq:3}` (tokens: {name} {num} {seq} {date} {folder} {camera} {lens} {iso} {rating} {title} {creator} {ext} {EXT} {original} {originalnum} {copyname} {text} {shoot} {total} {import} {image} {yyyymmdd} {yymmdd} {yyyy} {yy} {month} {mon} {mm} {dd} {julian} {hour} {minute} {second} {longdate} {caption} {copyright} {location} {city} {state} {country} {make} {model} {focal} {aperture} {shutter} {dimensions} {label}, {seq:N}, {date:%Y%m%d}; photo.renameTokens explains each), or templateName: a naming.templates name (Lightroom's ten built-ins or a saved one), text?: Custom Text, shoot?: Shoot Name, extCase?: keep|lower|upper, start?: first sequence number (1), ids?} — renames the files on disk (sidecars too, never overwriting: collisions get -1, -2…); undoable", has_selection, |s, p| {
            let (ids, template, start, batch) = rename_args(s, p, "photo.renamePreview")?;
            Ok(serde_json::to_value(s.plan_rename_in(&ids, &template, start, &batch)).unwrap_or_default())
        }),
        cmd!(
            "photo.rename",
            "Rename Photos",
            [],
            None,
            "{template: e.g. `{date}_{name}`, `Trip-{seq:3}` (tokens: {name} {num} {seq} {date} {folder} {camera} {lens} {iso} {rating} {title} {creator} {ext} {EXT} {original} {originalnum} {copyname} {text} {shoot} {total} {import} {image} {yyyymmdd} {yymmdd} {yyyy} {yy} {month} {mon} {mm} {dd} {julian} {hour} {minute} {second} {longdate} {caption} {copyright} {location} {city} {state} {country} {make} {model} {focal} {aperture} {shutter} {dimensions} {label}, {seq:N}, {date:%Y%m%d}; photo.renameTokens explains each), or templateName: a naming.templates name (Lightroom's ten built-ins or a saved one), text?: Custom Text, shoot?: Shoot Name, extCase?: keep|lower|upper, start?: first sequence number (1), ids?} — renames the files on disk (sidecars too, never overwriting: collisions get -1, -2…); undoable",
            has_selection,
            |s, p| {
                let (ids, template, start, batch) = rename_args(s, p, "photo.rename")?;
                let plans = s.plan_rename_in(&ids, &template, start, &batch);
                let n = s.apply_rename(&plans)?;
                Ok(json!({"renamed": n, "plans": plans}))
            }
        ),
        cmd!(
            "photo.setCaptureTime",
            "Edit Capture Time",
            [],
            None,
            "{ids?, anchor?: id (the photo `time` is for: default the active photo), time?: `2026-09-30 14:05:00`, `3:34 pm`, `15:34`, `2026-06-14 3:34 pm`, or a step `+1h` `-30m` `+1d` `+1h30m` (the anchor gets it, the others shift by the same amount; seconds not typed are kept), each?: bool (every photo gets `time`), shift?: seconds, hours?: time-zone shift in hours, match?: id (match camera clocks: the anchor takes this photo's time — one from another camera taken at the same moment — and the selected photos from the anchor's camera move by the difference), revert?: bool (each photo back to the time its file records — Lightroom's Revert Capture Time to Original), fileDate?: bool (each photo to its file's creation date, UTC), dryRun?: bool (only the plan)} → {changed, captured: [..], anchor, delta, photos: [{id, from, to, follows}], skipped: [{id, reason}], copies, stacked} — virtual copies and originals move with the photo, photos in a collapsed stack with its top; photos without a capture time stay as they are when the others shift; one undo step",
            has_selection,
            |s, p| {
                let c = "photo.setCaptureTime";
                if bool_or(p, "revert", false) || bool_or(p, "fileDate", false) {
                    let targets: Vec<_> = s.targets(p).into_iter().filter(|id| s.catalog.photo(*id).is_some()).collect();
                    if targets.is_empty() {
                        return Err(bad(c, "no photos"));
                    }
                    let all: Vec<_> = crate::capture_time::with_followers(s, &targets).into_iter().map(|(id, _)| id).collect();
                    return capture_time_from_files(s, &all, bool_or(p, "fileDate", false));
                }
                let plan = crate::capture_time::plan(s, p)?;
                let mut out = plan.to_json();
                out["captured"] = json!(plan.rows.iter().map(|r| r.to.clone()).collect::<Vec<_>>());
                let ops: Vec<Op> = plan.changed().map(|r| Op::SetCaptured { id: r.id, captured: Some(r.to.clone()) }).collect();
                let n = ops.len();
                out["changed"] = json!(n);
                if n > 0 && !bool_or(p, "dryRun", false) {
                    s.commit("Edit Capture Time", Op::Batch { ops })?;
                }
                Ok(out)
            }
        ),
        cmd!(
            "photo.autoTagTracklog",
            "Auto-Tag Photos from Tracklog",
            [],
            None,
            "{path?: GPX file | gpx?: GPX text, ids?, offset?: camera clock's UTC offset for photos whose capture time has no zone (`+02:00`, or hours; default UTC), maxGap?: seconds (600), replace?: bool (also photos that already have GPS), dryRun?: bool} → {tagged, interpolated, nearest, skipped: {noTime, outside, hasGps}, assumedUtc, points, start, end, photos: [{id, gps, match}]} — positions from the track log by capture time; one undo step",
            has_selection,
            auto_tag_tracklog
        ),
        cmd!(query "label.names", "Color Label Names", [], None, "{} → [{label, name, custom}]", always, |s, _| {
            Ok(json!(ColorLabel::ALL
                .iter()
                .map(|l| json!({"label": format!("{l:?}").to_lowercase(), "name": s.catalog.label_name(*l), "custom": s.catalog.custom_label_name(*l)}))
                .collect::<Vec<_>>()))
        }),
        cmd!(
            "label.setNames",
            "Edit Color Label Names",
            [],
            None,
            "{names: {red?: name|null, yellow?, green?, blue?, purple?}} — null or empty restores the colour's name",
            always,
            |s, p| {
                let names = p.get("names").and_then(Value::as_object).ok_or_else(|| bad("label.setNames", "missing `names`"))?;
                let mut ops = Vec::new();
                for (k, v) in names {
                    let label = ColorLabel::parse(k).ok_or_else(|| bad("label.setNames", format!("unknown label `{k}`")))?;
                    let name =
                        v.as_str().map(str::trim).filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case(&format!("{label:?}"))).map(str::to_string);
                    if s.catalog.custom_label_name(label) != name.as_deref() {
                        ops.push(Op::SetLabelName { label, name });
                    }
                }
                let n = ops.len();
                if n > 0 {
                    s.commit("Edit Label Names", Op::Batch { ops })?;
                }
                Ok(json!({"changed": n}))
            }
        ),
        cmd!(query "label.sets", "Color Label Sets", [], None, "{} → {sets: [{name, names: [red, yellow, green, blue, purple], builtin}], current: name|null}", always, |s, _| Ok(label_sets_json(s))),
        cmd!("label.applySet", "Apply Color Label Set", [], None, "{name} — use a set's label names (undoable)", always, |s, p| {
            let name = str_param(p, "name").ok_or_else(|| bad("label.applySet", "missing `name`"))?;
            let set = all_label_sets(s)
                .into_iter()
                .find(|x| x.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| bad("label.applySet", format!("no label set `{name}`")))?;
            let ops = set_names_ops(s, &set.names);
            let n = ops.len();
            if n > 0 {
                s.commit(&format!("Label Set: {}", set.name), Op::Batch { ops })?;
            }
            Ok(json!({"changed": n}))
        }),
        cmd!(
            "label.saveSet",
            "Save Color Label Set",
            [],
            None,
            "{name} — save the current label names as a set (replaces a user set of that name)",
            always,
            |s, p| {
                let name =
                    str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("label.saveSet", "missing `name`"))?.to_string();
                if builtin_label_sets().iter().any(|b| b.name.eq_ignore_ascii_case(&name)) {
                    return Err(bad("label.saveSet", format!("`{name}` is a built-in set")));
                }
                let set = LabelSet { name: name.clone(), names: ColorLabel::ALL.map(|l| s.catalog.custom_label_name(l).unwrap_or("").to_string()) };
                match s.label_sets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)) {
                    Some(x) => *x = set,
                    None => s.label_sets.push(set),
                }
                s.save_prefs()?;
                Ok(json!({"name": name}))
            }
        ),
        cmd!("label.deleteSet", "Delete Color Label Set", [], None, "{name} — user sets only", always, |s, p| {
            let name = str_param(p, "name").ok_or_else(|| bad("label.deleteSet", "missing `name`"))?;
            let before = s.label_sets.len();
            s.label_sets.retain(|x| !x.name.eq_ignore_ascii_case(name));
            if s.label_sets.len() == before {
                return Err(bad("label.deleteSet", format!("no user label set `{name}`")));
            }
            s.save_prefs()?;
            Ok(json!({"deleted": name}))
        }),
    ]
}
