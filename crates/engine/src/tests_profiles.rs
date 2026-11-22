//! Camera Matching / Legacy / imported camera profiles, Adaptive profiles, Manage Profiles and
//! Lightroom's own (adaptive) presets (Bryan's fork: Adobe's data installed on this Mac is read
//! at run time; the tests that need it skip where it isn't installed).

use std::path::PathBuf;

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use crate::Session;

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(name: &str) -> Tmp {
    let dir = std::env::temp_dir().join(format!("lc-profiles-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Tmp(dir)
}

/// Adobe's camera profiles for the Canon EOS R6 are installed here (Camera Raw + the DNG SDK).
fn r6_installed() -> bool {
    crate::adobe::enabled() && crate::adobe::camera_profiles(Some("Canon"), "Canon EOS R6").iter().any(|e| e.id == "cam:Camera Standard")
}

/// A synthetic raw "shot" with a Canon EOS R6, imported into a fresh session.
fn r6_session(t: &Tmp) -> (Session, PhotoId) {
    let meta = lightcraft_meta::Metadata { make: Some("Canon".into()), model: Some("Canon EOS R6".into()), ..Default::default() };
    let path = t.0.join("r6.dng");
    std::fs::write(&path, crate::tests_xmp::synthetic_dng_with(None, meta)).unwrap();
    let mut s = Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [path.to_string_lossy()]})).unwrap();
    let id = PhotoId(r["imported"][0].as_u64().unwrap());
    s.execute("library.select", &json!({"ids": [id.0], "active": id.0})).unwrap();
    (s, id)
}

#[test]
fn lightroom_camera_profile_names_map_to_camera_profiles() {
    let id = |name: &str| crate::crs::camera_profile_id(name);
    assert_eq!(id("Camera Standard").as_deref(), Some("cam:Camera Standard"));
    assert_eq!(id("Camera PROVIA/Standard").as_deref(), Some("cam:Camera PROVIA/Standard"));
    assert_eq!(id("ACR 4.4").as_deref(), Some("legacy:ACR 4.4"));
    assert_eq!(id("Camera Raw 2.4").as_deref(), Some("legacy:Camera Raw 2.4"));
    assert_eq!(id("Adobe Standard"), None, "Adobe's own base maps as before");
    assert_eq!(id("Some Profile Nobody Installed"), None);
    let xmp = |camera: &str, look: &str| {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:CameraProfile="{camera}" crs:Exposure2012="0.1">{look}</rdf:Description></rdf:RDF></x:xmpmeta>"#
        )
    };
    let profile_of = |x: &str| {
        let props = lightcraft_meta::parse_xmp(x).unwrap().properties;
        crate::crs::to_partial(&props, Some(true))["profile"]["id"].as_str().map(str::to_string)
    };
    assert_eq!(profile_of(&xmp("Camera Neutral", "")).as_deref(), Some("cam:Camera Neutral"));
    assert_eq!(profile_of(&xmp("ACR 3.3", "")).as_deref(), Some("legacy:ACR 3.3"));
    let adaptive = r#"<crs:Look><rdf:Description crs:Amount="1"><crs:Name>Adaptive Color</crs:Name></rdf:Description></crs:Look>"#;
    assert_eq!(profile_of(&xmp("Adobe Standard", adaptive)).as_deref(), Some("adobe:Adaptive Color"));
}

#[test]
fn a_camera_profile_without_its_dcp_renders_our_nearest_look() {
    use lightcraft_pipeline::profiles::{effective_for, nearest_own};
    assert_eq!(nearest_own("cam:Camera Neutral"), "lc.neutral");
    assert_eq!(nearest_own("cam:Camera Monochrome"), "lc.mono");
    assert_eq!(nearest_own("cam:Camera Velvia/Vivid"), "lc.vivid");
    assert_eq!(nearest_own("cam:Camera Standard"), "lc.color");
    assert_eq!(nearest_own("legacy:ACR 4.4"), "lc.color");
    let mut d = lightcraft_develop::DevelopSettings::default();
    d.profile.id = "cam:Camera Monochrome".into();
    // no base renders it: our monochrome look
    assert_eq!(effective_for(&d, None).treatment, lightcraft_develop::Treatment::Bw);
    // a base registered as that profile: the profile itself (nothing added on top)
    let curve: Vec<f32> = (0..=lightcraft_pipeline::adobe::CURVE_N).map(|i| i as f32 / lightcraft_pipeline::adobe::CURVE_N as f32).collect();
    let base = lightcraft_pipeline::adobe::Base { hue_sat: None, hue_sat_ev: 0.0, look: None, curve, black: 0.0 };
    assert!(lightcraft_pipeline::adobe::register_base(0x7e57_ca3e, base));
    lightcraft_pipeline::adobe::register_base_profile(0x7e57_ca3e, "cam:Camera Monochrome");
    assert!(matches!(effective_for(&d, Some(0x7e57_ca3e)), std::borrow::Cow::Borrowed(_)));
}

#[test]
fn manage_profiles_hides_groups_and_keeps_the_view() {
    let t = tmp("manage");
    let mut s = Session::new().with_fs();
    s.open_library(t.0.join("lib"), true).unwrap();
    let groups = |s: &Session, all: bool| s.profile_groups_for(None, all).into_iter().map(|g| g.name).collect::<Vec<_>>();
    assert!(groups(&s, false).contains(&"Film".to_string()));
    s.execute("profile.group.show", &json!({"group": "Film", "show": false})).unwrap();
    assert!(!groups(&s, false).contains(&"Film".to_string()));
    assert!(groups(&s, true).contains(&"Film".to_string()), "still listed for Manage Profiles");
    let menu = s.execute("profiles.menu", &json!({})).unwrap();
    assert!(menu["groups"].as_array().unwrap().iter().any(|g| g["name"] == "Film" && g["hidden"] == true));
    s.execute("profile.view", &json!({"view": "list"})).unwrap();
    assert!(s.execute("profile.view", &json!({"view": "huge"})).is_err());
    assert!(s.execute("profile.group.show", &json!({})).is_err());
    // persisted with the library
    s.close_library().unwrap();
    drop(s);
    let mut s2 = Session::new().with_fs();
    s2.open_library(t.0.join("lib"), false).unwrap();
    assert_eq!(s2.profile_hidden_groups, vec!["Film".to_string()]);
    assert_eq!(s2.profile_view, "list");
    // toggling shows it again
    s2.execute("profile.group.show", &json!({"group": "film"})).unwrap();
    assert!(s2.profile_hidden_groups.is_empty());
}

#[test]
fn camera_profile_ids_are_profiles_and_favourites() {
    let mut s = Session::with_demo();
    assert!(s.is_profile("cam:Camera Standard"));
    assert!(s.is_profile("legacy:ACR 4.4"));
    assert!(!s.is_profile("cam:"));
    assert_eq!(s.profile_info("cam:Camera Faithful"), Some(("Camera Faithful", "Camera Matching")));
    assert_eq!(s.profile_info("cam:Kodak Portra 400 SO"), Some(("Kodak Portra 400 SO", "Profiles")));
    assert_eq!(s.profile_info("legacy:ACR 4.4"), Some(("ACR 4.4", "Legacy")));
    s.execute("profile.favorite", &json!({"id": "cam:Camera Standard"})).unwrap();
    assert_eq!(s.profile_favorites, vec!["cam:Camera Standard".to_string()]);
    // a demo photo (not a raw) on a camera profile renders our nearest look, never an error
    s.execute("develop.profile", &json!({"id": "cam:Camera Monochrome"})).unwrap();
    let id = s.active().unwrap();
    assert!(s.render_now(id, 64, 64).is_ok());
}

#[test]
fn installed_camera_matching_profiles_render_from_their_own_dcp() {
    if !r6_installed() {
        return;
    }
    let list = crate::adobe::camera_profiles(Some("Canon"), "Canon EOS R6");
    let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
    for want in [
        "cam:Camera Standard",
        "cam:Camera Neutral",
        "cam:Camera Portrait",
        "cam:Camera Landscape",
        "cam:Camera Faithful",
        "cam:Camera Monochrome",
        "legacy:ACR 4.4",
    ] {
        assert!(ids.contains(&want), "{want} not in {ids:?}");
    }
    assert_eq!(ids.first(), Some(&"cam:Camera Standard"), "Camera Standard first, as in Lightroom");
    let cam = crate::adobe::profile_for(Some("Canon"), "Canon EOS R6", "cam:Camera Neutral").expect("loads");
    assert!(cam.path.to_string_lossy().ends_with("Canon EOS R6 Camera Neutral.dcp"));
    assert_eq!(lightcraft_pipeline::adobe::base_profile(cam.key).as_deref(), Some("cam:Camera Neutral"));
    let legacy = crate::adobe::profile_for(Some("Canon"), "Canon EOS R6", "legacy:ACR 4.4").expect("loads");
    assert!(legacy.legacy);
    assert!(lightcraft_pipeline::adobe::base(legacy.key).unwrap().look.is_none(), "matrices only");
    assert!(crate::adobe::profile_for(Some("Canon"), "Canon EOS R6", "cam:Camera Nothing").is_none());

    // Adobe's Adaptive profile: its look table (the per-photo part needs Camera Raw's own model)
    if crate::adobe::find_look("Adaptive Color").is_some() {
        let look = lightcraft_pipeline::adobe::look("adobe:Adaptive Color").expect("registered with the camera's base");
        assert!(look.table.is_some() && look.supports_amount);
        assert!(lightcraft_pipeline::adobe::look("adobe:Adaptive B&W").is_some_and(|l| l.grayscale || l.table.is_some()));
    }

    // a raw from that camera: each profile decodes on its own DCP and renders differently
    let t = tmp("r6");
    let (mut s, id) = r6_session(&t);
    let camera: Vec<String> = s.camera_profiles_of(id).into_iter().map(|p| p.id).collect();
    assert!(camera.contains(&"cam:Camera Standard".to_string()), "{camera:?}");
    let groups = s.profile_groups_for(Some(id), false);
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    let pos = |n: &str| names.iter().position(|x| *x == n).unwrap_or_else(|| panic!("{n} not in {names:?}"));
    assert!(pos("Adobe Raw") < pos("Camera Matching") && pos("Camera Matching") < pos("Legacy"));
    let render = |s: &mut Session, profile: &str| {
        s.execute("develop.profile", &json!({"id": profile})).unwrap();
        let r = s.render_now(id, 96, 96).unwrap();
        (r.image.as_bytes().to_vec(), s.source_info(id).adobe)
    };
    let (std_px, std_key) = render(&mut s, "adobe:Adobe Standard");
    let (cam_px, cam_key) = render(&mut s, "cam:Camera Standard");
    let (mono_px, _) = render(&mut s, "cam:Camera Monochrome");
    let (legacy_px, legacy_key) = render(&mut s, "legacy:ACR 4.4");
    let (adaptive_px, _) = render(&mut s, "adobe:Adaptive Color");
    assert_ne!(adaptive_px, std_px, "the Adaptive look table renders");
    assert_ne!(std_key, cam_key, "decoded on another base");
    assert_ne!(cam_key, legacy_key);
    assert_eq!(cam_key.and_then(lightcraft_pipeline::adobe::base_profile).as_deref(), Some("cam:Camera Standard"));
    assert_ne!(std_px, cam_px);
    assert_ne!(cam_px, mono_px);
    assert_ne!(std_px, legacy_px);
    // a hover preview / browser variant on another profile decodes on that profile's DCP
    let mut d = (*s.develop_of(id).unwrap()).clone();
    d.profile.id = "cam:Camera Neutral".into();
    let job = s.variant_job(id, &d, 64).unwrap();
    assert!(matches!(&job.source, crate::media::SourceRef::File { base: Some(b), .. } if &**b == "cam:Camera Neutral"));
}

#[test]
fn imported_dcp_profiles_are_listed_for_their_camera() {
    if !r6_installed() {
        return;
    }
    let src = crate::adobe::camera_profiles(Some("Canon"), "Canon EOS R6").iter().find(|e| e.id == "cam:Camera Portrait").unwrap().path.clone();
    let t = tmp("import-dcp");
    let mut s = Session::new().with_fs();
    s.open_library(t.0.join("lib"), false).unwrap();
    // an Adobe profile copied to scratch under a new name stands in for a third-party DCP
    let copy = t.0.join("My R6 Look.dcp");
    std::fs::copy(&src, &copy).unwrap();
    let r = s.execute("profile.import", &json!({"paths": [copy.to_string_lossy()]})).unwrap();
    assert_eq!(r["imported"][0]["camera"], "Canon EOS R6", "{r}");
    assert_eq!(r["imported"][0]["group"], "Profiles");
    assert!(t.0.join("lib/Profiles").read_dir().unwrap().flatten().any(|e| e.path().extension().is_some_and(|x| x == "dcp")));
    let again = s.execute("profile.import", &json!({"paths": [copy.to_string_lossy()]})).unwrap();
    assert_eq!(again["alreadyImported"], 1);
    let bad = t.0.join("broken.dcp");
    std::fs::write(&bad, b"IIRC not a profile").unwrap();
    let r = s.execute("profile.import", &json!({"paths": [bad.to_string_lossy()]})).unwrap();
    assert_eq!(r["failed"].as_array().map(Vec::len), Some(1), "{r}");
    crate::adobe::set_imported_dirs(Vec::new());
}

#[test]
fn adaptive_preset_masks_map_to_ai_masks() {
    let xmp = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:PresetType="Normal" crs:UUID="ABC">
  <crs:Name><rdf:Alt><rdf:li xml:lang="x-default">Adaptive Test</rdf:li></rdf:Alt></crs:Name>
  <crs:Group><rdf:Alt><rdf:li xml:lang="x-default">Adaptive: Portrait</rdf:li></rdf:Alt></crs:Group>
  <crs:MaskGroupBasedCorrections><rdf:Seq>
   <rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" crs:CorrectionActive="true" crs:LocalExposure2012="0.05">
    <crs:CorrectionMasks><rdf:Seq><rdf:li crs:What="Mask/Image" crs:MaskSubType="3" crs:MaskSubCategoryID="12" crs:MaskName="Teeth"/></rdf:Seq></crs:CorrectionMasks>
   </rdf:Description></rdf:li>
   <rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" crs:CorrectionActive="true" crs:LocalSaturation="0.2">
    <crs:CorrectionMasks><rdf:Seq><rdf:li crs:What="Mask/Image" crs:MaskSubType="0" crs:MaskSubCategoryID="50005" crs:MaskName="Vegetation"/></rdf:Seq></crs:CorrectionMasks>
   </rdf:Description></rdf:li>
   <rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" crs:CorrectionActive="true" crs:LocalSaturation="0.2">
    <crs:CorrectionMasks><rdf:Seq><rdf:li crs:What="Mask/Image" crs:MaskSubType="0" crs:MaskSubCategoryID="50008" crs:MaskName="Snow"/></rdf:Seq></crs:CorrectionMasks>
   </rdf:Description></rdf:li>
  </rdf:Seq></crs:MaskGroupBasedCorrections>
 </rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let found = crate::preset_import::read_presets("Adaptive Test.xmp", xmp.as_bytes(), None).unwrap();
    let p = &found[0].preset;
    assert_eq!(p.group, "Adaptive: Portrait");
    let masks = p.settings["masks"].as_array().unwrap();
    let shapes: Vec<&Value> = masks.iter().map(|m| &m["components"][0]["shape"]).collect();
    assert_eq!(shapes[0], &json!({"kind": "people", "person": 0, "parts": ["teeth"]}));
    assert_eq!(shapes[1], &json!({"kind": "landscape", "class": "vegetation"}));
    assert!(found[0].unmapped.iter().any(|u| u.contains("50008")), "Snow is reported: {:?}", found[0].unmapped);
    // applied to a photo: the masks are added (computed, or heuristics without the models)
    let mut s = Session::with_demo();
    s.add_presets(vec![p.clone()]);
    let id = s.active().unwrap();
    let pid = s.presets.iter().find(|x| x.name == "Adaptive Test").unwrap().id.clone();
    let r = s.execute("preset.apply", &json!({"id": pid})).unwrap();
    assert!(r.get("aiErrors").is_some(), "{r}");
    let d = s.develop_of(id).unwrap();
    assert_eq!(d.masks.len(), 2);
    assert!(d.masks.iter().all(|m| m.components.iter().all(|c| c.shape.is_ai())));
    assert!(s.render_now(id, 64, 64).is_ok());
}

#[test]
fn lightrooms_installed_presets_are_offered_read_only() {
    let list = crate::presets::adobe_presets();
    if list.is_empty() {
        return;
    }
    assert!(list.iter().all(|p| p.builtin && p.id.starts_with("adobe.")));
    let mut ids = std::collections::HashSet::new();
    assert!(list.iter().all(|p| ids.insert(p.id.clone())), "ids are unique");
    // Adaptive: Sky ▸ Blue Drama carries a sky mask; Adaptive: Portrait ▸ Whiten Teeth the teeth
    if let Some(p) = list.iter().find(|p| p.group == "Adaptive: Sky") {
        assert!(p.settings["masks"].as_array().is_some_and(|m| m.iter().any(|m| m["components"][0]["shape"]["kind"] == "sky")), "{}", p.settings);
    }
    if let Some(p) = list.iter().find(|p| p.group == "Adaptive: Portrait" && p.name.contains("Teeth")) {
        assert!(p.settings["masks"].to_string().contains("teeth"), "{}", p.settings);
    }
    let mut s = Session::with_demo();
    let n = s.add_adobe_presets();
    assert_eq!(n, list.len());
    assert_eq!(s.add_adobe_presets(), 0, "loaded once");
    let r = s.execute("presets.adobe", &json!({})).unwrap();
    assert_eq!(r["added"], 0);
}

/// Lightroom's adaptive presets on a real photo with the AI-mask models (skipped unless
/// `LIGHTCRAFT_MODELS_DIR` and `LIGHTCRAFT_AI_PHOTO` are set and Camera Raw's presets are here):
/// the masks they add are computed (Sky, People parts, Landscape), not left to the heuristics.
#[cfg(feature = "sam")]
#[test]
fn adaptive_presets_compute_their_masks_with_the_installed_models() {
    let (Some(models), Some(photo)) = (std::env::var_os("LIGHTCRAFT_MODELS_DIR"), std::env::var_os("LIGHTCRAFT_AI_PHOTO")) else {
        eprintln!("skipped: set LIGHTCRAFT_MODELS_DIR and LIGHTCRAFT_AI_PHOTO");
        return;
    };
    let t = tmp("adaptive-real");
    let mut s = Session::new().with_fs();
    s.open_library(t.0.join("lib"), false).unwrap();
    s.segmenter.models = Some(models.into());
    s.segmenter.dir = Some(t.0.join("no-sam3"));
    if s.add_adobe_presets() == 0 {
        eprintln!("skipped: no Camera Raw presets installed");
        return;
    }
    let r = s.execute("library.import", &json!({"paths": [photo.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().unwrap();
    s.execute("library.select", &json!({"ids": [id], "active": id})).unwrap();
    let id = s.active().unwrap();
    let out = std::env::var_os("LIGHTCRAFT_AI_OUT").map(PathBuf::from);
    let wanted = std::env::var("LIGHTCRAFT_AI_PRESETS")
        .unwrap_or_else(|_| "Adaptive: Sky/Blue Drama,Adaptive: Portrait/Whiten Teeth,Adaptive: Landscape/Autumn".into());
    for pair in wanted.split(',') {
        let Some((group, name)) = pair.split_once('/') else { continue };
        let Some(p) = s.presets.iter().find(|p| p.group == group && p.name.contains(name)).cloned() else {
            eprintln!("{group} ▸ {name}: not installed");
            continue;
        };
        s.execute("develop.reset", &json!({})).unwrap();
        let t0 = std::time::Instant::now();
        let r = s.execute("preset.apply", &json!({"id": p.id})).unwrap();
        let d = s.develop_of(id).unwrap();
        let computed = d.masks.iter().flat_map(|m| &m.components).filter(|c| c.shape.is_ai() && c.shape.seg().is_some()).count();
        let ai = d.masks.iter().flat_map(|m| &m.components).filter(|c| c.shape.is_ai()).count();
        eprintln!("{group} ▸ {}: {computed}/{ai} AI masks computed in {:?} ({r})", p.name, t0.elapsed());
        if let Some(o) = &out {
            let img = s.render_now(id, 1200, 1200).unwrap().image;
            let path = o.join(format!("{}.png", crate::presets::slug(&p.name)));
            let bytes = img.as_bytes();
            let png = lightcraft_codecs::encode_png(
                &lightcraft_codecs::EncodeImage::new(img.width as u32, img.height as u32, 4, lightcraft_codecs::Samples::U8(&bytes)),
                &lightcraft_codecs::EncodeMeta::default(),
            )
            .unwrap();
            std::fs::write(path, png).unwrap();
        }
        assert!(r["aiErrors"].as_array().is_some_and(Vec::is_empty), "{r}");
        assert!(computed > 0, "{group} ▸ {}: nothing computed", p.name);
    }
}

/// Every profile Bryan's Lightroom catalog uses resolves (skipped unless `LIGHTCRAFT_LRCAT` names
/// a copy of a catalog; read only): `crs:CameraProfile` + `Look` of each photo → our profile id,
/// or a creative look whose profile file is installed. Prints the counts.
#[test]
fn every_profile_in_the_lightroom_catalog_resolves() {
    let Some(cat) = std::env::var_os("LIGHTCRAFT_LRCAT") else {
        eprintln!("skipped: set LIGHTCRAFT_LRCAT to a copy of a catalog");
        return;
    };
    let rec = crate::lr_migrate::records_from_db(std::path::Path::new(&cat), "catalog").unwrap();
    // the creative profiles a migration imports: Camera Raw's and Lightroom's profile folders
    let dirs = crate::lr_migrate::default_preset_dirs(None);
    let names: Vec<String> = crate::cmd::lut_profiles::profile_names(&dirs).into_iter().map(|n| n.to_lowercase()).collect();
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    let (mut unresolved, mut total) = (0usize, 0usize);
    for im in &rec.images {
        total += 1;
        let raw = !["jpg", "jpeg", "png", "tif", "tiff", "psd", "heic"].contains(&im.format.to_ascii_lowercase().as_str());
        let Ok(m) = crate::lr_migrate::map_develop(&im.develop, raw, 1.5) else {
            *by.entry("(unreadable develop text)".into()).or_default() += 1;
            continue;
        };
        let id = m.partial["profile"]["id"].as_str().unwrap_or("(default)").to_string();
        let key = match &m.creative_look {
            Some((look, _)) if names.contains(&look.to_lowercase()) => format!("{id} + look {look} (installed)"),
            Some((look, _)) => {
                unresolved += 1;
                format!("{id} + look {look} (NOT installed)")
            }
            None => id.clone(),
        };
        if m.unmapped.iter().any(|u| u == "CameraProfile") {
            unresolved += 1;
        }
        *by.entry(key).or_default() += 1;
    }
    for (k, n) in &by {
        eprintln!("{n:6} {k}");
    }
    eprintln!("{total} photos, {unresolved} unresolved");
    assert_eq!(unresolved, 0);
}
