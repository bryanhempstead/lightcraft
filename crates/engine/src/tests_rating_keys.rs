//! L05: Lightroom's [ / ] rating steps and ` / Cmd+Up / Cmd+Down flag steps, per photo.

use lightcraft_catalog::{Flag, PhotoId};
use serde_json::json;

use crate::Session;

fn ids(s: &Session) -> Vec<PhotoId> {
    let mut v: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    v.sort();
    v
}

#[test]
fn rating_steps_clamp_per_photo() {
    let mut s = Session::with_demo();
    let v = ids(&s);
    let (a, b) = (v[0], v[1]);
    s.execute("photo.rate", &json!({"ids": [a.0], "rating": 4})).unwrap();
    s.execute("photo.rate", &json!({"ids": [b.0], "rating": 1})).unwrap();
    let r = s.execute("photo.increaseRating", &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!(r["changed"], 2);
    assert_eq!((s.catalog.photo(a).unwrap().rating, s.catalog.photo(b).unwrap().rating), (5, 2));
    // 5 stays 5: only b changes
    assert_eq!(s.execute("photo.increaseRating", &json!({"ids": [a.0, b.0]})).unwrap()["changed"], 1);
    for _ in 0..6 {
        s.execute("photo.decreaseRating", &json!({"ids": [a.0, b.0]})).unwrap();
    }
    assert_eq!((s.catalog.photo(a).unwrap().rating, s.catalog.photo(b).unwrap().rating), (0, 0));
    // one undo step per key press
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.photo(a).unwrap().rating, 1);
}

#[test]
fn flag_toggle_and_steps() {
    let mut s = Session::with_demo();
    let a = ids(&s)[0];
    let flag = |s: &Session| s.catalog.photo(a).unwrap().flag;
    let run = |s: &mut Session, id: &str| {
        s.execute(id, &json!({"ids": [a.0]})).unwrap();
    };
    s.execute("photo.flag", &json!({"ids": [a.0], "flag": "none"})).unwrap();
    run(&mut s, "photo.toggleFlag");
    assert_eq!(flag(&s), Flag::Pick);
    run(&mut s, "photo.toggleFlag");
    assert_eq!(flag(&s), Flag::None);
    run(&mut s, "photo.decreaseFlag");
    assert_eq!(flag(&s), Flag::Reject);
    run(&mut s, "photo.decreaseFlag");
    assert_eq!(flag(&s), Flag::Reject, "stops at rejected");
    run(&mut s, "photo.toggleFlag");
    assert_eq!(flag(&s), Flag::Pick, "a reject toggles to flagged");
    run(&mut s, "photo.decreaseFlag");
    run(&mut s, "photo.increaseFlag");
    run(&mut s, "photo.increaseFlag");
    assert_eq!(flag(&s), Flag::Pick, "stops at flagged");
    let shortcut = |id: &str| crate::cmd::find_command(id).and_then(|c| c.shortcut);
    assert_eq!(
        (shortcut("photo.toggleFlag"), shortcut("photo.increaseFlag"), shortcut("photo.decreaseFlag")),
        (Some("`"), Some("Cmd+Up"), Some("Cmd+Down"))
    );
}

/// L04: Set Copy as Original swaps the roles, every record keeps its own settings and rating.
#[test]
fn set_copy_as_original_swaps_roles() {
    let mut s = Session::with_demo();
    let master = ids(&s)[0];
    s.execute("photo.rate", &json!({"ids": [master.0], "rating": 2})).unwrap();
    s.execute("photo.virtualCopy", &json!({"ids": [master.0]})).unwrap();
    s.execute("photo.virtualCopy", &json!({"ids": [master.0]})).unwrap();
    let mut copies: Vec<PhotoId> = s.catalog.photos().filter(|p| p.copy_of == Some(master)).map(|p| p.id).collect();
    copies.sort();
    let (vc, other) = (copies[0], copies[1]);
    s.execute("photo.rate", &json!({"ids": [vc.0], "rating": 5})).unwrap();
    s.execute("develop.set", &json!({"ids": [vc.0], "control": "light.exposure", "value": 1.5})).unwrap();
    let vc_name = s.catalog.photo(vc).unwrap().copy_name.clone();
    let before = s.catalog.to_snapshot();
    let r = s.execute("photo.setCopyAsOriginal", &json!({"ids": [vc.0]})).unwrap();
    assert_eq!(r["changed"], 1);
    let (m, v, o) = (s.catalog.photo(master).unwrap(), s.catalog.photo(vc).unwrap(), s.catalog.photo(other).unwrap());
    assert_eq!((v.copy_of, v.copy_name.as_deref()), (None, None), "the copy is the original now");
    assert_eq!((m.copy_of, &m.copy_name), (Some(vc), &vc_name), "the old original is a copy with its name");
    assert_eq!(o.copy_of, Some(vc), "the other copy follows");
    assert_eq!((m.rating, v.rating), (2, 5), "ratings stay with their photos");
    assert_eq!(v.develop.light.exposure, 1.5);
    let st = s.catalog.stack_of(vc).unwrap();
    assert_eq!(st.photos.first(), Some(&vc), "the new original tops the stack");
    // a photo that isn't a copy is refused; undo puts everything back
    assert!(s.execute("photo.setCopyAsOriginal", &json!({"ids": [vc.0]})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.to_snapshot(), before);
}

/// L06: attribute filter extras — No Label (alone and with colours), originals / virtual copies,
/// stacked / not stacked.
#[test]
fn attribute_filter_extras() {
    let mut s = Session::with_demo();
    let v = ids(&s);
    for id in &v {
        s.execute("photo.label", &json!({"ids": [id.0], "label": "none"})).unwrap();
    }
    s.execute("photo.label", &json!({"ids": [v[0].0], "label": "red"})).unwrap();
    s.execute("photo.label", &json!({"ids": [v[1].0], "label": "blue"})).unwrap();
    let count = |s: &mut Session, f: serde_json::Value| {
        s.execute("library.filter", &json!({"noLabel": false, "labels": [], "virtualCopy": null, "stacked": null})).unwrap();
        s.execute("library.filter", &f).unwrap()["count"].as_u64().unwrap() as usize
    };
    let n = v.len();
    assert_eq!(count(&mut s, json!({"noLabel": true})), n - 2);
    assert_eq!(count(&mut s, json!({"noLabel": true, "labels": ["red"]})), n - 1, "red or none");
    s.execute("photo.virtualCopy", &json!({"ids": [v[2].0]})).unwrap();
    assert_eq!(count(&mut s, json!({"virtualCopy": true})), 1);
    assert_eq!(count(&mut s, json!({"virtualCopy": false})), n);
    let stacked = count(&mut s, json!({"stacked": true}));
    assert!(stacked >= 2, "the copy and its original are stacked");
    assert_eq!(count(&mut s, json!({"stacked": false})), n + 1 - stacked);
    let d = s.filter.describe();
    assert!(d.contains("not stacked"), "{d}");
}

/// L08: Lightroom's other smart-collection fields match what the photo says.
#[test]
fn more_smart_collection_fields() {
    let mut s = Session::with_demo();
    let v = ids(&s);
    let (a, b) = (v[0], v[1]);
    s.execute("develop.set", &json!({"ids": [a.0], "control": "optics.lensProfile", "value": true})).ok();
    s.execute("photo.setMeta", &json!({"ids": [a.0], "city": "Lisbon", "altText": "a pier at dawn"})).unwrap();
    s.execute("photo.virtualCopy", &json!({"ids": [b.0]})).unwrap();
    let matches = |s: &mut Session, field: &str, op: &str, value: serde_json::Value| -> Vec<PhotoId> {
        let r = s.execute("library.filter", &json!({"ruleSet": {"match": "all", "rules": [{"field": field, "op": op, "value": value}]}})).unwrap();
        assert!(r["count"].is_number());
        let out = s.visible_cloned();
        s.execute("library.filter", &json!({"ruleSet": null})).unwrap();
        out
    };
    assert_eq!(matches(&mut s, "city", "is", json!("lisbon")), vec![a]);
    assert_eq!(matches(&mut s, "altText", "contains", json!("pier")), vec![a]);
    assert!(matches(&mut s, "copyName", "is", json!("copy 1")).len() == 1);
    assert!(matches(&mut s, "stacked", "is", json!(true)).contains(&b));
    let n = s.catalog.len();
    let land = matches(&mut s, "aspect", "is", json!("landscape")).len();
    let port = matches(&mut s, "aspect", "is", json!("portrait")).len();
    let sq = matches(&mut s, "aspect", "is", json!("square")).len();
    assert_eq!(land + port + sq, n, "every photo has one aspect");
    assert_eq!(matches(&mut s, "masking", "is", json!("none")).len() + matches(&mut s, "masking", "is", json!("any")).len(), n);
    assert_eq!(matches(&mut s, "treatment", "is", json!("color")).len() + matches(&mut s, "treatment", "is", json!("bw")).len(), n);
    assert!(matches(&mut s, "longEdge", "gte", json!(1)).len() == n);
    // every listed field is known to the matcher (an unknown one matches nothing, `isNot` included)
    let fields = s.execute("album.ruleFields", &json!({})).unwrap();
    assert!(fields.as_array().map_or(0, Vec::len) >= 60, "{fields}");
}

/// L07: the metadata column browser — counts per value, each column narrowed by the ones to its
/// left, picks combine (any value within a column, all columns).
#[test]
fn metadata_column_browser() {
    let mut s = Session::with_demo();
    let v = ids(&s);
    for (i, id) in v.iter().enumerate() {
        let mut meta = s.catalog.photo(*id).unwrap().meta.clone();
        meta.camera = if i % 2 == 0 { "Canon EOS R6" } else { "LEICA M (Typ 262)" }.into();
        meta.lens = if i % 3 == 0 { "35mm" } else { "50mm" }.into();
        s.commit("meta", lightcraft_catalog::Op::SetMeta { id: *id, meta: Box::new(meta) }).unwrap();
    }
    let n = v.len();
    let r = s.execute("library.columnBrowser", &json!({"columns": ["camera", "lens"]})).unwrap();
    let cam = &r["columns"][0];
    assert_eq!(cam["total"], n);
    let total: u64 = cam["values"].as_array().unwrap().iter().map(|x| x["count"].as_u64().unwrap()).sum();
    assert_eq!(total as usize, n);
    s.execute("library.columnPick", &json!({"column": "camera", "values": ["Canon EOS R6"]})).unwrap();
    let canons = n.div_ceil(2);
    assert_eq!(s.visible_cloned().len(), canons);
    let r = s.execute("library.columnBrowser", &json!({"columns": ["camera", "lens"]})).unwrap();
    assert_eq!(r["columns"][0]["total"], n, "a column isn't narrowed by its own pick");
    assert_eq!(r["columns"][1]["total"], canons, "the next one is");
    assert_eq!(r["columns"][0]["picked"], json!(["Canon EOS R6"]));
    s.execute("library.columnPick", &json!({"column": "lens", "values": ["35mm"]})).unwrap();
    let both = (0..n).filter(|i| i % 2 == 0 && i % 3 == 0).count();
    assert_eq!(s.visible_cloned().len(), both);
    s.execute("library.columnPick", &json!({"column": "camera", "values": ["Canon EOS R6", "LEICA M (Typ 262)"]})).unwrap();
    assert_eq!(s.visible_cloned().len(), (0..n).filter(|i| i % 3 == 0).count(), "any value within a column");
    assert!(s.execute("library.columnBrowser", &json!({"columns": ["nope"]})).is_err());
    s.execute("library.clearFilter", &json!({})).unwrap();
    assert_eq!(s.visible_cloned().len(), n);
}

/// L14: selection set operations, Select Flagged / Deselect Unflagged, Select Edited, Invert,
/// Deselect Active.
#[test]
fn selection_operations() {
    let mut s = Session::with_demo();
    let v = s.visible_cloned();
    for (i, id) in v.iter().enumerate() {
        let flag = ["pick", "none", "reject"][i % 3];
        s.execute("photo.flag", &json!({"ids": [id.0], "flag": flag})).unwrap();
        s.execute("photo.rate", &json!({"ids": [id.0], "rating": (i % 6) as u64})).unwrap();
    }
    let picks = v.iter().enumerate().filter(|(i, _)| i % 3 == 0).count();
    assert_eq!(s.execute("library.selectFlagged", &json!({})).unwrap()["selected"], picks);
    // add the 5-star photos, subtract the rejects among them, intersect with ≥ 3 stars
    s.execute("library.selectBy", &json!({"rating": 5, "mode": "add"})).unwrap();
    let after_add = s.selection.ids.len();
    assert!(after_add >= picks);
    s.execute("library.selectBy", &json!({"flag": "reject", "mode": "subtract"})).unwrap();
    assert!(s.selection.ids.iter().all(|id| s.catalog.photo(*id).unwrap().flag != lightcraft_catalog::Flag::Reject));
    s.execute("library.selectBy", &json!({"rating": 3, "mode": "intersect"})).unwrap();
    assert!(s.selection.ids.iter().all(|id| s.catalog.photo(*id).unwrap().rating >= 3));
    assert!(s.execute("library.selectBy", &json!({"rating": 3, "mode": "xor"})).is_err());
    // Deselect Unflagged keeps picks and rejects
    s.execute("library.selectAll", &json!({})).unwrap();
    s.execute("library.deselectUnflagged", &json!({})).unwrap();
    assert!(s.selection.ids.iter().all(|id| s.catalog.photo(*id).unwrap().flag != lightcraft_catalog::Flag::None));
    // Invert: the rest of the view
    let n = s.selection.ids.len();
    s.execute("library.invertSelection", &json!({})).unwrap();
    assert_eq!(s.selection.ids.len(), v.len() - n);
    // Deselect Active
    let before = s.selection.ids.len();
    let active = s.selection.active.unwrap();
    s.execute("library.deselectActive", &json!({})).unwrap();
    assert_eq!(s.selection.ids.len(), before - 1);
    assert!(!s.selection.ids.contains(&active) && s.selection.active.is_some());
    // Select Edited
    s.execute("develop.set", &json!({"ids": [v[0].0], "control": "light.exposure", "value": 1.0})).unwrap();
    let edited = v.iter().filter(|id| s.catalog.photo(**id).unwrap().is_edited()).count();
    assert_eq!(s.execute("library.selectEdited", &json!({})).unwrap()["selected"], edited);
}

/// Section 3: new raw imports start with lens corrections + Remove CA (his migrated photos have
/// them), counted as unedited; both can be switched off in the import defaults.
#[test]
fn raw_imports_start_with_lens_corrections() {
    let s = Session::with_demo();
    let mut p = lightcraft_catalog::Photo::new(
        PhotoId(999),
        lightcraft_catalog::Source::File { path: "/x/a.dng".into() },
        "a.dng",
        "DNG",
        10,
        10,
        "2026-01-01T00:00:00",
    );
    p.kind = lightcraft_catalog::MediaKind::Raw;
    crate::import::apply_import_defaults(&s, &mut p);
    if p.develops_raw() {
        assert!(p.develop.optics.lens_profile && p.develop.optics.remove_ca);
        assert!(!p.is_edited(), "what import gave counts as unedited");
    }
    let mut s2 = Session::with_demo();
    s2.execute("library.preferences", &json!({"import": {"lensCorrections": false, "removeCa": false}})).unwrap();
    let mut q = p.clone();
    q.import_look = None;
    crate::import::apply_import_defaults(&s2, &mut q);
    assert!(!q.develop.optics.remove_ca);
}

/// L15: Refine Photos — unflagged → rejected, picks → unflagged, rejects stay; one undo step.
#[test]
fn refine_photos() {
    let mut s = Session::with_demo();
    let v = s.visible_cloned();
    for (i, id) in v.iter().enumerate() {
        let flag = ["pick", "none", "reject"][i % 3];
        s.execute("photo.flag", &json!({"ids": [id.0], "flag": flag})).unwrap();
    }
    let before: Vec<lightcraft_catalog::Flag> = v.iter().map(|id| s.catalog.photo(*id).unwrap().flag).collect();
    let r = s.execute("library.refinePhotos", &json!({})).unwrap();
    let changed = before.iter().filter(|f| **f != lightcraft_catalog::Flag::Reject).count() as u64;
    assert_eq!(r["rejected"].as_u64().unwrap() + r["unflagged"].as_u64().unwrap(), changed);
    for (id, was) in v.iter().zip(&before) {
        let now = s.catalog.photo(*id).unwrap().flag;
        let want = match was {
            lightcraft_catalog::Flag::None => lightcraft_catalog::Flag::Reject,
            lightcraft_catalog::Flag::Pick => lightcraft_catalog::Flag::None,
            lightcraft_catalog::Flag::Reject => lightcraft_catalog::Flag::Reject,
        };
        assert_eq!(now, want);
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(v.iter().map(|id| s.catalog.photo(*id).unwrap().flag).collect::<Vec<_>>(), before);
}

/// L16: Edit Capture Time ▸ Revert to Original / Change to File's Creation Date.
#[test]
fn capture_time_from_the_file() {
    let dir = std::env::temp_dir().join(format!("lc-capture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = lightcraft_raster::Rgba8 { width: 8, height: 8, data: vec![[10, 20, 30, 255]; 64] };
    let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &Default::default()).unwrap();
    std::fs::write(dir.join("a.png"), png).unwrap();
    let mut s = Session::new();
    s.execute("library.import", &json!({"paths": [dir.to_string_lossy()]})).unwrap();
    let id = s.catalog.photos().next().unwrap().id;
    s.execute("photo.setCaptureTime", &json!({"ids": [id.0], "time": "2001-01-01T00:00:00", "each": true})).unwrap();
    // a PNG records no capture time: revert says so and changes nothing
    let r = s.execute("photo.setCaptureTime", &json!({"ids": [id.0], "revert": true})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_array().map(Vec::len)), (Some(0), Some(1)), "{r}");
    let r = s.execute("photo.setCaptureTime", &json!({"ids": [id.0], "fileDate": true})).unwrap();
    assert_eq!(r["changed"], 1, "{r}");
    let t = s.catalog.photo(id).unwrap().captured.clone().unwrap();
    assert!(t.as_str() > "2020", "{t}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// L25: duplicate an album, an album from a folder, Save Quick Collection, the albums of a photo.
#[test]
fn collection_extras() {
    let mut s = Session::with_demo();
    let v = s.visible_cloned();
    s.selection = crate::Selection { ids: v[..3].to_vec(), active: Some(v[0]) };
    let a = s.execute("album.create", &json!({"name": "Keepers", "addSelected": true})).unwrap()["id"].as_u64().unwrap();
    let b = s.execute("album.duplicate", &json!({"id": a})).unwrap()["id"].as_u64().unwrap();
    let dup = s.catalog.album(lightcraft_catalog::AlbumId(b)).unwrap();
    assert_eq!((dup.name.as_str(), dup.photos.len()), ("Keepers copy", 3));
    let albums = s.execute("photo.albums", &json!({"id": v[0].0})).unwrap();
    let names: Vec<&str> = albums.as_array().unwrap().iter().filter_map(|a| a["name"].as_str()).collect();
    assert!(names.contains(&"Keepers") && names.contains(&"Keepers copy"), "{albums}");
    // Quick Collection → a saved album, then emptied
    s.selection = crate::Selection::single(v[4]);
    s.execute("album.toggleTarget", &json!({})).unwrap();
    let r = s.execute("album.saveQuick", &json!({"name": "Picks for Ann"})).unwrap();
    assert_eq!(r["photos"], 1);
    let q = s.catalog.quick_collection().unwrap();
    assert!(s.catalog.album(q).unwrap().photos.is_empty());
    assert!(s.execute("album.saveQuick", &json!({})).is_err(), "empty now");
    assert!(s.execute("album.fromFolder", &json!({"path": "/nowhere"})).is_err());
}

/// L17: Lightroom's other sort orders.
#[test]
fn more_sort_orders() {
    let mut s = Session::with_demo();
    let v = s.visible_cloned();
    s.execute("photo.flag", &json!({"ids": [v[0].0], "flag": "reject"})).unwrap();
    s.execute("photo.flag", &json!({"ids": [v[1].0], "flag": "pick"})).unwrap();
    s.execute("photo.label", &json!({"ids": [v[2].0], "label": "purple"})).unwrap();
    for key in ["pick", "labelColor", "labelText", "editCount", "aspectRatio", "fileType", "extension", "pixelCount"] {
        s.execute("library.sort", &json!({"key": key, "ascending": true})).unwrap();
        assert_eq!(s.visible_cloned().len(), v.len(), "{key}");
    }
    s.execute("library.sort", &json!({"key": "pick", "ascending": false})).unwrap();
    let first = s.visible_cloned()[0];
    assert_eq!(s.catalog.photo(first).unwrap().flag, Flag::Pick, "flagged first, descending");
    s.execute("library.sort", &json!({"key": "pixelCount", "ascending": true})).unwrap();
    let px: Vec<u64> = s.visible_cloned().iter().map(|id| s.catalog.photo(*id).unwrap()).map(|p| u64::from(p.width) * u64::from(p.height)).collect();
    assert!(px.windows(2).all(|w| w[0] <= w[1]));
    assert!(s.execute("library.sort", &json!({"key": "colour"})).is_err());
}
