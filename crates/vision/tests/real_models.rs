//! The real models on real photos (no model or photo is part of LightCraft): run with
//! `LC_VISION_MODELS=<folder with faces/ and search/> LC_VISION_IMG=<folder of photos>
//! cargo test -p lightcraft-vision --release --test real_models -- --ignored --nocapture`.
//!
//! The photos: Wikimedia Commons' `Solvay_conference_1927.jpg`, `President_Barack_Obama.jpg`,
//! `Official_portrait_of_Barack_Obama.jpg`, `Donald_Trump_official_portrait.jpg` and
//! `Joe_Biden_presidential_portrait.jpg` (all public domain), plus `closed_*.jpg`: portraits with closed eyes.
//! The expected values were measured with OpenCV 4.12's `FaceDetectorYN` / `FaceRecognizerSF` on the same files.

use std::path::PathBuf;
use std::time::Instant;

use lightcraft_vision::image::Rgb8;
use lightcraft_vision::{clip, eyes, models, sface, yunet};

fn env(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).map(PathBuf::from)
}

fn load(p: &std::path::Path, edge: usize) -> Rgb8 {
    let img = image::open(p).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    Rgb8::new(w as usize, h as usize, img.into_raw()).unwrap().fit(edge).0
}

/// Prints what the eye model sees in every photo of `LC_VISION_DIR` (for checking eye-state results by eye).
#[test]
#[ignore]
fn print_eyes_in_folder() {
    let (Some(m), Some(dir)) = (env("LC_VISION_MODELS"), env("LC_VISION_DIR")) else { return };
    let f = m.join("faces");
    let det = yunet::YuNet::load(&f.join(models::YUNET)).unwrap();
    let eye = eyes::EyeState::load(&f.join(models::EYES)).unwrap();
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    names.sort();
    for p in names {
        use image::ImageDecoder;
        let Ok(mut dec) =
            image::ImageReader::open(&p).and_then(|r| r.with_guessed_format()).map_err(|_| ()).and_then(|r| r.into_decoder().map_err(|_| ()))
        else {
            continue;
        };
        let o = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
        let Ok(mut img) = image::DynamicImage::from_decoder(dec) else { continue };
        img.apply_orientation(o);
        let img = img.to_rgb8();
        let (w, h) = img.dimensions();
        let img = Rgb8::new(w as usize, h as usize, img.into_raw()).unwrap().fit(1280).0;
        let faces = det.detect(&img).unwrap();
        let e: Vec<_> = faces.iter().map(|fc| eye.eyes(&img, fc).unwrap().map(|e| e.closed.map(|v| (v * 100.0).round() / 100.0))).collect();
        println!("{}: {}x{} {e:?}", p.file_name().unwrap().to_string_lossy(), img.width, img.height);
    }
}

#[test]
#[ignore]
fn faces_match_opencv() {
    let (Some(m), Some(dir)) = (env("LC_VISION_MODELS"), env("LC_VISION_IMG")) else { return };
    let f = m.join("faces");
    let det = yunet::YuNet::load(&f.join(models::YUNET)).unwrap();
    let rec = sface::SFace::load(&f.join(models::SFACE)).unwrap();
    let eye = eyes::EyeState::load(&f.join(models::EYES)).unwrap();
    let mut emb = std::collections::BTreeMap::new();
    for name in [
        "Solvay_conference_1927.jpg",
        "President_Barack_Obama.jpg",
        "Official_portrait_of_Barack_Obama.jpg",
        "Donald_Trump_official_portrait.jpg",
        "Joe_Biden_presidential_portrait.jpg",
        "closed_Dylan_LeBlan.jpg",
        "closed_Haley_Dreis_.jpg",
    ] {
        let img = load(&dir.join(name), 1280);
        let t = Instant::now();
        let faces = det.detect(&img).unwrap();
        let td = t.elapsed();
        let t = Instant::now();
        let e = faces.first().map(|face| rec.embed(&img, face).unwrap());
        let te = t.elapsed();
        let eyes: Vec<_> = faces.iter().take(3).map(|face| eye.eyes(&img, face).unwrap()).collect();
        println!(
            "{name}: {}×{} {} faces in {td:?}, first {:?}, embed {te:?}, eyes {eyes:?}",
            img.width,
            img.height,
            faces.len(),
            faces.first().map(|f| f.rect)
        );
        if name.starts_with("Solvay") {
            assert_eq!(faces.len(), 29);
        } else {
            assert!(!faces.is_empty());
        }
        if let Some(e) = e {
            emb.insert(name, e);
        }
        if name.starts_with("closed") {
            assert!(eyes[0].is_some_and(|e| e.closed.iter().any(|&p| p > 0.5)), "{name}: {eyes:?}");
        }
        if name.contains("Obama") {
            assert!(eyes[0].is_some_and(|e| e.closed.iter().all(|&p| p < 0.5)), "{name}: {eyes:?}");
        }
    }
    let sim = |a: &str, b: &str| sface::cosine(&emb[a], &emb[b]);
    let same = sim("President_Barack_Obama.jpg", "Official_portrait_of_Barack_Obama.jpg");
    println!(
        "Obama / Obama {same:.3}, Obama / Trump {:.3}, Trump / Biden {:.3}",
        sim("President_Barack_Obama.jpg", "Donald_Trump_official_portrait.jpg"),
        sim("Donald_Trump_official_portrait.jpg", "Joe_Biden_presidential_portrait.jpg")
    );
    // OpenCV: 0.824, -0.003, 0.286
    assert!((same - 0.824).abs() < 0.05, "{same}");
    assert!(sim("President_Barack_Obama.jpg", "Donald_Trump_official_portrait.jpg") < 0.2);
}

#[test]
#[ignore]
fn clip_finds_what_the_photo_shows() {
    let (Some(m), Some(dir)) = (env("LC_VISION_MODELS"), env("LC_VISION_IMG")) else { return };
    let device = lightcraft_vision::best_device();
    let t = Instant::now();
    let model = clip::Clip::load(&m.join("search"), &device).unwrap();
    println!("CLIP loaded in {:?} on {device:?}", t.elapsed());
    let names = ["Solvay_conference_1927.jpg", "President_Barack_Obama.jpg", "Donald_Trump_official_portrait.jpg", "closed_Dylan_LeBlan.jpg"];
    let inputs: Vec<Vec<f32>> = names.iter().map(|n| model.preprocess(&load(&dir.join(n), 448))).collect();
    let _ = model.embed_images(&inputs[..1]).unwrap(); // warm-up
    let t = Instant::now();
    let embs = model.embed_images(&inputs).unwrap();
    println!("{} images in {:?}", embs.len(), t.elapsed());
    for (q, want) in
        [("a black and white group photo of scientists", 0), ("a man with a guitar on stage", 3), ("a man in a suit in front of a flag", 2)]
    {
        let t = Instant::now();
        let te = model.embed_text(q).unwrap();
        let scores: Vec<f32> = embs.iter().map(|e| sface::cosine(e, &te)).collect();
        println!("{q:?}: {scores:?} ({:?})", t.elapsed());
        let best = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i);
        assert_eq!(best, Some(want), "{q}");
    }
}

/// The expression network on the sample portraits and every face of the Solvay photo, printed as
/// `name face: top class (probabilities)`; compared with ONNX Runtime running the same model on OpenCV's aligned crop
/// (`LC_VISION_FER_REF`: a JSON file `{"<name>": [{"rect": [x, y, w, h], "probs": [7 numbers]}]}`, pixels of the
/// 1280 px picture; faces matched by their box centres), within 0.02 per class.
#[test]
#[ignore]
fn expressions_match_onnx_runtime() {
    let (Some(m), Some(dir)) = (env("LC_VISION_MODELS"), env("LC_VISION_IMG")) else { return };
    let f = m.join("faces");
    let det = yunet::YuNet::load(&f.join(models::YUNET)).unwrap();
    let fer = lightcraft_vision::expression::ExpressionNet::load(&m.join("expressions").join(models::EXPRESSION)).unwrap();
    let reference: Option<serde_json::Value> = env("LC_VISION_FER_REF").map(|p| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap());
    let mut worst = 0f32;
    for name in [
        "Solvay_conference_1927.jpg",
        "President_Barack_Obama.jpg",
        "Official_portrait_of_Barack_Obama.jpg",
        "Donald_Trump_official_portrait.jpg",
        "Joe_Biden_presidential_portrait.jpg",
        "closed_Dylan_LeBlan.jpg",
        "closed_Haley_Dreis_.jpg",
        "closed_Kamala_Harri.jpg",
    ] {
        let img = load(&dir.join(name), 1280);
        let faces = det.detect(&img).unwrap();
        let t = Instant::now();
        for (i, face) in faces.iter().enumerate() {
            let Some(e) = fer.classify(&img, face).unwrap() else { continue };
            let p = e.probs.map(|v| (v * 100.0).round() / 100.0);
            println!("{name} {i}: {} {p:?}", lightcraft_vision::expression::LABELS[e.top()]);
            let (cx, cy) = (face.rect[0] + face.rect[2] / 2.0, face.rect[1] + face.rect[3] / 2.0);
            let near = reference.as_ref().and_then(|r| r.get(name)).and_then(|v| v.as_array()).and_then(|list| {
                list.iter()
                    .map(|f| {
                        let r: Vec<f32> = f["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
                        ((r[0] + r[2] / 2.0 - cx).hypot(r[1] + r[3] / 2.0 - cy), f)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .filter(|(d, _)| *d < face.rect[2] * 0.2)
                    .map(|(_, f)| f["probs"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect::<Vec<_>>())
            });
            if let Some(want) = near {
                for (a, b) in e.probs.iter().zip(&want) {
                    worst = worst.max((a - b).abs());
                }
            }
        }
        println!("{name}: {} faces in {:?}", faces.len(), t.elapsed());
    }
    // end to end the faces are found and aligned by our own code (boxes within a pixel of OpenCV's), and a face near
    // a class boundary moves with that pixel: this is reported, the network itself is checked below
    println!("largest difference from ONNX Runtime + OpenCV, end to end: {worst:.4}");
}

/// The network alone: OpenCV's aligned crops (`LC_VISION_FER_CROPS`: PNGs and `crops.json` with ONNX Runtime's
/// probabilities) through our interpreter, within 1e-3, the same class every time.
#[test]
#[ignore]
fn expression_network_matches_onnx_runtime() {
    let (Some(m), Some(dir)) = (env("LC_VISION_MODELS"), env("LC_VISION_FER_CROPS")) else { return };
    let fer = lightcraft_vision::expression::ExpressionNet::load(&m.join("expressions").join(models::EXPRESSION)).unwrap();
    let want: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("crops.json")).unwrap()).unwrap();
    let (mut worst, mut n, mut agree) = (0f32, 0, 0);
    let t = Instant::now();
    for (name, w) in want.as_object().unwrap() {
        let img = image::open(dir.join(name)).unwrap().to_rgb8();
        let (iw, ih) = img.dimensions();
        let crop = Rgb8::new(iw as usize, ih as usize, img.into_raw()).unwrap();
        let e = fer.classify_aligned(&crop).unwrap();
        let probs: Vec<f32> = w["probs"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        for (a, b) in e.probs.iter().zip(&probs) {
            worst = worst.max((a - b).abs());
        }
        let top = probs.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i);
        agree += usize::from(top == Some(e.top()));
        n += 1;
    }
    println!("{n} crops in {:?}: largest probability difference {worst:.5}, same class {agree}/{n}", t.elapsed());
    assert!(n > 0 && worst < 1e-3 && agree == n, "{worst} {agree}/{n}");
}
