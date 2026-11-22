//! Lightroom's automatic AI masks (Subject, Sky, Background, Landscape, People) and Update AI
//! Settings.
//!
//! Without models: Subject / Sky / Background keep the heuristics, Landscape and People ask for
//! the models, nothing waits. With the models installed (`LIGHTCRAFT_MODELS_DIR`, and a photo:
//! `LIGHTCRAFT_AI_PHOTO`, optional `LIGHTCRAFT_AI_OUT` for PNGs of the masks) the whole flow runs
//! on the real parsers.

use serde_json::json;

use crate::Session;
use lightcraft_develop::MaskShape;

#[test]
fn without_models_heuristics_stay_and_the_rest_asks_for_them() {
    let mut s = Session::with_demo();
    s.segmenter.models = Some(std::env::temp_dir().join(format!("lc-no-models-{}", std::process::id())));
    for kind in ["sky", "subject", "background"] {
        s.execute("mask.add", &json!({"kind": kind})).unwrap();
    }
    let id = s.active().unwrap();
    let d = s.develop_of(id).unwrap();
    assert_eq!(d.masks.len(), 3);
    assert!(d.masks.iter().all(|m| m.components[0].shape.seg().is_none()), "heuristics: nothing stored");
    // Landscape sky falls back to the sky heuristic; other classes and People need the models
    s.execute("mask.add", &json!({"kind": "landscape", "class": "sky"})).unwrap();
    let e = s.execute("mask.add", &json!({"kind": "landscape", "classes": ["water"]})).unwrap_err().to_string();
    assert!(e.contains(crate::segment::NO_AUTO_MODEL) || e.contains("not available"), "{e}");
    let e = s.execute("mask.add", &json!({"kind": "people", "parts": ["hair"]})).unwrap_err().to_string();
    assert!(e.contains(crate::segment::NO_AUTO_MODEL) || e.contains("not available"), "{e}");
    // bad classes and parts are errors, not crashes
    assert!(s.execute("mask.add", &json!({"kind": "landscape", "class": "lava"})).is_err());
    assert!(s.execute("mask.add", &json!({"kind": "people", "parts": ["tail"]})).is_err());
    assert!(s.execute("mask.add", &json!({"kind": "people", "parts": []})).is_err());
    // detections need a model; nothing is found yet
    assert!(s.execute("segment.detectPeople", &json!({})).is_err());
    let f = s.execute("segment.found", &json!({})).unwrap();
    assert!(f["people"].is_null() && f["classes"].is_null(), "{f}");
    assert_eq!(f["stale"], json!([]));
    // Update AI Settings with nothing computed: nothing to do, no error for the heuristics
    let st = s.execute("segment.parsers.status", &json!({})).unwrap();
    assert_eq!(st["installed"], false);
    let e = s.execute("segment.parsers.download", &json!({})).unwrap_err().to_string();
    assert!(e.contains("acknowledged") && e.contains("MB"), "{e}");
}

#[test]
fn a_stored_shape_is_taken_as_it_is_and_goes_stale_when_the_photo_changes() {
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    let d = s.develop_of(id).unwrap();
    let mut seg = lightcraft_develop::SegMask::from_logits(4, &[3.0; 16]);
    seg.basis = Some(lightcraft_develop::ai::basis(&d));
    let shape = MaskShape::Landscape { class: "water,vegetation".into(), seg: Some(seg) };
    s.execute("mask.add", &json!({"shape": shape, "name": "Water"})).unwrap();
    let f = s.execute("segment.found", &json!({})).unwrap();
    assert_eq!(f["stale"], json!([]));
    // a spot changes the pixels under the mask: out of date
    s.execute("spot.add", &json!({"points": [[0.5, 0.5]]})).unwrap();
    let f = s.execute("segment.found", &json!({})).unwrap();
    assert_eq!(f["stale"], json!([{"mask": 1, "component": 0}]), "{f}");
    // updating needs a model
    if crate::segment::Segmenter::AVAILABLE {
        assert!(s.execute("mask.updateAI", &json!({})).is_err());
    }
}

/// The real parsers on a real photo (skipped unless `LIGHTCRAFT_MODELS_DIR` and
/// `LIGHTCRAFT_AI_PHOTO` are set).
#[cfg(feature = "sam")]
#[test]
fn landscape_and_people_masks_with_the_installed_models() {
    let (Some(models), Some(photo)) = (std::env::var_os("LIGHTCRAFT_MODELS_DIR"), std::env::var_os("LIGHTCRAFT_AI_PHOTO")) else {
        eprintln!("skipped: set LIGHTCRAFT_MODELS_DIR and LIGHTCRAFT_AI_PHOTO");
        return;
    };
    let out = std::env::var_os("LIGHTCRAFT_AI_OUT").map(std::path::PathBuf::from);
    let dir = std::env::temp_dir().join(format!("lc-ai-masks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.segmenter.models = Some(models.into());
    s.segmenter.dir = Some(dir.join("no-sam3"));
    let r = s.execute("library.import", &json!({"paths": [photo.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().unwrap();
    s.execute("library.select", &json!({"ids": [id], "active": id})).unwrap();
    let id = s.active().expect("an active photo");
    let t = std::time::Instant::now();
    let cov = s.execute("segment.detectLandscape", &json!({})).unwrap();
    eprintln!("landscape {:?}: {cov}", t.elapsed());
    let t = std::time::Instant::now();
    let r = s.execute("mask.add", &json!({"kind": "sky"}));
    eprintln!("sky {:?}: {r:?}", t.elapsed());
    let t = std::time::Instant::now();
    let ppl = s.execute("segment.detectPeople", &json!({})).unwrap();
    eprintln!("people {:?}: {ppl}", t.elapsed());
    let n = ppl["people"].as_array().map_or(0, Vec::len);
    if n > 0 {
        // the most prominent (largest) person
        let k = ppl["people"]
            .as_array()
            .unwrap()
            .iter()
            .max_by(|a, b| {
                let area = |p: &serde_json::Value| {
                    let r: Vec<f64> = p["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                    (r[2] - r[0]) * (r[3] - r[1])
                };
                area(a).total_cmp(&area(b))
            })
            .unwrap()["index"]
            .clone();
        for parts in [
            json!(["person"]),
            json!(["faceSkin"]),
            json!(["eyebrows", "lips"]),
            json!(["eyeSclera"]),
            json!(["iris"]),
            json!(["teeth"]),
            json!(["hair"]),
            json!(["clothes"]),
            json!(["bodySkin"]),
        ] {
            let t = std::time::Instant::now();
            let r = s.execute("mask.add", &json!({"kind": "people", "person": k, "parts": parts}));
            eprintln!("people {parts} {:?}: {}", t.elapsed(), r.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string()));
        }
    }
    for c in ["water", "vegetation", "mountains", "architecture", "naturalGround", "artificialGround"] {
        if let Err(e) = s.execute("mask.add", &json!({"kind": "landscape", "class": c})) {
            eprintln!("{c}: {e}");
        }
    }
    let d = s.develop_of(id).unwrap();
    assert!(d.masks.iter().any(|m| matches!(&m.components[0].shape, MaskShape::Sky { seg: Some(_) }) || n > 0), "something was computed");
    // every computed mask records what the photo looked like, and nothing is stale yet
    assert!(d.masks.iter().all(|m| m.components[0].shape.seg().is_some_and(|s| s.basis.is_some())));
    assert!(lightcraft_develop::ai::stale_components(&d).is_empty());
    // Update AI Settings recomputes them all, in one step
    let t = std::time::Instant::now();
    let u = s.execute("mask.updateAI", &json!({})).unwrap();
    eprintln!("update {:?}: {u}", t.elapsed());
    assert_eq!(u["updated"].as_u64(), Some(d.masks.len() as u64));
    // Remove ▸ Detect: each person becomes content-aware Remove spots
    let before = s.develop_of(id).unwrap().spots.len();
    let r = s.execute("remove.detect", &json!({"dust": false, "add": true})).unwrap();
    eprintln!("remove.detect: {r}");
    assert_eq!(r["objects"].as_u64(), Some(n as u64));
    assert!(n == 0 || s.develop_of(id).unwrap().spots.len() > before);
    if let Some(out) = out {
        std::fs::create_dir_all(&out).unwrap();
        let p = s.catalog.photo(id).unwrap();
        let (pw, ph) = (p.width.max(1) as usize, p.height.max(1) as usize);
        let (w, h) = if pw >= ph { (768, 768 * ph / pw) } else { (768 * pw / ph, 768) };
        for (i, m) in d.masks.iter().enumerate() {
            let a = rasterize(&m.components[0].shape, w, h);
            let png = lightcraft_codecs::encode_png(
                &lightcraft_codecs::EncodeImage::new(w as u32, h as u32, 1, lightcraft_codecs::Samples::U8(&a)),
                &lightcraft_codecs::EncodeMeta::default(),
            )
            .unwrap();
            let label = serde_json::to_value(&m.components[0].shape).unwrap();
            let name = format!(
                "{i:02}-{}-{}.png",
                label["kind"].as_str().unwrap_or("x"),
                label.get("class").or(label.get("parts")).map(|v| v.to_string()).unwrap_or_default().replace(['"', '[', ']', ','], "")
            );
            std::fs::write(out.join(name), png).unwrap();
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The selection of an AI shape at `w × h` (0..255), as the pipeline samples it.
#[cfg(feature = "sam")]
fn rasterize(shape: &MaskShape, w: usize, h: usize) -> Vec<u8> {
    let (seg, detail): (Option<&lightcraft_develop::SegMask>, Vec<lightcraft_develop::SegMask>) = match shape {
        MaskShape::People { seg, detail, .. } => (seg.as_ref(), detail.clone()),
        s => (s.seg(), vec![]),
    };
    let grid = |m: &lightcraft_develop::SegMask| (m.logits().unwrap(), m.side as usize, m.bounds().unwrap());
    let coarse = seg.map(grid);
    let patches: Vec<_> = detail.iter().map(grid).collect();
    let at = |(l, side, r): &(Vec<f32>, usize, [f64; 4]), x: f64, y: f64| -> Option<f32> {
        let (u, v) = ((x - r[0]) / (r[2] - r[0]), (y - r[1]) / (r[3] - r[1]));
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return None;
        }
        let (cx, cy) = (((u * *side as f64) as usize).min(side - 1), ((v * *side as f64) as usize).min(side - 1));
        l.get(cy * side + cx).copied()
    };
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = ((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
            let fine = patches.iter().filter_map(|p| at(p, nx, ny)).reduce(f32::max);
            let l = fine.or_else(|| coarse.as_ref().and_then(|c| at(c, nx, ny))).unwrap_or(-16.0);
            let mut a = 1.0 / (1.0 + (-l).exp());
            if matches!(shape, MaskShape::Background { .. }) {
                a = 1.0 - a;
            }
            out[y * w + x] = (a * 255.0) as u8;
        }
    }
    out
}
