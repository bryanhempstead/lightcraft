//! Generative Remove through the command layer: without the model (a content-aware stand-in, a notice, variations
//! cycled and added, keys that follow the spot) and, opt-in, with LaMa on a real photo (previews and exports match).

use serde_json::json;

use crate::Session;

fn spots(s: &Session) -> Vec<lightcraft_develop::Spot> {
    s.develop_of(s.active().unwrap()).unwrap().spots.clone()
}

/// Strongly red (the generated test colour), not the demo photo.
fn red(c: [u8; 4]) -> bool {
    i32::from(c[0]) > i32::from(c[1]) + 100 && i32::from(c[0]) > i32::from(c[2]) + 100
}

fn no_models(s: &mut Session, tag: &str) {
    s.enhancer.models_root = std::env::temp_dir().join(format!("lc-genremove-no-models-{tag}-{}", std::process::id()));
}

#[test]
fn a_generative_spot_without_the_model_fills_content_aware_and_says_so() {
    let mut s = Session::with_demo();
    no_models(&mut s, "a");
    let r = s.execute("spot.add", &json!({"generative": true, "points": [[0.4, 0.5], [0.45, 0.5]], "size": 0.03})).unwrap();
    assert_eq!(r["index"], 0);
    let notice = r["notice"].as_str().unwrap_or_default();
    assert!(notice.contains("enhance.model.download") && notice.contains("inpaint"), "{r}");
    assert_eq!(r["status"]["model"], false);
    let sp = &spots(&s)[0];
    assert_eq!(sp.mode, lightcraft_develop::SpotMode::Fill);
    assert_eq!(sp.source_offset, None);
    let g = sp.generative.clone().unwrap();
    assert_eq!((g.variation, g.count, g.key.len()), (0, 3, 16));
    // it renders (the content-aware stand-in)
    let id = s.active().unwrap();
    assert!(s.render_job(id, 256, 256, false, true).unwrap().run().rendered.is_ok());
    // variations: cycle with `<` / `>` (wrapping), pick one, more of them
    assert_eq!(s.execute("spot.variation", &json!({"delta": 1})).unwrap()["variation"], 1);
    assert_eq!(s.execute("spot.variation", &json!({"delta": -2})).unwrap()["variation"], 2);
    assert_eq!(s.execute("spot.variation", &json!({"variation": 0})).unwrap()["variation"], 0);
    assert!(s.execute("spot.variation", &json!({"variation": 3})).is_err());
    assert!(s.execute("spot.variation", &json!({})).is_err());
    let r = s.execute("spot.moreVariations", &json!({})).unwrap();
    assert_eq!((r["variation"].as_u64(), r["count"].as_u64()), (Some(3), Some(6)));
    s.execute("spot.moreVariations", &json!({})).unwrap();
    assert_eq!(spots(&s)[0].generative.as_ref().unwrap().count, 8);
    assert!(s.execute("spot.moreVariations", &json!({})).is_err(), "at most 8");
    // moving the spot gives it a new key (its pixels are made again); undo brings the old one back
    let k = spots(&s)[0].generative.clone().unwrap().key;
    s.execute("spot.update", &json!({"move": [0.05, 0.0]})).unwrap();
    let k2 = spots(&s)[0].generative.clone().unwrap().key;
    assert_ne!(k, k2);
    // a spot added before it changes what it sees: rekeyed too; feather alone doesn't
    s.execute("spot.update", &json!({"feather": 10})).unwrap();
    assert_eq!(spots(&s)[0].generative.clone().unwrap().key, k2);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spots(&s)[0].generative.clone().unwrap().key, k);
    // status, make (nothing to make without the model: the notice again)
    let st = s.execute("remove.status", &json!({})).unwrap();
    assert_eq!(st["spots"][0]["made"].as_array().unwrap().len(), 8);
    let r = s.execute("remove.generate", &json!({})).unwrap();
    assert_eq!(r["spots"], 0);
    assert!(r["errors"][0].as_str().unwrap().contains("inpaint"), "{r}");
    // switching it off makes a plain Fill spot; heal mode drops it too
    s.execute("spot.update", &json!({"generative": false})).unwrap();
    assert!(spots(&s)[0].generative.is_none());
    s.execute("spot.update", &json!({"generative": true})).unwrap();
    assert_eq!(spots(&s)[0].generative.as_ref().unwrap().count, 3);
    s.execute("spot.update", &json!({"mode": "heal"})).unwrap();
    assert!(spots(&s)[0].generative.is_none());
    assert!(s.execute("spot.variation", &json!({"delta": 1})).is_err(), "not generative");
    // the settings round-trip through JSON (the catalog, XMP of LightCraft's own)
    let d = (*s.develop_of(s.active().unwrap()).unwrap()).clone();
    let back: lightcraft_develop::DevelopSettings = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
    assert_eq!(back.spots, d.spots);
}

#[test]
fn generated_pixels_in_the_cache_replace_the_stand_in_at_every_size() {
    // pixels written by hand (as the model would): a flat colour over the spot's crop
    let mut s = Session::with_demo();
    no_models(&mut s, "b");
    s.execute("spot.add", &json!({"generative": true, "points": [[0.5, 0.5]], "size": 0.05})).unwrap();
    let id = s.active().unwrap();
    let sp = spots(&s)[0].clone();
    let g = sp.generative.clone().unwrap();
    let image = lightcraft_raster::Rgb32f::from_fn(64, 64, |_, _| [0.9, 0.05, 0.05]);
    let fill = lightcraft_pipeline::spots::GenFill { geometry: lightcraft_pipeline::spots::gen_geometry(&sp), rect: [0.3, 0.3, 0.7, 0.7], image };
    crate::genremove::save(&crate::genremove::cache_dir(), &g.name(), &fill).unwrap();
    let st = s.execute("remove.status", &json!({})).unwrap();
    assert_eq!(st["spots"][0]["made"][0], true, "{st}");
    for size in [160, 640] {
        let img = s.render_job(id, size, size, false, false).unwrap().run().rendered.unwrap().image;
        let c = img.data[(img.height / 2) * img.width + img.width / 2];
        assert!(red(c), "{size}: the centre is the generated red: {c:?}");
    }
    // another variation (not made) falls back to the stand-in
    s.execute("spot.variation", &json!({"delta": 1})).unwrap();
    let img = s.render_job(id, 160, 160, false, false).unwrap().run().rendered.unwrap().image;
    let c = img.data[(img.height / 2) * img.width + img.width / 2];
    assert!(!red(c), "variation 2 isn't made: {c:?}");
    // a moved spot doesn't use pixels made for its old place
    s.execute("spot.variation", &json!({"variation": 0})).unwrap();
    let mut d = (*s.develop_of(id).unwrap()).clone();
    d.spots[0].points[0].x = 0.52;
    s.set_develop(id, d, "test move without rekey").unwrap();
    let img = s.render_job(id, 160, 160, false, false).unwrap().run().rendered.unwrap().image;
    let c = img.data[(img.height / 2) * img.width + img.width / 2];
    assert!(!red(c), "geometry changed: {c:?}");
}

#[test]
fn engines_have_their_own_keys_and_lama_stand_ins_are_remade() {
    let mut s = Session::with_demo();
    no_models(&mut s, "e");
    s.execute("spot.add", &json!({"generative": true, "points": [[0.55, 0.45]], "size": 0.05})).unwrap();
    let g = spots(&s)[0].generative.clone().unwrap();
    assert!(!g.quick, "gen.ai. is the diffusion model by default");
    let st = s.execute("remove.status", &json!({})).unwrap();
    assert_eq!((st["spots"][0]["engine"].as_str(), st["diffusion"].as_bool(), st["lama"].as_bool()), (Some("diffusion"), Some(false), Some(false)));
    // quick. (LaMa) has its own key, so switching back finds the diffusion fills again
    s.execute("spot.update", &json!({"quick": true})).unwrap();
    let q = spots(&s)[0].generative.clone().unwrap();
    assert!(q.quick && q.key != g.key && q.key.len() == 16);
    assert_eq!(s.execute("remove.status", &json!({})).unwrap()["spots"][0]["engine"], "quick");
    s.execute("spot.update", &json!({"quick": false})).unwrap();
    assert_eq!(spots(&s)[0].generative.clone().unwrap().key, g.key);
    let r = s.execute("spot.add", &json!({"generative": true, "quick": true, "points": [[0.2, 0.2]], "size": 0.03})).unwrap();
    assert_eq!(r["status"]["spots"][1]["engine"], "quick");
    // LaMa stood in for the missing diffusion model: reported, and forgotten (remade) on request
    let sp = spots(&s)[0].clone();
    let image = lightcraft_raster::Rgb32f::from_fn(32, 32, |_, _| [0.5; 3]);
    let fill = lightcraft_pipeline::spots::GenFill { geometry: lightcraft_pipeline::spots::gen_geometry(&sp), rect: [0.45, 0.35, 0.65, 0.55], image };
    let dir = crate::genremove::cache_dir();
    for v in 0..2 {
        let name = format!("{}-{v}", g.key);
        crate::genremove::save(&dir, &name, &fill).unwrap();
    }
    crate::genremove::set_fallback(&dir, &format!("{}-0", g.key), Some(crate::genremove::Fallback::Missing));
    let st = s.execute("remove.status", &json!({})).unwrap();
    assert_eq!(st["spots"][0]["made"], json!([true, true, false]), "{st}");
    assert_eq!(st["spots"][0]["fallback"], json!(["missing", null, null]), "{st}");
    let r = s.execute("remove.generate", &json!({"remake": "fallbacks"})).unwrap();
    assert_eq!(r["forgotten"], 1, "{r}");
    assert_eq!(s.execute("remove.status", &json!({})).unwrap()["spots"][0]["made"], json!([false, true, false]));
    let r = s.execute("remove.generate", &json!({"remake": true})).unwrap();
    assert_eq!(r["forgotten"], 1, "{r}");
    assert!(s.execute("remove.generate", &json!({"remake": 3})).is_err());
    // nothing running: nothing to stop
    assert_eq!(s.execute("remove.cancel", &json!({})).unwrap()["stopped"], 0);
    // seeds: the same for a key and variation everywhere, different between them
    assert_eq!(crate::genremove::seed_of(&g.key, 1), crate::genremove::seed_of(&g.key, 1));
    assert_ne!(crate::genremove::seed_of(&g.key, 1), crate::genremove::seed_of(&g.key, 2));
    // old catalogs (no `quick`) read as the diffusion engine; quick spots write it
    let back: lightcraft_develop::Generative = serde_json::from_value(json!({"variation": 1, "count": 3, "key": "00000000000000ab"})).unwrap();
    assert!(!back.quick);
    assert!(serde_json::to_string(&q).unwrap().contains("\"quick\":true"));
    assert!(!serde_json::to_string(&g).unwrap().contains("quick"));
}

/// Opt-in, with the real models: `LC_GEN_MODELS=<a models folder with inpaint/big-lama.safetensors and, for
/// gen.ai., gen-fill/{unet,vae,text_encoder}.safetensors> LC_GEN_PHOTO=<a JPEG> [LC_GEN_QUICK=1: LaMa alone]
/// [LC_OUT=<folder>] cargo test -p lightcraft-engine --features enhance --release -- --ignored generative_remove_with
/// --nocapture`
#[test]
#[ignore]
fn generative_remove_with_lama() {
    let (Ok(models), Ok(photo)) = (std::env::var("LC_GEN_MODELS"), std::env::var("LC_GEN_PHOTO")) else { return };
    if !crate::enhance::AVAILABLE {
        return;
    }
    let mut s = Session::new().with_fs();
    s.enhancer.models_root = models.into();
    let r = s.execute("library.import", &json!({"paths": [photo]})).unwrap();
    let id = lightcraft_catalog::PhotoId(r["imported"][0].as_u64().unwrap());
    s.selection = crate::Selection::single(id);
    let (x, y) = (
        std::env::var("LC_GEN_X").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5),
        std::env::var("LC_GEN_Y").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5),
    );
    let size: f64 = std::env::var("LC_GEN_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.04);
    let t = std::time::Instant::now();
    let points: serde_json::Value =
        std::env::var("LC_GEN_POINTS").ok().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_else(|| json!([[x, y], [x + size, y]]));
    let quick = std::env::var_os("LC_GEN_QUICK").is_some();
    let r = s.execute("spot.add", &json!({"generative": true, "quick": quick, "points": points, "size": size})).unwrap();
    eprintln!("3 variations in {:.1} s: {r}", t.elapsed().as_secs_f64());
    assert!(r.get("notice").is_none(), "{r}");
    assert_eq!(r["status"]["spots"][0]["made"], json!([true, true, true]), "{r}");
    let out = std::env::var("LC_OUT").ok();
    let mut renders = Vec::new();
    for v in 0..3 {
        s.execute("spot.variation", &json!({"variation": v})).unwrap();
        for size in [600usize, 2400] {
            let t = std::time::Instant::now();
            let img = s.render_job(id, size, size, false, true).unwrap().run().rendered.unwrap().image;
            eprintln!("variation {v} at {size}: {:.2} s", t.elapsed().as_secs_f64());
            if let Some(o) = &out {
                let mut ppm = format!("P6 {} {} 255\n", img.width, img.height).into_bytes();
                ppm.extend(img.data.iter().flat_map(|p| [p[0], p[1], p[2]]));
                std::fs::write(format!("{o}/gen-v{v}-{size}.ppm"), ppm).unwrap();
            }
            renders.push((v, size, img));
        }
    }
    // a preview matches the export, downsampled, around the spot
    for v in 0..3 {
        let small = &renders.iter().find(|r| r.0 == v && r.1 == 600).unwrap().2;
        let big = &renders.iter().find(|r| r.0 == v && r.1 == 2400).unwrap().2;
        let k = big.width as f64 / small.width as f64;
        let (cx, cy) = ((x * small.width as f64) as usize, (y * small.height as f64) as usize);
        let (mut err, mut n) = (0f64, 0);
        for yy in cy.saturating_sub(8)..(cy + 8).min(small.height) {
            for xx in cx.saturating_sub(8)..(cx + 8).min(small.width) {
                let mut acc = 0f64;
                let mut m = 0;
                for by in (yy as f64 * k) as usize..((yy + 1) as f64 * k) as usize {
                    for bx in (xx as f64 * k) as usize..((xx + 1) as f64 * k) as usize {
                        if let Some(p) = big.data.get(by * big.width + bx) {
                            acc += f64::from(p[1]);
                            m += 1;
                        }
                    }
                }
                err += (acc / m.max(1) as f64 - f64::from(small.data[yy * small.width + xx][1])).abs();
                n += 1;
            }
        }
        let mean = err / n.max(1) as f64;
        eprintln!("variation {v}: preview vs export {mean:.1} levels");
        assert!(mean < 12.0, "variation {v}: {mean}");
    }
}

#[test]
fn detected_objects_are_picked_then_removed_in_one_step() {
    use crate::remove_detect::{Candidate, Found, Kind};
    use lightcraft_geom::Point;
    let mut s = Session::with_demo();
    no_models(&mut s, "c");
    let id = s.active().unwrap();
    // nothing detected yet: an actionable error
    assert!(s.execute("remove.apply", &json!({"all": true})).is_err());
    let row = |y: f64| lightcraft_develop::Spot {
        mode: lightcraft_develop::SpotMode::Fill,
        points: vec![Point::new(0.3, y), Point::new(0.35, y)],
        size: 0.01,
        ..Default::default()
    };
    let person = Candidate { kind: Kind::Person, label: "1.".into(), spots: vec![row(0.5), row(0.51), row(0.52)], rect: [0.29, 0.49, 0.36, 0.53] };
    let other = Candidate { kind: Kind::Person, label: "2.".into(), spots: vec![row(0.7)], rect: [0.29, 0.69, 0.36, 0.71] };
    let dust = Candidate {
        kind: Kind::Dust,
        label: "dust.1".into(),
        spots: vec![lightcraft_develop::Spot {
            mode: lightcraft_develop::SpotMode::Heal,
            points: vec![Point::new(0.8, 0.2)],
            size: 0.005,
            ..Default::default()
        }],
        rect: [0.79, 0.19, 0.81, 0.21],
    };
    s.remove_found = Found { photo: Some(id), items: vec![person, other, dust], pending: 0 };
    let c = s.execute("remove.candidates", &json!({})).unwrap();
    assert_eq!(c["candidates"].as_array().unwrap().len(), 3);
    assert_eq!(c["candidates"][2]["kind"], "dust");
    // generatively: one merged spot for the person (all its rows' points); dust gets a heal spot
    let r = s.execute("remove.apply", &json!({"indices": [0, 2, 9], "generative": true})).unwrap();
    assert_eq!((r["added"].as_u64(), r["remaining"].as_u64()), (Some(2), Some(1)));
    let sp = spots(&s);
    assert_eq!(sp.len(), 2);
    assert_eq!(sp[0].points.len(), 6);
    assert!(sp[0].generative.is_some() && sp[0].mode == lightcraft_develop::SpotMode::Fill);
    assert_eq!(sp[1].mode, lightcraft_develop::SpotMode::Heal);
    // one undo step takes both back
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(spots(&s).is_empty());
    // the rest without the model: content-aware rows
    let r = s.execute("remove.apply", &json!({"all": true, "generative": false})).unwrap();
    assert_eq!((r["added"].as_u64(), r["remaining"].as_u64()), (Some(1), Some(0)));
    assert!(spots(&s)[0].generative.is_none());
    assert!(s.execute("remove.apply", &json!({"indices": [0]})).is_err(), "nothing left to pick");
    s.execute("remove.clearCandidates", &json!({})).unwrap();
    assert_eq!(s.execute("remove.candidates", &json!({})).unwrap()["candidates"].as_array().unwrap().len(), 0);
}

#[test]
fn remove_reflections_without_its_models_says_what_to_get_and_keys_follow_the_pixels() {
    let mut s = Session::with_demo();
    no_models(&mut s, "d");
    let id = s.active().unwrap();
    let r = s.execute("reflection.set", &json!({"enabled": true})).unwrap();
    let n = r["notice"].as_str().unwrap_or_default();
    assert!(n.contains("reflection") && n.contains("vgg19") && n.contains("enhance.model.download"), "{r}");
    assert_eq!((r["enabled"].as_bool(), r["made"].as_bool(), r["models"].as_bool()), (Some(true), Some(false), Some(false)));
    let k = s.develop_of(id).unwrap().reflection.key.clone();
    assert_eq!(k.len(), 16);
    // renders as it is until the layers are made
    assert!(s.render_job(id, 200, 200, false, true).unwrap().run().rendered.is_ok());
    // Amount is a develop control; Keep a switch; neither changes what the layers are made from
    s.execute("develop.set", &json!({"control": "reflection.amount", "value": 40})).unwrap();
    s.execute("reflection.set", &json!({"keep": true})).unwrap();
    let d = s.develop_of(id).unwrap();
    assert_eq!((d.reflection.amount, d.reflection.keep, d.reflection.key.as_str()), (40.0, true, k.as_str()));
    assert!(s.execute("reflection.set", &json!({"amount": "lots"})).is_ok(), "ignored");
    // the tone doesn't change them, a rotation does
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 1.0})).unwrap();
    s.execute("reflection.set", &json!({})).unwrap();
    assert_eq!(s.develop_of(id).unwrap().reflection.key, k);
    let mut d = (*s.develop_of(id).unwrap()).clone();
    d.orientation = lightcraft_geom::Orientation::Rotate90;
    s.set_develop(id, d, "test rotate").unwrap();
    s.execute("reflection.set", &json!({})).unwrap();
    assert_ne!(s.develop_of(id).unwrap().reflection.key, k);
    // round trip, and off
    let d = (*s.develop_of(id).unwrap()).clone();
    let back: lightcraft_develop::DevelopSettings = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
    assert_eq!(back.reflection, d.reflection);
    s.execute("reflection.set", &json!({"enabled": false})).unwrap();
    assert!(!s.develop_of(id).unwrap().reflection.active());
}

/// Opt-in, with the real models: `LC_GEN_MODELS=<a models folder with reflection/ and vgg19/> LC_REFL_PHOTO=<a
/// JPEG> [LC_OUT=<folder>] cargo test -p lightcraft-engine --features enhance --release -- --ignored
/// remove_reflections_with_dsrnet --nocapture`
#[test]
#[ignore]
fn remove_reflections_with_dsrnet() {
    let (Ok(models), Ok(photo)) = (std::env::var("LC_GEN_MODELS"), std::env::var("LC_REFL_PHOTO")) else { return };
    if !crate::enhance::AVAILABLE {
        return;
    }
    let mut s = Session::new().with_fs();
    s.enhancer.models_root = models.into();
    let r = s.execute("library.import", &json!({"paths": [photo]})).unwrap();
    let id = lightcraft_catalog::PhotoId(r["imported"][0].as_u64().unwrap());
    s.selection = crate::Selection::single(id);
    let before = s.render_job(id, 1200, 1200, false, true).unwrap().run().rendered.unwrap().image;
    let t = std::time::Instant::now();
    let r = s.execute("reflection.set", &json!({"enabled": true})).unwrap();
    eprintln!("layers in {:.1} s: {r}", t.elapsed().as_secs_f64());
    assert_eq!(r["made"], true, "{r}");
    let out = std::env::var("LC_OUT").ok();
    let mut diffs = Vec::new();
    for (keep, size) in [(false, 1200usize), (false, 2400), (true, 1200)] {
        s.execute("reflection.set", &json!({"keep": keep})).unwrap();
        let img = s.render_job(id, size, size, false, true).unwrap().run().rendered.unwrap().image;
        if let Some(o) = &out {
            let mut ppm = format!("P6 {} {} 255\n", img.width, img.height).into_bytes();
            ppm.extend(img.data.iter().flat_map(|p| [p[0], p[1], p[2]]));
            std::fs::write(format!("{o}/refl-{}-{size}.ppm", if keep { "keep" } else { "remove" }), ppm).unwrap();
        }
        if size == 1200 && !keep {
            let d: f64 = img.data.iter().zip(&before.data).map(|(a, b)| (0..3).map(|k| f64::from(a[k].abs_diff(b[k]))).sum::<f64>()).sum::<f64>()
                / (3 * img.data.len()) as f64;
            diffs.push(d);
        }
    }
    eprintln!("mean change with Remove: {:.2} levels", diffs[0]);
    assert!(diffs[0] > 0.5, "the photo changed");
}
