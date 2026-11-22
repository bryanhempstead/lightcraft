//! Photo ▸ Enhance through the command layer, with no model installed (Denoise uses the built-in denoiser, Raw
//! Details needs none): a new `-Enhanced-*.dng` next to the raw (imported, stacked, the original's settings), the
//! in-place result renders instead of the mosaic, actionable errors for missing models; Lens Blur and Depth Range
//! from a depth map.

use serde_json::json;

use crate::Session;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-enhance-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A noisy Bayer DNG of a soft gradient with a few bars (written by our own DNG writer).
fn noisy_dng(w: usize, h: usize) -> Vec<u8> {
    use lightcraft_raw::*;
    let cfa = Cfa::bayer("RGGB").unwrap();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 1000) as f32 / 1000.0 - 0.5
    };
    let data: Vec<u16> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let base = 0.2 + 0.4 * x as f32 / w as f32 + if (x / 12) % 2 == 0 && y > h / 3 { 0.15 } else { 0.0 };
            let v = 256.0 + (base + 0.06 * noise()) * 15000.0;
            v.clamp(0.0, 16000.0) as u16
        })
        .collect();
    let raw = RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16(data),
        cfa: Some(cfa),
        bits: 14,
        black: BlackLevel::uniform(256.0),
        white: vec![16000.0],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::Normal,
        color: ColorData {
            illuminant: [17, 21],
            color_matrix: [
                Some(lightcraft_color::Mat3([[0.9, 0.2, -0.15], [-0.3, 1.25, 0.08], [0.02, -0.12, 0.85]])),
                Some(lightcraft_color::Mat3([[0.7, 0.3, -0.1], [-0.35, 1.3, 0.1], [0.05, -0.2, 1.0]])),
            ],
            as_shot_neutral: Some([0.5, 1.0, 0.7]),
            ..Default::default()
        },
        wb_multipliers: None,
        linearized: false,
        opcodes: OpcodeLists::default(),
        metadata: Default::default(),
    };
    write_dng(&raw, &DngWriteOptions::default()).unwrap()
}

fn session_with(dir: &std::path::Path) -> (Session, u64) {
    let p = dir.join("IMG_0001.dng");
    std::fs::write(&p, noisy_dng(192, 128)).unwrap();
    let mut s = Session::new().with_fs();
    // no models: the built-in denoiser, and actionable errors for the rest
    s.enhancer.models_root = dir.join("no-models");
    let r = s.execute("library.import", &json!({"paths": [p.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().unwrap();
    s.selection = crate::Selection::single(lightcraft_catalog::PhotoId(id));
    (s, id)
}

/// Pixel-to-pixel spread of green over a smooth patch of a render of the photo (its noise).
fn noise_of(s: &mut Session, id: u64) -> f32 {
    let job = s.render_job(lightcraft_catalog::PhotoId(id), 192, 192, false, false).unwrap();
    let img = job.run().rendered.unwrap().image;
    let (w, h) = (img.width, img.height);
    // neighbour differences: the noise, not the gradient under it
    let v: Vec<f32> = (h / 10..h / 4)
        .flat_map(|y| (w / 10..w / 3).map(move |x| (x, y)))
        .map(|(x, y)| img.data[y * w + x + 1][1] as f32 - img.data[y * w + x][1] as f32)
        .collect();
    (v.iter().map(|d| d * d).sum::<f32>() / v.len() as f32).sqrt()
}

#[test]
fn enhance_writes_a_stacked_dng_next_to_the_raw() {
    if !crate::enhance::AVAILABLE {
        return;
    }
    let dir = temp_dir("dng");
    let (mut s, id) = session_with(&dir);
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.5})).unwrap();
    let r = s.execute("photo.enhance", &json!({"rawDetails": true})).unwrap();
    let p = &r["photos"][0];
    assert_eq!(p["ok"], true, "{r}");
    let path = p["path"].as_str().unwrap();
    assert!(path.ends_with("IMG_0001-Enhanced-RD.dng"), "{path}");
    assert!(std::path::Path::new(path).is_file());
    let new = lightcraft_catalog::PhotoId(p["newId"].as_u64().unwrap());
    // the original's edit came along; the new photo is selected and stacked with the original
    assert_eq!(s.develop_of(new).unwrap().light.exposure, 0.5);
    assert_eq!(s.selection.active, Some(new));
    let st = s.catalog.stack_of(new).expect("stacked");
    assert_eq!(st.photos.first(), Some(&new), "the enhanced photo on top");
    assert!(st.photos.contains(&lightcraft_catalog::PhotoId(id)));
    // the enhanced DNG is a linear DNG the library decodes at the original's size
    let bytes = std::fs::read(path).unwrap();
    let (img, info) = crate::files::load_bytes(&bytes, 4096).unwrap();
    assert!(info.raw && img.width == 192 && img.height == 128, "{}×{}", img.width, img.height);
    // a second run of other options doesn't replace the first; the same one again adds nothing
    let r2 = s.execute("photo.enhance", &json!({"ids": [id], "denoise": 30, "stack": false})).unwrap();
    assert!(r2["photos"][0]["path"].as_str().unwrap().ends_with("IMG_0001-Enhanced-NR.dng"), "{r2}");
    let r3 = s.execute("photo.enhance", &json!({"ids": [id], "rawDetails": true})).unwrap();
    assert_eq!(r3["photos"][0]["ok"], false, "{r3}");
    assert!(!dir.join("IMG_0001-Enhanced-RD-2.dng").exists());
}

#[test]
fn in_place_denoise_renders_from_the_cache_and_can_be_removed() {
    if !crate::enhance::AVAILABLE {
        return;
    }
    let dir = temp_dir("inplace");
    let (mut s, id) = session_with(&dir);
    let before = noise_of(&mut s, id);
    let r = s.execute("photo.enhance", &json!({"denoise": 100, "inPlace": true})).unwrap();
    assert_eq!(r["photos"][0]["denoiser"], "builtin", "{r}");
    let st = s.execute("enhance.status", &json!({})).unwrap();
    assert_eq!(st["photos"][0]["ready"], true, "{st}");
    assert_eq!(s.develop_of(lightcraft_catalog::PhotoId(id)).unwrap().enhance.denoise, 100.0);
    let after = noise_of(&mut s, id);
    assert!(after < before * 0.7, "in-place denoise renders cleaner: {before} → {after}");
    // undo goes back to the mosaic
    s.execute("enhance.remove", &json!({})).unwrap();
    let removed = noise_of(&mut s, id);
    assert!((removed - before).abs() < before * 0.15, "{before} vs {removed}");
    // a missing cache file: the plain raw renders and the status says so
    s.execute("edit.undo", &json!({})).unwrap();
    let e = crate::enhance::enhanced_source(s.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap()).unwrap();
    std::fs::remove_file(&e.product).unwrap();
    s.media.forget(lightcraft_catalog::PhotoId(id));
    assert!((noise_of(&mut s, id) - before).abs() < before * 0.15);
    let st = s.execute("enhance.status", &json!({})).unwrap();
    assert_eq!(st["photos"][0]["missing"], true, "{st}");
}

#[test]
fn missing_models_say_how_to_get_them() {
    if !crate::enhance::AVAILABLE {
        return;
    }
    let dir = temp_dir("models");
    let (mut s, _) = session_with(&dir);
    let e = s.execute("photo.enhance", &json!({"superResolution": true})).unwrap_err().to_string();
    assert!(e.contains("enhance.model.download") && e.contains("super-resolution"), "{e}");
    let e = s.execute("lensBlur.apply", &json!({})).unwrap_err().to_string();
    assert!(e.contains("enhance.model.download") && e.contains("depth"), "{e}");
    let e = s.execute("enhance.model.download", &json!({"model": "depth"})).unwrap_err().to_string();
    assert!(e.contains("acknowledged") && e.contains("Apache-2.0"), "{e}");
    assert!(s.execute("photo.enhance", &json!({})).is_err());
    assert!(s.execute("photo.enhance", &json!({"denoise": "lots"})).is_err());
    let m = s.execute("enhance.models", &json!({})).unwrap();
    assert_eq!(m["models"].as_array().unwrap().len(), 6);
}

#[test]
fn lens_blur_and_depth_range_follow_the_depth_map() {
    let dir = temp_dir("lensblur");
    let (mut s, id) = session_with(&dir);
    let pid = lightcraft_catalog::PhotoId(id);
    // a depth map: near on the left, far on the right (as if estimated)
    let mut d = (*s.develop_of(pid).unwrap()).clone();
    let v: Vec<f32> = (0..32 * 16).map(|i| if i % 32 < 16 { 0.05 } else { 0.95 }).collect();
    d.depth = Some(lightcraft_develop::DepthMap::from_values(32, 16, &v));
    s.set_develop(pid, d, "test depth").unwrap();
    s.execute("lensBlur.apply", &json!({})).unwrap();
    let lb = s.develop_of(pid).unwrap().lens_blur.clone();
    assert!(lb.enabled && lb.amount == 50.0 && lb.focal_distance < 10.0, "{lb:?}");
    s.execute("lensBlur.focusAt", &json!({"x": 0.9, "y": 0.5})).unwrap();
    assert!(s.develop_of(pid).unwrap().lens_blur.focal_distance > 90.0);
    s.execute("lensBlur.bokeh", &json!({"shape": "fiveBlade"})).unwrap();
    s.execute("lensBlur.brush", &json!({"kind": "blur", "points": [[0.1, 0.5]], "size": 0.05})).unwrap();
    assert_eq!(s.develop_of(pid).unwrap().lens_blur.blur_strokes.len(), 1);
    assert!(s.execute("lensBlur.brush", &json!({"kind": "sideways", "points": [[0.1, 0.5]]})).is_err());
    s.execute("lensBlur.clearBrushes", &json!({})).unwrap();
    // the depth map stays with this photo: never auto-synced
    let h = s.execute("enhance.depthHistogram", &json!({"bins": 4})).unwrap();
    assert!(h["bins"][0].as_f64().unwrap() > 0.4 && h["bins"][3].as_f64().unwrap() > 0.4, "{h}");
    // a Depth Range mask on the near half
    s.execute("mask.add", &json!({"kind": "depthRange", "lo": 0.0, "hi": 0.3, "feather": 0.05})).unwrap();
    let m = s.develop_of(pid).unwrap().masks.last().cloned().unwrap();
    assert!(matches!(m.components[0].shape, lightcraft_develop::MaskShape::DepthRange { .. }));
    let job = s.render_job(pid, 192, 192, false, false).unwrap();
    assert!(job.run().rendered.is_ok());
}
