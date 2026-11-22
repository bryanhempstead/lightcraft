//! Library preferences: import defaults (raw / other / per camera) applied by import, and their
//! persistence in prefs.json.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lightcraft_catalog::MediaKind;
use lightcraft_develop::Preset;
use serde_json::json;

use crate::Session;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-prefs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn user_preset(id: &str, settings: serde_json::Value) -> Preset {
    Preset { id: id.into(), name: id.into(), group: "User Presets".into(), settings, favorite: false, builtin: false }
}

fn with_presets(s: &mut Session) {
    s.presets.push(user_preset("user.rawlook", json!({"light": {"exposure": 0.5}, "effects": {"clarity": 20.0}})));
    s.presets.push(user_preset("user.canonlook", json!({"light": {"contrast": 35.0}})));
    s.presets.push(user_preset("user.jpeglook", json!({"color": {"vibrance": 15.0}})));
}

fn dng(dir: &Path, name: &str, make: &str, model: &str) {
    let meta = lightcraft_meta::Metadata { make: Some(make.into()), model: Some(model.into()), ..Default::default() };
    std::fs::write(dir.join(name), crate::tests_xmp::synthetic_dng_with(None, meta)).unwrap();
}

fn png(path: &Path) {
    let (w, h) = (24usize, 16usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i * 7) as u8, (i * 3) as u8, 90, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn photo_named<'a>(s: &'a Session, name: &str) -> &'a lightcraft_catalog::Photo {
    s.catalog.photos().find(|p| p.file_name == name).unwrap_or_else(|| panic!("{name} not imported"))
}

#[test]
fn raw_and_other_defaults_are_applied_on_import() {
    let src = temp_dir("apply");
    dng(&src, "nikon.dng", "Nikon", "Z 9");
    dng(&src, "canon.dng", "Canon", "EOS R5");
    png(&src.join("plain.png"));
    let mut s = Session::new().with_fs();
    with_presets(&mut s);
    let r = s
        .execute(
            "library.preferences",
            &json!({"import": {"rawPreset": "user.rawlook", "otherPreset": "user.jpeglook", "perCamera": true,
                "cameras": [{"camera": "Canon EOS R5", "preset": "user.canonlook"}]}}),
        )
        .unwrap();
    assert_eq!(r["import"]["rawPreset"], "user.rawlook", "{r}");
    assert!(s.execute("library.preferences", &json!({"import": {"rawPreset": "nope"}})).is_err(), "unknown presets are rejected");

    s.execute("library.import", &json!({"paths": [src.to_string_lossy()]})).unwrap();
    assert_eq!(s.catalog.len(), 3);
    let nikon = photo_named(&s, "nikon.dng");
    assert_eq!(nikon.kind, MediaKind::Raw);
    assert_eq!(nikon.meta.camera, "Nikon Z 9");
    assert_eq!((nikon.develop.light.exposure, nikon.develop.effects.clarity), (0.5, 20.0), "global raw default");
    assert_eq!(nikon.develop.detail.sharpen_amount, 40.0, "raw camera defaults stay underneath the preset");
    assert!(!nikon.is_edited(), "the default look counts as unedited");
    assert!(!crate::import::has_import_look(nikon), "a preset look isn't what the embedded preview shows");
    let canon = photo_named(&s, "canon.dng");
    assert_eq!((canon.develop.light.contrast, canon.develop.light.exposure), (35.0, 0.0), "per-camera default wins");
    let plain = photo_named(&s, "plain.png");
    assert_eq!((plain.develop.color.vibrance, plain.develop.light.exposure), (15.0, 0.0), "non-raw default");

    // editing then Reset returns to the default look
    let id = nikon.id;
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 2.0})).unwrap();
    assert!(s.catalog.photo(id).unwrap().is_edited());
    s.execute("develop.reset", &json!({})).unwrap();
    let p = s.catalog.photo(id).unwrap();
    assert_eq!(p.develop.light.exposure, 0.5);
    assert!(!p.is_edited());

    // per-camera off: the Canon gets the global raw default; clearing it gives the plain defaults
    let mut s2 = Session::new().with_fs();
    with_presets(&mut s2);
    s2.execute("library.preferences", &json!({"camera": {"camera": "canon eos r5", "preset": "user.canonlook"}})).unwrap();
    s2.execute("library.import", &json!({"paths": [src.join("canon.dng").to_string_lossy()]})).unwrap();
    let c = s2.catalog.photos().next().unwrap();
    assert_eq!(c.develop.light.contrast, 0.0, "per-camera defaults are off unless enabled");
    // only the raw defaults (lens corrections, Remove CA) come with the import, no preset look
    assert!(c.import_look.as_ref().is_none_or(|l| l.light == lightcraft_develop::DevelopSettings::default().light));
    let _ = std::fs::remove_dir_all(&src);
}

#[test]
fn preferences_persist_with_the_library() {
    let lib = temp_dir("lib");
    {
        let mut s = Session::new().with_fs();
        s.open_library(&lib, false).unwrap();
        with_presets(&mut s);
        s.persist().unwrap(); // presets.json
        s.execute("library.xmpPreferences", &json!({"autoWrite": true})).unwrap();
        s.execute(
            "library.preferences",
            &json!({"import": {"rawPreset": "user.rawlook", "perCamera": true}, "camera": {"camera": "Canon EOS R5", "preset": "user.canonlook"}, "cacheMb": 512}),
        )
        .unwrap();
        s.close_library().unwrap();
    }
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    assert!(s.xmp.auto_write);
    assert_eq!(s.import_defaults.raw_preset.as_deref(), Some("user.rawlook"));
    assert!(s.import_defaults.per_camera);
    assert_eq!(s.import_defaults.cameras.len(), 1);
    assert_eq!(s.import_defaults.preset_for(true, "Canon EOS R5"), Some("user.canonlook"));
    assert_eq!(s.cache_mb, 512);
    assert_eq!(s.cache_bytes(), 512 << 20);
    let r = s.execute("library.preferences", &json!({})).unwrap();
    assert_eq!(r["cacheMb"], 512);
    assert_eq!(r["persistent"], true);
    // export presets are library preferences too
    s.execute("export.savePreset", &json!({"name": "Proof", "params": {"format": "jpeg", "percent": 25}})).unwrap();
    drop(s); // one session per library (issue #99)
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    assert_eq!(s.export_params(&json!({"preset": "Proof"})).unwrap(), json!({"format": "jpeg", "percent": 25}));
    // older prefs.json files (snake_case `last_export`, no import section) still load
    std::fs::write(lib.join("prefs.json"), r#"{"xmp": {"autoWrite": false}, "last_export": {"format": "png"}}"#).unwrap();
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    assert_eq!(s.last_export, Some(json!({"format": "png"})));
    assert_eq!(s.import_defaults, Default::default());
    let _ = std::fs::remove_dir_all(&lib);
}

#[test]
fn default_copyright_and_creator_fill_gaps_on_import() {
    let dir = temp_dir("copyright");
    let lib = dir.join("lib");
    png(&dir.join("plain.png"));
    // a DNG that names its own artist keeps it
    let meta = lightcraft_meta::Metadata { artist: Some("Someone Else".into()), ..Default::default() };
    std::fs::write(dir.join("own.dng"), crate::tests_xmp::synthetic_dng_with(None, meta)).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let r = s.execute("library.preferences", &json!({"import": {"copyright": " © 2026 Me ", "creator": "Me"}})).unwrap();
    assert_eq!(r["import"]["copyright"], "© 2026 Me");
    s.execute("library.import", &json!({"paths": [dir.join("plain.png").to_string_lossy(), dir.join("own.dng").to_string_lossy()]})).unwrap();
    let plain = photo_named(&s, "plain.png");
    assert_eq!((plain.meta.copyright.as_str(), plain.meta.creator.as_str()), ("© 2026 Me", "Me"));
    let own = photo_named(&s, "own.dng");
    assert_eq!(own.meta.creator, "Someone Else", "the file's own creator wins");
    assert_eq!(own.meta.copyright, "© 2026 Me");
    // saved with the library
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    assert_eq!(s.import_defaults.creator, "Me");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_presets_apply_to_photos_and_imports() {
    let dir = temp_dir("metapresets");
    let lib = dir.join("lib");
    png(&dir.join("new.png"));
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    s.execute("metadata.savePreset", &json!({"name": "Studio", "fields": {"copyright": "© Studio", "creator": "Ann", "keywords": ["studio"]}}))
        .unwrap();
    assert!(s.execute("metadata.savePreset", &json!({"name": "Bad", "fields": {"iso": "100"}})).is_err());
    // on import
    s.execute("library.preferences", &json!({"import": {"metadataPreset": "studio"}})).unwrap();
    assert!(s.execute("library.preferences", &json!({"import": {"metadataPreset": "nope"}})).is_err());
    s.execute("library.import", &json!({"paths": [dir.join("new.png").to_string_lossy()]})).unwrap();
    let p = photo_named(&s, "new.png");
    assert_eq!((p.meta.copyright.as_str(), p.meta.creator.as_str(), p.meta.keywords.clone()), ("© Studio", "Ann", vec!["studio".to_string()]));
    let id = p.id;
    // applied to a photo: text replaces, keywords add (one undo step)
    s.execute("photo.setMeta", &json!({"ids": [id.0], "keywords": ["mine"], "creator": "Bob"})).unwrap();
    s.execute("metadata.applyPreset", &json!({"name": "Studio", "ids": [id.0]})).unwrap();
    let m = &s.catalog.photo(id).unwrap().meta;
    assert_eq!(m.creator, "Ann");
    assert_eq!(m.keywords, vec!["mine".to_string(), "studio".to_string()]);
    // from the active photo, then persisted
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    s.execute("photo.setMeta", &json!({"city": "Lyon"})).unwrap();
    s.execute("metadata.savePreset", &json!({"name": "Here"})).unwrap();
    let saved = s.metadata_presets.iter().find(|m| m.name == "Here").unwrap().fields.clone();
    assert_eq!(saved, json!({"copyright": "© Studio", "creator": "Ann", "city": "Lyon"}));
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    assert_eq!(s.metadata_presets.len(), 2);
    assert_eq!(s.import_defaults.metadata_preset.as_deref(), Some("studio"));
    s.execute("metadata.deletePreset", &json!({"name": "STUDIO"})).unwrap();
    assert!(s.import_defaults.metadata_preset.is_none(), "deleting the import preset clears it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn curve_presets_save_apply_export_import_and_persist() {
    let lib = temp_dir("curves");
    let file = lib.join("out").join("looks");
    {
        let mut s = Session::new().with_fs();
        s.open_library(&lib, true).unwrap();
        let r = s.execute("curve.presets", &json!({})).unwrap();
        let names: Vec<&str> = r["presets"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
        assert_eq!(names, ["Linear", "Medium Contrast", "Strong Contrast"]);
        assert_eq!(r["current"], "Linear", "a fresh photo has linear point curves");
        // built-in: an S-curve on the master channel, parametric untouched
        s.execute("develop.set", &json!({"control": "curve.shadows", "value": 20})).unwrap();
        s.execute("curve.applyPreset", &json!({"name": "strong contrast"})).unwrap();
        let d = s.develop_of(s.active().unwrap()).unwrap();
        assert_eq!(d.curve.master.len(), 5);
        assert!(d.curve.master[1].y < 0.25 && d.curve.master[3].y > 0.75, "an S-curve");
        assert_eq!(d.curve.shadows, 20.0);
        assert_eq!(s.execute("curve.presets", &json!({})).unwrap()["current"], "Strong Contrast");
        // save a custom one (with a red channel)
        s.execute("develop.curve", &json!({"channel": "red", "points": [[0.0, 0.1], [1.0, 0.9]]})).unwrap();
        assert!(s.execute("curve.savePreset", &json!({"name": "Linear"})).is_err(), "built-in names are reserved");
        s.execute("curve.savePreset", &json!({"name": "Faded Red"})).unwrap();
        assert_eq!(s.execute("curve.presets", &json!({})).unwrap()["current"], "Faded Red");
        // Linear resets every point channel
        s.execute("curve.applyPreset", &json!({"name": "Linear"})).unwrap();
        let d = s.develop_of(s.active().unwrap()).unwrap();
        assert!(d.curve.master.is_empty() && d.curve.red.is_empty());
        s.execute("curve.applyPreset", &json!({"name": "Faded Red"})).unwrap();
        assert_eq!(s.develop_of(s.active().unwrap()).unwrap().curve.red.len(), 2);
        // export (extension added), delete, re-import
        let r = s.execute("curve.exportPresets", &json!({"path": file.to_string_lossy()})).unwrap();
        assert_eq!(r["count"], 1);
        let path = r["path"].as_str().unwrap().to_string();
        assert!(path.ends_with(".lccurve"));
        assert!(s.execute("curve.deletePreset", &json!({"name": "Medium Contrast"})).is_err());
        s.execute("curve.deletePreset", &json!({"name": "faded red"})).unwrap();
        assert!(s.curve_presets.is_empty());
        let r = s.execute("curve.importPresets", &json!({"path": path})).unwrap();
        assert_eq!(r["imported"], json!(["Faded Red"]));
        assert_eq!(s.curve_presets.len(), 1);
        // a file carrying a built-in name doesn't shadow it; junk fails cleanly
        let other = lib.join("one.lccurve");
        std::fs::write(&other, r#"{"name": "Linear", "master": [[0, 0.05], [1, 1]]}"#).unwrap();
        let junk = lib.join("junk.lccurve");
        std::fs::write(&junk, "not json").unwrap();
        let r = s.execute("curve.importPresets", &json!({"paths": [other.to_string_lossy(), junk.to_string_lossy()]})).unwrap();
        assert_eq!(r["imported"], json!(["Linear (imported)"]));
        assert_eq!(r["failed"].as_array().unwrap().len(), 1);
        s.close_library().unwrap();
    }
    // saved with the library preferences
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let names: Vec<String> = s.curve_presets.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, ["Faded Red", "Linear (imported)"]);
    let _ = std::fs::remove_dir_all(&lib);
}

#[test]
fn resizing_the_disk_cache_keeps_the_cache_and_its_thumbnails() {
    let lib = temp_dir("resize");
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let cache = s.media.rendered.clone();
    let key = lightcraft_preview::hash_bytes(b"thumbnail");
    let img = lightcraft_raster::Rgba8 { width: 4, height: 4, data: vec![[10, 20, 30, 255]; 16] };
    cache.put(key, Arc::new(img));
    let generation = cache.generation();
    s.execute("library.preferences", &json!({"cacheMb": 300})).unwrap();
    // the same cache object (the UI keeps its textures), the same generation, the new budget
    assert!(Arc::ptr_eq(&cache, &s.media.rendered));
    assert_eq!(s.media.rendered.generation(), generation);
    assert_eq!(s.media.rendered.disk().unwrap().budget(), 300 << 20);
    assert!(s.media.rendered.get(key).is_some());
    // an explicit clear still invalidates
    s.execute("library.clearPreviews", &json!({})).unwrap();
    assert_ne!(s.media.rendered.generation(), generation);
    assert!(s.media.rendered.get(key).is_none());
    drop(s);
    let _ = std::fs::remove_dir_all(&lib);
}

#[test]
fn reopening_a_library_retires_the_old_cache_so_its_jobs_cannot_write_after_a_clear() {
    let lib = temp_dir("reopen-writer");
    let mut s = Session::new().with_fs();
    s.open_library(&lib, true).unwrap();
    let id = s.catalog.photos().next().unwrap().id;
    // a thumbnail job still holding the first open's cache object
    let job = s.thumb_job(id, 128).unwrap();
    let (old, key) = job.cache.clone().unwrap();
    // a thumbnail written before the reopen is still valid afterwards
    let kept = lightcraft_preview::hash_bytes(b"kept thumbnail");
    let img = lightcraft_raster::Rgba8 { width: 4, height: 4, data: vec![[10, 20, 30, 255]; 16] };
    old.put(kept, Arc::new(img.clone()));
    s.open_library(&lib, false).unwrap();
    assert!(!Arc::ptr_eq(&old, &s.media.rendered));
    assert!(s.media.rendered.get(kept).is_some(), "a plain reopen keeps the disk cache");
    s.execute("library.clearPreviews", &json!({})).unwrap();
    // the old job finishes: its render must not land in the cleared directory
    let r = job.run();
    assert!(r.rendered.is_ok());
    old.put(kept, Arc::new(img));
    let fresh = lightcraft_preview::PreviewCache::with_disk(1 << 20, &lib.join("thumbs"), 1 << 30);
    assert!(fresh.get(key).is_none(), "stale thumbnail written through the replaced cache");
    assert!(fresh.get(kept).is_none(), "stale put through the replaced cache");
    assert!(s.media.rendered.get(key).is_none());
    drop(s);
    let _ = std::fs::remove_dir_all(&lib);
}

/// U01: Lightroom's per-setting copy checklist: copy just Exposure (not Contrast), sync just
/// Sharpening (not noise reduction), the Modified subset, choose masks, saved subsets persist.
#[test]
fn per_setting_copy_sync_modified_and_saved_subsets() {
    let lib = temp_dir("subsets");
    {
        let mut s = Session::new().with_fs();
        s.open_library(&lib, true).unwrap();
        let ids: Vec<_> = s.visible().iter().copied().take(3).collect();
        s.execute("library.select", &json!({"ids": [ids[0].0]})).unwrap();
        let v = json!({"light.exposure": 1.0, "light.contrast": 30, "detail.sharpenAmount": 90, "detail.nrLuminance": 40});
        s.execute("develop.set", &json!({"values": v})).unwrap();
        let groups = s.execute("develop.modifiedGroups", &json!({})).unwrap()["groups"].clone();
        for g in ["exposure", "contrast", "sharpening", "luminanceNoise"] {
            assert!(groups.as_array().unwrap().iter().any(|x| x == g), "{g} modified: {groups}");
        }
        assert!(!groups.as_array().unwrap().iter().any(|x| x == "highlights"), "{groups}");
        // copy only Exposure
        s.execute("develop.copy", &json!({"groups": ["exposure"]})).unwrap();
        s.execute("develop.paste", &json!({"ids": [ids[1].0]})).unwrap();
        let d = s.develop_of(ids[1]).unwrap();
        assert_eq!((d.light.exposure, d.light.contrast), (1.0, 0.0));
        // sync only Sharpening, from the active photo to the selection
        s.execute("library.select", &json!({"ids": [ids[0].0, ids[2].0]})).unwrap();
        s.execute("develop.sync", &json!({"groups": ["sharpening"]})).unwrap();
        let d = s.develop_of(ids[2]).unwrap();
        assert_eq!((d.detail.sharpen_amount, d.detail.nr_luminance, d.light.exposure), (90.0, 0.0, 0.0));
        // a heading id still works, unknown ids are an error, not a silent default
        s.execute("develop.copy", &json!({"groups": ["light"]})).unwrap();
        assert!(s.copy_groups.contains(&lightcraft_develop::SettingsGroup::Blacks));
        assert!(s.execute("develop.copy", &json!({"groups": ["exposur"]})).unwrap_err().to_string().contains("exposur"));
        // subsets: modified (on the active photo), saved ones by name
        let r = s.execute("develop.copy", &json!({"subset": "modified"})).unwrap();
        assert_eq!(r["groups"], groups);
        assert!(s.execute("develop.saveSubset", &json!({"name": "Default", "groups": ["grain"]})).is_err(), "built-in name");
        s.execute("develop.saveSubset", &json!({"name": "Tone only", "groups": ["light", "toneCurve"]})).unwrap();
        s.execute("develop.renameSubset", &json!({"name": "tone only", "to": "Tone"})).unwrap();
        let r = s.execute("develop.copy", &json!({"subset": "tone"})).unwrap();
        assert_eq!(r["groups"].as_array().unwrap().len(), 8, "{r}");
        assert!(s.execute("develop.copy", &json!({"subset": "nope"})).is_err());
        // masks: only the chosen one goes along
        s.execute("library.select", &json!({"ids": [ids[0].0]})).unwrap();
        s.execute("mask.add", &json!({"kind": "linear"})).unwrap();
        s.execute("mask.add", &json!({"kind": "radial"})).unwrap();
        let masks: Vec<u32> = s.develop_of(ids[0]).unwrap().masks.iter().map(|m| m.id).collect();
        assert_eq!(masks.len(), 2);
        s.execute("develop.copy", &json!({"groups": ["masks"], "masks": [masks[1]]})).unwrap();
        s.execute("develop.paste", &json!({"ids": [ids[1].0]})).unwrap();
        assert_eq!(s.develop_of(ids[1]).unwrap().masks.len(), 1);
        let list = s.execute("develop.settingsGroups", &json!({})).unwrap();
        assert!(list["headings"].as_array().unwrap().len() >= 12, "{list}");
        s.close_library().unwrap();
    }
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let r = s.execute("develop.subsets", &json!({})).unwrap();
    assert_eq!(r["subsets"][0]["name"], "Tone", "saved with the library preferences: {r}");
    s.execute("develop.deleteSubset", &json!({"name": "tone"})).unwrap();
    assert!(s.copy_subsets.is_empty());
    let _ = std::fs::remove_dir_all(&lib);
}

/// U02: relative paste (the changes since the first / a chosen history state move each photo's
/// own values) and merge-or-replace masks on paste.
#[test]
fn relative_paste_and_merge_or_replace_masks() {
    let mut s = Session::with_demo();
    let ids: Vec<_> = s.visible().iter().copied().take(3).collect();
    s.execute("library.select", &json!({"ids": [ids[1].0]})).unwrap();
    s.execute("develop.set", &json!({"values": {"light.exposure": 1.0, "light.contrast": 10}})).unwrap();
    s.execute("library.select", &json!({"ids": [ids[0].0]})).unwrap();
    s.execute("develop.set", &json!({"values": {"light.exposure": 0.5}})).unwrap();
    s.execute("develop.set", &json!({"values": {"light.exposure": 0.75, "light.contrast": 20}})).unwrap();
    // since the first history state: +0.75 EV, +20 contrast on top of photo 1's own
    let r = s.execute("develop.copy", &json!({"groups": ["light"], "mode": "sinceFirst"})).unwrap();
    assert_eq!(r["mode"], "sinceFirst");
    let r = s.execute("develop.paste", &json!({"ids": [ids[1].0]})).unwrap();
    assert_eq!(r["relative"], true);
    let d = s.develop_of(ids[1]).unwrap();
    assert!((d.light.exposure - 1.75).abs() < 1e-9 && (d.light.contrast - 30.0).abs() < 1e-9, "{} {}", d.light.exposure, d.light.contrast);
    assert_eq!(d.light.highlights, 0.0, "unchanged sliders stay the photo's own");
    // since a chosen step (the one after the first edit): +0.25 EV
    let n = s.catalog.photo(ids[0]).unwrap().history.len();
    assert!(n >= 2, "history: {n}");
    s.execute("develop.copy", &json!({"groups": ["exposure"], "mode": "sinceStep", "step": n - 2})).unwrap();
    s.execute("develop.paste", &json!({"ids": [ids[2].0]})).unwrap();
    assert!((s.develop_of(ids[2]).unwrap().light.exposure - 0.25).abs() < 1e-9);
    assert!(s.execute("develop.copy", &json!({"mode": "sinceStep", "step": 999})).is_err());
    assert!(s.execute("develop.copy", &json!({"mode": "nope"})).is_err());
    // masks: merge adds the copied mask, replace swaps the photo's own for it
    s.execute("mask.add", &json!({"kind": "linear", "name": "Sky"})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["masks"]})).unwrap();
    s.execute("library.select", &json!({"ids": [ids[1].0]})).unwrap();
    s.execute("mask.add", &json!({"kind": "radial", "name": "Face"})).unwrap();
    s.execute("develop.paste", &json!({"ids": [ids[1].0], "maskMode": "merge"})).unwrap();
    let names: Vec<String> = s.develop_of(ids[1]).unwrap().masks.iter().map(|m| m.name.clone()).collect();
    assert_eq!(names, ["Face", "Sky"]);
    s.execute("develop.paste", &json!({"ids": [ids[1].0], "maskMode": "merge"})).unwrap();
    assert_eq!(s.develop_of(ids[1]).unwrap().masks.len(), 2, "the same mask isn't added twice");
    s.execute("develop.paste", &json!({"ids": [ids[1].0], "maskMode": "replace"})).unwrap();
    let names: Vec<String> = s.develop_of(ids[1]).unwrap().masks.iter().map(|m| m.name.clone()).collect();
    assert_eq!(names, ["Sky"]);
    assert!(s.execute("develop.paste", &json!({"maskMode": "blend"})).is_err());
}

/// U04: local adjustment ("Effect") presets: a Lightroom local preset folder imports in place
/// (titles delocalised, values scaled), applying sets the mask's sliders, new masks of that kind
/// start with it, saved / deleted effects persist with the library.
#[test]
fn mask_effects_import_lightroom_presets_apply_and_persist() {
    let lib = temp_dir("effects");
    let presets = lib.join("lr").join("Local Adjustment Presets");
    std::fs::create_dir_all(&presets).unwrap();
    // synthetic presets in the shape Lightroom writes (our own values)
    std::fs::write(
        presets.join("Burn.lrtemplate"),
        "s = {\n\tid = \"1\",\n\tinternalName = \"Burn\",\n\ttitle = ZSTR \"$$$/AgDevelop/Localized/Preset/Burn=Burn (Darken)\",\n\ttype = \"LocalizedAdjustmentPreset\",\n\tvalue = {\n\t\texposure2012 = -0.075,\n\t\tsaturation = 0,\n\t\ttoningHue = 240,\n\t},\n\tversion = 1,\n}\n",
    )
    .unwrap();
    std::fs::write(
        presets.join("Teeth.lrtemplate"),
        "s = { title = \"Teeth Whitening\", type = \"LocalizedAdjustmentPreset\", value = { exposure2012 = 0.1, saturation = -0.6 } }",
    )
    .unwrap();
    std::fs::write(presets.join("Develop.lrtemplate"), "s = { title = \"X\", type = \"Develop\", value = { settings = { Exposure2012 = 1 } } }")
        .unwrap();
    {
        let mut s = Session::new().with_fs();
        s.open_library(&lib, true).unwrap();
        let r = s.execute("mask.importEffects", &json!({"path": presets.to_string_lossy()})).unwrap();
        let names: Vec<&str> = r["imported"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).collect();
        assert_eq!(names, ["Burn (Darken)", "Teeth Whitening"], "{r}");
        assert!(r["failed"].as_array().unwrap().is_empty(), "a develop preset is skipped, not failed: {r}");
        let burn = s.mask_effects.iter().find(|e| e.name == "Burn (Darken)").unwrap();
        assert!((burn.adjust.exposure + 0.3).abs() < 1e-9, "exposure2012 is a quarter of the stops: {}", burn.adjust.exposure);
        // apply to a mask: its sliders
        s.execute("mask.add", &json!({"kind": "brush"})).unwrap();
        s.execute("mask.applyEffect", &json!({"name": "teeth whitening"})).unwrap();
        let id = s.active().unwrap();
        let m = s.develop_of(id).unwrap().masks[0].clone();
        assert!((m.adjust.exposure - 0.4).abs() < 1e-9 && (m.adjust.saturation + 60.0).abs() < 1e-9, "{:?}", m.adjust);
        assert_eq!(s.execute("mask.effects", &json!({})).unwrap()["current"], "Teeth Whitening");
        // the next brush mask starts with it (as Lightroom carries the effect over)
        s.execute("mask.add", &json!({"kind": "brush"})).unwrap();
        assert!((s.develop_of(id).unwrap().masks[1].adjust.saturation + 60.0).abs() < 1e-9);
        // save one of our own, delete an imported one
        s.execute("mask.adjust", &json!({"values": {"clarity": 30}})).unwrap();
        s.execute("mask.saveEffect", &json!({"name": "Crisp", "fields": "changed"})).unwrap();
        s.execute("mask.deleteEffect", &json!({"name": "Burn (Darken)"})).unwrap();
        assert!(s.execute("mask.applyEffect", &json!({"name": "nope"})).is_err());
        s.close_library().unwrap();
    }
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let names: Vec<String> = s.mask_effects.iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["Teeth Whitening", "Crisp"], "saved with the library preferences");
    assert_eq!(s.mask_tool_effects.get("brush").map(String::as_str), Some("Teeth Whitening"));
    let _ = std::fs::remove_dir_all(&lib);
}
