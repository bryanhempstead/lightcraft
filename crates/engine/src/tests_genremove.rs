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

/// Opt-in, with the real model: `LC_GEN_MODELS=<a models folder with inpaint/big-lama.safetensors>
/// LC_GEN_PHOTO=<a JPEG> [LC_OUT=<folder>] cargo test -p lightcraft-engine --features enhance --release -- --ignored
/// generative_remove_with_lama --nocapture`
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
    let r = s.execute("spot.add", &json!({"generative": true, "points": points, "size": size})).unwrap();
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
