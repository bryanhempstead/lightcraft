use super::*;
use lightcraft_develop::DevelopSettings;
use serde_json::json;

fn plain_camera() -> CameraChart {
    let shot = chart_shot(1.0, None);
    let mut src = Rgb32f::new(shot.width, shot.height);
    for (o, p) in src.data.iter_mut().zip(&shot.pixels) {
        *o = *p;
    }
    CameraChart { src, info: SourceInfo { raw: true, as_shot_temp: 5500.0, as_shot_tint: 0.0, ..Default::default() } }
}

#[test]
fn ciede2000_matches_published_test_pairs() {
    // Sharma, Wu & Dalal (2005), pairs 1, 7 and 17
    assert!((de2000([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485]) - 2.0425).abs() < 1e-3);
    assert!((de2000([50.0, 0.0, 0.0], [50.0, -1.0, 2.0]) - 2.3669).abs() < 1e-3);
    assert!((de2000([50.0, 2.5, 0.0], [73.0, 25.0, -18.0]) - 27.1492).abs() < 1e-3);
    assert_eq!(de2000([40.0, 10.0, 5.0], [40.0, 10.0, 5.0]), 0.0);
}

#[test]
fn chart_holds_the_published_patches() {
    assert_eq!(PATCHES.len(), 40);
    // neutrals are neutral in the working space, and come back as their L*
    let n5 = lab_d50_to_working([50.76, 0.0, 0.0]);
    assert!((n5[0] - n5[1]).abs() < 1e-3 && (n5[2] - n5[1]).abs() < 1e-3, "{n5:?}");
    assert!((n5[1] - 0.19).abs() < 0.01, "L* 50.8 is Y ≈ 0.19: {n5:?}");
    let back = working_to_lab(n5);
    assert!((back[0] - 50.76).abs() < 0.01 && back[1].abs() < 0.01 && back[2].abs() < 0.01, "{back:?}");
    // a chromatic patch comes back within the Bradford round trip's rounding
    let skin = working_to_lab(lab_d50_to_working(PATCHES[1].1));
    assert!(de2000(skin, PATCHES[1].1) < 2.0, "{skin:?}");
    let shot = chart_shot(0.5, None);
    assert_eq!(shot.pixels.len(), shot.width * shot.height);
    assert_eq!((shot.width, shot.height), (64, 40));
}

#[test]
fn a_render_of_the_chart_measures_every_patch() {
    let cam = plain_camera();
    let lab = cam.render(&DevelopSettings::default()).expect("renders");
    assert_eq!(lab.len(), PATCHES.len());
    // the tone curve renders the ramp in order
    let ramp: Vec<f64> = [29, 23, 30, 22, 31, 32, 33].iter().map(|&k| lab[k][0]).collect();
    assert!(ramp.windows(2).all(|w| w[0] < w[1]), "{ramp:?}");
}

#[test]
fn the_solver_recovers_a_known_twist() {
    // camera B renders like camera A with Red Hue +25 and Saturation −15 on top: matching B to A
    // must find that twist (undone) and bring the charts together
    let cam = plain_camera();
    let base = DevelopSettings::default();
    let mut twisted = base.clone();
    twisted.mixer.red.hue = 25.0;
    twisted.color.saturation = -15.0;
    let reference = cam.render(&twisted).expect("renders");
    let m = solve(&reference, &base, &cam.info, &|d| cam.render(d)).expect("solves");
    assert!(m.before.0 > 1.0, "{:?}", m.before);
    assert!(m.after.0 < 0.5 && m.after.0 < m.before.0 / 4.0, "{:?} → {:?}: {:?}", m.before, m.after, m.changed);
    assert!(m.settings.mixer.red.hue > 12.0, "{:?}", m.changed);
    assert!(m.settings.color.saturation + m.settings.color.vibrance < -5.0, "{:?}", m.changed);
    // values are ordinary whole-number slider values
    assert_eq!(m.settings.mixer.red.hue.fract(), 0.0);
}

#[test]
fn the_solver_recovers_a_white_balance_shift() {
    let cam = plain_camera();
    let base = DevelopSettings::default();
    let mut warm = base.clone();
    warm.wb.mode = WbMode::Custom;
    warm.wb.temp = 6200.0;
    warm.wb.tint = 8.0;
    let reference = cam.render(&warm).expect("renders");
    let m = solve(&reference, &base, &cam.info, &|d| cam.render(d)).expect("solves");
    assert_eq!(m.settings.wb.mode, WbMode::Custom, "{:?}", m.changed);
    assert!((m.settings.wb.temp - 6200.0).abs() < 250.0, "{:?}", m.settings.wb);
    assert!((m.settings.wb.tint - 8.0).abs() < 3.0, "{:?}", m.settings.wb);
    assert!(m.after.0 < 0.5, "{:?}", m.after);
}

#[test]
fn the_same_camera_changes_nothing() {
    let cam = plain_camera();
    let mut d = DevelopSettings::default();
    d.light.contrast = 20.0;
    d.mixer.orange.sat = -10.0;
    let reference = cam.render(&d).expect("renders");
    let m = solve(&reference, &d, &cam.info, &|x| cam.render(x)).expect("solves");
    assert!(m.changed.is_empty(), "{:?}", m.changed);
    assert_eq!(m.settings, d);
    assert!(m.before.0 < 1e-6);
}

#[test]
fn the_reference_takes_the_targets_light() {
    let mut r = DevelopSettings::default();
    r.wb.mode = WbMode::Custom;
    r.wb.temp = 4000.0;
    r.light.exposure = 2.0;
    r.mixer.red.hue = 7.0;
    let t = DevelopSettings::default();
    let x = reference_settings(&r, &t);
    assert_eq!((x.wb, x.light.exposure, x.mixer.red.hue), (t.wb, 0.0, 7.0));
}

#[test]
fn hostile_inputs_are_errors_not_panics() {
    assert!(crate::files::chart_source(b"not a raw file at all", &chart_shot(1.0, None)).is_err());
    assert!(crate::files::chart_source(&[], &chart_shot(1.0, None)).is_err());
    // a truncated TIFF header that sniffs as raw
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    tiff.extend([0xff; 64]);
    assert!(crate::files::chart_source(&tiff, &chart_shot(1.0, None)).is_err());
    for t in ["", "2024", "2024-13-45T99:99:99", "xxxx-xx-xxTxx:xx:xx", "2024-01-01T00:00:0é", "２０２４-01-01T00:00:00"] {
        let _ = capture_seconds(t);
    }
    assert_eq!(capture_seconds("2024-03-01T00:00:00").unwrap() - capture_seconds("2024-02-29T23:30:00").unwrap(), 1800);
    assert_eq!(scene_stats(&[]), SceneStats::default());
    assert_eq!(scene_stats(&[[f64::NAN, f64::NAN, f64::NAN]; 4]), SceneStats::default());
    // a chart render of an empty source or settings at the extremes still answers
    let cam = plain_camera();
    let mut d = DevelopSettings::default();
    d.wb.mode = WbMode::Custom;
    d.wb.temp = f64::NAN;
    d.light.exposure = 5.0;
    let _ = cam.render(&d);
}

#[test]
fn scene_stats_find_the_neutral_cast_and_hue_bands() {
    let mut px = vec![[60.0, 2.0, -3.0]; 50];
    px.extend(vec![[50.0, 40.0, 10.0]; 30]); // red (hue 14°), chroma 41.2
    px.extend(vec![[95.0, 0.0, 0.0]; 20]); // too bright: ignored
    let st = scene_stats(&px);
    let (a, b) = st.neutral.unwrap();
    assert!((a - 2.0).abs() < 1e-9 && (b + 3.0).abs() < 1e-9);
    let band = st.bands[0].unwrap();
    assert!((band.1 - 41.23).abs() < 0.1 && (band.0 - 0.3).abs() < 1e-9, "{band:?}");
    assert!(st.bands[1..].iter().all(Option::is_none));
}

#[test]
fn sync_and_the_command_match_colour_without_crashing() {
    use lightcraft_catalog::PhotoId;
    let mut s = Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(3).map(|p| p.id.0).collect();
    s.execute("develop.set", &json!({"control": "light.contrast", "value": 25.0, "ids": [ids[0]]})).unwrap();
    s.execute("library.select", &json!({"ids": ids})).unwrap();
    s.selection.active = Some(PhotoId(ids[0]));
    let r = s.execute("develop.sync", &json!({"matchCameraColor": true})).unwrap();
    assert_eq!(r["changed"].as_u64(), Some(2), "{r}");
    assert_eq!(r["colour"].as_array().map(Vec::len), Some(2), "{r}");
    for &id in &ids[1..] {
        assert_eq!(s.develop_of(PhotoId(id)).unwrap().light.contrast, 25.0, "the settings still sync");
    }
    // the same camera and lens without a scene: nothing to change
    let r = s.execute("develop.matchCameraColor", &json!({"scene": false})).unwrap();
    assert_eq!(r["changed"].as_u64(), Some(0), "{r}");
    assert_eq!(r["perPhoto"].as_array().map(Vec::len), Some(2));
    // forced scene pass runs on the renders (demo scenes differ, so it may or may not move)
    let r = s.execute("develop.matchCameraColor", &json!({"scene": true, "ids": [ids[1]]})).unwrap();
    assert!(r["perPhoto"][0]["scene"].is_object(), "{r}");
    // hostile parameters
    assert!(s.execute("develop.matchCameraColor", &json!({"ids": [999_999], "scene": "yes"})).is_ok());
    assert!(s.execute("develop.matchCameraColor", &json!({"ids": "nope"})).is_ok());
}
